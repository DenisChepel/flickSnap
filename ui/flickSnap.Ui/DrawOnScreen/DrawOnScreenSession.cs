using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Reactive;
using Avalonia.Threading;
using FlickSnap.Config;
using FlickSnap.Drawing;
using FlickSnap.UI.Helpers;
using FlickSnap.Util;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>
    /// A draw-on-screen session: one <see cref="DrawOnScreenWindow"/> per monitor, the
    /// <see cref="DrawOnScreenToolbar"/> that drives them, one <see cref="DrawHistory"/> across their
    /// separate undo stacks, and a private table of tool settings. Closing a live session keeps its
    /// drawing and controls in process memory. Single-instance via
    /// <see cref="ActiveInstance"/>, UI thread only, like the recording and share pages.
    /// <para>
    /// <see cref="Apply"/> is the one place the mode is written: every transition of
    /// <see cref="DrawOnScreenState"/> goes through it and lands on every canvas, so the toolbar
    /// (which only reads <see cref="State"/>) can never show a mode the canvases are not in.
    /// </para>
    /// <para>
    /// The hotkeys (<see cref="DrawHotkeys"/>) are live while the toolbar or a canvas has the
    /// keyboard. The toolbar stays engaged for the drawing session's lifetime.
    /// </para>
    /// <para>
    /// A frozen session (<see cref="ToggleOnScreenshot"/>, issues #111 and #114) grabs the monitor the
    /// mouse is on before anything is shown and draws over that picture instead of the live desktop,
    /// opening zoomed in on the mouse by <see cref="SettingsDrawOnScreen.ScreenshotZoom"/>; the other
    /// monitors get no window. It is drawing for
    /// its whole life (<see cref="DrawOnScreenState.Frozen"/>).
    /// </para>
    /// </summary>
    public sealed class DrawOnScreenSession : IDrawOnScreenController
    {
        // display changes arrive in bursts (one per monitor, then the work areas); settle first
        private static readonly TimeSpan HotplugSettle = TimeSpan.FromMilliseconds(500);

        // the keyboard moves between the session's own windows (into the text tool's edit and back out)
        // by deactivating one before activating the other: a loss is only believed once this has passed
        private static readonly TimeSpan FocusSettle = TimeSpan.FromMilliseconds(150);

        /// <summary>The live session, or null. UI thread only.</summary>
        public static DrawOnScreenSession ActiveInstance { get; private set; }

        private static SavedDrawing _savedDrawing;

        internal static DrawEyedropper ActiveEyedropper => ActiveInstance?._toolbar?.ActiveEyedropper;

        private readonly Dictionary<ToolType, SavedToolSettings> _settings;
        private readonly DrawHistory _history;
        private readonly List<Overlay> _overlays = new();
        private DrawOnScreenToolbar _toolbar;
        private DispatcherTimer _hotplug;

        // one-way latch: every terminal path sets it first, so a Closed raised by our own Close()
        // calls, or a late canvas event, cannot start a second teardown
        private bool _closing;

        // The canvases' hotkeys. The toolbar routes its own.
        private readonly DrawHotkeys _canvasHotkeys;
        private DispatcherTimer _focusLoss;
        private IDisposable _foregroundWatch;

        private SettingsDrawOnScreen _drawSettings;

        // over a screenshot taken as the session opened (see ToggleOnScreenshot)
        private readonly bool _frozen;

        private DrawOnScreenSession(bool frozen, SavedDrawing saved)
        {
            _frozen = frozen;
            _settings = saved?.Settings ?? new();
            _history = saved?.History ?? new();
            _canvasHotkeys = new DrawHotkeys(this);
        }

        public DrawOnScreenState State { get; private set; } = DrawOnScreenState.Initial;

        public Color PaletteColor => _overlays.Select(overlay => overlay.Canvas.SelectedColor)
            .FirstOrDefault(color => color.HasValue) ?? State.DrawingColor;

        public bool IsEngaged => !_closing;

        public bool CanUndo => _history.CanUndo;

        public bool CanClear { get; private set; }

        public event EventHandler Changed;

        /// <summary>
        /// Opens Draw on Screen or brings the current drawing session to the front.
        /// </summary>
        public static void Show() => Start(frozen: false);

        /// <summary>The global shortcut opens or closes a live drawing session.</summary>
        public static void Toggle() => Start(frozen: false, closeLiveSession: true);

        /// <summary>
        /// The Draw on Instant Screenshot action: starts a session drawn over a picture of the
        /// mouse's monitor taken right now, so whatever was on screen holds still to be drawn on. A session
        /// already up, frozen or not, is brought back to the front as by <see cref="Show"/>: its
        /// ink is never thrown away for a new one.
        /// </summary>
        public static void ToggleOnScreenshot() => Start(frozen: true);

        private static void Start(bool frozen, bool closeLiveSession = false)
        {
            Dispatcher.UIThread.VerifyAccess();

            var active = ActiveInstance;
            if (closeLiveSession && active is { _frozen: false })
            {
                active.Close();
                return;
            }

            // Only closing a live session is allowed while capture owns the overlay.
            if (ScreenCapturePage.IsCaptureOverlayOpen)
                return;

            if (active != null)
            {
                active.Raise();

                // the press found a session already up: point at its toolbar, wherever it was left, and
                // hand it the keyboard again
                active._toolbar?.ReplayIntro();
                active.TakeKeyboard();
                return;
            }

            // switched off in settings: the entry points are gone, and this backstops any caller
            // that still holds one (a strip built before the switch)
            if (SettingsRoot.Current?.DrawOnScreen is { IsEnabled: false })
                return;

            // unpermitted, macOS would freeze the bare desktop picture; Linux cannot grab at all
            if (frozen && !ScreenPixels.IsAvailable)
            {
                if (ScreenPixels.IsSupported)
                    _ = ScreenCapturePage.EnsureScreenRecordingPermissionAsync();
                return;
            }

            var saved = frozen ? null : _savedDrawing;
            new DrawOnScreenSession(frozen, saved).Open(saved);
        }

        private void Open(SavedDrawing saved)
        {
            if (ActiveInstance != null)
                return;

            ActiveInstance = this;

            _drawSettings = SettingsRoot.Current?.DrawOnScreen;
            if (_frozen)
            {
                var general = SettingsRoot.Current?.General;
                State = DrawOnScreenState.Restored(general?.DrawOnScreenColorIndex, general?.DrawOnScreenSizeIndex,
                        customColor: general?.DrawOnScreenCustomColor)
                    .Freeze(general?.DrawOnScreenshotTool) with { ShowGrid = _drawSettings?.ShowPixelGridByDefault == true };
            }
            else
            {
                State = saved?.State ?? DrawOnScreenState.Initial;
            }

            Bitmap zoomShot = null;
            try
            {
                // The toolbar exists first and is SHOWN last. DesktopScreens needs a live window on
                // macOS (Avalonia's screen list hangs off Window.Screens; on Windows the argument is
                // ignored), and nothing else is up yet. Showing it after the canvases keeps it above
                // them: among topmost peers the later Show() wins, and RaiseTopmostNoActivate re-asserts
                // it below anyway.
                _toolbar = new DrawOnScreenToolbar(this, OnEyedropperZoomWheel);
                if (saved != null)
                    _toolbar.Tray.Orientation = saved.ToolbarOrientation;
                _toolbar.Closed += OnToolbarClosed;
                _toolbar.Opened += (s, e) => KeepToolbarOnTop();
                _toolbar.PropertyChanged += OnToolbarPropertyChanged;
                _toolbar.PaletteFocusChanged += OnFocusMoved;
                _toolbar.Screens.Changed += OnScreensChanged;
                // a click on the strip can bring the keyboard without IsActive changing (the strip was
                // already its thread's active window, just not in front), so look again after one
                _toolbar.AddHandler(InputElement.PointerPressedEvent,
                    (s, e) => Dispatcher.UIThread.Post(OnFocusMoved, DispatcherPriority.Background),
                    RoutingStrategies.Tunnel, handledEventsToo: true);

                var screens = DesktopScreens.All(_toolbar);
                if (screens.Count == 0 && DesktopScreens.Primary(_toolbar) is { } primary)
                    screens = new[] { primary };

                // the strip goes to the monitor the user is looking at, which the mouse is the best
                // guess of before any pointer event has reached us
                var hasCursor = CursorPosition.TryGet(out var cursor);
                var anchor = (hasCursor ? DesktopScreens.FromPoint(_toolbar, cursor) : null)
                    ?? DesktopScreens.Primary(_toolbar)
                    ?? screens.FirstOrDefault();

                // a frozen session covers only that monitor; the others stay the live desktop
                if (_frozen)
                    screens = anchor != null ? new[] { anchor } : Array.Empty<DesktopScreen>();

                // grabbed before the first canvas shows, which would otherwise be in the picture
                var backdrops = _frozen ? GrabScreens(screens) : null;
                if (_frozen && (backdrops == null || backdrops.Length == 0))
                {
                    _ = ShutdownAsync();
                    return;
                }

                // Capture the current desktop before either the restored ink or toolbar can appear in it.
                var zoom = saved?.Zoom;
                zoomShot = zoom is { } rememberedZoom
                    && screens.Any(s => s.Bounds == rememberedZoom.Screen) && ScreenPixels.IsAvailable
                    ? ScreenPixels.Grab(rememberedZoom.Screen)
                    : null;
                if (State.Zoomed && zoomShot == null)
                    State = State.ResetZoom();

                for (var i = 0; i < screens.Count; i++)
                {
                    var screen = screens[i];
                    var target = saved?.Canvases.GetValueOrDefault(screen.Bounds);
                    var restoreZoom = zoomShot != null && zoom.Screen == screen.Bounds;
                    var overlay = AddOverlay(screen, backdrops?[i], target,
                        restoreZoom ? (zoomShot, zoom.View) : null);

                    // opened zoomed in on the mouse (the screen's middle if it could not be read)
                    if (_frozen && _drawSettings is { } settings)
                    {
                        var bounds = screens[i].Bounds;
                        overlay.Window.ZoomBackdropAt(settings.ScreenshotZoom / 100.0,
                            hasCursor ? cursor : new PixelPoint(bounds.X + bounds.Width / 2, bounds.Y + bounds.Height / 2));
                    }
                }

                var toolbarScreen = saved == null ? null : DesktopScreens.FromPoint(_toolbar, saved.ToolbarPosition);
                if (toolbarScreen != null)
                    _toolbar.ShowAtTopOf(toolbarScreen.WorkingArea, saved.ToolbarPosition);
                else if (anchor != null)
                    _toolbar.ShowAtTopOf(anchor.WorkingArea, upwardOffset: 20);
                else
                    _toolbar.Show();
                RaiseToolbar();

                // The foreground watch also sees changes that Avalonia's IsActive misses on Windows.
                _foregroundWatch = WindowNativeExtensions.WatchForeground(OnFocusMoved);

                // opened from the tray or the hotkey, the session starts focused on its toolbar (its
                // hotkeys live). The toolbar opens without activation, and the hotkey that got us here
                // was swallowed by the keyboard hook, not delivered to us: see TakeKeyboard.
                TakeKeyboard();

                if (saved != null)
                {
                    foreach (var (bounds, target) in saved.Canvases)
                        if (!screens.Any(s => s.Bounds == bounds))
                            _history.Forget(target);
                    _savedDrawing = null;
                }
                RecomputeCanClear();
                OnChanged();
            }
            catch
            {
                if (_zoomed == null)
                    zoomShot?.Dispose();

                // Release the active instance and close any windows created before opening failed.
                _ = ShutdownAsync();
                throw;
            }
        }

        // ====================================================================
        // IDrawOnScreenController
        // ====================================================================

        public void PickTool(ToolType tool)
        {
            CommitTextEdits();
            Apply(State.PickTool(tool));
        }

        public void HandleEscape()
        {
            if (_closing)
                return;

            if (_toolbar?.ActiveEyedropper is { } eyedropper)
            {
                eyedropper.Stop();
                return;
            }

            var activeCanvas = _overlays.FirstOrDefault(o => o.Canvas.IsToolDragActive)?.Canvas;
            if (activeCanvas != null)
            {
                activeCanvas.CancelCurrentOperation();
                return;
            }

            if (_overlays.Any(o => o.Canvas.SelectedCount > 0))
            {
                foreach (var overlay in _overlays)
                    overlay.Canvas.UnselectAll();
                return;
            }

            if (!State.IsToolLit(ToolType.Pointer))
                PickTool(ToolType.Pointer);
        }

        public void ToggleHide()
        {
            CommitTextEdits();
            var next = State.ToggleHide();
            if (next.Hidden && !State.Hidden)
            {
                foreach (var overlay in _overlays)
                {
                    if (overlay.Canvas.IsToolDragActive)
                        overlay.Canvas.CancelCurrentOperation();
                }
            }
            Apply(next);
        }


        public void ToggleGrid()
        {
            if (!_closing)
                Apply(State.ToggleGrid());
        }

        public void ActivateEyedropper()
        {
            if (!_closing)
                _toolbar?.ActivateEyedropper();
        }

        public void SetPanKey(bool held)
        {
            if (_closing)
                return;

            var next = State.HoldHand(held);
            if (next == State)
                return;

            if (held)
                CommitTextEdits();

            Apply(next);
        }

        public void SelectColor(int index)
        {
            if (_closing)
                return;

            CommitTextEdits();
            var next = State.SelectColor(index, hasSelection: true);
            var hasSelection = RecolorSelection(next.DrawingColor, mergeWithPrevious: false);
            Apply(hasSelection ? next : State.SelectColor(index));
        }

        public void SelectCustomColor(Color color)
        {
            if (_closing)
                return;

            CommitTextEdits();
            RecolorSelection(color, mergeWithPrevious: true);

            if (State.CustomColor != color)
                Apply(State.SelectCustomColor(color));
        }

        public void SelectSize(int index)
        {
            if (_closing)
                return;

            CommitTextEdits();
            var next = State.SelectSize(index, hasSelection: true);
            var size = DrawPalette.Sizes[next.SizeIndex];
            var hasSelection = EditSelection((canvas, _) => canvas.SetSelectedSize(tool =>
                (DrawPalette.LineWidthFor(tool, size), DrawPalette.FontSizeFor(tool, size))));
            Apply(hasSelection ? next : State.SelectSize(index));
        }

        private bool RecolorSelection(Color color, bool mergeWithPrevious) =>
            EditSelection((canvas, singleCanvas) =>
                canvas.SetSelectedColor(color, mergeWithPrevious && singleCanvas));

        private bool EditSelection(Action<DrawingCanvas, bool> edit)
        {
            var selected = _overlays.Where(overlay => overlay.Canvas.SelectedCount > 0).ToArray();
            if (selected.Length == 0)
                return false;

            _history.BeginGroup();
            try
            {
                foreach (var overlay in selected)
                    edit(overlay.Canvas, selected.Length == 1);
            }
            finally
            {
                _history.EndGroup();
            }

            return true;
        }

        public void Undo()
        {
            if (_closing)
                return;

            // committing an open edit records it first, so this undo takes the text back
            CommitTextEdits();
            _history.Undo();
            RecomputeCanClear();
            OnChanged();
        }

        public void Redo()
        {
            if (_closing)
                return;

            CommitTextEdits();
            _history.Redo();
            RecomputeCanClear();
            OnChanged();
        }

        public void ClearAll()
        {
            if (_closing)
                return;

            CommitTextEdits();

            // every canvas's clear is its own history step; the group makes them one undo
            _history.BeginGroup();
            try
            {
                foreach (var overlay in _overlays)
                {
                    if (overlay.Canvas.GraphicsList.Count > 0)
                        overlay.Canvas.DeleteAll();
                }
            }
            finally
            {
                _history.EndGroup();
            }

            RecomputeCanClear();
            Apply(State.AfterClear());
        }

        /// <summary>The select tool's Delete. Unlike clear, the mode is left alone: the select tool stays
        /// in hand for the next pick. A selection only lives while that tool is (see <see cref="PushState"/>).</summary>
        public void DeleteSelection()
        {
            if (_closing || !_overlays.Any(o => o.Canvas.SelectedCount > 0))
                return;

            // a selection on several monitors is one undo, as for clear
            _history.BeginGroup();
            try
            {
                foreach (var overlay in _overlays)
                {
                    if (overlay.Canvas.SelectedCount > 0)
                        overlay.Canvas.Delete();
                }
            }
            finally
            {
                _history.EndGroup();
            }

            RecomputeCanClear();
            OnChanged();
        }

        public void Close() => Shutdown(remember: !_frozen);

        /// <summary>Closes the drawing windows during application shutdown.</summary>
        public Task ShutdownAsync()
        {
            Shutdown(remember: false);
            return Task.CompletedTask;
        }

        private void Shutdown(bool remember)
        {
            if (_closing)
                return;

            if (remember)
            {
                CommitTextEdits();
                foreach (var overlay in _overlays)
                    if (overlay.Canvas.IsToolDragActive)
                        overlay.Canvas.CancelCurrentOperation();

                var state = State.HoldHand(false);
                var zoom = _zoomed == null || _pendingZoomAnchor.HasValue
                    ? null : new SavedZoom(_zoomed.Window.Screen.Bounds, _zoomed.Window.ZoomView);
                _savedDrawing = new SavedDrawing(state, _settings, _history,
                    _overlays.ToDictionary(o => o.Window.Screen.Bounds, o => o.Target),
                    _toolbar.Position, _toolbar.Orientation, zoom);
            }

            if (_frozen && State.CustomColor != SettingsRoot.Current?.General?.DrawOnScreenCustomColor)
                RememberColorAndSize();

            _closing = true;

            _hotplug?.Stop();
            _hotplug = null;
            _focusLoss?.Stop();
            _focusLoss = null;
            _foregroundWatch?.Dispose();
            _foregroundWatch = null;


            if (_toolbar != null)
            {
                _toolbar.Closed -= OnToolbarClosed;
                _toolbar.PropertyChanged -= OnToolbarPropertyChanged;
                _toolbar.PaletteFocusChanged -= OnFocusMoved;
                _toolbar.Screens.Changed -= OnScreensChanged;
                try { _toolbar.Close(); }
                catch { }
            }

            foreach (var overlay in _overlays.ToArray())
            {
                overlay.Detach();
                if (!_frozen)
                    overlay.Window.DetachCanvas();
                try { overlay.Window.Close(); }
                catch { }
            }
            _overlays.Clear();

            if (ReferenceEquals(ActiveInstance, this))
                ActiveInstance = null;
        }

        // ====================================================================
        // State
        // ====================================================================

        /// <summary>The single writer of the mode: stores it, lands it on every canvas window, and
        /// tells the toolbar.</summary>
        private void Apply(DrawOnScreenState next)
        {
            if (_closing)
                return;

            var prev = State;
            State = next;

            if (prev.Zoomed && !next.Zoomed)
                ZoomOut();

            foreach (var overlay in _overlays)
                PushState(overlay);

            if (prev.Hand != next.Hand)
                _toolbar?.ActiveEyedropper?.SetPanTarget(next.Hand ? _zoomed?.Window : null);

            if (prev.DrawingColor != next.DrawingColor || prev.SizeIndex != next.SizeIndex)
                ApplyColorAndSize();

            // The custom picker updates on pointer motion. Store its last colour when the session ends.
            if (_frozen && (prev.SizeIndex != next.SizeIndex || next.CustomColor == null &&
                    (prev.ColorIndex != next.ColorIndex || prev.CustomColor != null)))
            {
                RememberColorAndSize();
            }

            if (next.Frozen && prev.Tool != next.Tool && DrawOnScreenState.IsDrawingTool(next.Tool))
                RememberScreenshotTool();

            OnChanged();
        }

        // ====================================================================
        // Zoom
        // ====================================================================

        // how long the session's windows are left out of capture before the grab, so the compositor
        // has presented a frame without them (the display affinity takes effect at composition)
        private static readonly TimeSpan ZoomGrabSettle = TimeSpan.FromMilliseconds(50);

        private Overlay _zoomed;
        private Point? _pendingZoomAnchor;
        private int _zoomGeneration;

        private void OnEyedropperZoomWheel(double delta, PixelPoint screenPoint)
        {
            if (_closing)
                return;

            var overlay = _overlays.FirstOrDefault(o => o.Window.Screen.Bounds.Contains(screenPoint));
            if (overlay?.Window.ZoomFromEyedropper(delta, screenPoint) == true && State.Frozen)
                _toolbar?.ActiveEyedropper?.RefreshSample();
        }

        private void OnZoomWheel(Overlay overlay, double delta, Point anchor)
        {
            if (_closing || (_zoomed != null && _zoomed != overlay))
                return;

            var next = State.ZoomBy(delta, _drawSettings?.ShowPixelGridByDefault == true);
            if (next == State)
                return;

            if (!next.Zoomed)
            {
                CommitTextEdits();
                Apply(next);
                _toolbar?.ActiveEyedropper?.RefreshSample();
                return;
            }

            if (!State.Zoomed)
            {
                if (!ScreenPixels.IsAvailable)
                {
                    if (ScreenPixels.IsSupported)
                        _ = ScreenCapturePage.EnsureScreenRecordingPermissionAsync();
                    return;
                }

                CommitTextEdits();
                if (overlay.Canvas.IsToolDragActive)
                    overlay.Canvas.CancelCurrentOperation();

                _zoomed = overlay;
                _pendingZoomAnchor = anchor;
                Apply(next);
                _ = CaptureZoomBackdropAsync(overlay);
                return;
            }

            if (_pendingZoomAnchor.HasValue)
                _pendingZoomAnchor = anchor;
            else
            {
                overlay.Window.ZoomBackdrop(next.ZoomScale, anchor);
                _toolbar?.ActiveEyedropper?.RefreshSample();
            }
            Apply(next);
        }

        /// <summary>Captures the monitor once per entry into zoom, with the drawing windows excluded.
        /// Further wheel input can adjust the requested scale while the compositor settles.</summary>
        private async Task CaptureZoomBackdropAsync(Overlay overlay)
        {
            var generation = ++_zoomGeneration;
            var bounds = overlay.Window.Screen.Bounds;
            Bitmap shot = null;

            SetExcludedFromCapture(true);
            try
            {
                await Task.Delay(ZoomGrabSettle);
                if (generation == _zoomGeneration && !_closing)
                    shot = ScreenPixels.Grab(bounds);
            }
            finally
            {
                if (generation == _zoomGeneration && !_closing)
                    SetExcludedFromCapture(false);
            }

            if (generation != _zoomGeneration || _closing || _pendingZoomAnchor is not { } anchor)
            {
                shot?.Dispose();
                return;
            }

            _pendingZoomAnchor = null;
            if (shot == null)
            {
                Debug.WriteLine($"Draw on Screen zoom: could not grab the screen at {bounds}");
                Apply(State.ResetZoom());
                return;
            }

            overlay.Window.BeginZoom(shot, State.ZoomScale, anchor);
            PushState(overlay);
            KeepToolbarOnTop();
            _toolbar?.ActiveEyedropper?.RefreshSample();
        }

        private void ZoomOut()
        {
            ++_zoomGeneration;
            if (_pendingZoomAnchor.HasValue)
            {
                _pendingZoomAnchor = null;
                SetExcludedFromCapture(false);
            }

            var overlay = _zoomed;
            _zoomed = null;
            overlay?.Window.EndZoom();
        }

        /// <summary>Leaves the session's windows out of screen capture for the zoom's grab, and puts
        /// them back after: the canvases are content (always captured), the strip whatever the user's
        /// Hide from screen capture setting makes a toolbar.</summary>
        private void SetExcludedFromCapture(bool excluded)
        {
            foreach (var overlay in _overlays)
                WindowNativeExtensions.SetExcludedFromScreenCapture(overlay.Window, excluded);

            if (_toolbar != null)
            {
                var setting = SettingsRoot.Current?.General?.HideFromScreenCapture ?? default;
                var excludeChrome = excluded || CaptureExclusion.IsExcluded(CaptureRole.Toolbar, setting);
                WindowNativeExtensions.SetExcludedFromScreenCapture(_toolbar, excludeChrome);
                _toolbar.SetPaletteCaptureExclusion(excludeChrome);
            }
        }

        /// <summary>Stores the colour and size for the next session (see <see cref="Open"/>). Saved at
        /// the pick rather than at close, so a crash or a kill still keeps it.</summary>
        private void RememberColorAndSize()
        {
            // null before App startup assigns SettingsRoot.Current (and in harnesses that never do)
            if (SettingsRoot.Current?.General is not { } general)
                return;

            general.DrawOnScreenColorIndex = State.ColorIndex;
            general.DrawOnScreenCustomColor = State.CustomColor;
            general.DrawOnScreenSizeIndex = State.SizeIndex;
            try { SettingsService.Save(SettingsRoot.Current); }
            catch { }
        }

        /// <summary>Stores a frozen session's drawing tool for the next frozen session, at the pick like
        /// the colour. Live sessions use the default tool.</summary>
        private void RememberScreenshotTool()
        {
            if (SettingsRoot.Current?.General is not { } general)
                return;

            general.DrawOnScreenshotTool = State.Tool;
            try { SettingsService.Save(SettingsRoot.Current); }
            catch { }
        }

        /// <summary>Lands <see cref="State"/> on one window. A selection only lives while the select
        /// tool is in hand, and is dropped before the tool changes: with a selection up the canvas's
        /// colour and size are bound to the selected graphics, and the colour and size writes that
        /// follow a tool change must land in the tool's settings, not restyle the selection.</summary>
        private void PushState(Overlay overlay)
        {
            if (State.Tool != ToolType.Pointer || State.Hidden)
                overlay.Canvas.UnselectAll();
            overlay.Canvas.Tool = State.Tool;
            overlay.Canvas.Opacity = State.Hidden ? 0 : 1;
            overlay.Window.ShowGrid = State.ShowGrid;

            // the hand: the zoomed screen shows it and drags its picture; any other screen simply
            // stops drawing while it is up, so no tool is live anywhere
            if (overlay == _zoomed)
                overlay.Window.SetPanning(State.Hand);
            overlay.Canvas.IsHitTestVisible = !State.Hidden && !State.Hand;
        }

        /// <summary>
        /// Writes the selected colour and size into every tool's settings entry, then onto the
        /// canvases directly (the canvas properties are bound to the active tool's entry, and the
        /// direct write also redraws the brush cursor at the new width).
        /// </summary>
        private void ApplyColorAndSize()
        {
            var color = State.DrawingColor;
            var size = DrawPalette.Sizes[State.SizeIndex];

            foreach (var (tool, settings) in _settings)
                Style(tool, settings, color, size);

            foreach (var overlay in _overlays)
                if (overlay.Canvas.SelectedCount == 0)
                    PushStyle(overlay.Canvas, color, size);
        }

        private void PushStyle(DrawingCanvas canvas, Color color, DrawPalette.DrawSize size)
        {
            canvas.ObjectColor = color;
            canvas.LineWidth = DrawPalette.LineWidthFor(State.Tool, size);
            canvas.TextFontSize = DrawPalette.FontSizeFor(State.Tool, size);
        }

        private static void Style(ToolType tool, SavedToolSettings settings, Color color, DrawPalette.DrawSize size)
        {
            settings.ObjectColor = color;
            settings.LineWidth = DrawPalette.LineWidthFor(tool, size);
            settings.FontSize = DrawPalette.FontSizeFor(tool, size);
        }

        /// <summary>The canvases' settings lookup: a fresh default entry per tool, styled with the
        /// current colour and size, so a tool picked for the first time draws like the others.</summary>
        private SavedToolSettings ResolveSettings(ToolType tool)
        {
            if (!_settings.TryGetValue(tool, out var settings))
            {
                settings = SavedToolSettings.CreateDefault(tool);
                // text is ink on the desktop, not a card
                settings.FillColor = Colors.Transparent;
                Style(tool, settings, State.DrawingColor, DrawPalette.Sizes[State.SizeIndex]);
                _settings[tool] = settings;
            }

            return settings;
        }

        private void CommitTextEdits()
        {
            foreach (var overlay in _overlays)
                overlay.Canvas.CommitTextEdit();
        }

        /// <summary>
        /// Whether any canvas holds a graphic. Only read at history appends, undo and hotplug — never
        /// from the list's own change events: the eraser's marquee is a graphic in the list while it
        /// is dragged, and following the list would flash the Clear button through every drag. At
        /// an append the marquee is already gone, so a plain count is the right answer.
        /// </summary>
        private void RecomputeCanClear()
        {
            CanClear = _overlays.Any(o => o.Canvas.GraphicsList.Count > 0);
        }

        private void OnChanged() => Changed?.Invoke(this, EventArgs.Empty);

        /// <summary>
        /// Puts the strip back above the canvases. Showing it last is not enough on its own: a canvas
        /// finishes opening (and is restyled) after its Show() returns, which can land it above the
        /// strip, and a canvas over the strip makes Close unreachable. Posted so it runs after any
        /// such late work; cheap enough to repeat on every press on a canvas.
        /// </summary>
        private void KeepToolbarOnTop()
        {
            RaiseToolbar();
            Dispatcher.UIThread.Post(RaiseToolbar, DispatcherPriority.Background);
        }

        private void RaiseToolbar()
        {
            if (!_closing && !ScreenCapturePage.IsCaptureOverlayOpen)
                WindowNativeExtensions.RaiseTopmostNoActivate(_toolbar);
        }

        /// <summary>Raises the drawing windows, with the toolbar above the canvases.</summary>
        private void Raise()
        {
            foreach (var overlay in _overlays)
                WindowNativeExtensions.RaiseTopmostNoActivate(overlay.Window);
            RaiseToolbar();
        }

        // ====================================================================
        // Canvas windows
        // ====================================================================

        /// <summary>Every monitor's picture, in <paramref name="screens"/>' order, or null (with none
        /// left undisposed) if any of them could not be taken: a frozen session with a live monitor
        /// in it would be neither one thing nor the other.</summary>
        private static Bitmap[] GrabScreens(IReadOnlyList<DesktopScreen> screens)
        {
            var shots = new Bitmap[screens.Count];
            for (var i = 0; i < screens.Count; i++)
            {
                shots[i] = ScreenPixels.Grab(screens[i].Bounds);
                if (shots[i] == null)
                {
                    Debug.WriteLine($"Draw on Instant Screenshot: could not grab the screen at {screens[i].Bounds}");
                    foreach (var shot in shots)
                        shot?.Dispose();
                    return null;
                }
            }

            return shots;
        }

        /// <summary>Creates, wires and shows the window for <paramref name="screen"/>, already in the
        /// current mode so its first frame is right. A frozen session passes the monitor's picture,
        /// which the window owns from here on.</summary>
        private Overlay AddOverlay(DesktopScreen screen, Bitmap backdrop = null, CanvasUndoTarget target = null,
            (Bitmap Shot, (double Scale, Point Pan) View)? zoom = null)
        {
            var window = new DrawOnScreenWindow(screen, ResolveSettings, backdrop, target?.Canvas);
            var overlay = new Overlay(this, window, target);
            _overlays.Add(overlay);
            if (zoom is { } restoredZoom)
            {
                _zoomed = overlay;
                window.RestoreZoom(restoredZoom.Shot, restoredZoom.View);
            }

            PushState(overlay);
            if (target == null)
                PushStyle(overlay.Canvas, State.DrawingColor, DrawPalette.Sizes[State.SizeIndex]);
            overlay.Attach();

            window.Show();

            return overlay;
        }

        /// <summary>Takes a window out of the session (its monitor is gone, or it was closed from
        /// outside): its ink is forgotten by the history and the window closed if it still is open.</summary>
        private void RemoveOverlay(Overlay overlay)
        {
            overlay.Detach();
            _overlays.Remove(overlay);
            _history.Forget(overlay.Target);

            // the zoomed monitor is gone: there is nothing left to be zoomed
            if (overlay == _zoomed)
            {
                Apply(State.ResetZoom());
            }

            try { overlay.Window.Close(); }
            catch { }
        }

        private void OnHistoryAppended(Overlay overlay)
        {
            if (_closing)
                return;

            _history.Record(overlay.Target);
            RecomputeCanClear();
            OnChanged();
        }

        /// <summary>
        /// Backstop for a silent tool reset inside the canvas: the overlay's tools are sticky, so
        /// while the user is drawing the canvas must hold the picked tool. Pushes <see cref="State"/>
        /// back if the canvas disagrees.
        /// </summary>
        private void OnCanvasToolChanged(Overlay overlay, ToolType tool)
        {
            if (_closing || tool == State.Tool)
                return;

            overlay.Canvas.Tool = State.Tool;
        }

        /// <summary>
        /// The text tool needs the keyboard, and a WS_EX_NOACTIVATE window never gets it from a click.
        /// Activate() is an explicit request (NOACTIVATE blocks only click activation), so the window
        /// is made foreground for the edit and the keyboard handed back to the toolbar afterwards, so
        /// the hotkeys carry on — if the edit's end still finds this window focused (the keyboard may
        /// have gone elsewhere meanwhile, which is what ended the edit; <see cref="OnFocusSettled"/>
        /// releases held keys in that case).
        /// </summary>
        private void OnTextEditingChanged(Overlay overlay)
        {
            if (_closing || ScreenCapturePage.IsCaptureOverlayOpen)
                return;

            if (overlay.Canvas.IsTextEditing)
            {
                overlay.Window.Activate();
                RaiseToolbar();
            }
            else if (overlay.Window.IsActive)
            {
                _toolbar?.Activate();
            }
        }

        // ====================================================================
        // Focus
        // ====================================================================

        /// <summary>Whether the toolbar or a canvas has the keyboard right now (unsettled): the system's
        /// foreground window on Windows, not <see cref="WindowBase.IsActive"/>, which a refused
        /// SetForegroundWindow still sets; the key window of the active app on macOS (see
        /// <see cref="WindowNativeExtensions.IsForeground"/>).</summary>
        private bool HasKeyboard =>
            WindowNativeExtensions.IsForeground(_toolbar) || _toolbar?.HasPaletteKeyboard == true ||
            _overlays.Any(o => WindowNativeExtensions.IsForeground(o.Window));

        /// <summary>
        /// Puts the keyboard on the open palette or the toolbar, past the foreground lock on Windows
        /// (<see cref="WindowNativeExtensions.ForceForeground"/>) — a hotkey swallowed by the keyboard
        /// hook does not count as input this process received, so a plain Activate is refused there
        /// while Avalonia still reports the strip active — then Activate for Avalonia's own focus.
        /// </summary>
        private void TakeKeyboard()
        {
            if (_closing || ScreenCapturePage.IsCaptureOverlayOpen)
                return;

            var target = _toolbar?.KeyboardTarget;
            if (target == null)
                return;

            WindowNativeExtensions.ForceForeground(target);
            target.Activate();
        }

        /// <summary>Remember who had the keyboard before the picker took it.
        /// A file save, clipboard copy or cancellation returns to that drawing session.</summary>
        internal static Action GetCaptureFocusReturn()
        {
            var session = ActiveInstance;
            return session is { _closing: false } && session.HasKeyboard ? session.RestoreKeyboard : null;
        }

        private void RestoreKeyboard()
        {
            if (_closing || ActiveInstance != this || ScreenCapturePage.IsCaptureActive)
                return;

            KeepToolbarOnTop();
            TakeKeyboard();
        }

        private void OnToolbarPropertyChanged(object sender, AvaloniaPropertyChangedEventArgs e)
        {
            if (e.Property == WindowBase.IsActiveProperty)
                OnFocusMoved();
        }

        /// <summary>One of the session's windows gained or lost the keyboard. Not the Activated and
        /// Deactivated events: Avalonia raises them before it updates <see cref="WindowBase.IsActive"/>.</summary>
        private void OnFocusMoved()
        {
            if (_closing)
                return;

            if (HasKeyboard)
            {
                _focusLoss?.Stop();
                return;
            }

            if (_focusLoss == null)
            {
                _focusLoss = new DispatcherTimer { Interval = FocusSettle };
                _focusLoss.Tick += (s, e) =>
                {
                    _focusLoss.Stop();
                    OnFocusSettled();
                };
            }

            _focusLoss.Stop();
            _focusLoss.Start();
        }

        /// <summary>
        /// Release held keys whose key-ups may have gone elsewhere.
        /// </summary>
        private void OnFocusSettled()
        {
            if (_closing)
                return;

            // Activation can land a moment after the IsActive change that announced it.
            if (HasKeyboard)
                return;

            _canvasHotkeys.Reset();
            SetPanKey(false);
        }

        /// <summary>
        /// A press on a canvas re-asserts the strip above the canvases and, should the session be drawing
        /// without the keyboard, takes it back for the toolbar: a press
        /// is input this process received, which is what lets it take the foreground. Windows only: a
        /// macOS canvas is made the key window by its own click, which already keeps the keyboard ours.
        /// </summary>
        private void OnCanvasPressed()
        {
            _toolbar?.ClosePalette();
            KeepToolbarOnTop();

            if (OperatingSystem.IsWindows() && !HasKeyboard)
                TakeKeyboard();
        }

        private void OnCanvasHotkeyDown(KeyEventArgs e)
        {
            if (!_closing)
                _canvasHotkeys.KeyDown(e.Key, e.KeyModifiers);
        }

        private void OnCanvasHotkeyUp(KeyEventArgs e) => _canvasHotkeys.KeyUp(e.Key);

        /// <summary>A canvas window closed by something other than this session (App.CloseAllWindows
        /// at exit): the session cannot carry on with a hole in it.</summary>
        private void OnWindowClosed(Overlay overlay)
        {
            if (_closing)
                return;

            _ = ShutdownAsync();
        }

        private void OnToolbarClosed(object sender, EventArgs e)
        {
            if (_closing)
                return;

            _ = ShutdownAsync();
        }

        // ====================================================================
        // Display hotplug
        // ====================================================================

        /// <summary>Screens.Changed is a signal only (see <see cref="DesktopScreens"/> for why its
        /// list is not trusted); the monitors are re-read once the burst has settled.</summary>
        private void OnScreensChanged(object sender, EventArgs e)
        {
            if (_closing)
                return;

            if (_hotplug == null)
            {
                _hotplug = new DispatcherTimer { Interval = HotplugSettle };
                _hotplug.Tick += (s, args) =>
                {
                    _hotplug.Stop();
                    ReconcileScreens();
                };
            }

            // restart: another change inside the window pushes the reconcile out again
            _hotplug.Stop();
            _hotplug.Start();
        }

        /// <summary>
        /// Matches the windows to the monitors by bounds: a window whose monitor is still there keeps
        /// its ink (re-covering it if the scaling changed), a window whose monitor is gone is closed
        /// and forgotten by the history, and a new monitor gets an empty window (unless frozen).
        /// </summary>
        private void ReconcileScreens()
        {
            if (_closing)
                return;

            var screens = DesktopScreens.All(_toolbar);
            if (screens.Count == 0)
                return; // an enumeration failure is not "every monitor is gone"

            var matched = new HashSet<DesktopScreen>();
            foreach (var overlay in _overlays.ToArray())
            {
                var screen = screens.FirstOrDefault(s => s.Bounds == overlay.Window.Screen.Bounds);
                if (screen == null)
                {
                    RemoveOverlay(overlay);
                    continue;
                }

                matched.Add(screen);
                if (screen != overlay.Window.Screen)
                    overlay.Window.SetScreen(screen);
            }

            var added = false;
            foreach (var screen in screens)
            {
                // a frozen session only ever has a picture of the monitor it opened on: every other
                // one, present then or plugged in since, stays the live desktop
                if (matched.Add(screen) && !State.Frozen)
                {
                    AddOverlay(screen);
                    added = true;
                }
            }

            // a freshly shown canvas is the newest topmost window; the strip goes back above it
            if (added)
                RaiseToolbar();

            RecomputeCanClear();
            OnChanged();
        }

        private sealed record SavedDrawing(DrawOnScreenState State,
            Dictionary<ToolType, SavedToolSettings> Settings, DrawHistory History,
            Dictionary<PixelRect, CanvasUndoTarget> Canvases, PixelPoint ToolbarPosition,
            Orientation ToolbarOrientation, SavedZoom Zoom);

        private sealed record SavedZoom(PixelRect Screen, (double Scale, Point Pan) View);

        /// <summary>One screen's window with the session's subscriptions on it, so attaching and
        /// detaching cannot drift apart.</summary>
        private sealed class Overlay
        {
            private readonly DrawOnScreenSession _session;
            private IDisposable _toolSubscription;

            public Overlay(DrawOnScreenSession session, DrawOnScreenWindow window, CanvasUndoTarget target)
            {
                _session = session;
                Window = window;
                Target = target ?? new CanvasUndoTarget(window.Canvas);
            }

            public DrawOnScreenWindow Window { get; }

            public DrawingCanvas Canvas => Window.Canvas;

            public CanvasUndoTarget Target { get; }

            public void Attach()
            {
                Canvas.StepNumberScope = () => _session._overlays.SelectMany(overlay => overlay.Canvas.GraphicsList);
                Canvas.HistoryAppended += OnHistoryAppended;
                Canvas.TextEditingChanged += OnTextEditingChanged;
                Window.HotkeyDown += OnHotkeyDown;
                Window.HotkeyUp += OnHotkeyUp;
                if (!_session._frozen)
                    Window.ZoomWheel += OnZoomWheel;
                Window.PropertyChanged += OnWindowPropertyChanged;
                Window.Closed += OnClosed;
                Window.Opened += OnOpened;
                Window.AddHandler(InputElement.PointerPressedEvent, OnPressed, RoutingStrategies.Tunnel, handledEventsToo: true);
                _toolSubscription = Canvas.GetObservable(DrawingCanvas.ToolProperty).Subscribe(new AnonymousObserver<ToolType>(OnToolChanged));
            }

            public void Detach()
            {
                Canvas.StepNumberScope = null;
                Canvas.HistoryAppended -= OnHistoryAppended;
                Canvas.TextEditingChanged -= OnTextEditingChanged;
                Window.HotkeyDown -= OnHotkeyDown;
                Window.HotkeyUp -= OnHotkeyUp;
                Window.ZoomWheel -= OnZoomWheel;
                Window.PropertyChanged -= OnWindowPropertyChanged;
                Window.Closed -= OnClosed;
                Window.Opened -= OnOpened;
                Window.RemoveHandler(InputElement.PointerPressedEvent, OnPressed);
                _toolSubscription?.Dispose();
                _toolSubscription = null;
            }

            private void OnHistoryAppended(object sender, EventArgs e) => _session.OnHistoryAppended(this);
            private void OnTextEditingChanged(object sender, EventArgs e) => _session.OnTextEditingChanged(this);
            private void OnClosed(object sender, EventArgs e) => _session.OnWindowClosed(this);
            private void OnOpened(object sender, EventArgs e) => _session.KeepToolbarOnTop();
            private void OnPressed(object sender, PointerPressedEventArgs e) => _session.OnCanvasPressed();
            private void OnHotkeyDown(object sender, KeyEventArgs e) => _session.OnCanvasHotkeyDown(e);
            private void OnHotkeyUp(object sender, KeyEventArgs e) => _session.OnCanvasHotkeyUp(e);
            private void OnZoomWheel(double delta, Point anchor) => _session.OnZoomWheel(this, delta, anchor);

            private void OnWindowPropertyChanged(object sender, AvaloniaPropertyChangedEventArgs e)
            {
                if (e.Property == WindowBase.IsActiveProperty)
                    _session.OnFocusMoved();
            }
            private void OnToolChanged(ToolType tool) => _session.OnCanvasToolChanged(this, tool);
        }
    }
}
