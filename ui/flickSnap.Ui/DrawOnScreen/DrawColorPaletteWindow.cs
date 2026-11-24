using System;
using System.Globalization;
using Avalonia;
using Avalonia.Automation;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.VisualTree;
using FlickSnap.Interop;
using FlickSnap.Localization;
using FlickSnap.UI.Controls;
using FlickSnap.UI.Controls.Tray;
using FlickSnap.UI.Dialogs.ColorPicker;
using FlickSnap.UI.Helpers;
using FlickSnap.Util;

namespace FlickSnap.UI.DrawOnScreen;

internal sealed class DrawColorPaletteWindow : Window
{
    internal const double ToolbarGap = 10;
    private const double ContentPadding = 16;
    private const double FramePadding = ContentPadding + TrayTokens.EngagedRing;
    private const double EyedropperSize = 24;
    private const double HueTrackHeight = 16;
    private const double HueTrackGap = 12;
    private const double RowSpacing = HueTrackGap - (EyedropperSize - HueTrackHeight) / 2;
    private const double EyedropperIconSize = 14;
    private const double EyedropperIconPadding = (EyedropperSize - EyedropperIconSize) / 2;
    private const double ControlColumnGap = 10;
    private const string HexCaption = "Hex";
    private const double HexRowHeight = 25;
    private const double FrameInsets = 2 * FramePadding;
    private const double ControlsHeight = FrameInsets + EyedropperSize + HexRowHeight + 2 * RowSpacing;
    private const double MarkerDiameter = 16;
    private const double InnerRingThickness = 4;
    private const double OuterRingThickness = InnerRingThickness + 1;
    private const double MarkerRadius = (MarkerDiameter - OuterRingThickness) / 2;
    private static readonly IPen OuterRing = new Pen(Brushes.Black, OuterRingThickness);
    private static readonly IPen InnerRing = new Pen(Brushes.White, InnerRingThickness);
    internal static IBrush HueRingBrush { get; } =
        WithHueSpectrum(new ConicGradientBrush { Center = RelativePoint.Center }).ToImmutable();
    private readonly SaturationValueSquare _square;
    private readonly ChannelSlider _hue;
    private readonly TextBox _hex;
    private readonly ToolButton _dropper;
    private readonly DrawEyedropper _eyedropper;
    private Color _color;
    private HsvColor _hsv;

    public event Action<Color> ColorPicked;
    internal Size VisibleFrameSize { get; }

