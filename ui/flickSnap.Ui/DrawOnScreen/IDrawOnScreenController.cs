using System;
using Avalonia.Media;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>
    /// What the draw-on-screen toolbar drives. The session owns the canvases and the history; the
    /// toolbar only reads <see cref="State"/>, <see cref="IsEngaged"/>, <see cref="CanUndo"/> and
    /// <see cref="CanClear"/> and calls the commands. <see cref="Changed"/> fires after any of the
    /// readable members changes.
    /// </summary>
    public interface IDrawOnScreenController
    {
        DrawOnScreenState State { get; }

        Color PaletteColor { get; }

        /// <summary>Whether to show the toolbar's engaged appearance and shortcut hints.</summary>
        bool IsEngaged { get; }

        bool CanUndo { get; }
        bool CanClear { get; }
        event EventHandler Changed;
        void PickTool(ToolType tool);
        void HandleEscape();

        void ToggleHide();

        /// <summary>Turns a zoomed picture's pixel grid off or back on (G).</summary>
        void ToggleGrid();
        void ActivateEyedropper();

        /// <summary>The pan key (Space) went down or up: while held, a zoomed screen's picture moves
        /// with the mouse.</summary>
        void SetPanKey(bool held);
        void SelectColor(int index);
        void SelectCustomColor(Color color);
        void SelectSize(int index);
        void Undo();

        void Redo();
        void ClearAll();

        /// <summary>Deletes whatever the select tool has selected, on every canvas, as one undo step.</summary>
        void DeleteSelection();

        void Close();
    }
}
