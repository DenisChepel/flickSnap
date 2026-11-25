using Avalonia;
using Avalonia.Media;
using Avalonia.Platform;
using Avalonia.Rendering.SceneGraph;
using Avalonia.Skia;
using SkiaSharp;

namespace FlickSnap.Drawing.Ink
{
    /// <summary>
    /// The hover outline of a freehand stroke: a ring of <see cref="Width"/> straddling the edge of
    /// the ink's filled area, drawn as pixels rather than as geometry. The stroke's pieces overlap
    /// (chunks share vertices, the highlighter is one hull per segment, a scribble crosses itself),
    /// so outlining them as vectors needs their union first, and a path union is superlinear in
    /// how much the pieces overlap: a long scribbled highlighter took 40 s on the UI thread and hung
    /// the app. Here the ink is filled once into a layer that dilates it by half the width, and
    /// filled again eroded by the other half to cut the inside out, which costs what drawing the
    /// ink costs however much it crosses itself.
    /// </summary>
    internal sealed class InkOutlineDrawOperation : ICustomDrawOperation
    {
        private readonly InkRun[] _runs;
        private readonly Color _color;

        /// <param name="runs">The stroke's pieces, local to the current transform.</param>
        /// <param name="inkBounds">Their bounds, in the same space.</param>
        /// <param name="color">The ring's colour.</param>
        /// <param name="width">The ring's width, in the same space.</param>
        public InkOutlineDrawOperation(InkRun[] runs, Rect inkBounds, Color color, double width)
        {
            _runs = runs;
            _color = color;
            Width = width;
            Bounds = inkBounds.Inflate(Width + 1);
        }

        public double Width { get; }

        public Rect Bounds { get; }

        public bool HitTest(Point p) => false;

        public bool Equals(ICustomDrawOperation other) => false;

        public void Dispose()
        { }

        public void Render(ImmediateDrawingContext context)
        {
            var feature = context.TryGetFeature<ISkiaSharpApiLeaseFeature>();
            if (feature == null || _runs.Length == 0)
                return;

            using var lease = feature.Lease();
            var canvas = lease.SkCanvas;
            if (canvas == null)
                return;

            var b = Bounds;
            var rect = new SKRect((float)b.Left, (float)b.Top, (float)b.Right, (float)b.Bottom);
            float half = (float)(Width / 2);

            using var group = new SKPaint { Color = new SKColor(255, 255, 255, _color.A) };
            using var ink = new SKPaint { Color = new SKColor(_color.R, _color.G, _color.B), IsAntialias = true, Style = SKPaintStyle.Fill };
            using var dilate = SKImageFilter.CreateDilate(half, half);
            using var grow = new SKPaint { ImageFilter = dilate };
            using var erode = SKImageFilter.CreateErode(half, half);
            using var cut = new SKPaint { ImageFilter = erode, BlendMode = SKBlendMode.DstOut };

            // the ring is built opaque and laid down once at the colour's alpha, so where the
            // pieces overlap it does not darken
            canvas.SaveLayer(rect, group);

            canvas.SaveLayer(rect, grow);
            foreach (var run in _runs)
                canvas.DrawPath(run.GetPath(), ink);
            canvas.Restore();

            canvas.SaveLayer(rect, cut);
            foreach (var run in _runs)
                canvas.DrawPath(run.GetPath(), ink);
            canvas.Restore();

            canvas.Restore();
        }
    }
}
