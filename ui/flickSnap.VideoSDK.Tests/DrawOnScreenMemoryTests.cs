using System.Collections.Generic;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Headless.XUnit;
using Avalonia.Input;
using Avalonia.Markup.Xaml.Styling;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using FlickSnap.Config;
using FlickSnap.Drawing.Graphics;
using FlickSnap.Interop.Testing;
using FlickSnap.UI;
using FlickSnap.UI.DrawOnScreen;
using FlickSnap.UI.Helpers;
using Xunit;

namespace FlickSnap.VideoSDK.Tests
{
    [Collection(HeadlessApp.Collection)]
    public class DrawOnScreenMemoryTests
    {
        [AvaloniaFact]
        public void Global_shortcut_toggles_drawing_and_reopens_with_the_saved_color()
        {
            InteropTestGate.Require();
            var previousSettings = SettingsRoot.Current;
            SettingsRoot.Current = new SettingsRoot();

            try
            {
                LoadDrawingToolbarResources();
                DrawOnScreenSession.Toggle();
                var session = Assert.IsType<DrawOnScreenSession>(DrawOnScreenSession.ActiveInstance);
                session.SelectCustomColor(Colors.CornflowerBlue);

                DrawOnScreenSession.Show();
                Assert.Same(session, DrawOnScreenSession.ActiveInstance);

                DrawOnScreenSession.Toggle();
                Assert.Null(DrawOnScreenSession.ActiveInstance);

                DrawOnScreenSession.Toggle();
                var reopened = Assert.IsType<DrawOnScreenSession>(DrawOnScreenSession.ActiveInstance);
                Assert.NotSame(session, reopened);
                Assert.Equal(Colors.CornflowerBlue, reopened.State.DrawingColor);
            }
            finally
            {
                DrawOnScreenSession.ActiveInstance?.ShutdownAsync();
                SettingsRoot.Current = previousSettings;
            }
        }

        [AvaloniaTheory]
        [InlineData(false)]
        [InlineData(true)]
        public async Task Capture_preflight_blocks_opening_drawing_but_allows_closing_an_existing_session(bool drawFirst)
        {
            InteropTestGate.Require();
            var previousSettings = SettingsRoot.Current;
            SettingsRoot.Current = new SettingsRoot();
            var permission = new TaskCompletionSource<bool>();
            Task capture = null;
            try
            {
                LoadDrawingToolbarResources();
                if (drawFirst)
                    DrawOnScreenSession.Toggle();
                var drawing = DrawOnScreenSession.ActiveInstance;
                Assert.Equal(drawFirst, drawing != null);

                var page = new ScreenCapturePage();
                capture = page.OpenAsync(CaptureMode.Region, RegionIntent.Capture, () => permission.Task);
                Assert.True(ScreenCapturePage.IsCaptureOverlayOpen);
                var duplicateEnteredPreflight = false;
                await page.OpenAsync(CaptureMode.Region, RegionIntent.Capture, () =>
                {
                    duplicateEnteredPreflight = true;
                    return Task.FromResult(false);
                });
                Assert.False(duplicateEnteredPreflight);

                DrawOnScreenSession.Show();
                DrawOnScreenSession.ToggleOnScreenshot();
                Assert.Same(drawing, DrawOnScreenSession.ActiveInstance);
                DrawOnScreenSession.Toggle();
                Assert.Null(DrawOnScreenSession.ActiveInstance);

                permission.SetResult(false);
                await capture;
                Assert.False(ScreenCapturePage.IsCaptureOverlayOpen);
                DrawOnScreenSession.Toggle();
                Assert.NotNull(DrawOnScreenSession.ActiveInstance);
            }
            finally
            {
                permission.TrySetResult(false);
                if (capture != null)
                    await capture;
                DrawOnScreenSession.ActiveInstance?.ShutdownAsync();
                SettingsRoot.Current = previousSettings;
            }
        }

