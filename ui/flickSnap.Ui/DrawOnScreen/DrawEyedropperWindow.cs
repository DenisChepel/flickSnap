using System;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using FlickSnap.Drawing;
using FlickSnap.Interop;
using FlickSnap.UI.Controls.Tray;
using FlickSnap.UI.Helpers;
using FlickSnap.Util;

namespace FlickSnap.UI.DrawOnScreen;

/// <summary>Intercepts a single pick on one monitor and draws a live loupe as the cursor.</summary>
internal sealed class DrawEyedropperWindow : Window
{
    private const double LoupeSize = 120;
    private const double LabelGap = 5;
    private readonly DesktopScreen _screen;
    private readonly Preview _preview;
    private readonly Action<Color?> _finish;
    private readonly Action<double, PixelPoint> _zoom;
    private readonly Action<bool> _pan;
    private readonly Cursor _cursor;
    private readonly Cursor _grabCursor;
    private readonly Cursor _grabbingCursor;
    private DrawOnScreenWindow _panTarget;
    private IPointer _panPointer;
    private bool _pickOnRelease;
    private bool _inScreenshot;

    public DrawEyedropperWindow(DesktopScreen screen, Action<Color?> finish, Action<double, PixelPoint> zoom,
        Action<bool> pan)
    {
        _screen = screen;
        _finish = finish;
        _zoom = zoom;
        _pan = pan;
        var scale = OperatingSystem.IsMacOS() ? 1 : screen.Scaling;
        // A real transparent cursor handle keeps Windows from substituting an arrow.
        using var cursorBitmap = new RenderTargetBitmap(new PixelSize(32, 32));
        using (var context = cursorBitmap.CreateDrawingContext())
            context.FillRectangle(Brushes.Transparent, new Rect(0, 0, 32, 32));
        _cursor = new Cursor(cursorBitmap, default);
        _grabCursor = CursorResources.Grab;
        _grabbingCursor = CursorResources.Grabbing;
        _preview = new Preview(screen.Bounds) { Cursor = _cursor };

        Position = screen.Bounds.Position;
        Width = screen.Bounds.Width / scale;
        Height = screen.Bounds.Height / scale;
        WindowStartupLocation = WindowStartupLocation.Manual;
        WindowDecorations = WindowDecorations.None;
        Background = Brushes.Transparent;
        TransparencyLevelHint = new[] { WindowTransparencyLevel.Transparent };
        ShowActivated = false;
        ShowInTaskbar = false;
        CanResize = false;
        Topmost = true;
        Cursor = _cursor;
        Content = _preview;
        CaptureExclusion.SetRole(this, CaptureRole.Tooltip);
        WindowNativeExtensions.AddExStyles(this,
            WindowNativeExtensions.WS_EX_LAYERED | WindowNativeExtensions.WS_EX_TOOLWINDOW | WindowNativeExtensions.WS_EX_NOACTIVATE);

        Opened += (_, _) =>
        {
            WindowNativeExtensions.PrepareOverlay(this,
                OverlayFlags.CoverMenuBar | OverlayFlags.OpaqueLayered | OverlayFlags.TakesMouse);
            var scaling = OperatingSystem.IsMacOS() ? 1 : RenderScaling;
            WindowNativeExtensions.SetPhysicalBounds(this, screen.Bounds.Position, screen.Bounds.Width, screen.Bounds.Height, scaling);
            Dispatcher.UIThread.Post(() =>
            {
                if (!IsVisible || _inScreenshot)
                    return;
                // The loupe must never sample its own contents.
                WindowNativeExtensions.SetExcludedFromScreenCapture(this, true);
                RefreshPointer();
            }, DispatcherPriority.Background);
        };
        Closed += (_, _) =>
        {
            EndPickerDrag();
            _preview.Clear();
            _preview.Cursor = null;
            Cursor = null;
            _cursor.Dispose();
        };
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        if (_inScreenshot)
            return;

        if (_panTarget != null)
        {
            if (e.Pointer == _panPointer)
                _panTarget.MovePanDrag(PanPoint(e));
            return;
        }

        var point = e.GetPosition(this);
        _preview.Move(point, this.PointToScreen(point));
    }

    protected override void OnPointerPressed(PointerPressedEventArgs e)
    {
        if (_inScreenshot || e.GetCurrentPoint(this).Properties.PointerUpdateKind != PointerUpdateKind.LeftButtonPressed)
            return;

        if (_panTarget == null)
        {
            _pickOnRelease = true;
            return;
        }

        e.Handled = true;
        var screenPoint = this.PointToScreen(e.GetPosition(this));
        if (_panTarget.Screen.Bounds.Contains(screenPoint) &&
            _panTarget.BeginPanDrag(_panTarget.PointToClient(screenPoint)))
        {
            _panPointer = e.Pointer;
            UpdateCursor();
            e.Pointer.Capture(_preview);
        }
    }

