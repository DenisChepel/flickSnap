using System;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Headless.XUnit;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using FlickSnap.Drawing.Graphics;
using FlickSnap.Drawing.Ink;
using FlickSnap.Drawing.Rendering;
using SkiaSharp;
using Xunit;
using Sample = FlickSnap.Drawing.Graphics.GraphicBrush.Sample;

namespace FlickSnap.Drawing.Tests
{
    /// <summary>
    /// The hover outline of a brush or highlighter stroke is drawn as pixels around the ink, never
    /// as a union of the stroke's pieces: a union of a long scribble's overlapping pieces took
    /// 40 s for a 37k-sample highlighter and hung the UI thread the moment the pointer rested on it.
    /// </summary>
    public class InkHoverOutlineTests
    {
        /// <summary>A hand scribbling back and forth over one area: a deterministic random walk
        /// that keeps turning back on itself, the case that makes the pieces overlap the most.</summary>
        private static Sample[] Scribble(int n, int seed = 7)
        {
            var rnd = new Random(seed);
            var samples = new Sample[n];
            double x = 0, y = 0, angle = 0;
            for (int i = 0; i < n; i++)
            {
                angle += (rnd.NextDouble() - 0.5) * 0.9;
                x += Math.Cos(angle) * 3;
                y += Math.Sin(angle) * 3;
                // fold back into a 600 x 400 area, like a hand colouring in a patch
                if (x < 0 || x > 600) angle = Math.PI - angle;
                if (y < 0 || y > 400) angle = -angle;
                x = Math.Clamp(x, 0, 600);
                y = Math.Clamp(y, 0, 400);
                samples[i] = new Sample(new Point(x, y), i * 4);
            }

            return samples;
        }

        private static byte[] Render(Rect area, Action<DrawingContext> draw, out int w, out int h)
        {
            w = (int)Math.Ceiling(area.Width);
            h = (int)Math.Ceiling(area.Height);
            using var rtb = new RenderTargetBitmap(new PixelSize(w, h));
            using (var ctx = rtb.CreateDrawingContext())
            using (ctx.PushTransform(Matrix.CreateTranslation(-area.X, -area.Y)))
                draw(ctx);

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

            return px;
        }

        public static TheoryData<string> Kinds => new TheoryData<string> { "highlighter", "brush" };

        private static GraphicBrush Stroke(string kind, Sample[] samples, double width) =>
            kind == "highlighter"
                ? new GraphicHighlighter(Colors.Yellow, width, new Point(0, 0), 555317733) { Samples = samples }
                : new GraphicBrush(Colors.Red, width, new Point(0, 0)) { Samples = samples };

        [AvaloniaTheory]
        [MemberData(nameof(Kinds))]
        public void LongScribble_HoverOutline_DoesNotStall(string kind)
        {
            var g = Stroke(kind, Scribble(40_000), 20);
            g.GetOutline();
            var pen = RenderResources.GetPen(GraphicBase.HandleColor, 1.5);

            var sw = Stopwatch.StartNew();
            Render(g.Bounds.Inflate(4), ctx => g.DrawHoverOutline(ctx, pen), out _, out _);
            sw.Stop();

            // the union this replaced took ~40 s for the highlighter and ~2 s for the brush
            Assert.True(sw.ElapsedMilliseconds < 5000, $"{kind}: {sw.ElapsedMilliseconds} ms");
        }

        [AvaloniaTheory]
        [MemberData(nameof(Kinds))]
        public void HoverOutline_RingsTheInk(string kind)
        {
            // a straight pass along y = 50
            var samples = Enumerable.Range(0, 60).Select(i => new Sample(new Point(i * 4, 0), i * 8)).ToArray();
            var g = Stroke(kind, samples, 10);
            g.Origin = new Point(50, 50);
            var ink = g.Bounds;
            var area = ink.Inflate(20);
            var px = Render(area, ctx => g.DrawHoverOutline(ctx, RenderResources.GetPen(Colors.Blue, 2)), out int w, out _);

            byte Alpha(double x, double y) => px[((int)(y - area.Y) * w + (int)(x - area.X)) * 4 + 3];

            double midX = ink.Center.X;
            Assert.Equal(0, Alpha(midX, ink.Center.Y));             // the inside is cut out
            Assert.True(Alpha(midX, ink.Top) > 100, $"top edge: {Alpha(midX, ink.Top)}");
            Assert.True(Alpha(midX, ink.Bottom - 1) > 100, $"bottom edge: {Alpha(midX, ink.Bottom - 1)}");
            Assert.Equal(0, Alpha(midX, ink.Top - 6));              // nothing beyond the ring
            Assert.Equal(0, Alpha(midX, ink.Bottom + 6));
        }

        private sealed class RectRun : InkRun
        {
            protected override SKPath BuildPath()
            {
                var path = new SKPath();
                path.AddRect(new SKRect(2, 2, 8, 6));
                return path;
            }
        }

        [AvaloniaTheory]
        [InlineData(1)]
        [InlineData(10)]
        public void HoverOutline_KeepsItsScreenWidthAtZoom(int zoom)
        {
            var outline = new InkOutlineDrawOperation(new InkRun[] { new RectRun() }, new Rect(2, 2, 6, 4), Colors.Blue, 1.5 / zoom);
            var px = Render(new Rect(0, 0, 100, 80), ctx =>
            {
                using var transform = ctx.PushTransform(Matrix.CreateScale(zoom, zoom));
                ctx.Custom(outline);
            }, out int w, out _);

            int x = 5 * zoom;
            int top = 2 * zoom;
            byte AlphaAt(int y) => px[(y * w + x) * 4 + 3];
            Assert.True(AlphaAt(top - 1) > 0 || AlphaAt(top) > 0, "The outline is visible at the edge.");
            Assert.Equal(0, AlphaAt(top - 2));
            Assert.Equal(0, AlphaAt(top + 2));
        }

        [AvaloniaFact]
        public void BrushRuns_AreTheBrushGeometry_AndSurviveCompact()
        {
            var g = new GraphicBrush(Colors.Red, 6, new Point(0, 0));
            foreach (var s in Scribble(3000, seed: 3).Skip(1))
                g.AddSample(s.P, s.T, 0.5);
            g.EndStroke(); // compacts the builder

            var outline = g.GetOutline();
            var runs = outline.Runs;
            Assert.True(runs.Length > 1, "a long stroke has sealed pieces and a tail");

            var b = outline.Bounds;
            int checkedPoints = 0, mismatches = 0;
            for (double y = b.Top; y <= b.Bottom; y += 3.7)
            {
                for (double x = b.Left; x <= b.Right; x += 3.7)
                {
                    bool geometry = outline.FillContains(new Point(x, y));
                    bool path = runs.Any(r => r.GetPath().Contains((float)x, (float)y));
                    checkedPoints++;
                    if (geometry != path)
                        mismatches++;
                }
            }

            // the same figures with the same smoothing: only points on an edge may disagree
            Assert.True(mismatches <= checkedPoints / 500, $"{mismatches} of {checkedPoints}");
        }
    }
}