        [AvaloniaFact]
        public void Color_preset_commands_recolor_the_selection_and_leave_select_active()
        {
            InteropTestGate.Require();
            var previousSettings = SettingsRoot.Current;
            SettingsRoot.Current = new SettingsRoot();
            DrawOnScreenWindow window = null;
            using var opened = Window.WindowOpenedEvent.AddClassHandler<DrawOnScreenWindow>((canvasWindow, _) => window ??= canvasWindow);
            try
            {
                LoadDrawingToolbarResources();
                DrawOnScreenSession.Show();
                var session = Assert.IsType<DrawOnScreenSession>(DrawOnScreenSession.ActiveInstance);
                var canvas = Assert.IsType<DrawOnScreenWindow>(window).Canvas;
                session.PickTool(ToolType.Pointer);
                var line = new GraphicLine(Colors.DeepPink, 7, new Point(10, 10), new Point(100, 100));
                canvas.AddGraphic(line);

                session.SelectColor(session.State.ColorIndex);
                Assert.Equal(session.State.DrawingColor, line.ObjectColor);
                Assert.True(line.IsSelected);
                Assert.Equal(ToolType.Pointer, session.State.Tool);

                var shortcut = DrawHotkeys.Color(4).ForApp();
                Assert.True(new DrawHotkeys(session).KeyDown(shortcut.Key, shortcut.Modifiers));
                Assert.Equal(DrawPalette.Colors[4], line.ObjectColor);
                Assert.True(line.IsSelected);
                Assert.Equal(ToolType.Pointer, session.State.Tool);
                Assert.Equal(7, line.LineWidth);
                Assert.Null(session.State.CustomColor);
                Assert.Equal(4, session.State.ColorIndex);

                session.Undo();
                Assert.Equal(DrawPalette.Colors[DrawPalette.DefaultColorIndex], line.ObjectColor);
                session.Undo();
                Assert.Equal(Colors.DeepPink, line.ObjectColor);
            }
            finally
            {
                DrawOnScreenSession.ActiveInstance?.ShutdownAsync();
                SettingsRoot.Current = previousSettings;
            }
        }

        private static void LoadDrawingToolbarResources()
        {
            var resources = Application.Current.Resources.MergedDictionaries;
            foreach (var path in new[] { "Assets/VectorIcons.axaml", "Controls/Tray/TrayTips.axaml", "Controls/Tray/FloatingTray.axaml" })
            {
                var uri = new System.Uri($"avares://flickSnap/{path}");
                resources.Add(new ResourceInclude(uri) { Source = uri });
            }
        }

        [AvaloniaFact]
        public void Control_wheel_requests_zoom_even_without_a_backdrop()
        {
            SettingsRoot.Current ??= new SettingsRoot();
            var screen = new DesktopScreen { Bounds = new PixelRect(0, 0, 800, 600) };
            var window = new DrawOnScreenWindow(screen, SavedToolSettings.CreateDefault);
            var requests = new List<(double Delta, Point Anchor)>();
            window.ZoomWheel += (delta, anchor) => requests.Add((delta, anchor));

            window.Show();
            try
            {
                var pointer = new Point(window.Bounds.Width / 4, window.Bounds.Height / 2);
                window.MouseWheel(pointer, new Vector(0, 1), RawInputModifiers.None);
                Assert.Empty(requests);

                window.MouseWheel(pointer, new Vector(0, 0.5), RawInputModifiers.Control);
                Assert.Equal((0.5, new Point(0.25, 0.5)), Assert.Single(requests));
            }
            finally
            {
                window.Close();
            }
        }

        [AvaloniaFact]
        public void Eyedropper_control_wheel_reuses_canvas_zoom_at_the_screen_pointer()
        {
            SettingsRoot.Current ??= new SettingsRoot();
            var screen = new DesktopScreen { Bounds = new PixelRect(0, 0, 800, 600) };
            var canvas = new DrawOnScreenWindow(screen, SavedToolSettings.CreateDefault);
            var pickerSurface = new Window { Width = 800, Height = 600, Background = Brushes.Transparent };
            var requests = new List<(double Delta, Point Anchor)>();
            canvas.ZoomWheel += (delta, anchor) => requests.Add((delta, anchor));
            pickerSurface.PointerWheelChanged += (_, e) =>
                DrawEyedropperWindow.RouteZoomWheel(pickerSurface, e,
                    (delta, point) => canvas.ZoomFromEyedropper(delta, point));

            canvas.Show();
            pickerSurface.Show();
            try
            {
                var local = new Point(canvas.Bounds.Width / 4, canvas.Bounds.Height / 2);
                var pointer = pickerSurface.PointToClient(canvas.PointToScreen(local));
                pickerSurface.MouseWheel(pointer, new Vector(0, 1), RawInputModifiers.None);
                pickerSurface.MouseWheel(pointer, new Vector(1, 0), RawInputModifiers.Control);
                Assert.Empty(requests);

                pickerSurface.MouseWheel(pointer, new Vector(0, 0.5), RawInputModifiers.Control);
                pickerSurface.MouseWheel(pointer, new Vector(0, -1), RawInputModifiers.Control);
                Assert.Equal(new[] { (0.5, new Point(0.25, 0.5)), (-1d, new Point(0.25, 0.5)) }, requests);
            }
            finally
            {
                pickerSurface.Close();
                canvas.Close();
            }
        }