    protected override void OnPointerExited(PointerEventArgs e)
    {
        if (!_inScreenshot)
            _preview.Clear();
    }

    protected override void OnPointerWheelChanged(PointerWheelEventArgs e)
    {
        if (!_inScreenshot)
            RouteZoomWheel(this, e, _zoom);
    }

    internal static void RouteZoomWheel(TopLevel window, PointerWheelEventArgs e, Action<double, PixelPoint> zoom)
    {
        if (zoom == null || e.KeyModifiers != KeyModifiers.Control || e.Delta.Y == 0)
            return;

        e.Handled = true;
        zoom(e.Delta.Y, window.PointToScreen(e.GetPosition(window)));
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        if (_inScreenshot)
            return;

        var button = e.InitialPressMouseButton;
        if (_panTarget != null)
        {
            if (button == MouseButton.Left && e.Pointer == _panPointer)
                EndPickerDrag();
            e.Handled = true;
            return;
        }

        if (button == MouseButton.Left)
        {
            if (!_pickOnRelease)
                return;
            _pickOnRelease = false;
            var point = this.PointToScreen(e.GetPosition(this));
            var color = ScreenPixels.GetColorAt(point);
            e.Handled = true;
            _finish(color);
        }
        else if (button == MouseButton.Right)
        {
            e.Handled = true;
            _finish(null);
        }
    }

    protected override void OnPointerCaptureLost(PointerCaptureLostEventArgs e)
    {
        if (e.Pointer == _panPointer)
            EndPickerDrag();
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        if (_inScreenshot || RoutePanKey(e, _pan, held: true))
            return;

        if (e.Key == Key.Escape)
        {
            e.Handled = true;
            _finish(null);
        }
    }

    protected override void OnKeyUp(KeyEventArgs e)
    {
        if (!_inScreenshot)
            RoutePanKey(e, _pan, held: false);
    }

    internal static bool RoutePanKey(KeyEventArgs e, Action<bool> pan, bool held)
    {
        if (e.Key != DrawHotkeys.PanKey || pan == null)
            return false;

        e.Handled = true;
        pan(held);
        return true;
    }

    private Point PanPoint(PointerEventArgs e) =>
        _panTarget.PointToClient(this.PointToScreen(e.GetPosition(this)));

    private void EndPickerDrag()
    {
        if (_panPointer != null)
            EndPickerDrag(_panTarget);
    }

    private void EndPickerDrag(DrawOnScreenWindow nextTarget)
    {
        var pointer = _panPointer;
        _panPointer = null;
        if (pointer != null)
            _panTarget?.EndDrag();
        _panTarget = nextTarget;
        UpdateCursor();
        // Set the cursor before native capture changes the pointer's target.
        if (pointer?.Captured == _preview)
            pointer.Capture(null);
    }

    internal void SetPanTarget(DrawOnScreenWindow canvas)
    {
        if (_panTarget == canvas)
            return;

        _pickOnRelease = false;
        _preview.Clear();
        EndPickerDrag(canvas);
        RefreshPointer();
    }

    private void UpdateCursor()
    {
        var cursor = _panTarget == null ? _cursor
            : _panPointer == null ? _grabCursor : _grabbingCursor;
        _preview.Cursor = cursor;
        Cursor = cursor;
    }

    internal void SetScreenshotPreview(bool enabled, bool passMouse)
    {
        if (!IsVisible || _inScreenshot == enabled)
            return;

        _inScreenshot = enabled;
        if (enabled)
        {
            EndPickerDrag();
            _pickOnRelease = false;
            _preview.SuspendSampling();
        }

        WindowNativeExtensions.SetExcludedFromScreenCapture(this, !enabled);
        // Live capture raises its own overlay over the picker. Until then, keep the
        // transparent cursor in place so the window underneath cannot show an arrow.
        if (passMouse)
            WindowNativeExtensions.PrepareOverlay(this, enabled ? OverlayFlags.ClickThrough : OverlayFlags.TakesMouse);

        if (!enabled)
        {
            _preview.Clear();
            RefreshPointer();
        }
    }

    private void RefreshPointer()
    {
        if (_panTarget == null && !_inScreenshot &&
            CursorPosition.TryGet(out var point) && _screen.Bounds.Contains(point))
            _preview.Move(this.PointToClient(point), point);
    }

