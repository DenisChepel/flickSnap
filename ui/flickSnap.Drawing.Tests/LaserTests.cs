using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Headless;
using Avalonia.Headless.XUnit;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using FlickSnap.Drawing.Graphics;
using FlickSnap.Drawing.Laser;
using FlickSnap.Drawing.Tools;
using Xunit;

namespace FlickSnap.Drawing.Tests
{
    /// <summary>
    /// The laser pointer: a trail widest at the tip that thins away behind it and fades out on its
    /// own once released, drawn as one seamless stroke, and never touching the document —
    /// no graphic, no history step, no selection change.
    /// </summary>
    public class LaserTests
    {
        static LaserTests()
        {
            FlickSnap.Config.SettingsRoot.Current ??= new FlickSnap.Config.SettingsRoot();
        }

        private const double R = 2;

        /// <summary>A pointer moving right at 5 units every 10 ms, <paramref name="n"/> samples.</summary>
        private static LaserTrail Swipe(int n, out double end)
        {
            var trail = new LaserTrail();
            for (int i = 0; i < n; i++)
                trail.Add(new Point(i * 5, 0), i * 10);
            end = (n - 1) * 10;
            return trail;
        }

        private static List<(Point P, double R)> Shape(LaserTrail trail, double now)
        {
            var shape = new List<(Point P, double R)>();
            trail.GetShape(now, R, shape);
            return shape;
        }

        [Fact]
        public void Trail_IsWidestAtTheTip_AndThinsAwayBehindIt()
        {
            var trail = Swipe(30, out double now);
            var shape = Shape(trail, now);

            Assert.InRange(shape[^1].R, 1.99, R);
            for (int i = 1; i < shape.Count; i++)
                Assert.True(shape[i].R >= shape[i - 1].R, $"radius dips at {i}: {shape[i - 1].R} > {shape[i].R}");
            Assert.True(shape[0].R < shape[^1].R);
        }

        [Fact]
        public void Trail_DrawsOnlyTheLastDecayLengthPoints()
        {
            var trail = Swipe(LaserTrail.DecayLength * 4, out double now);
            Assert.True(Shape(trail, now).Count <= LaserTrail.DecayLength);

            Assert.True(trail.Count <= LaserTrail.DecayLength + 1);
        }

        [Fact]
        public void ReleasedTrail_FadesOutWithinTheDecayTime()
        {
            var trail = Swipe(20, out double now);
            trail.End();

            double before = Shape(trail, now).Max(c => c.R);
            double halfway = Shape(trail, now + LaserTrail.DecayMs / 2).Max(c => c.R);
            Assert.True(halfway < before, $"{halfway} should be below {before}");

            Assert.True(trail.IsAlive(now + LaserTrail.DecayMs - 20));
            Assert.False(trail.IsAlive(now + LaserTrail.DecayMs));
            Assert.Empty(Shape(trail, now + LaserTrail.DecayMs));
        }

        [Fact]
        public void TrailBeingDrawn_StaysAliveWhileThePointerRests_AndTakesNoPointsAfterRelease()
        {
            var trail = Swipe(5, out double now);
            Assert.True(trail.IsAlive(now + 10 * LaserTrail.DecayMs));

            trail.End();
            trail.Add(new Point(1000, 1000), now + 1);
            Assert.Equal(5, trail.Count);
        }

        [Fact]
        public void Click_ShrinksAwayEvenIfThePointerIsHeldStill()
        {
            var trail = new LaserTrail();
            trail.Add(new Point(0, 0), 0);
            trail.Add(new Point(0, 0), 900);
            Assert.Equal(1, trail.Count);
            Assert.InRange(Assert.Single(Shape(trail, 0)).R, 1.99, R);
            Assert.True(Assert.Single(Shape(trail, 600)).R < R);
            Assert.Empty(Shape(trail, 1000));
            Assert.True(trail.IsDrawing);
        }

        [AvaloniaFact]
        public void FastGestureSpansTheScreen_WhileTheSameSlowGestureLeavesATail()
        {
            const double width = 1769;
            var fast = new LaserTrail();
            for (int i = 0; i <= 24; i++)
                fast.Add(new Point(50 + width * i / 24, 8), 6 * i);
            fast.End();

            var flick = Shape(fast, 145);
            Assert.Equal(50, flick[0].P.X);
            Assert.InRange(flick[^1].P.X - flick[0].P.X, width * 0.97, width);
            var px = RenderTrail(flick, 1864, 16, out _);
            for (int x = 51; x < (int)flick[^1].P.X; x++)
                Assert.True(px[(8 * 1864 + x) * 4 + 3] > 240, $"gap in the flick at {x}");

            var slow = new LaserTrail();
            for (int i = 0; i <= 95; i++)
                slow.Add(new Point(width * i / 95, 0), i * 33);
            var tail = Shape(slow, 95 * 33);
            Assert.InRange(tail[^1].P.X - tail[0].P.X, width * 0.25, width * 0.4);
        }

