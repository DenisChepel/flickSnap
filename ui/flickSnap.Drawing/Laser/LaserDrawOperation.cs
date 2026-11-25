using System;
using System.Collections.Generic;
using Avalonia;
using Avalonia.Media;
using Avalonia.Platform;
using Avalonia.Rendering.SceneGraph;
using Avalonia.Skia;
using SkiaSharp;

namespace FlickSnap.Drawing.Laser
{
    /// <summary>
    /// Paints a solid, variable-width stroke. Circular joins and tangent bands keep sparse
    /// flicks connected and tight turns filled, including when the trail crosses itself.
    /// </summary>
    internal sealed class LaserDrawOperation : ICustomDrawOperation
    {
        private readonly (Point P, double R)[][] _trails;
        private readonly Color _color;

        /// <param name="trails">Each trail's circles, oldest first, in the current transform's space.</param>
        /// <param name="color">The fill of the trail.</param>
        /// <param name="antialiasPadding">The edge padding in the current transform's space.</param>
        public LaserDrawOperation((Point P, double R)[][] trails, Color color, double antialiasPadding = 1)
        {
            _trails = trails;
            _color = color;

            var bounds = default(Rect);
            foreach (var trail in trails)
            {
                foreach (var (p, r) in trail)
                {
                    // A zero-radius sample can still be the tip of a long tangent band.
                    // Pad before Union so Avalonia doesn't drop it as an empty rectangle.
                    double extent = r + antialiasPadding;
                    bounds = bounds.Union(new Rect(p.X - extent, p.Y - extent, 2 * extent, 2 * extent));
                }
            }

            Bounds = bounds;
        }

        public Rect Bounds { get; }

        public bool HitTest(Point p) => false;

        public bool Equals(ICustomDrawOperation other) => false;

        public void Dispose()
        { }

        public void Render(ImmediateDrawingContext context)
        {
            if (_trails.Length == 0)
                return;

            var feature = context.TryGetFeature<ISkiaSharpApiLeaseFeature>();
            if (feature == null)
                return;

            using var lease = feature.Lease();
            var canvas = lease.SkCanvas;
            if (canvas == null)
                return;

            using var beam = new SKPath { FillType = SKPathFillType.Winding };
            foreach (var trail in _trails)
                AddStroke(beam, trail);

            using var fill = new SKPaint { IsAntialias = true, Color = new SKColor(_color.R, _color.G, _color.B, _color.A) };
            canvas.DrawPath(beam, fill);
        }

        private static void AddStroke(SKPath path, IReadOnlyList<(Point P, double R)> trail)
        {
            for (int i = 0; i < trail.Count; i++)
            {
                var (p, r) = trail[i];
                if (r > 0)
                    path.AddCircle((float)p.X, (float)p.Y, (float)r, SKPathDirection.Clockwise);
                if (i > 0)
                    AddBand(path, trail[i - 1].P, (float)trail[i - 1].R, p, (float)r);
            }
        }

        /// <summary>The quad along the two outer tangents of circles (a, ra) and (b, rb), wound
        /// clockwise like the circles. Nothing when one circle holds the other.</summary>
        private static void AddBand(SKPath path, Point a, float ra, Point b, float rb)
        {
            double dx = b.X - a.X, dy = b.Y - a.Y;
            double d = Math.Sqrt(dx * dx + dy * dy);
            if (d <= Math.Abs(ra - rb) + 1e-6)
                return;

            double ux = dx / d, uy = dy / d;   // along
            double nx = -uy, ny = ux;          // across
            double s = (ra - rb) / d;          // the tangents lean in toward the smaller circle
            double k = Math.Sqrt(Math.Max(0, 1 - s * s));

            // unit vectors from each centre to its tangent points, one per side
            double v1x = ux * s + nx * k, v1y = uy * s + ny * k;
            double v2x = ux * s - nx * k, v2y = uy * s - ny * k;

            Span<SKPoint> quad = stackalloc SKPoint[4];
            quad[0] = new SKPoint((float)(a.X + ra * v1x), (float)(a.Y + ra * v1y));
            quad[1] = new SKPoint((float)(b.X + rb * v1x), (float)(b.Y + rb * v1y));
            quad[2] = new SKPoint((float)(b.X + rb * v2x), (float)(b.Y + rb * v2y));
            quad[3] = new SKPoint((float)(a.X + ra * v2x), (float)(a.Y + ra * v2y));

            // clockwise on screen (y down) is a positive shoelace sum
            double area = 0;
            for (int i = 0; i < 4; i++)
            {
                var p0 = quad[i];
                var p1 = quad[(i + 1) % 4];
                area += (double)p0.X * p1.Y - (double)p1.X * p0.Y;
            }

            if (area < 0)
            {
                (quad[1], quad[3]) = (quad[3], quad[1]);
            }

            path.MoveTo(quad[0]);
            path.LineTo(quad[1]);
            path.LineTo(quad[2]);
            path.LineTo(quad[3]);
            path.Close();
        }
    }
}