    internal void RefreshSample()
    {
        if (IsVisible && !_inScreenshot)
            _preview.RefreshSample();
    }

    private sealed class Preview : Canvas
    {
        private const int SampleRadius = 7;
        private static readonly Size LabelSize = new(124, 24);
        private static readonly TimeSpan MovingSampleInterval = TimeSpan.FromMilliseconds(16);
        private static readonly TimeSpan ZoomSampleDelay = TimeSpan.FromMilliseconds(50);
        private static readonly TimeSpan IdleSampleInterval = TimeSpan.FromMilliseconds(125);
        private readonly PixelRect _screenBounds;
        private readonly DispatcherTimer _sampleTimer = new();
        private readonly TranslateTransform _lensPosition = new();
        private readonly TranslateTransform _labelPosition = new();
        private readonly Lens _lens;
        private readonly Border _label;
        private readonly Border _swatch;
        private readonly TextBlock _hex;
        private PixelPoint? _requestedPoint;
        private Sample _sample;
        private bool _sampling;
        private bool _refreshPending;
        private int _generation;

        public Preview(PixelRect screenBounds)
        {
            _screenBounds = screenBounds;
            Background = Brushes.Transparent;
            _lens = new Lens
            {
                Width = LoupeSize, Height = LoupeSize, RenderTransform = _lensPosition,
                IsVisible = false, IsHitTestVisible = false,
            };
            _swatch = new Border
            {
                Width = 16, Height = 16, CornerRadius = new CornerRadius(4),
                BorderBrush = TrayTokens.Ring, BorderThickness = new Thickness(1),
            };
            _hex = new TextBlock
            {
                FontFamily = new FontFamily("Consolas"), FontSize = 12,
                Foreground = TrayTokens.FgBrush, VerticalAlignment = VerticalAlignment.Center,
                // Compensate for the gap below the baseline in the font's line metrics.
                RenderTransform = new TranslateTransform(0, 2),
            };
            _label = new Border
            {
                Width = LabelSize.Width, Height = LabelSize.Height, RenderTransform = _labelPosition,
                IsVisible = false, IsHitTestVisible = false, CornerRadius = new CornerRadius(5),
                Background = TrayTokens.TrayBrush, BorderBrush = TrayTokens.Ring, BorderThickness = new Thickness(1),
                Child = new StackPanel
                {
                    Orientation = Orientation.Horizontal, Spacing = 6,
                    HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
                    Children = { _swatch, _hex },
                },
            };
            Children.Add(_lens);
            Children.Add(_label);

            _sampleTimer.Tick += (_, _) =>
            {
                _sampleTimer.Stop();
                _ = UpdateSampleAsync();
            };
        }

        public void Move(Point pointer, PixelPoint screenPoint)
        {
            if (!_screenBounds.Contains(screenPoint))
                return;

            // Move the existing visuals immediately, independently of the desktop read.
            _requestedPoint = screenPoint;
            _lensPosition.X = pointer.X - LoupeSize / 2;
            _lensPosition.Y = pointer.Y - LoupeSize / 2;
            var origin = PlaceLabel(pointer, Bounds.Size, LabelSize);
            _labelPosition.X = origin.X;
            _labelPosition.Y = origin.Y;
            _lens.IsVisible = true;
            QueueSample();
        }

        public void Clear()
        {
            SuspendSampling();
            _lens.IsVisible = false;
            _label.IsVisible = false;
            _lens.SetSample(null);
            _sample?.Image?.Dispose();
            _sample = null;
        }

        public void SuspendSampling()
        {
            _generation++;
            _requestedPoint = null;
            _refreshPending = false;
            _sampleTimer.Stop();
        }

        public void RefreshSample()
        {
            if (_requestedPoint == null)
                return;

            _generation++;
            _refreshPending = true;
            _sampleTimer.Stop();
            QueueSample();
        }

        private void QueueSample()
        {
            if (_sampling || _requestedPoint is not { } point)
                return;

            // Wait for the compositor after zooming. At rest, keep the preview live at a lower rate.
            var interval = _refreshPending ? ZoomSampleDelay
                : _sample?.Point != point ? MovingSampleInterval : IdleSampleInterval;
            if (_sampleTimer.IsEnabled && _sampleTimer.Interval <= interval)
                return;

            _sampleTimer.Stop();
            _sampleTimer.Interval = interval;
            _sampleTimer.Start();
        }

