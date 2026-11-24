using System;
using System.Collections.Generic;
using System.Globalization;
using Avalonia;
using Avalonia.Media;
using FlickSnap.Drawing.Rendering;

namespace FlickSnap.Drawing.Graphics
{
    /// <summary>
    /// An area measure (Draw on Screen's ruler): a rectangle outlined in the object color over a faint
    /// wash of it, with a capsule label under its bottom edge reading the size as "W × H px". The size
    /// is in pixels: on the draw-on-screen overlay, whose canvas units are the window's DIPs, the
    /// monitor's physical pixels (canvas units × the window's RenderScaling, whatever the overlay's
    /// zoom); in the image editor, whose canvas units are the image's pixels, those. Never rotates:
    /// the label reads the axis-aligned width and height, so the rotation handle is dropped.
    /// </summary>
    [GraphicDesc("Measure Area", Skills = Skill.Color)]
    public class GraphicMeasureArea : GraphicRectangle
    {
        // the label is a readout, not ink: its font, padding and gap stay fixed whatever the stroke
        private const string LabelFontName = EditorFonts.Text;
        private const double LabelFontSize = 15;
        private const double LabelPaddingX = 10;
        private const double LabelPaddingY = 4;
        private const double LabelGap = 8;

        private static readonly Color LabelBackColor = Color.FromArgb(0xE6, 0x1B, 0x1C, 0x24);
        private static readonly Color LabelBorderColor = Color.FromArgb(0x33, 0xFF, 0xFF, 0xFF);
        private static readonly Color LabelTextColor = Color.FromRgb(0xF1, 0xE9, 0xE0);
        private const double LabelBorderWidth = 1;

        protected GraphicMeasureArea()
        { }

        public GraphicMeasureArea(Color objectColor, double lineWidth, Rect rect)
            : base(objectColor, lineWidth, rect, 0, dropShadowEffect: false)
        { }

        // pixels per canvas unit: the overlay's RenderScaling, 1 in the editor and off-canvas. Not
        // document state — the canvas it is on decides (OnAttached, and again at a scaling change).
        [Transient] private double _pixelsPerUnit = 1;

        // device pixels per canvas unit as drawn (the canvas's ContentScale, zoom included): the
        // frame is a hairline one device pixel wide at any zoom
        [Transient] private double _devicePerUnit = 1;

        // the part of a move too small to reach the next pixel yet, carried to the next move
        [Transient] private Vector _moveRemainder;

        internal override void OnAttached(DrawingCanvas canvas)
        {
            canvas.KeepMarchingAnts();
            var device = canvas.ContentScale > 0 ? canvas.ContentScale : 1;
            var scale = canvas.IsOverlayMode ? canvas.DpiZoom : 1;
            if (scale == _pixelsPerUnit && device == _devicePerUnit)
                return;
            _pixelsPerUnit = scale;
            _devicePerUnit = device;
            RenderCache.Clear(InvalidationAspects.Bounds | InvalidationAspects.Geometry | InvalidationAspects.Text);
        }

        /// <summary>The marching ants' phase, in device pixels: the dashes creep along the frame as
        /// the canvas redraws (DrawingCanvas.KeepMarchingAnts).</summary>
        internal static double AntsPhase => (Environment.TickCount64 / 90) % (AntsDash * 2);

        private const double AntsDash = 4;

        // the frame takes whole pixels, like a selection: every edge sits on a pixel boundary
        private double Snap(double v) => Math.Round(v * _pixelsPerUnit) / _pixelsPerUnit;

        internal override void MoveHandleTo(Point point, int handleNumber)
        {
            base.MoveHandleTo(new Point(Snap(point.X), Snap(point.Y)), handleNumber);
            SnapEdges();
        }

        internal override void Move(double deltaX, double deltaY)
        {
            var want = _moveRemainder + new Vector(deltaX, deltaY);
            var step = new Vector(Snap(want.X), Snap(want.Y));
            _moveRemainder = want - step;
            if (step != default)
                base.Move(step.X, step.Y);
            SnapEdges();
        }

        private void SnapEdges()
        {
            var (l, t, r, b) = (Snap(Left), Snap(Top), Snap(Right), Snap(Bottom));
            if (l == Left && t == Top && r == Right && b == Bottom)
                return;
            Left = l;
            Top = t;
            Right = r;
            Bottom = b;
            OnPropertyChanged();
        }