        [Fact]
        public void SparseFlick_SmoothsTheEndpointWithoutBreakingTheLine()
        {
            var trail = new LaserTrail();
            trail.Add(new Point(0, 0), 0);
            trail.Add(new Point(1769, 0), 16);
            trail.End();
            var shape = Shape(trail, 16);

            Assert.Equal(2, shape.Count);
            Assert.Equal(1061.4, shape[1].P.X, 6);
            Assert.Equal(shape[0].R, shape[1].R, 6);
        }

        private static byte[] RenderTrail(IReadOnlyList<(Point P, double R)> shape, int w, int h, out bool rgba)
        {
            using var rtb = new RenderTargetBitmap(new PixelSize(w, h));
            using (var ctx = rtb.CreateDrawingContext())
                ctx.Custom(new LaserDrawOperation(new[] { shape.ToArray() }, LaserLayer.DefaultColor));

            var px = new byte[w * h * 4];
            var handle = GCHandle.Alloc(px, GCHandleType.Pinned);
            try
            {
                rtb.CopyPixels(new PixelRect(0, 0, w, h), handle.AddrOfPinnedObject(), px.Length, w * 4);
            }
            finally
            {
                handle.Free();
            }

            rgba = rtb.Format == PixelFormats.Rgba8888;
            return px;
        }

        /// <summary>A hairpin turn overlaps the stroke onto itself: every piece is wound the same
        /// way, so the overlap is filled, not cut out, and the whole centreline is solid beam.</summary>
        [AvaloniaFact]
        public void SharpTurn_IsSolidAllAlongTheCentreline()
        {
            var points = new List<Point>();
            for (int i = 0; i <= 20; i++)
                points.Add(new Point(20 + i * 4, 20 + i * 4));      // down-right
            for (int i = 1; i <= 20; i++)
                points.Add(new Point(100 - i * 4, 100 - i * 3));    // back up-left, crossing near the start
            var shape = points.Select(p => (p, R)).ToList();

            var px = RenderTrail(shape, 140, 140, out _);
            foreach (var p in points)
            {
                int i = ((int)p.Y * 140 + (int)p.X) * 4;
                Assert.True(px[i + 3] > 240, $"a hole at {p}: alpha {px[i + 3]}");
            }
        }

        [AvaloniaFact]
        public void Beam_IsSolidRed_AndTransparentOutsideItsWidth()
        {
            var shape = Enumerable.Range(0, 20).Select(i => (new Point(10 + i * 4, 30.5), R)).ToList();
            var px = RenderTrail(shape, 120, 60, out bool rgba);
            (byte r, byte g, byte b) At(int x, int y)
            {
                int i = (y * 120 + x) * 4;
                return rgba ? (px[i], px[i + 1], px[i + 2]) : (px[i + 2], px[i + 1], px[i]);
            }

            Assert.Equal(((byte)255, (byte)0, (byte)0), At(50, 30));
            Assert.Equal(At(50, 30), At(50, 31));
            Assert.Equal(0, px[(27 * 120 + 50) * 4 + 3]);
        }

        [AvaloniaFact]
        public void DamageBounds_CoverAScreenWideSegmentWithAnExpiredTailPoint()
        {
            var tip = new Point(25, 12);
            var shape = new[] { (tip, 0d), (new Point(1800, 24), R) };
            var px = RenderTrail(shape, 1864, 40, out _);
            Assert.True(px[(15 * 1864 + 500) * 4 + 3] > 0, "the taper still paints far from the live point");

            using var operation = new LaserDrawOperation(new[] { shape }, LaserLayer.DefaultColor);
            Assert.True(operation.Bounds.Contains(tip), $"the compositor would miss the taper: {operation.Bounds}");
        }

        private static PointerState Ptr(Point p, bool down) =>
            new PointerState(p, KeyModifiers.None, down, false, false, null, 0, null);

