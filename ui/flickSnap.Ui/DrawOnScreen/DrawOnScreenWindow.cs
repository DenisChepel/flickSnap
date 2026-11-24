using System;
using Avalonia;
using Avalonia.Animation;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using Avalonia.VisualTree;
using FlickSnap.Config;
using FlickSnap.Drawing;
using FlickSnap.Interop;
using FlickSnap.Localization;
using FlickSnap.UI.Helpers;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>
    /// One screen's draw-on-screen canvas: a borderless, transparent, topmost window covering exactly
    /// one monitor, hosting a <see cref="DrawingCanvas"/> in overlay mode. There is one per monitor
    /// (an Avalonia window has a single RenderScaling, and macOS windows cannot span displays), and
    /// <see cref="DrawOnScreenSession"/> owns the set.
    /// <para>
    /// The overlay takes mouse input across the monitor. On Windows it keeps the keyboard with
    /// the toolbar unless the session activates it for text editing. Its ink appears in screenshots,
    /// recordings and shares.
    /// </para>
    /// <para>
    /// A frozen session (Draw on Instant Screenshot) hands its window the monitor's picture, shown
    /// under the canvas rather than added to it as a graphic: the eraser, the select tool, Clear all
    /// and the undo history only ever see the ink. The mouse wheel zooms the picture alone
    /// (<see cref="ZoomBackdropAt"/>), with nearest-neighbour sampling so its pixels stay sharp; the
    /// ink stays where it was drawn on the screen.
    /// </para>
    /// </summary>
    internal sealed class DrawOnScreenWindow : Window
    {
        // the ink's hide/show fade (the session drives Canvas.Opacity)
        private static readonly TimeSpan HideFade = TimeSpan.FromMilliseconds(150);

        // the backdrop's zoom range, and one wheel notch's step within it: the capture overlay's
        // magnifier (flicksnap_capture's ZOOM_STEP and ZOOM_MAX), so both zoom alike
        private const double MaxBackdropZoom = 256;
        private const double FrozenWheelZoomFactor = 2;

        // how far one wheel notch scrolls a zoomed screen (Shift: across, Alt: down), in window DIPs
        private const double WheelScrollStep = 120;

        private readonly Panel _root;

        // laid over the canvas while the pan key is held: it wears the grabbing hand, and the canvas,
        // which sets its own cursor on hover and would draw on a press, never sees the mouse
        private readonly Border _panShield;
        private Bitmap _backdrop;
        private PixelBackdrop _backdropImage;

        // the backdrop's zoom, and where its top-left corner sits as a fraction of the window
        // (always in [1 - zoom, 0], so the picture covers the whole screen)
        private double _zoom = 1;
        private Point _pan;

        // In zoom mode the ink follows the captured picture. Holding Space allows dragging it.
        private bool _zoomMode;
        private bool _panning;
        private bool _dragging;
        private Point _lastPointer; // window DIPs, the drag's last position

        /// <param name="backdrop">The monitor's frozen picture, owned (and disposed) by the window from
        /// here on; null to draw over the live desktop.</param>
        public DrawOnScreenWindow(DesktopScreen screen, Func<ToolType, SavedToolSettings> resolver, Bitmap backdrop = null, DrawingCanvas rememberedCanvas = null)
        {
            Screen = screen ?? throw new ArgumentNullException(nameof(screen));

            Title = Loc.T("Draw_WindowTitle");
            WindowDecorations = WindowDecorations.None;
            Background = Brushes.Transparent;
            TransparencyLevelHint = new[] { WindowTransparencyLevel.Transparent };
            Topmost = true;
            ShowInTaskbar = false;
            ShowActivated = false;
            CanResize = false;
            SizeToContent = SizeToContent.Manual;

            // the ink is what the user wants recorded, so no capture setting hides it
            CaptureExclusion.SetRole(this, CaptureRole.Content);

            // the resolver and the mode go in before the tool: picking the tool binds its settings,
            // and those must come from the session's own table, never the editor's persisted ones
            Canvas = rememberedCanvas ?? new DrawingCanvas
            {
                IsOverlayMode = true,
                ToolSettingsResolver = resolver,
                HandleColor = AppStyles.AccentColor,
                Tool = ToolType.Brush,
                Transitions = new Transitions
                {
                    new DoubleTransition { Property = OpacityProperty, Duration = HideFade },
                },
            };
            if (rememberedCanvas != null)
            {
                Canvas.ToolSettingsResolver = resolver;
                Canvas.IsHitTestVisible = true;
            }

            // the keystroke rows ride above the ink as a sibling, outside the canvas: hiding the ink
            // fades the canvas alone, and the keys are never a graphic the user could erase
            _panShield = new Border
            {
                // transparent rather than null so it is hit-tested across the whole window
                Background = Brushes.Transparent,
                // a plain arrow until the pan key is held (SetPanning)
                Cursor = CursorResources.Grab,
                IsVisible = false,
            };
            _panShield.PointerPressed += OnPanShieldPressed;
            _panShield.PointerMoved += OnPanShieldMoved;
            _panShield.PointerReleased += OnPanShieldReleased;
            _panShield.PointerCaptureLost += (s, e) => EndDrag();
            _root = new Panel
            {
                Background = Brushes.Transparent,
                Children = { Canvas, _panShield },
            };

            if (backdrop != null)
                InstallBackdrop(backdrop);

            // ahead of the canvas, which pins itself 1:1 in overlay mode anyway; a no-op while there
            // is no picture to zoom
            AddHandler(PointerWheelChangedEvent, OnPreviewPointerWheel, RoutingStrategies.Tunnel);
            Content = _root;

            // pre-show geometry from the monitor's own scaling as a best guess; Opened re-applies it
            // from the window's actual RenderScaling, the only value that is trustworthy
            SeedGeometry();

            WindowNativeExtensions.AddExStyles(this,
                WindowNativeExtensions.WS_EX_LAYERED | WindowNativeExtensions.WS_EX_TOOLWINDOW | WindowNativeExtensions.WS_EX_NOACTIVATE);

            // keyboard defence: no key reaches the canvas, whose own shortcuts are the image editor's.
            // Only the text tool's TextBox may see keys; every other key is the session's hotkeys'
            // (HotkeyDown/HotkeyUp), for while this window has the keyboard.
            AddHandler(KeyDownEvent, OnPreviewKeyDown, RoutingStrategies.Tunnel);
            AddHandler(KeyUpEvent, OnPreviewKeyUp, RoutingStrategies.Tunnel);

            Opened += OnOpened;
            ScalingChanged += OnScalingChanged;
        }

        public DrawingCanvas Canvas { get; }

        /// <summary>The keystroke rows over this screen (blank unless the session points it at its
        /// keys).</summary>

        /// <summary>The monitor this window covers (capture-space bounds, see <see cref="DesktopScreens"/>).</summary>
        public DesktopScreen Screen { get; private set; }

        /// <summary>A key edge outside the text tool's edit box, while this window has the keyboard (the
        /// text tool's edit, or on macOS any click, gives it the keyboard). Marked handled before
        /// anything else sees it.</summary>
        public event EventHandler<KeyEventArgs> HotkeyDown, HotkeyUp;

        public event Action<double, Point> ZoomWheel;

        /// <summary>The monitor changed under this window (a scaling or bounds update at hotplug):
        /// re-covers it.</summary>
        public void SetScreen(DesktopScreen screen)
        {
            Screen = screen ?? throw new ArgumentNullException(nameof(screen));
            ApplyGeometry();
            Canvas.UpdateForScalingChange();
        }

        /// <summary>
        /// Zooms the backdrop to <paramref name="zoom"/> (clamped to 1..<see cref="MaxBackdropZoom"/>),
        /// keeping the picture's pixel under <paramref name="screenPoint"/> (in
        /// <see cref="DesktopScreens"/> units, like <see cref="CursorPosition"/>) where it is. A no-op
        /// without a backdrop. Usable before the window is shown.
        /// </summary>
        public void ZoomBackdropAt(double zoom, PixelPoint screenPoint)
        {
            var bounds = Screen.Bounds;
            if (bounds.Width <= 0 || bounds.Height <= 0)
                return;

            ZoomBackdrop(zoom, new Point(
                (double)(screenPoint.X - bounds.X) / bounds.Width,
                (double)(screenPoint.Y - bounds.Y) / bounds.Height));
        }

        /// <param name="anchor">The fixed point, as a fraction of the window.</param>
        public void ZoomBackdrop(double zoom, Point anchor)
        {
            if (_backdropImage == null)
                return;

            zoom = Math.Clamp(zoom, 1, _zoomMode ? DrawOnScreenState.MaxLiveZoom : MaxBackdropZoom);
            anchor = new Point(Math.Clamp(anchor.X, 0, 1), Math.Clamp(anchor.Y, 0, 1));

            // the picture's point under the anchor (as a fraction of the picture) stays under it
            var u = (anchor - _pan) / _zoom;
            _zoom = zoom;
            _pan = ClampPan(zoom, anchor - u * zoom);
            UpdateBackdropTransform();
        }

        private void UpdateBackdropTransform()
        {
            if (_backdropImage == null)
                return;

            var size = _backdropImage.Bounds.Size;
            _backdropImage.SetView(_zoom, new Point(_pan.X * size.Width, _pan.Y * size.Height));

            // zoom mode carries the ink along: the canvas is magnified and shifted exactly as the
            // picture, so a stroke stays on what it was drawn over
            if (_zoomMode)
                Canvas.SetOverlayView(_zoom, new Point(_pan.X * size.Width, _pan.Y * size.Height));
        }

        /// <summary>The bitmap is a 96 DPI grab of the monitor's real pixels (ScreenPixels.Grab), so
        /// stretching it to fill the window maps it 1:1. The window owns it from here on.</summary>
        private void InstallBackdrop(Bitmap backdrop)
        {
            _backdrop = backdrop;
            _backdropImage = new PixelBackdrop(backdrop) { ShowGrid = _showGrid };
            _backdropImage.PropertyChanged += (s, e) =>
            {
                if (e.Property == BoundsProperty)
                    UpdateBackdropTransform();
            };
            _root.Children.Insert(0, _backdropImage);
        }

        private void RemoveBackdrop()
        {
            if (_backdropImage != null)
                _root.Children.Remove(_backdropImage);
            _backdropImage = null;
            _backdrop?.Dispose();
            _backdrop = null;
            _zoom = 1;
            _pan = default;
        }

        // Only the canvas survives a normal close. The window, its handlers and its backdrop are released.
        public void DetachCanvas()
        {
            Canvas.SetOverlayView(1, default);
            _root.Children.Remove(Canvas);
            Canvas.ToolSettingsResolver = null;
        }

        private bool _showGrid;

        /// <summary>Whether a magnified picture shows its pixel grid (from <see cref="PixelBackdrop.GridZoom"/>).</summary>
        public bool ShowGrid
        {
            get => _showGrid;
            set
            {
                _showGrid = value;
                if (_backdropImage != null)
                    _backdropImage.ShowGrid = value;
            }
        }

        /// <summary>Starts live drawing zoom around a point expressed as a fraction of this monitor.</summary>
        public void BeginZoom(Bitmap shot, double scale, Point anchor) =>
            RestoreZoom(shot, (scale, anchor * (1 - scale)));

        public (double Scale, Point Pan) ZoomView => (_zoom, _pan);

        public void RestoreZoom(Bitmap shot, (double Scale, Point Pan) view)
        {
            if (shot == null)
                return;

            if (_zoomMode)
                EndZoom();
            else if (_backdropImage != null)
                RemoveBackdrop();

            _zoomMode = true;
            _zoom = view.Scale;
            _pan = ClampPan(_zoom, view.Pan);
            InstallBackdrop(shot);
            UpdateBackdropTransform();
        }

        /// <summary>
        /// The pan key (Space) went down or up: the hand tool, as in Figma. While it is held in zoom
        /// mode the drawing tool is put down (a gesture in progress is ended, a half-drawn shape
        /// dropped), the canvas stops seeing the mouse, and an open hand shows; pressing closes it and
        /// drags the picture, and the ink with it. Letting go of the key gives the tool back. Ignored
        /// outside zoom mode.
        /// </summary>
        public void SetPanning(bool on)
        {
            on &= _zoomMode;
            if (on == _panning)
                return;

            _panning = on;

            if (on)
            {
                Canvas.CommitTextEdit();
                Canvas.CancelCurrentOperation();
            }
            else
            {
                EndDrag();
            }

            Canvas.IsHitTestVisible = !on;
            _panShield.IsVisible = on;
            if (!on)
                Canvas.RefreshToolCursor();
        }

        private void OnPanShieldPressed(object sender, PointerPressedEventArgs e)
        {
            e.Handled = true;
            if (!e.GetCurrentPoint(_panShield).Properties.IsLeftButtonPressed || !BeginPanDrag(e.GetPosition(this)))
                return;

            e.Pointer.Capture(_panShield);
            _panShield.Cursor = CursorResources.Grabbing;
        }

        private void OnPanShieldMoved(object sender, PointerEventArgs e) => MovePanDrag(e.GetPosition(this));

        internal bool BeginPanDrag(Point point)
        {
            if (!_panning)
                return false;

            _dragging = true;
            _lastPointer = point;
            return true;
        }

        internal void MovePanDrag(Point point)
        {
            if (!_dragging)
                return;

            var from = _lastPointer;
            _lastPointer = point;

            var size = Bounds.Size;
            if (size.Width <= 0 || size.Height <= 0)
                return;

            var delta = new Point((point.X - from.X) / size.Width, (point.Y - from.Y) / size.Height);
            _pan = ClampPan(_zoom, _pan + delta);
            UpdateBackdropTransform();
        }

        private void OnPanShieldReleased(object sender, PointerReleasedEventArgs e)
        {
            e.Handled = true;
            if (_dragging)
                e.Pointer.Capture(null);
            EndDrag();
        }

        internal void EndDrag()
        {
            if (!_dragging)
                return;

            _dragging = false;
            _panShield.Cursor = CursorResources.Grab;
        }

        /// <summary>Restores the drawing to the live desktop at 1:1 scale.</summary>
        public void EndZoom()
        {
            if (!_zoomMode)
                return;

            SetPanning(false);
            _zoomMode = false;
            _zoom = 1;
            _pan = default;
            RemoveBackdrop();
            Canvas.SetOverlayView(_zoom, _pan);
        }

        private static Point ClampPan(double zoom, Point pan) =>
            new Point(Math.Clamp(pan.X, 1 - zoom, 0), Math.Clamp(pan.Y, 1 - zoom, 0));

        private static Point WheelAnchor(Point point, Size size) =>
            new(Math.Clamp(point.X / size.Width, 0, 1), Math.Clamp(point.Y / size.Height, 0, 1));

        internal bool ZoomFromEyedropper(double delta, PixelPoint screenPoint)
        {
            var size = Bounds.Size;
            return size.Width > 0 && size.Height > 0 &&
                ApplyWheelZoom(delta, WheelAnchor(this.PointToClient(screenPoint), size));
        }

        private bool ApplyWheelZoom(double delta, Point anchor)
        {
            if (delta == 0)
                return false;
            if (ZoomWheel is { } scroll)
                scroll(delta, anchor);
            else if (_backdropImage != null && !_zoomMode)
                ZoomBackdrop(_zoom * Math.Pow(FrozenWheelZoomFactor, delta), anchor);
            else
                return false;

            return true;
        }

        private void OnPreviewPointerWheel(object sender, PointerWheelEventArgs e)
        {
            var modifiers = e.KeyModifiers & (KeyModifiers.Shift | KeyModifiers.Alt | KeyModifiers.Control | KeyModifiers.Meta);
            var size = Bounds.Size;
            if (size.Width <= 0 || size.Height <= 0)
                return;

            var anchor = WheelAnchor(e.GetPosition(this), size);
            if (modifiers == KeyModifiers.Control && ApplyWheelZoom(e.Delta.Y, anchor))
            {
                e.Handled = true;
                return;
            }

            if (_backdropImage == null)
                return;

            e.Handled = true;

            // Shift and Alt pan a live zoom. A horizontal wheel or touchpad swipe also pans.
            var motion = e.Delta.Y != 0 ? e.Delta.Y : e.Delta.X;
            var across = modifiers == KeyModifiers.Shift || (modifiers == KeyModifiers.None && e.Delta.Y == 0 && e.Delta.X != 0);
            var down = modifiers == KeyModifiers.Alt;
            if (_zoomMode && (across || down))
            {
                // a notch up/left shows more of the top/left: the picture moves toward the bottom/right
                var step = motion * WheelScrollStep;
                var delta = across ? new Point(step / size.Width, 0) : new Point(0, step / size.Height);
                _pan = ClampPan(_zoom, _pan + delta);
                UpdateBackdropTransform();
                return;
            }

            // The screenshot editor keeps its existing wheel gesture.
            if (!_zoomMode)
                ApplyWheelZoom(e.Delta.Y, anchor);
        }

        /// <summary>
        /// Only a programmatic Close() (the session's, or App.CloseAllWindows at exit) may close a
        /// canvas: Alt+F4 on one monitor's window would otherwise tear a hole in the session while
        /// the toolbar and the other screens carry on.
        /// </summary>
        protected override void OnClosing(WindowClosingEventArgs e)
        {
            if (!e.IsProgrammatic)
                e.Cancel = true;

            base.OnClosing(e);
        }

        protected override void OnClosed(EventArgs e)
        {
            base.OnClosed(e);
            _backdrop?.Dispose();
            _backdrop = null;
        }

        private void OnOpened(object sender, EventArgs e)
        {
            // Set the overlay level before positioning, and take input over transparent pixels.
            WindowNativeExtensions.PrepareOverlay(this, OverlayFlags.CoverMenuBar | OverlayFlags.OpaqueLayered | OverlayFlags.TakesMouse);
            ApplyGeometry();
        }

        private void OnScalingChanged(object sender, EventArgs e)
        {
            ApplyGeometry();
            Canvas.UpdateForScalingChange();

            // Avalonia.Win32 raises ScalingChanged inline from WM_DPICHANGED and only then applies
            // Windows' suggested rect (the old rect scaled by newDpi/oldDpi), overwriting the bounds
            // just set. Re-cover the monitor once that handler has returned.
            Dispatcher.UIThread.Post(() =>
            {
                if (IsVisible)
                {
                    ApplyGeometry();
                    Canvas.UpdateForScalingChange();
                }
            }, DispatcherPriority.Send);
        }

        /// <summary>Best-effort placement before the native window exists: the monitor's own
        /// scaling stands in for RenderScaling. CG points are already logical on macOS.</summary>
        private void SeedGeometry()
        {
            var scaling = OperatingSystem.IsMacOS() ? 1.0 : Screen.Scaling;
            Position = Screen.Bounds.Position;
            Width = Screen.Bounds.Width / scaling;
            Height = Screen.Bounds.Height / scaling;
        }

        /// <summary>Covers <see cref="Screen"/> exactly, in one native call (see
        /// <see cref="WindowNativeExtensions.SetPhysicalBounds"/>).</summary>
        private void ApplyGeometry()
        {
            var scaling = OperatingSystem.IsMacOS() ? 1.0 : RenderScaling;
            WindowNativeExtensions.SetPhysicalBounds(this, Screen.Bounds.Position, Screen.Bounds.Width, Screen.Bounds.Height, scaling);
        }

        private void OnPreviewKeyDown(object sender, KeyEventArgs e)
        {
            if (IsForTextBox(e))
                return;

            e.Handled = true;
            HotkeyDown?.Invoke(this, e);
        }

        private void OnPreviewKeyUp(object sender, KeyEventArgs e)
        {
            if (IsForTextBox(e))
                return;

            e.Handled = true;
            HotkeyUp?.Invoke(this, e);
        }

        // the text tool's in-place TextBox is the one place keys are meant to land
        private static bool IsForTextBox(KeyEventArgs e) =>
            e.Source is Visual source && source.FindAncestorOfType<TextBox>(includeSelf: true) != null;
    }
}