        [AvaloniaTheory]
        [InlineData(true)]
        [InlineData(false)]
        public void Eyedropper_space_drag_pans_without_picking_in_either_release_order(bool spaceFirst)
        {
            SettingsRoot.Current ??= new SettingsRoot();
            var screen = new DesktopScreen { Bounds = new PixelRect(0, 0, 800, 600) };
            var canvas = new DrawOnScreenWindow(screen, SavedToolSettings.CreateDefault);
            var picks = 0;
            DrawEyedropperWindow picker = null;
            picker = new DrawEyedropperWindow(screen, _ => picks++, null, held =>
            {
                canvas.SetPanning(held);
                picker.SetPanTarget(held ? canvas : null);
            });

            canvas.Show();
            picker.Show();
            try
            {
                canvas.BeginZoom(new RenderTargetBitmap(new PixelSize(40, 30)), 2, new Point(0.5, 0.5));
                var eyedropperCursor = picker.Cursor;
                var before = canvas.ZoomView.Pan;

                picker.RaiseEvent(new KeyEventArgs { RoutedEvent = InputElement.KeyDownEvent, Key = Key.Space });
                var hand = picker.Cursor;
                Assert.NotSame(eyedropperCursor, hand);
                picker.MouseDown(new Point(300, 200), MouseButton.Left, RawInputModifiers.None);
                Assert.NotSame(hand, picker.Cursor);
                picker.MouseMove(new Point(380, 240), RawInputModifiers.LeftMouseButton);
                Assert.True(canvas.ZoomView.Pan.X > before.X);
                Assert.True(canvas.ZoomView.Pan.Y > before.Y);

                if (!spaceFirst)
                {
                    picker.MouseUp(new Point(380, 240), MouseButton.Left, RawInputModifiers.None);
                    Assert.Same(hand, picker.Cursor);
                }

                picker.RaiseEvent(new KeyEventArgs { RoutedEvent = InputElement.KeyUpEvent, Key = Key.Space });
                Assert.Same(eyedropperCursor, picker.Cursor);
                if (spaceFirst)
                    picker.MouseUp(new Point(380, 240), MouseButton.Left, RawInputModifiers.None);
                Assert.Equal(0, picks);
            }
            finally
            {
                picker.Close();
                canvas.Close();
            }
        }

        [AvaloniaFact]
        public void Recoloring_the_selection_changes_only_ink_and_can_be_undone()
        {
            SettingsRoot.Current ??= new SettingsRoot();
            var screen = new DesktopScreen { Bounds = new PixelRect(0, 0, 800, 600) };
            var window = new DrawOnScreenWindow(screen, SavedToolSettings.CreateDefault);
            window.Show();
            try
            {
                var canvas = window.Canvas;
                canvas.Tool = ToolType.Pointer;
                var line = new GraphicLine(Colors.Red, 7, new Point(10, 10), new Point(100, 100));
                var other = new GraphicRectangle(Colors.Orange, 2, new Rect(0, 0, 40, 40));
                var text = new GraphicText(Colors.White, 0, new Point(0, 100), body: "Sample")
                {
                    Foreground = Colors.Green,
                    FontSize = 24,
                };
                canvas.AddGraphic(other);
                canvas.AddGraphic(line);
                Assert.Equal(Colors.Red, canvas.SelectedColor);
                canvas.AddGraphic(text);
                line.IsSelected = true;

                canvas.SetSelectedColor(Colors.Blue, mergeWithPrevious: true);

                Assert.Equal(2, canvas.SelectedCount);
                Assert.Equal(ToolType.Pointer, canvas.Tool);
                Assert.Equal(Colors.Blue, line.ObjectColor);
                Assert.Equal(7, line.LineWidth);
                Assert.Equal(Colors.Blue, text.Foreground);
                Assert.Equal(Colors.White, text.ObjectColor);
                Assert.Equal(24, text.FontSize);
                Assert.Equal(Colors.Orange, other.ObjectColor);

                canvas.Undo();
                Assert.Equal(Colors.Red, line.ObjectColor);
                Assert.Equal(Colors.Green, text.Foreground);
            }
            finally
            {
                window.Close();
            }
        }

