using System;
using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>
    /// A draw-on-screen window's picture of its monitor (a frozen session's, or zoom mode's): the grab
    /// stretched over the window, magnified by <see cref="Zoom"/> and shifted by <see cref="Offset"/>,
    /// always sampled nearest-neighbour so a magnified pixel is a sharp square, never a smear. From
    /// <see cref="GridZoom"/> up an enabled grid is laid over it, each piece of line light over dark pixels
    /// and dark over light ones, so it reads on any picture.
    /// </summary>
    internal sealed class PixelBackdrop : Control
    {
        /// <summary>The zoom from which the pixel grid shows (500%).</summary>
        public const double GridZoom = 5;

        private static readonly IPen LightLine = new Pen(new SolidColorBrush(Color.FromArgb(0x66, 0xFF, 0xFF, 0xFF)), 1);
        private static readonly IPen DarkLine = new Pen(new SolidColorBrush(Color.FromArgb(0x59, 0x00, 0x00, 0x00)), 1);

        private readonly Bitmap _bitmap;

        // per picture pixel: whether it is light (a dark line goes over it); read on the grid's first showing
        private bool[] _light;

        private double _zoom = 1;
        private Point _offset;

        public PixelBackdrop(Bitmap bitmap)
        {
            _bitmap = bitmap ?? throw new ArgumentNullException(nameof(bitmap));
            IsHitTestVisible = false;
            ClipToBounds = true;
        }

        public double Zoom => _zoom;

        private bool _showGrid;

        /// <summary>Whether the grid is enabled (G toggles it).</summary>
        public bool ShowGrid
        {
            get => _showGrid;
            set
            {
                if (_showGrid == value)
                    return;
                _showGrid = value;
                InvalidateVisual();
            }
        }

        /// <summary>Where the picture's top-left corner sits, in window DIPs.</summary>
        public Point Offset => _offset;

        public void SetView(double zoom, Point offset)
        {
            if (zoom == _zoom && offset == _offset)
                return;

            _zoom = zoom;
            _offset = offset;
            InvalidateVisual();
        }

        public override void Render(DrawingContext context)
        {
            var size = Bounds.Size;
            var px = _bitmap.PixelSize;
            if (size.Width <= 0 || size.Height <= 0 || px.Width <= 0 || px.Height <= 0)
                return;

            var dest = new Rect(_offset, new Size(size.Width * _zoom, size.Height * _zoom));
            using (context.PushRenderOptions(new RenderOptions { BitmapInterpolationMode = BitmapInterpolationMode.None }))
                context.DrawImage(_bitmap, new Rect(0, 0, px.Width, px.Height), dest);

            if (_showGrid && _zoom >= GridZoom)
                DrawGrid(context, size, px);
        }

        private void DrawGrid(DrawingContext context, Size size, PixelSize px)
        {
            _light ??= ReadLightness();
            if (_light == null)
                return;

            // one picture pixel, in window DIPs
            var cellW = size.Width * _zoom / px.Width;
            var cellH = size.Height * _zoom / px.Height;

            // the visible cells only
            var c0 = Math.Max(0, (int)Math.Floor(-_offset.X / cellW));
            var c1 = Math.Min(px.Width, (int)Math.Ceiling((size.Width - _offset.X) / cellW));
            var r0 = Math.Max(0, (int)Math.Floor(-_offset.Y / cellH));
            var r1 = Math.Min(px.Height, (int)Math.Ceiling((size.Height - _offset.Y) / cellH));
            if (c1 <= c0 || r1 <= r0)
                return;

            // lines a device pixel wide, on device pixel centres, so each is one crisp row of pixels
            var scale = TopLevel.GetTopLevel(this)?.RenderScaling ?? 1;
            var thickness = 1 / scale;
            double Snap(double v) => (Math.Floor(v * scale) + 0.5) / scale;

            var light = new StreamGeometry();
            var dark = new StreamGeometry();
            using (var lc = light.Open())
            using (var dc = dark.Open())
            {
                // vertical lines: the left edge of each column, its pieces coloured by the pixel to its right
                for (var c = c0 + 1; c < c1; c++)
                {
                    var x = Snap(_offset.X + c * cellW);
                    var runStart = r0;
                    for (var r = r0 + 1; r <= r1; r++)
                    {
                        if (r < r1 && _light[(r * px.Width) + c] == _light[(runStart * px.Width) + c])
                            continue;
                        var ctx = _light[(runStart * px.Width) + c] ? dc : lc;
                        ctx.BeginFigure(new Point(x, _offset.Y + runStart * cellH), false);
                        ctx.LineTo(new Point(x, _offset.Y + r * cellH));
                        ctx.EndFigure(false);
                        runStart = r;
                    }
                }

                // horizontal lines: the top edge of each row, coloured by the pixel below
                for (var r = r0 + 1; r < r1; r++)
                {
                    var y = Snap(_offset.Y + r * cellH);
                    var row = r * px.Width;
                    var runStart = c0;
                    for (var c = c0 + 1; c <= c1; c++)
                    {
                        if (c < c1 && _light[row + c] == _light[row + runStart])
                            continue;
                        var ctx = _light[row + runStart] ? dc : lc;
                        ctx.BeginFigure(new Point(_offset.X + runStart * cellW, y), false);
                        ctx.LineTo(new Point(_offset.X + c * cellW, y));
                        ctx.EndFigure(false);
                        runStart = c;
                    }
                }
            }

            context.DrawGeometry(null, Thick(LightLine, thickness), light);
            context.DrawGeometry(null, Thick(DarkLine, thickness), dark);
        }

        private static IPen Thick(IPen pen, double thickness) =>
            thickness == 1 ? pen : new Pen(pen.Brush, thickness);

        private bool[] ReadLightness()
        {
            var px = _bitmap.PixelSize;
            var stride = px.Width * 4;
            var buffer = new byte[stride * px.Height];
            var handle = GCHandle.Alloc(buffer, GCHandleType.Pinned);
            try
            {
                _bitmap.CopyPixels(new PixelRect(px), handle.AddrOfPinnedObject(), buffer.Length, stride);
            }
            catch (Exception)
            {
                return null;
            }
            finally
            {
                handle.Free();
            }

            // BGRA or RGBA: green sits in the middle either way, and red and blue weigh alike here
            var light = new bool[px.Width * px.Height];
            for (int i = 0, p = 0; i < light.Length; i++, p += 4)
                light[i] = buffer[p] + (2 * buffer[p + 1]) + buffer[p + 2] > 4 * 140;
            return light;
        }
    }
}