        private async Task UpdateSampleAsync()
        {
            if (_requestedPoint is not { } point)
                return;

            var generation = _generation;
            _refreshPending = false;
            _sampling = true;
            try
            {
                var sample = await Task.Run(() => Capture(point));
                if (generation != _generation)
                {
                    sample.Image?.Dispose();
                    return;
                }

                var previous = _sample;
                _sample = sample;
                _lens.SetSample(sample);
                previous?.Image?.Dispose();
                if (sample.Color is { } color && previous?.Color != color)
                {
                    _swatch.Background = new SolidColorBrush(color);
                    _hex.Text = DrawColorPaletteWindow.FormatHex(color);
                }
                _label.IsVisible = sample.Color.HasValue;
            }
            finally
            {
                _sampling = false;
                QueueSample();
            }
        }

        private Sample Capture(PixelPoint point)
        {
            int left = Math.Max(point.X - SampleRadius, _screenBounds.X);
            int top = Math.Max(point.Y - SampleRadius, _screenBounds.Y);
            int right = Math.Min(point.X + SampleRadius + 1, _screenBounds.Right);
            int bottom = Math.Min(point.Y + SampleRadius + 1, _screenBounds.Bottom);
            var bounds = new PixelRect(left, top, right - left, bottom - top);
            var image = ScreenPixels.Grab(bounds);
            if (image == null)
                return new Sample(point, bounds, null, ScreenPixels.GetColorAt(point));

            // Native captures can contain multiple bitmap pixels per screen point on Retina.
            using var pixels = image.Lock();
            int x = (int)((point.X - left + 0.5) * pixels.Size.Width / bounds.Width);
            int y = (int)((point.Y - top + 0.5) * pixels.Size.Height / bounds.Height);
            uint rgb = (uint)Marshal.ReadInt32(pixels.Address, y * pixels.RowBytes + x * 4) & 0xffffff;
            return new Sample(point, bounds, image, Color.FromUInt32(0xff000000u | rgb));
        }

        private sealed record Sample(PixelPoint Point, PixelRect Bounds, WriteableBitmap Image, Color? Color);

        private sealed class Lens : Control
        {
            private static readonly IPen Outline = new Pen(Brushes.White, 1);
            private static readonly IPen Grid = new Pen(Brushes.Black, 0.35);
            private static readonly IPen TargetOutline = new Pen(Brushes.Black, 3);
            private static readonly IPen Target = new Pen(Brushes.White, 1.5);
            private Sample _sample;

            public void SetSample(Sample sample)
            {
                _sample = sample;
                InvalidateVisual();
            }

            public override void Render(DrawingContext context)
            {
                var bounds = new Rect(Bounds.Size);
                const int cells = 2 * SampleRadius + 1;
                const double cell = LoupeSize / cells;

                const double radius = LoupeSize / 2;
                using (context.PushClip(new RoundedRect(bounds, radius)))
                {
                    var brush = _sample?.Color is { } color ? new SolidColorBrush(color) : TrayTokens.SegBrush;
                    context.FillRectangle(brush, bounds);
                    if (_sample is { Image: { } image, Point: var point, Bounds: var sampledBounds })
                    {
                        var destination = new Rect((sampledBounds.X - point.X + SampleRadius) * cell,
                            (sampledBounds.Y - point.Y + SampleRadius) * cell,
                            sampledBounds.Width * cell, sampledBounds.Height * cell);
                        using (context.PushRenderOptions(new RenderOptions { BitmapInterpolationMode = BitmapInterpolationMode.None }))
                            context.DrawImage(image, destination);
                    }
                    for (int i = 1; i < cells; i++)
                    {
                        var dx = bounds.X + i * cell;
                        var dy = bounds.Y + i * cell;
                        context.DrawLine(Grid, new Point(dx, bounds.Y), new Point(dx, bounds.Bottom));
                        context.DrawLine(Grid, new Point(bounds.X, dy), new Point(bounds.Right, dy));
                    }
                }

                context.DrawEllipse(null, Outline, bounds.Center, radius - 0.5, radius - 0.5);
                var center = new Rect(SampleRadius * cell, SampleRadius * cell, cell, cell);
                context.DrawRectangle(null, TargetOutline, center, 1, 1);
                context.DrawRectangle(null, Target, center, 1, 1);
            }
        }
    }

    internal static Point PlaceLabel(Point pointer, Size screen, Size label)
    {
        const double distance = LoupeSize / 2 + LabelGap;
        double x = pointer.X - label.Width / 2;
        double y = pointer.Y + distance;
        if (y + label.Height > screen.Height)
            y = pointer.Y - distance - label.Height;
        return new Point(Math.Clamp(x, 0, Math.Max(0, screen.Width - label.Width)),
            Math.Clamp(y, 0, Math.Max(0, screen.Height - label.Height)));
    }
}