    public DrawColorPaletteWindow(Color color, Size available, DrawEyedropper eyedropper = null)
    {
        _eyedropper = eyedropper;
        var side = Math.Clamp(Math.Min(available.Width - FrameInsets, available.Height - ControlsHeight), 1, 208);
        VisibleFrameSize = new Size(side + FrameInsets, side + ControlsHeight);
        Width = VisibleFrameSize.Width + 2 * TrayTokens.GlowReserve;
        Height = VisibleFrameSize.Height + 2 * TrayTokens.GlowReserve;
        Title = Loc.T("Draw_CustomColor");
        WindowDecorations = WindowDecorations.None;
        WindowStartupLocation = WindowStartupLocation.Manual;
        ShowInTaskbar = false;
        CanResize = false;
        Topmost = true;
        Background = Brushes.Transparent;
        TransparencyLevelHint = new[] { WindowTransparencyLevel.Transparent };
        RequestedThemeVariant = ThemeVariant.Dark;
        CaptureExclusion.SetRole(this, CaptureRole.Toolbar);

        _square = new SaturationValueSquare(side);
        AutomationProperties.SetName(_square, Loc.T("Draw_ColorSaturationValue"));
        _square.Changed += (s, v) => Choose(new HsvColor(_hsv.Hue, s, v));

        _hue = new HueSlider { ValueMax = 360, VerticalAlignment = VerticalAlignment.Center };
        Resources["ColorDialogAccentStrongBrush"] = new SolidColorBrush(AppStyles.CaptureAccentColor);
        AutomationProperties.SetName(_hue, Loc.T("Draw_ColorHue"));
        _hue.ValueChanged += (_, hue) => Choose(new HsvColor(hue, _hsv.Saturation, _hsv.Value));

        _dropper = new ToolButton
        {
            Width = EyedropperSize, Height = EyedropperSize,
            CornerRadius = TrayTokens.Corner,
            Padding = new Thickness(EyedropperIconPadding),
            IconPath = (Geometry)Application.Current.FindResource("IconDrawColorDropper"),
            IsVisible = ScreenPixels.IsSupported,
            IsEnabled = ScreenPixels.IsAvailable,
        };
        AutomationProperties.SetName(_dropper, Loc.T("Draw_ColorEyedropper"));
        ToolTip.SetShowOnDisabled(_dropper, true);
        var shortcuts = DrawHotkeys.ReadEyedropperShortcuts();
        var dropperLabel = Loc.T("Draw_ColorEyedropper");
        var dropperTip = shortcuts.Count != 0 && InteropLibrary.IsAvailable ? $"{dropperLabel} ({shortcuts})" : dropperLabel;
        ToolTip.SetTip(_dropper, _dropper.IsEnabled ? dropperTip : EyedropperButton.PermissionTip);
        _dropper.Click += (_, _) => StartPicking();
        _hex = new TextBox
        {
            Height = HexRowHeight, MinHeight = 0, Padding = new Thickness(7, 2),
            FontSize = 12, VerticalContentAlignment = VerticalAlignment.Center,
            Background = TrayTokens.SegBrush, Foreground = TrayTokens.FgBrush, BorderBrush = TrayTokens.Ring,
            // Keep pasted line breaks for validation. The palette handles Enter before the textbox.
            AcceptsReturn = true,
        };
        AutomationProperties.SetName(_hex, HexCaption);
        _hex.LostFocus += (_, _) => RestoreHex();
        var label = new TextBlock
        {
            Text = HexCaption, FontSize = 11, Foreground = TrayTokens.FgBrush, Opacity = 0.7,
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
        };
        var colorControls = new Grid
        {
            ColumnDefinitions = new ColumnDefinitions("Auto,*"),
            ColumnSpacing = ControlColumnGap,
            RowDefinitions =
            {
                new RowDefinition(new GridLength(EyedropperSize)),
                new RowDefinition(new GridLength(HexRowHeight)),
            },
            RowSpacing = RowSpacing,
            Children = { _dropper, _hue, label, _hex },
        };
        Grid.SetColumn(_hue, 1);
        Grid.SetRow(label, 1);
        Grid.SetRow(_hex, 1);
        Grid.SetColumn(_hex, 1);

        TrayTokens.EnsureAccentResources(this);
        var frame = new FloatingTray
        {
            Width = VisibleFrameSize.Width, Height = VisibleFrameSize.Height,
            Margin = new Thickness(TrayTokens.GlowReserve),
            IsEngaged = true,
        };
        frame.Classes.Add("uniform-frame");
        frame.Items.Add(new StackPanel
        {
            Margin = new Thickness(FramePadding),
            Spacing = RowSpacing,
            Children = { _square, colorControls },
        });
        Content = frame;

        ApplyRgb(color, commit: false);
        AddHandler(KeyDownEvent, OnPaletteKeyDown, RoutingStrategies.Tunnel);
        AddHandler(KeyUpEvent, OnPaletteKeyUp, RoutingStrategies.Tunnel);
        AddHandler(PointerPressedEvent, OnPalettePointerPressed, RoutingStrategies.Tunnel);
        AddHandler(PointerWheelChangedEvent, (_, e) => _eyedropper?.RouteZoomWheel(this, e), RoutingStrategies.Tunnel);
        Opened += (_, _) => WindowNativeExtensions.RaiseTopmostNoActivate(this);
        Deactivated += (_, _) => RestoreHex();
        Closed += (_, _) => _eyedropper?.Stop(this);
    }

    private void StartPicking()
    {
        ToolTip.SetIsOpen(_dropper, false);
        _eyedropper?.Start(this);
    }

    internal void ApplyEyedropperColor(Color color) => ApplyRgb(color);

    private void OnPaletteKeyDown(object sender, KeyEventArgs e)
    {
        if (_eyedropper?.RoutePanKey(e, held: true) == true)
            return;

        if (!_hex.IsFocused && DrawHotkeys.IsEyedropperGesture(e.Key, e.KeyModifiers))
        {
            StartPicking();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.Escape)
        {
            if (_eyedropper?.IsPicking == true)
                _eyedropper.Stop();
            else if (_hex.IsFocused)
                CancelHexEdit();
            else
                Close();
            e.Handled = true;
        }
        else if (e.Key == Key.Enter && _hex.IsFocused)
        {
            if (TryParseHex(_hex.Text, out var rgb))
                ApplyRgb(rgb);
            else
                RestoreHex();
            _hex.SelectAll();
            e.Handled = true;
        }
    }