        /// <summary>The laser never touches the document: no graphic, no history step, the
        /// selection left as it was, and the trail fading on the canvas's laser layer.</summary>
        [AvaloniaFact]
        public void Tool_LeavesTheDocumentAlone()
        {
            var canvas = new DrawingCanvas { Tool = ToolType.None };
            var rect = new GraphicRectangle(Colors.Blue, 2, new Rect(10, 10, 50, 50));
            canvas.GraphicsList.Add(rect);
            canvas.AddCommandToHistory(false);
            rect.IsSelected = true;
            canvas.Tool = ToolType.Laser;
            var tool = canvas.ToolLaser;

            int steps = 0;
            Action<UndoManager, SortedSet<string>> hook = (_, changes) => { if (changes.Count > 0) steps++; };
            UndoManager.DiagnosticCommitBuilt += hook;
            try
            {
                tool.OnMouseDown(canvas, Ptr(new Point(0, 0), true), 1);
                Assert.True(canvas.LaserLayer.IsDrawing);
                for (int i = 1; i < 20; i++)
                    tool.OnMouseMove(canvas, Ptr(new Point(i * 6, i * 3), true));
                tool.OnMouseUp(canvas, Ptr(new Point(120, 60), false));
            }
            finally
            {
                UndoManager.DiagnosticCommitBuilt -= hook;
            }

            Assert.Equal(0, steps);
            Assert.Single(canvas.GraphicsList);
            Assert.True(rect.IsSelected);
            Assert.False(canvas.LaserLayer.IsDrawing);
            Assert.True(canvas.LaserLayer.HasTrails);
            Assert.Equal(ToolType.Laser, canvas.Tool);
        }

        /// <summary>The layer's own default colour is the opaque laser red. (A static-initialisation
        /// order slip once registered the property before the colour was set: a transparent
        /// default, and a laser that drew nothing at all.)</summary>
        [AvaloniaFact]
        public void Layer_DefaultsToTheOpaqueLaserColour()
        {
            var layer = new LaserLayer();
            Assert.Equal(LaserLayer.DefaultColor, layer.Color);
            Assert.Equal(255, layer.Color.A);
        }

        /// <summary>End to end: a canvas in a real window, the laser picked, a real press and
        /// drag — and the frame shows the trail along the path.</summary>
        [AvaloniaFact]
        public void DraggingTheLaser_ShowsATrailOnScreen()
        {
            var canvas = new DrawingCanvas { IsOverlayMode = true };
            var window = new Avalonia.Controls.Window { Width = 400, Height = 300, Content = canvas, Background = Brushes.White };
            window.Show();
            try
            {
                Avalonia.Threading.Dispatcher.UIThread.RunJobs();
                canvas.Tool = ToolType.Laser;

                var start = new Point(50, 120);
                window.MouseDown(start, MouseButton.Left);
                for (int i = 1; i <= 12; i++)
                {
                    window.MouseMove(start + new Vector(i * 20, 0));
                    AvaloniaHeadlessPlatform.ForceRenderTimerTick();
                    Avalonia.Threading.Dispatcher.UIThread.RunJobs();
                }
                Avalonia.Threading.Dispatcher.UIThread.RunJobs();

                var frame = Avalonia.Headless.HeadlessWindowExtensions.CaptureRenderedFrame(window);
                Assert.NotNull(frame);
                var size = frame.PixelSize;
                var px = new byte[size.Width * size.Height * 4];
                using (var fb = frame.Lock())
                {
                    for (int y = 0; y < size.Height; y++)
                        Marshal.Copy(fb.Address + y * fb.RowBytes, px, y * size.Width * 4, size.Width * 4);
                }

                bool rgba = frame.Format == PixelFormats.Rgba8888;
                for (int x = 200; x < 270; x++)
                {
                    int i0 = ((int)start.Y * size.Width + x) * 4;
                    byte r = rgba ? px[i0] : px[i0 + 2], g = px[i0 + 1];
                    Assert.True(r > 200 && g < 200, $"no trail on screen at ({x}, {start.Y}): r={r} g={g}");
                }

                window.MouseUp(start + new Vector(240, 0), MouseButton.Left);
                Assert.False(canvas.LaserLayer.IsDrawing);
                Assert.Empty(canvas.GraphicsList);
            }
            finally
            {
                window.Close();
            }
        }

        [Theory]
        [InlineData(28, 6)]
        [InlineData(56, 12)]
        public void Cursor_PointsUpLeftWithTheSparkAsItsHotspot(int size, int hotspotCoordinate)
        {
            var px = LaserCursor.Rasterize(size, out var hotspot);
            Assert.Equal(size * size * 4, px.Length);
            Assert.Equal(new PixelPoint(hotspotCoordinate, hotspotCoordinate), hotspot);

            int AlphaAt(int x, int y) => px[(y * size + x) * 4 + 3];
            int handleCoordinate = size * 22 / 28;
            Assert.Equal(255, AlphaAt(hotspot.X, hotspot.Y));
            Assert.Equal(255, AlphaAt(handleCoordinate, handleCoordinate));
            Assert.Equal(0, AlphaAt(hotspot.X, handleCoordinate));
            Assert.Equal(0, AlphaAt(0, 0));
        }
    }
}