        /// <summary>The measured width in whole pixels.</summary>
        public int MeasuredWidth => (int)Math.Round(Math.Abs(Right - Left) * _pixelsPerUnit);

        /// <summary>The measured height in whole pixels.</summary>
        public int MeasuredHeight => (int)Math.Round(Math.Abs(Bottom - Top) * _pixelsPerUnit);

        /// <summary>The label text, "W × H px".</summary>
        public string Label => FormatLabel(MeasuredWidth, MeasuredHeight);

        public static string FormatLabel(int width, int height) =>
            string.Format(CultureInfo.InvariantCulture, "{0} × {1} px", width, height);

        // no rotation handle (9): see the class summary
        internal override int HandleCount => 8;

        // the label string derives from the edges, so an edge change also drops the cached text
        internal override void DeclarePropertyEffects(Dictionary<string, InvalidationAspects> map)
        {
            base.DeclarePropertyEffects(map);
            const InvalidationAspects shape = InvalidationAspects.Bounds | InvalidationAspects.Geometry |
                                              InvalidationAspects.Text;
            map[nameof(Left)] = shape;
            map[nameof(Top)] = shape;
            map[nameof(Right)] = shape;
            map[nameof(Bottom)] = shape;
        }

        // the pill hangs below the frame, so it has to be inside the bounds (invalidation, export)
        protected override Rect ComputeBounds()
        {
            ComputeLabel(out _, out var pill);
            return base.ComputeBounds().Union(pill);
        }

        internal override void DrawRectangle(DrawingContext ctx)
        {
            var bounds = UnrotatedBounds;

            // marching ants: a one-device-pixel frame just inside the edges (on the measured pixels,
            // as a selection's), black dashes creeping over white
            var hair = 1 / _devicePerUnit;
            var frame = bounds.Width > hair && bounds.Height > hair ? bounds.Deflate(hair / 2) : bounds;
            ctx.DrawRectangle(null, new Pen(Brushes.White, hair), frame);
            ctx.DrawRectangle(null, new Pen(Brushes.Black, hair,
                new DashStyle(new[] { AntsDash, AntsDash }, AntsPhase)), frame);

            ComputeLabel(out var text, out var pill);
            var k = LabelScale;
            var outline = pill.Deflate(LabelBorderWidth * k / 2);
            var radius = outline.Height / 2; // a capsule
            ctx.DrawRectangle(RenderResources.GetBrush(LabelBackColor), RenderResources.GetPen(LabelBorderColor, LabelBorderWidth * k),
                              outline, radius, radius);
            ctx.DrawText(text, new Point(pill.X + (LabelPaddingX * k), pill.Y + (LabelPaddingY * k)));
        }

        // the label is a readout, not ink: it keeps its size on screen whatever the zoom, so canvas
        // units are scaled by the inverse of the zoom (1 at 1:1)
        private double LabelScale => _pixelsPerUnit / _devicePerUnit;

        // The pill, centred under the frame's bottom edge (clear of the stroke). Shared by
        // ComputeBounds and DrawRectangle so the two never disagree about where it sits.
        private void ComputeLabel(out FormattedText text, out Rect pill)
        {
            var k = LabelScale;
            text = GetLabelText(Label, k);
            var width = text.Width + (LabelPaddingX * 2 * k);
            var height = text.Height + (LabelPaddingY * 2 * k);
            var bounds = UnrotatedBounds;
            var centerX = bounds.Left + (bounds.Width / 2);
            var top = bounds.Bottom + (LabelGap * k);
            pill = new Rect(centerX - (width / 2), top, width, height);
        }

        // keyed on the label string and its scale: the font and colors are constant for this type
        private FormattedText GetLabelText(string label, double scale)
        {
            var key = label + "@" + scale.ToString("R", CultureInfo.InvariantCulture);
            if (RenderCache.Text is { } cached && key.Equals(RenderCache.TextKey))
                return cached;

            var form = new FormattedText(
                label,
                CultureInfo.InvariantCulture,
                FlowDirection.LeftToRight,
                new Typeface(FontUtil.CreateSafe(LabelFontName), FontStyle.Normal, FontWeight.SemiBold),
                LabelFontSize * scale,
                RenderResources.GetBrush(LabelTextColor));

            RenderCache.Text = form;
            RenderCache.TextKey = key;
            return form;
        }
    }
}