    private void OnPaletteKeyUp(object sender, KeyEventArgs e)
    {
        _eyedropper?.RoutePanKey(e, held: false);
    }

    private void OnPalettePointerPressed(object sender, PointerPressedEventArgs e)
    {
        if (_hex.IsFocused && (e.Source as Visual)?.FindAncestorOfType<TextBox>(includeSelf: true) != _hex)
            CancelHexEdit();
    }

    private void CancelHexEdit()
    {
        RestoreHex();
        FocusManager?.Focus(null, NavigationMethod.Unspecified, KeyModifiers.None);
    }

    private void ApplyRgb(Color color, bool commit = true)
    {
        var rgb = Color.FromRgb(color.R, color.G, color.B);
        var converted = HsvColor.FromColor(rgb);
        if (rgb != _color)
            _hsv = new HsvColor(converted.Saturation == 0 ? _hsv.Hue : converted.Hue, converted.Saturation, converted.Value);

        if (commit)
            Commit(rgb);
        else
            _color = rgb;

        SyncControls();
    }

    private void Choose(HsvColor hsv)
    {
        _hsv = hsv;
        Commit(hsv.ToColor());
        SyncControls();
    }

    private void Commit(Color rgb)
    {
        _color = rgb;
        ColorPicked?.Invoke(rgb);
    }

    private void SyncControls()
    {
        _square.SetColor(_hsv);
        _hue.Value = _hsv.Hue;
        RestoreHex();
    }

    private void RestoreHex() => _hex.Text = FormatHex(_color);

    internal static string FormatHex(Color color) => $"#{color.R:X2}{color.G:X2}{color.B:X2}";

    internal static bool TryParseHex(string value, out Color color)
    {
        var text = value.AsSpan();
        if (!text.IsEmpty && text[0] == '#')
            text = text[1..];
        // Number parsing also accepts trailing NULs. Require six literal ASCII hex digits.
        if (text.Length == 6 && text.IndexOfAnyExcept("0123456789ABCDEFabcdef".AsSpan()) < 0 &&
            uint.TryParse(text, NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var rgb))
        {
            color = Color.FromUInt32(0xff000000 | rgb);
            return true;
        }
        color = default;
        return false;
    }

    /// <summary>Prefer the left side. Try the other edges before overlapping the toolbar.</summary>
    internal static PixelPoint Place(PixelRect toolbar, PixelPoint anchor, PixelRect work, PixelSize panel, int gap)
    {
        var width = Math.Min(panel.Width, work.Width);
        var height = Math.Min(panel.Height, work.Height);
        var x = toolbar.X - gap - width;
        var y = toolbar.Y;
        if (x < work.X)
        {
            if (toolbar.Right + gap + width <= work.Right)
                x = toolbar.Right + gap;
            else if (toolbar.Bottom + gap + height <= work.Bottom)
                (x, y) = (anchor.X, toolbar.Bottom + gap);
            else if (toolbar.Y - gap - height >= work.Y)
                (x, y) = (anchor.X, toolbar.Y - gap - height);
            else if (work.Right - toolbar.Right > toolbar.X - work.X)
                x = toolbar.Right + gap;
        }

        return new PixelPoint(Math.Clamp(x, work.X, work.Right - width), Math.Clamp(y, work.Y, work.Bottom - height));
    }

    private static LinearGradientBrush Gradient(bool vertical) => new()
    {
        StartPoint = new RelativePoint(0, 0, RelativeUnit.Relative),
        EndPoint = new RelativePoint(vertical ? 0 : 1, vertical ? 1 : 0, RelativeUnit.Relative),
    };

    private static T WithHueSpectrum<T>(T brush) where T : GradientBrush
    {
        for (var step = 0; step <= 6; step++)
            brush.GradientStops.Add(new GradientStop(new HsvColor(step * 60, 1, 1).ToColor(), step / 6.0));
        return brush;
    }

    private static void DrawColorMarker(DrawingContext context, Point center, Color color)
    {
        context.DrawEllipse(new SolidColorBrush(color), OuterRing, center, MarkerRadius, MarkerRadius);
        context.DrawEllipse(null, InnerRing, center, MarkerRadius, MarkerRadius);
    }

    private sealed class HueSlider : ChannelSlider
    {
        private readonly LinearGradientBrush _spectrum = WithHueSpectrum(new LinearGradientBrush());

        public HueSlider()
        {
            SliderBrush = _spectrum;
        }

        protected override double TrackThickness => HueTrackHeight;
        protected override double ThumbRadius => MarkerDiameter / 2;

