using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Xml.Linq;
using Avalonia;
using Avalonia.Input;
using Avalonia.Media.Imaging;
using Avalonia.Platform;
using SkiaSharp;

namespace FlickSnap.Drawing.Laser
{
    /// <summary>The drawing toolbar's pointer glyph, outlined for contrast on the desktop.</summary>
    public static class LaserCursor
    {
        private static readonly Dictionary<int, Cursor> _cache = new();
        private const int GlyphSize = 24;
        private const int Padding = 2;
        private static readonly Point ToolbarSpark = new(4.25, 19.75);

        public static Cursor Get(double sizePx)
        {
            int size = Math.Clamp((int)Math.Round(sizePx), 16, 96);
            if (_cache.TryGetValue(size, out var cached))
                return cached;

            var pixels = Rasterize(size, out var hotspot);
            var bitmap = new WriteableBitmap(new PixelSize(size, size), new Vector(96, 96), PixelFormat.Bgra8888, AlphaFormat.Premul);
            using (var fb = bitmap.Lock())
            {
                for (int y = 0; y < size; y++)
                    Marshal.Copy(pixels, y * size * 4, fb.Address + y * fb.RowBytes, size * 4);
            }

            var cursor = new Cursor(bitmap, hotspot);
            _cache[size] = cursor;
            return cursor;
        }

        /// <summary>Rasterizes the 24-unit glyph to premultiplied BGRA.</summary>
        internal static byte[] Rasterize(int size, out PixelPoint hotspot)
        {
            using var source = typeof(LaserCursor).Assembly.GetManifestResourceStream("FlickSnap.Drawing.Laser.Cursor.svg");
            var root = XDocument.Load(source).Root;
            using var path = SKPath.ParseSvgPathData(root.Element(root.Name.Namespace + "path").Attribute("d").Value);
            using var bitmap = new SKBitmap(size, size, SKColorType.Bgra8888, SKAlphaType.Premul);
            using var canvas = new SKCanvas(bitmap);
            canvas.Clear(SKColors.Transparent);
            float scale = (float)size / (GlyphSize + 2 * Padding);
            canvas.Scale(scale);
            canvas.Translate(Padding, Padding + GlyphSize);
            canvas.Scale(1, -1); // The cursor points up-left, the toolbar glyph down-left.
            using var paint = new SKPaint
            {
                IsAntialias = true,
                Style = SKPaintStyle.Stroke,
                StrokeCap = SKStrokeCap.Round,
                StrokeJoin = SKStrokeJoin.Round,
                Color = SKColors.White,
                StrokeWidth = 2f,
            };
            canvas.DrawPath(path, paint);
            paint.Color = new SKColor(27, 27, 31);
            paint.Style = SKPaintStyle.Fill;
            canvas.DrawPath(path, paint);
            hotspot = new PixelPoint(
                (int)Math.Round((ToolbarSpark.X + Padding) * scale),
                (int)Math.Round((GlyphSize - ToolbarSpark.Y + Padding) * scale));
            return bitmap.Bytes;
        }
    }
}