        [AvaloniaFact]
        public void Size_preset_edits_each_selected_tool_style_in_one_undo_step()
        {
            SettingsRoot.Current ??= new SettingsRoot();
            var screen = new DesktopScreen { Bounds = new PixelRect(0, 0, 800, 600) };
            var window = new DrawOnScreenWindow(screen, SavedToolSettings.CreateDefault);
            window.Show();
            try
            {
                var canvas = window.Canvas;
                canvas.Tool = ToolType.Pointer;
                var line = new GraphicLine(Colors.Red, 2, new Point(10, 10), new Point(100, 100));
                var brush = new GraphicBrush(Colors.Blue, 2, new Point(50, 60));
                var text = new GraphicText(Colors.Transparent, 0, new Point(10, 120), body: "Test") { FontSize = 24 };
                var count = new GraphicCount(Colors.Green, 2, new Point(150, 100), body: "1") { FontSize = 19 };
                var unselectedLine = new GraphicLine(Colors.Red, 2, new Point(10, 150), new Point(100, 170));
                canvas.AddGraphic(unselectedLine);
                canvas.AddGraphics(new GraphicBase[] { line, brush, text, count });

                var size = DrawPalette.Sizes[1];
                canvas.SetSelectedSize(tool => (DrawPalette.LineWidthFor(tool, size), DrawPalette.FontSizeFor(tool, size)));

                Assert.Equal(ToolType.Pointer, canvas.Tool);
                Assert.Equal(4, canvas.SelectedCount);
                Assert.Equal(6, line.LineWidth);
                Assert.Equal(3, brush.LineWidth);
                Assert.Equal(28, text.FontSize);
                Assert.Equal(2, count.LineWidth);
                Assert.Equal(16.8, count.FontSize, precision: 6);
                Assert.Equal(2, unselectedLine.LineWidth);
                Assert.Equal(Colors.Red, line.ObjectColor);

                canvas.Undo();
                Assert.Equal(2, line.LineWidth);
                Assert.Equal(2, brush.LineWidth);
                Assert.Equal(24, text.FontSize);
                Assert.Equal(2, count.LineWidth);
                Assert.Equal(19, count.FontSize);
            }
            finally
            {
                window.Close();
            }
        }

        [AvaloniaFact]
        public void Reopening_the_canvas_keeps_drawing_and_both_history_directions()
        {
            SettingsRoot.Current ??= new SettingsRoot();
            var settings = new Dictionary<ToolType, SavedToolSettings>();
            SavedToolSettings Resolve(ToolType tool)
            {
                if (!settings.TryGetValue(tool, out var value))
                    settings.Add(tool, value = SavedToolSettings.CreateDefault(tool));
                return value;
            }

            var screen = new DesktopScreen { Bounds = new PixelRect(0, 0, 800, 600) };
            var first = new DrawOnScreenWindow(screen, Resolve);
            var target = new CanvasUndoTarget(first.Canvas);
            var history = new DrawHistory();
            first.Canvas.HistoryAppended += (_, _) => history.Record(target);
            first.Show();
            try
            {
                first.Canvas.AddGraphic(new GraphicRectangle(Colors.Red, 2, new Rect(0, 0, 20, 20)));
                first.Canvas.AddGraphic(new GraphicRectangle(Colors.Blue, 2, new Rect(0, 0, 40, 40)));
                Assert.True(history.Undo());
                Assert.Single(first.Canvas.GraphicsList);
                first.DetachCanvas();
            }
            finally
            {
                first.Close();
            }

            Assert.Null(TopLevel.GetTopLevel(target.Canvas));
            var reopened = new DrawOnScreenWindow(screen, Resolve, rememberedCanvas: target.Canvas);
            reopened.Show();
            try
            {
                Assert.Same(reopened, TopLevel.GetTopLevel(target.Canvas));
                Assert.True(reopened.Canvas.IsHitTestVisible);
                Assert.Single(reopened.Canvas.GraphicsList);
                Assert.True(history.Redo());
                Assert.Equal(2, reopened.Canvas.GraphicsList.Count);
                Assert.True(history.Undo());
                Assert.True(history.Undo());
                Assert.Empty(reopened.Canvas.GraphicsList);
            }
            finally
            {
                reopened.Close();
            }
        }
    }
}
