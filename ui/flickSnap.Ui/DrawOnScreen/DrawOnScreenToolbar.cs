using System;
using System.Collections.Generic;
using System.Linq;
using Avalonia;
using Avalonia.Automation;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.VisualTree;
using FlickSnap.Localization;
using FlickSnap.UI.Controls.Tray;
using FlickSnap.UI.Helpers;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>
    /// The draw-on-screen toolbar: a two-lane strip behind a full-height grip. The primary lane is
    /// the emblem, Select, the view controls (hide and zoom) and the remaining tools at full size; the compact
    /// secondary lane is the colours, the sizes, and the undo, clear and close buttons.
    /// <para>
    /// It owns no state. Everything it shows is read off <see cref="IDrawOnScreenController"/> in
    /// <see cref="Refresh"/>, which runs on every <see cref="IDrawOnScreenController.Changed"/>, and
    /// every click is a controller command — so the session stays the one writer of the mode and the
    /// toolbar can never show a state the canvases are not in. Unlike the other strips a click gives it
    /// the keyboard (<see cref="FloatingTrayOptions.TakesFocus"/>). While the session has the keyboard,
    /// the strip is engaged — lit with the engaged frame, the primary lane's keys
    /// hinted on its tiles — which is when its hotkeys (<see cref="DrawHotkeys"/>) are live (issue #110).
    /// It is the only draw-on-screen window kept
    /// out of screen captures: the ink must stay in the picture, the toolbar must not.
    /// </para>
    /// <para>
    /// Over a frozen screen (<see cref="DrawOnScreenState.Frozen"/>, fixed for the session's life and
    /// known before the strip is built) the hide and zoom tiles are left out, and
    /// their divider is omitted.
    /// </para>
    /// </summary>
    public sealed class DrawOnScreenToolbar : FloatingTrayWindow
    {
        // the mockup's `.tb.mini svg.glyph`: the stroked close X is drawn a size up from the filled mini
        // icons and with a 2-unit stroke, so it carries the same weight as Undo/Delete beside it.
        // Kept out of TrayGlyphs.All on purpose (TrayGlyphsTests pins that set and its 1.8 stroke).
        private const double MiniStrokeGlyphSize = 16;
        private static readonly TrayGlyph MiniX = new TrayGlyph("x-mini", TrayGlyphs.X.StrokePaths.ToArray(), null, 2.0);
        // Fit the visible shapes to the same icon slots as the neighbouring drawing tools.
        // The pencil outline comes from assets/icons/common/ico-pencil.svg.
        private static readonly Lazy<Geometry> Pencil = new(() => StreamGeometry.Parse(
            "M12 8.00012L4 16.0001V20.0001H8L16 12.0001M12 8.00012L14.8686 5.13146L14.8704 5.12976C15.2652 4.73488 15.463 4.53709 15.691 4.46301C15.8919 4.39775 16.1082 4.39775 16.3091 4.46301C16.5369 4.53704 16.7345 4.7346 17.1288 5.12892L18.8686 6.86872C19.2646 7.26474 19.4627 7.46284 19.5369 7.69117C19.6022 7.89201 19.6021 8.10835 19.5369 8.3092C19.4628 8.53736 19.265 8.73516 18.8695 9.13061L18.8686 9.13146L16 12.0001M12 8.00012L16 12.0001")
            .GetWidenedGeometry(new Pen(Brushes.Black, 1.6) { LineCap = PenLineCap.Round, LineJoin = PenLineJoin.Round }));

        // assets/icons/drawing/ico-broom.svg uses the SVG nonzero fill rule.
        private static readonly Lazy<Geometry> Broom = new(() => StreamGeometry.Parse(
            "F1 M20.6247 19.7797C18.7447 18.896 17.6664 17.0018 17.6664 14.5741V13.1859C17.6678 12.6589 17.3341 12.1848 16.8256 11.9913L14.7498 11.1993C14.4098 11.0682 14.2432 10.6973 14.3765 10.3686L16.1531 6.1109C16.6615 4.93261 16.0523 3.58305 14.8114 3.13854C13.6196 2.71756 12.2966 3.28723 11.8265 4.42389C11.8262 4.42709 11.8262 4.43032 11.8265 4.43353L10.0749 8.7089C9.93923 9.03824 9.55267 9.19959 9.21072 9.0696L7.09243 8.23814C6.59284 8.04298 6.0204 8.15927 5.64496 8.53217C3.89 10.2714 3.00002 12.3038 3.00002 14.5733C2.9954 16.8957 3.92706 19.1289 5.5983 20.8016C5.72484 20.9291 5.90022 21.0009 6.08328 21H20.333C20.8462 20.9995 21.1664 20.4637 20.9094 20.0354C20.843 19.9248 20.7438 19.8358 20.6247 19.7797ZM6.59495 9.43191L8.7124 10.2593C9.74057 10.6559 10.9073 10.1715 11.3148 9.17885L13.0648 4.92115C13.2781 4.43031 13.854 4.17646 14.3506 4.35882C14.8847 4.54986 15.1456 5.13182 14.9239 5.63773L13.1406 9.89463C12.7341 10.8833 13.2349 12.0027 14.2598 12.3962L16.3331 13.1859V14.6536L5.64746 10.5333C5.93155 10.1418 6.24843 9.77346 6.59495 9.43191ZM10.5949 19.7147C9.60231 18.7968 8.89917 17.6272 8.56657 16.3406C8.43068 15.8636 7.81003 15.7072 7.44941 16.0592C7.28714 16.2176 7.22145 16.4456 7.27576 16.662C7.56653 17.7687 8.087 18.8072 8.80573 19.7147H6.36995C5.05513 18.3025 4.32957 16.4708 4.33332 14.5733C4.32517 13.5679 4.54501 12.5729 4.97747 11.658L16.4606 16.086C16.7106 17.532 17.3406 18.7755 18.2847 19.7138L10.5949 19.7147Z"));

        // the text tool's and the select pointer's glyphs, below the other hinted tiles'
        // (TrayTokens.KeyHintGlyphSize, 17)
        private const double TextGlyphSize = 14, PointerGlyphSize = 15.5;
        private const double LaserGlyphSize = 19;

        // the mockup's gap between neighbouring colour and size tiles (its `.seg { gap: 2px }`)
        private const double DotSpacing = 2;

        private readonly IDrawOnScreenController _controller;
        private readonly DrawEyedropper _eyedropper;
        private readonly DrawHotkeys _hotkeys;

        private readonly TrayEmblem _emblem;
        private readonly TrayButton _hide;
        private readonly Border _zoomIndicator;
        private readonly TextBlock _zoomPercent;
        private readonly List<(ToolType Tool, TrayButton Button)> _tools = new();

        // the primary lane's tiles, the ones whose keys are hinted, in strip order
        private readonly List<TrayButton> _hinted = new();
        private readonly List<TraySwatch> _swatches = new();
        private readonly TraySwatch _customSwatch;
        private DrawColorPaletteWindow _palette;
        private readonly List<TrayDotButton> _sizes = new();
        private readonly TrayButton _undo;
        private readonly TrayButton _clear;
        private readonly TrayButton _close;

        public DrawOnScreenToolbar(IDrawOnScreenController controller, Action<double, PixelPoint> zoomFromEyedropper = null)
            : base(new FloatingTrayOptions
            {
                Title = Loc.T("Draw_WindowTitle"),
                HasEmblem = false,
                GripLayout = TrayGripLayout.Spanning,
                Shadow = TrayTokens.ShadowCompact,
                Intro = true,
                EngagedFrame = true,
                TakesFocus = true,
            })
        {
            _controller = controller ?? throw new ArgumentNullException(nameof(controller));
            _eyedropper = new DrawEyedropper(SelectEyedropperColor, zoomFromEyedropper, _controller.SetPanKey);
            _hotkeys = new DrawHotkeys(controller);

            // ---- primary lane: emblem, select, hide, zoom and remaining tools ----
            // the emblem lives in the lane rather than on the chassis (HasEmblem is off) so it opens
            // the first row instead of standing beside both. Its margin is set per axis, see
            // ApplyAxisMargins.
            _emblem = new TrayEmblem();

            _hide = new TrayButton();
            _hide.Click += (s, e) => _controller.ToggleHide();

            _zoomPercent = new TextBlock
            {
                Width = TrayTokens.ButtonMinWidth,
                FontSize = 11,
                FontWeight = FontWeight.SemiBold,
                Foreground = TrayTokens.FgBrush,
                TextAlignment = TextAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                IsHitTestVisible = false,
            };
            _zoomIndicator = new Border
            {
                Width = TrayTokens.ButtonMinWidth,
                Height = TrayTokens.ButtonHeight,
                CornerRadius = TrayTokens.Corner,
                Background = TrayTokens.SegBrush,
                Child = new TrayGeometryIcon
                {
                    Geometry = (Geometry)Application.Current.FindResource("IconZoomEffect"),
                    Foreground = TrayTokens.FgBrush,
                    Width = TrayTokens.KeyHintGlyphSize,
                    Height = TrayTokens.KeyHintGlyphSize,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                },
            };
            _zoomIndicator.Bind(Border.BorderBrushProperty, _zoomIndicator.GetResourceObservable(TrayTokens.AccentBrushKey));

            var frozen = _controller.State.Frozen;
            var primary = new DrawLanePanel { Spacing = TrayTokens.Gap };
            primary.Children.Add(_emblem);
            AddTool(ToolType.Pointer);
            if (!frozen)
            {
                Hint(_hide, DrawHotkeys.Hide);
                primary.Children.Add(_hide);
                primary.Children.Add(_zoomIndicator);

                primary.Children.Add(new TrayDivider());
            }

            foreach (var tool in DrawPalette.Tools)
            {
                if (tool != ToolType.Pointer)
                    AddTool(tool);
            }

            void AddTool(ToolType tool)
            {
                var button = new TrayButton
                {
                    Icon = tool == ToolType.Brush
                        ? Pencil.Value
                        : (Geometry)Application.Current.FindResource(DrawPalette.ToolIconKey(tool)),
                };
                button.Click += (s, e) => _controller.PickTool(tool);
                Hint(button, DrawHotkeys.Tool(tool));

                // the T fills its whole box edge to edge, so at the shared size it looms over its
                // neighbours and over its hint
                if (tool == ToolType.Text)
                    button.GlyphSize = TextGlyphSize;
                else if (tool == ToolType.Pointer)
                {
                    button.GlyphSize = PointerGlyphSize;
                    button.Classes.Add("nudge");
                }
                else if (tool == ToolType.Laser)
                {
                    button.GlyphSize = LaserGlyphSize;
                    button.Classes.Add("nudge-down");
                }
                _tools.Add((tool, button));
                primary.Children.Add(button);
            }

            // ---- secondary lane: colours | sizes | zoom readout · keys · undo · clear · close ----
            // the mockup's 2 px between neighbouring colours and sizes; the ways out sit 4 px apart,
            // which ApplyAxisMargins tops up
            var secondary = new TrayZonePanel { Spacing = DotSpacing };

            _customSwatch = new TraySwatch { RingBrush = DrawColorPaletteWindow.HueRingBrush };
            _customSwatch.Click += (_, _) => TogglePalette();
            TrayZonePanel.SetZone(_customSwatch, TrayZone.Start);
            secondary.Children.Add(_customSwatch);

            for (var i = 0; i < DrawPalette.Colors.Count; i++)
            {
                var index = i;
                var swatch = new TraySwatch { DotColor = DrawPalette.Colors[i] };
                swatch.Click += (s, e) => _controller.SelectColor(index);
                TrayZonePanel.SetZone(swatch, TrayZone.Start);
                _swatches.Add(swatch);
                secondary.Children.Add(swatch);
            }

            for (var i = 0; i < DrawPalette.Sizes.Count; i++)
            {
                var index = i;
                var size = new TrayDotButton { DotDiameter = DrawPalette.Sizes[i].PreviewDot };
                size.Click += (s, e) => _controller.SelectSize(index);
                TrayZonePanel.SetZone(size, TrayZone.Center);
                _sizes.Add(size);
                secondary.Children.Add(size);
            }

            _undo = NewMini();
            _undo.Icon = (Geometry)Application.Current.FindResource("IconUndo");
            _undo.Click += (s, e) => _controller.Undo();

            _clear = NewMini();
            _clear.Icon = Broom.Value;
            _clear.Click += (s, e) => _controller.ClearAll();

            _close = NewMini();
            _close.Glyph = MiniX;
            _close.GlyphSize = MiniStrokeGlyphSize; // a local value beats the .mini class setter (15)
            _close.Look = TrayButtonLook.Danger;
            _close.Click += (s, e) => _controller.Close();

            if (!frozen)
            {
                TrayZonePanel.SetZone(_zoomPercent, TrayZone.End);
                secondary.Children.Add(_zoomPercent);
            }

            foreach (var way in new[] { _undo, _clear, _close })
            {
                TrayZonePanel.SetZone(way, TrayZone.End);
                secondary.Children.Add(way);
            }

            // both lanes are filled before they join the deck: the deck forwards its axis into its
            // subtree when a lane is added and on a rotation, never when a lane's own children change.
            var deck = new TrayDeck();
            deck.Children.Add(primary);
            deck.Children.Add(secondary);
            Tray.Items.Insert(0, deck);
            Tray.Classes.Add("grip-at-end");

            // the deck is a direct item, so the tray pushes a rotation into it and the deck carries it
            // on; only the two optical margins are this strip's own business.
            Tray.PropertyChanged += (s, e) =>
            {
                if (e.Property == FloatingTray.OrientationProperty)
                    ApplyAxisMargins();
            };
            ApplyAxisMargins();

            // every key is the strip's own while it is focused: a hotkey, or nothing — a key must never
            // fall through to the button the last click focused, or Space would press Clear all again
            AddHandler(KeyDownEvent, OnPreviewKeyDown, RoutingStrategies.Tunnel);
            AddHandler(KeyUpEvent, OnPreviewKeyUp, RoutingStrategies.Tunnel);
            AddHandler(PointerWheelChangedEvent, (_, e) => _eyedropper.RouteZoomWheel(this, e), RoutingStrategies.Tunnel);
            AddHandler(PointerPressedEvent, OnToolbarPointerPressed, RoutingStrategies.Tunnel, handledEventsToo: true);

            _controller.Changed += OnControllerChanged;
            Loc.CultureChanged += OnCultureChanged;
            Refresh();
        }

        /// <summary>
        /// A strip losing the keyboard forgets the keys held on it: their key-ups land elsewhere. Whether
        /// the strip is engaged is the session's to say (<see cref="IDrawOnScreenController.IsEngaged"/>).
        /// Avalonia updates <see cref="WindowBase.IsActive"/> after raising Deactivated.
        /// </summary>
        protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
        {
            base.OnPropertyChanged(change);

            if (change.Property == IsActiveProperty && !IsActive)
                _hotkeys?.Reset();
        }

        protected override void OnClosed(EventArgs e)
        {
            _eyedropper.Stop();
            ClosePalette();
            _controller.Changed -= OnControllerChanged;
            Loc.CultureChanged -= OnCultureChanged;
            base.OnClosed(e);
        }

        private void OnControllerChanged(object sender, EventArgs e) => Refresh();

        internal event Action PaletteFocusChanged;

        internal bool HasPaletteKeyboard => _palette != null && WindowNativeExtensions.IsForeground(_palette);

        internal Window KeyboardTarget => _palette ?? (Window)this;

        internal DrawEyedropper ActiveEyedropper => _eyedropper.IsPicking ? _eyedropper : null;

        internal void ClosePalette() => _palette?.Close();

        internal void SetPaletteCaptureExclusion(bool excluded)
        {
            if (_palette != null)
                WindowNativeExtensions.SetExcludedFromScreenCapture(_palette, excluded);
        }

        private void OnToolbarPointerPressed(object sender, PointerPressedEventArgs e)
        {
            if (_palette != null && (e.Source as Visual)?.FindAncestorOfType<TraySwatch>(includeSelf: true) != _customSwatch)
                ClosePalette();
        }

        private void TogglePalette()
        {
            if (_palette != null)
                ClosePalette();
            else
                OpenPalette();
        }

        internal void ActivateEyedropper() => _eyedropper.Start(_palette ?? (Window)this);

        private async void SelectEyedropperColor(Color color)
        {
            if (_palette != null)
                _palette.ApplyEyedropperColor(color);
            else
                _controller.SelectCustomColor(color);

            try
            {
                await ClipboardImpl.SetClipboardText(Clipboard, DrawColorPaletteWindow.FormatHex(color));
            }
            catch (Exception ex)
            {
                System.Diagnostics.Debug.WriteLine($"Could not copy the eyedropper color: {ex}");
            }
        }

        private void OpenPalette()
        {
            var anchor = _customSwatch.PointToScreen(default);
            var toolbar = new PixelRect(Tray.PointToScreen(default), Tray.PointToScreen(new Point(Tray.Bounds.Width, Tray.Bounds.Height)));
            var screen = DesktopScreens.FromPoint(this, anchor) ?? DesktopScreens.FromRect(this, toolbar);
            if (screen == null)
                return;

            var work = screen.WorkingArea;
            var scale = screen.Scaling;
            var palette = new DrawColorPaletteWindow(_controller.PaletteColor,
                new Size(work.Width / scale, work.Height / scale), _eyedropper);
            var size = PixelSize.FromSize(palette.VisibleFrameSize, scale);
            var outline = DrawColorPaletteWindow.Place(toolbar, anchor, work, size,
                (int)Math.Round(DrawColorPaletteWindow.ToolbarGap * scale));
            var glow = (int)Math.Round(TrayTokens.GlowReserve * scale);
            palette.Position = new PixelPoint(outline.X - glow, outline.Y - glow);

            _palette = palette;
            palette.ColorPicked += _controller.SelectCustomColor;
            palette.PropertyChanged += (_, change) =>
            {
                if (change.Property == WindowBase.IsActiveProperty)
                    PaletteFocusChanged?.Invoke();
            };
            palette.Closed += (_, _) =>
            {
                _palette = null;
                Refresh();
                PaletteFocusChanged?.Invoke();
            };
            palette.Show(this);
            Refresh();
        }

        private void OnPreviewKeyDown(object sender, KeyEventArgs e)
        {
            _hotkeys.KeyDown(e.Key, e.KeyModifiers);
            e.Handled = true;
        }

        private void OnPreviewKeyUp(object sender, KeyEventArgs e)
        {
            _hotkeys.KeyUp(e.Key);
            e.Handled = true;
        }

        /// <summary>Show keyboard hints while the session is engaged.</summary>
        private void UpdateEngaged()
        {
            var engaged = _controller.IsEngaged;
            IsEngaged = engaged;
            foreach (var button in _hinted)
                button.IsAccessKeyShown = engaged;
        }

        private void OnCultureChanged(object sender, EventArgs e)
        {
            Title = Loc.T("Draw_WindowTitle");
            Refresh();
        }

        /// <summary>
        /// Writes the controller's state onto every tile: which tool is lit,
        /// the hide glyph, the selected colour and size, the size dots' colour, whether undo and clear
        /// have anything to do, and every tooltip and accessible name (they follow the UI language and,
        /// for the hide tile, the state).
        /// </summary>
        private void Refresh()
        {
            var state = _controller.State;

            UpdateEngaged();

            // lit while hidden, as in the mockup; the tip names what a press will do
            _hide.Icon = (Geometry)Application.Current.FindResource(state.Hidden ? "IconEyeOff" : "IconEye");
            _hide.IsActive = state.Hidden;
            SetLabel(_hide, state.Hidden ? "Draw_ShowInk" : "Draw_HideInk", DrawHotkeys.Hide);

            _zoomPercent.Text = $"{state.ZoomScale * 100:F0}%";
            // Keep the readout in the layout so the rest of the strip stays put.
            _zoomPercent.Opacity = state.Zoomed ? 1 : 0;
            _zoomIndicator.BorderThickness = state.Zoomed ? new Thickness(1) : default;
            var zoomHint = Loc.T("Draw_ZoomWheel");
            ToolTip.SetTip(_zoomIndicator, zoomHint);
            AutomationProperties.SetName(_zoomIndicator, state.Zoomed ? $"{_zoomPercent.Text}. {zoomHint}" : zoomHint);


            foreach (var (tool, button) in _tools)
            {
                button.IsActive = state.IsToolLit(tool);
                SetLabel(button, DrawPalette.ToolKey(tool), DrawHotkeys.Tool(tool));
            }

            var color = state.DrawingColor;
            _customSwatch.DotColor = color;
            var paletteLabel = $"{Loc.T("Draw_CustomColor")} ({DrawColorPaletteWindow.FormatHex(color)})";
            ToolTip.SetTip(_customSwatch, paletteLabel);
            AutomationProperties.SetName(_customSwatch, paletteLabel);

            for (var i = 0; i < _swatches.Count; i++)
            {
                _swatches[i].IsSelected = !state.CustomColor.HasValue && i == state.ColorIndex;
                SetLabel(_swatches[i], DrawPalette.ColorKeys[i], DrawHotkeys.Color(i));
            }

            for (var i = 0; i < _sizes.Count; i++)
            {
                _sizes[i].DotColor = color;
                _sizes[i].IsActive = i == state.SizeIndex;
                // the ruler and the laser have no thickness to choose
                _sizes[i].IsEnabled = !state.IsToolLit(ToolType.MeasureArea) && !state.IsToolLit(ToolType.Laser);
                SetLabel(_sizes[i], DrawPalette.Sizes[i].Key, DrawHotkeys.Size(i));
            }

            _undo.IsEnabled = _controller.CanUndo;
            SetLabel(_undo, "Draw_Undo", DrawHotkeys.Undo);

            _clear.IsEnabled = _controller.CanClear;
            SetLabel(_clear, "Draw_ClearAll", DrawHotkeys.ClearAll);

            SetLabel(_close, "Draw_Close", DrawHotkeys.Close);
        }

        /// <summary>
        /// The emblem's mark is pulled 2 px toward the start of the lane and the first colour
        /// pushed 2 px forward, so the round logo and the first swatch's
        /// selection ring start on one edge; and the mini buttons are spaced at the strip gap rather
        /// than the dots'. All along the strip axis, so they turn with it.
        /// </summary>
        private void ApplyAxisMargins()
        {
            var horizontal = Tray.Orientation == Orientation.Horizontal;
            Grip.Margin = horizontal
                ? new Thickness(TrayTokens.EndGripExtraGap, 0, 0, 0)
                : new Thickness(0, TrayTokens.EndGripExtraGap, 0, 0);
            _emblem.Margin = horizontal ? new Thickness(-2, 0, 2, 0) : new Thickness(0, -2, 0, 2);
            _customSwatch.Margin = horizontal ? new Thickness(2, 0, 5, 0) : new Thickness(0, 2, 0, 5);

            // the secondary lane's spacing is the 2 px between dots; the mini buttons keep the strip's
            // ordinary 4 px gap between them, as in the mockup
            var lead = TrayTokens.Gap - DotSpacing;
            var miniMargin = horizontal ? new Thickness(lead, 0, 0, 0) : new Thickness(0, lead, 0, 0);
            _undo.Margin = miniMargin;
            _clear.Margin = miniMargin;
            _close.Margin = miniMargin;
        }

        private static TrayButton NewMini()
        {
            var button = new TrayButton();
            button.Classes.Add("mini");
            return button;
        }

        /// <summary>Gives a primary-lane tile its key's hint, rippling in strip order, and draws its glyph
        /// a size down to make room for it.</summary>
        private void Hint(TrayButton button, SimpleKeyGesture gesture)
        {
            button.GlyphSize = TrayTokens.KeyHintGlyphSize;
            button.AccessKey = DrawHotkeys.Hint(gesture);
            button.AccessKeyDelay = TrayTokens.KeyHintStagger * _hinted.Count;
            _hinted.Add(button);
        }

        /// <summary>The tile's tooltip, the localised name with its hotkey ("Undo (Ctrl+Z)", as the image
        /// editor writes them), and its accessible name, with the hotkey as its accelerator key.</summary>
        private static void SetLabel(Control control, string key, SimpleKeyGesture gesture)
        {
            var name = Loc.T(key);
            var shortcut = DrawHotkeys.Label(gesture);
            ToolTip.SetTip(control, $"{name} ({shortcut})");
            AutomationProperties.SetName(control, name);
            AutomationProperties.SetAcceleratorKey(control, shortcut);
        }

        /// <summary>The primary lane: a plain stack that takes the strip's axis from the deck.
        /// StackPanel's own Orientation already has the shape <see cref="ITrayOrientable"/> asks for.</summary>
        private sealed class DrawLanePanel : StackPanel, ITrayOrientable
        {
        }
    }
}
