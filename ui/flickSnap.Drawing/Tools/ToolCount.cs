using System.Globalization;
using System.Linq;
using Avalonia;
using FlickSnap.Drawing.Graphics;

namespace FlickSnap.Drawing.Tools
{
    /// <summary>
    /// Places the next numbered step badge centered on the press point; dragging before release
    /// pulls its arrow out to the pointer (a release inside the badge leaves it arrowless).
    /// </summary>
    internal sealed class ToolCount : ToolBase
    {
        private GraphicCount _current;

        public ToolCount() : base(() => CursorResources.Numerical, SnapMode.All)
        { }

        protected override void OnMouseDownImpl(DrawingCanvas canvas, Point pt)
        {
            var usedNumbers = (canvas.StepNumberScope?.Invoke() ?? canvas.GraphicsList)
                .OfType<GraphicCount>()
                .Select(g => int.TryParse(g.Body, NumberStyles.Integer, CultureInfo.InvariantCulture, out var number) ? number : 0)
                .ToHashSet();
            var next = 1;
            while (usedNumbers.Contains(next))
                next++;

            _current = new GraphicCount(canvas, pt, next.ToString(CultureInfo.InvariantCulture));
            canvas.GraphicsList.Add(_current);
        }

        protected override void OnMouseMoveImpl(DrawingCanvas canvas, Point pt)
        {
            _current?.MoveHandleTo(pt, GraphicCount.ArrowHandle);
        }

        protected override void OnMouseUpImpl(DrawingCanvas canvas)
        {
            if (_current == null)
                return;

            _current.IsSelected = !canvas.IsOverlayMode;
            _current = null;
            canvas.AddCommandToHistory(false);
        }

        public override void AbortOperation(DrawingCanvas canvas)
        {
            if (_current != null)
            {
                canvas.GraphicsList.Remove(_current);
                _current = null;
            }
        }
    }
}
