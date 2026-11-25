using System;
using System.Linq;
using Avalonia.Media;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>The selected tool, ink visibility and view state for a drawing session.</summary>
    public sealed record DrawOnScreenState(ToolType Tool, bool Hidden, int ColorIndex, int SizeIndex)
    {
        public static DrawOnScreenState Initial { get; } =
            new(DrawPalette.DefaultTool, false, DrawPalette.DefaultColorIndex, DrawPalette.DefaultSizeIndex);

        /// <summary><see cref="Initial"/> with a remembered colour and size (null for the default),
        /// clamped to the palette: the stored indices may predate a palette change.</summary>
        public static DrawOnScreenState Restored(int? colorIndex, int? sizeIndex, Color? customColor = null) => Initial with
        {
            ColorIndex = Math.Clamp(colorIndex ?? DrawPalette.DefaultColorIndex, 0, DrawPalette.Colors.Count - 1),
            SizeIndex = Math.Clamp(sizeIndex ?? DrawPalette.DefaultSizeIndex, 0, DrawPalette.Sizes.Count - 1),
            CustomColor = customColor,
        };

        public Color? CustomColor { get; private init; }

        public Color DrawingColor => CustomColor ?? DrawPalette.Colors[ColorIndex];

        /// <summary>Whether the canvases are drawn over a screenshot taken as the session opened (Draw
        /// on Instant Screenshot, issue #111) rather than over the live desktop. Set once, by
        /// <see cref="Freeze"/>. The screenshot stays visible throughout the session.</summary>
        public bool Frozen { get; init; }

        /// <summary>A frozen session opens with the tool in hand: the picture is there to be drawn on.
        /// <paramref name="lastTool"/> is the last frozen session's tool (null for the default); one
        /// that is not a drawing tool on the toolbar falls back to the default.</summary>
        public DrawOnScreenState Freeze(ToolType? lastTool = null) => this with
        {
            Frozen = true, Hidden = false,
            Tool = lastTool is { } t && IsDrawingTool(t) ? t : DrawPalette.DefaultTool,
        };

        /// <summary>Whether <paramref name="tool"/> is a toolbar tool that puts ink down: not the
        /// select tool or the eraser, which have nothing to work on when a session opens, nor the
        /// laser, whose trail fades away (so picking a colour or size hands over the brush).</summary>
        public static bool IsDrawingTool(ToolType tool) =>
            tool is not (ToolType.Pointer or ToolType.Eraser or ToolType.Laser) && DrawPalette.Tools.Contains(tool);

        /// <summary>Whether the live monitor under the mouse is magnified along with its ink.
        /// Frozen sessions zoom separately with the wheel.</summary>
        public bool Zoomed => ZoomSteps > 0;

        public double ZoomScale => Math.Pow(MaxLiveZoom, ZoomSteps / MaxZoomSteps);

        internal const double MaxLiveZoom = 16;
        // Fit both endpoints to the wheel's step grid, with about 10% magnification per notch.
        private const double MaxZoomSteps = 29;
        private double ZoomSteps { get; init; }

        /// <summary>Whether the hand is in hand: on a zoomed screen, no drawing tool is active and the
        /// mouse only drags the picture. Picking a tool puts it down, and Space holds it up. Only while
        /// <see cref="Zoomed"/>.</summary>
        public bool Hand { get; init; }

        /// <summary>Whether the current zoom shows the pixel grid from 500% magnification.</summary>
        public bool ShowGrid { get; init; }

        public DrawOnScreenState ToggleGrid() => Zoomed || Frozen ? this with { ShowGrid = !ShowGrid } : this;

        /// <summary>Applies one wheel movement, including fractional trackpad deltas.</summary>
        public DrawOnScreenState ZoomBy(double wheelDelta, bool showGridByDefault = false)
        {
            if (Frozen || !double.IsFinite(wheelDelta) || wheelDelta == 0)
                return this;

            var steps = Math.Clamp(ZoomSteps + wheelDelta, 0, MaxZoomSteps);
            if (steps == 0 || (wheelDelta < 0 && steps < 0.5))
                return ResetZoom();
            if (steps == ZoomSteps)
                return this;

            return Zoomed ? this with { ZoomSteps = steps }
                : this with { ZoomSteps = steps, Hidden = false, ShowGrid = showGridByDefault };
        }

        public DrawOnScreenState ResetZoom() => Zoomed ? this with { ZoomSteps = 0, Hand = false } : this;

        /// <summary>Shows the hand while Space is held during live zoom.</summary>
        public DrawOnScreenState HoldHand(bool held) =>
            Zoomed && held != Hand ? this with { Hand = held } : this;

        /// <summary>Picking a tool shows the ink and lowers the pan hand.</summary>
        public DrawOnScreenState PickTool(ToolType tool) => this with { Tool = tool, Hidden = false, Hand = false };

        public DrawOnScreenState ToggleHide() =>
            Frozen ? this : Hidden ? this with { Hidden = false } : ResetZoom() with { Hidden = true };

        public DrawOnScreenState AfterClear() => Frozen || Zoomed ? this : PickTool(ToolType.Pointer);

        /// <summary>Picks a preset colour, keeping the tool in hand when editing a selection.</summary>
        public DrawOnScreenState SelectColor(int index, bool hasSelection = false) =>
            (hasSelection ? this : ResumeDrawing()) with
            {
                ColorIndex = Math.Clamp(index, 0, DrawPalette.Colors.Count - 1),
                CustomColor = null,
            };

        public DrawOnScreenState SelectCustomColor(Color color) => this with { CustomColor = color };

        /// <summary>Changes the size, keeping the tool in hand when editing a selection.</summary>
        public DrawOnScreenState SelectSize(int index, bool hasSelection = false) =>
            (hasSelection ? this : ResumeDrawing()) with { SizeIndex = Math.Clamp(index, 0, DrawPalette.Sizes.Count - 1) };

        private DrawOnScreenState ResumeDrawing() =>
            Hidden || !IsDrawingTool(Tool) ? PickTool(ToolType.Brush) : this;

        public bool IsToolLit(ToolType tool) => Tool == tool && !Hidden && !Hand;
    }
}
