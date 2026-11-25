using FlickSnap.Drawing.Laser;

namespace FlickSnap.Drawing.Tools
{
    /// <summary>
    /// The laser pointer: press and drag to leave a trail that fades out by itself. It
    /// draws into the canvas's <see cref="LaserLayer"/>, never into the document — nothing is
    /// added to the graphics, the history or an export, and the selection is left alone. The tool
    /// stays in hand after a gesture on every surface: pointing is done over and over.
    /// </summary>
    internal sealed class ToolLaser : ToolBase
    {
        private const int CursorSizeDip = 28;

        public ToolLaser() : base(() => LaserCursor.Get(CursorSizeDip))
        { }

        public override void SetCursor(DrawingCanvas canvas)
        {
            // Convert the screen size to native cursor pixels independently of canvas zoom.
            double perDip = BrushCursor.PixelsPerUnit(canvas) * canvas.DipsToUnits;
            canvas.Cursor = LaserCursor.Get(CursorSizeDip * perDip);
        }

        public override void OnMouseDown(DrawingCanvas canvas, PointerState s, int clickCount)
        {
            if (!s.LeftPressed)
                return;

            canvas.CaptureMouse(s.Pointer);
            canvas.LaserLayer.BeginTrail(s.Position);
        }

        public override void OnMouseMove(DrawingCanvas canvas, PointerState s) =>
            canvas.LaserLayer.AddPoint(s.Position);

        public override void OnMouseUp(DrawingCanvas canvas, PointerState s)
        {
            canvas.LaserLayer.EndTrail();
            canvas.ReleaseMouseCapture();
        }

        public override void AbortOperation(DrawingCanvas canvas) => canvas.LaserLayer.EndTrail();
    }
}