        protected override (double Start, double Length) GetThumbTravel(double width)
        {
            var inset = Math.Min(ThumbRadius, width / 2);
            return (inset, width - 2 * inset);
        }

        protected override Size ArrangeOverride(Size finalSize)
        {
            // The color at the thumb center uses the same scale as pointer input.
            var (start, length) = GetThumbTravel(finalSize.Width);
            _spectrum.StartPoint = new RelativePoint(start, 0, RelativeUnit.Absolute);
            _spectrum.EndPoint = new RelativePoint(start + length, 0, RelativeUnit.Absolute);
            return base.ArrangeOverride(finalSize);
        }

        protected override void DrawThumb(DrawingContext context, Point center)
        {
            if (Bounds.Width >= MarkerDiameter)
                DrawColorMarker(context, center, new HsvColor(Value, 1, 1).ToColor());
        }
    }

    private sealed class SaturationValueSquare : Control
    {
        private static readonly IBrush Shade = MakeShade();
        private readonly LinearGradientBrush _saturation = Gradient(false);
        private readonly DrawingBrush _surface;
        private HsvColor _color;

        public event Action<double, double> Changed;

        public SaturationValueSquare(double side)
        {
            Width = side;
            Height = side;
            Focusable = true;
            Cursor = new Cursor(StandardCursorType.Arrow);
            _saturation.GradientStops.Add(new GradientStop(Colors.White, 0));
            _saturation.GradientStops.Add(new GradientStop(Colors.Red, 1));

            // Composite the gradients before rounding their shared edge.
            var bounds = new RectangleGeometry(new Rect(0, 0, side, side));
            _surface = new DrawingBrush
            {
                Drawing = new DrawingGroup
                {
                    Children =
                    {
                        new GeometryDrawing { Brush = _saturation, Geometry = bounds },
                        new GeometryDrawing { Brush = Shade, Geometry = bounds },
                    },
                },
            };
        }

        public void SetColor(HsvColor color)
        {
            _color = color;
            _saturation.GradientStops[1].Color = new HsvColor(color.Hue, 1, 1).ToColor();
            InvalidateVisual();
        }

        public override void Render(DrawingContext ctx)
        {
            var rect = new Rect(Bounds.Size);
            ctx.DrawRectangle(_surface, null, rect, 6, 6);
            var center = new Point(rect.Width * _color.Saturation, rect.Height * (1 - _color.Value));
            DrawColorMarker(ctx, center, _color.ToColor());
        }

        protected override void OnPointerPressed(PointerPressedEventArgs e)
        {
            base.OnPointerPressed(e);
            if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
                return;
            Focus();
            e.Pointer.Capture(this);
            Sample(e);
        }

        protected override void OnPointerMoved(PointerEventArgs e)
        {
            base.OnPointerMoved(e);
            if (e.Pointer.Captured == this)
                Sample(e);
        }

        protected override void OnPointerReleased(PointerReleasedEventArgs e)
        {
            base.OnPointerReleased(e);
            if (e.Pointer.Captured != this)
                return;
            Sample(e);
            e.Pointer.Capture(null);
        }

        protected override void OnKeyDown(KeyEventArgs e)
        {
            base.OnKeyDown(e);
            var step = e.KeyModifiers.HasFlag(KeyModifiers.Shift) ? 0.1 : 0.01;
            var (s, v) = e.Key switch
            {
                Key.Left => (_color.Saturation - step, _color.Value),
                Key.Right => (_color.Saturation + step, _color.Value),
                Key.Down => (_color.Saturation, _color.Value - step),
                Key.Up => (_color.Saturation, _color.Value + step),
                _ => (double.NaN, double.NaN),
            };
            if (!double.IsNaN(s))
            {
                Changed?.Invoke(Math.Clamp(s, 0, 1), Math.Clamp(v, 0, 1));
                e.Handled = true;
            }
        }

        private void Sample(PointerEventArgs e)
        {
            if (Bounds.Width <= 0 || Bounds.Height <= 0)
                return;
            var p = e.GetPosition(this);
            Changed?.Invoke(Math.Clamp(p.X / Bounds.Width, 0, 1), 1 - Math.Clamp(p.Y / Bounds.Height, 0, 1));
            e.Handled = true;
        }

        private static IBrush MakeShade()
        {
            var brush = Gradient(true);
            brush.GradientStops.Add(new GradientStop(Color.FromArgb(0, 0, 0, 0), 0));
            brush.GradientStops.Add(new GradientStop(Colors.Black, 1));
            return brush;
        }
    }
}
