using FlickSnap.UI.DrawOnScreen;
using Xunit;

namespace FlickSnap.VideoSDK.Tests
{
    public class DrawOnScreenStateTests
    {
        private static readonly DrawOnScreenState Drawing = DrawOnScreenState.Initial.PickTool(ToolType.Brush);
        private static readonly DrawOnScreenState Hidden = Drawing with { Hidden = true };

        [Fact]
        public void Initial_is_red_thin_brush()
        {
            Assert.Equal(new DrawOnScreenState(ToolType.Brush, false, 0, 0), DrawOnScreenState.Initial);
        }

        [Fact]
        public void Restored_is_initial_with_the_remembered_colour_and_size()
        {
            Assert.Equal(DrawOnScreenState.Initial, DrawOnScreenState.Restored(null, null));
            Assert.Equal(DrawOnScreenState.Initial with { ColorIndex = 4, SizeIndex = 3 }, DrawOnScreenState.Restored(4, 3));
        }

        [Fact]
        public void Restored_clamps_out_of_range_indices()
        {
            var s = DrawOnScreenState.Restored(99, -1);
            Assert.Equal(DrawPalette.Colors.Count - 1, s.ColorIndex);
            Assert.Equal(0, s.SizeIndex);
        }

        [Fact]
        public void PickTool_resumes_drawing_from_any_mode()
        {
            foreach (var from in new[] { Drawing, Hidden })
            {
                var s = from.PickTool(ToolType.Rectangle);
                Assert.Equal(ToolType.Rectangle, s.Tool);
                Assert.False(s.Hidden);
            }
        }

        [Fact]
        public void PickTool_keeps_colour_and_size()
        {
            var s = (Drawing with { ColorIndex = 4, SizeIndex = 3 }).PickTool(ToolType.Eraser);
            Assert.Equal(4, s.ColorIndex);
            Assert.Equal(3, s.SizeIndex);
        }

        [Fact]
        public void Hiding_keeps_the_picked_tool()
        {
            Assert.Equal(Hidden, Drawing.ToggleHide());
        }

        [Fact]
        public void Unhiding_resumes_the_tool()
        {
            Assert.Equal(Drawing, Hidden.ToggleHide());
        }

        [Fact]
        public void AfterClear_shows_the_canvas_and_returns_to_select()
        {
            var select = Drawing.PickTool(ToolType.Pointer);
            Assert.Equal(select, Drawing.AfterClear());
            Assert.Equal(select, Hidden.AfterClear());
            Assert.Equal(select, Drawing.PickTool(ToolType.Arrow).AfterClear());
        }

        [Theory]
        [InlineData(-5, 0)]
        [InlineData(0, 0)]
        [InlineData(5, 5)]
        [InlineData(7, 7)]
        [InlineData(8, 7)]
        [InlineData(100, 7)]
        public void SelectColor_clamps(int index, int expected)
        {
            var s = Drawing.SelectColor(index);
            Assert.Equal(expected, s.ColorIndex);
            Assert.Equal(Drawing with { ColorIndex = expected }, s);
        }

        [Theory]
        [InlineData(-1, 0)]
        [InlineData(2, 2)]
        [InlineData(3, 3)]
        [InlineData(4, 3)]
        public void SelectSize_clamps(int index, int expected)
        {
            var s = Drawing.SelectSize(index);
            Assert.Equal(expected, s.SizeIndex);
            Assert.Equal(Drawing with { SizeIndex = expected }, s);
        }

        [Fact]
        public void Size_changes_keep_select_active_while_editing_a_selection()
        {
            var select = Drawing.PickTool(ToolType.Pointer);
            Assert.Equal(select with { SizeIndex = 2 }, select.SelectSize(2, hasSelection: true));
        }

        [Fact]
        public void Preset_colour_or_size_takes_up_the_brush_from_hidden_select_or_eraser()
        {
            var eraser = Drawing.PickTool(ToolType.Eraser);
            var select = Drawing.PickTool(ToolType.Pointer);
            foreach (var from in new[] { Hidden, eraser, select })
            {
                foreach (var s in new[] { from.SelectColor(3), from.SelectSize(2) })
                {
                    Assert.Equal(ToolType.Brush, s.Tool);
                    Assert.False(s.Hidden);
                }
            }
        }

        [Fact]
        public void Custom_color_keeps_select_eraser_and_the_pan_hand_in_place()
        {
            var select = Drawing.PickTool(ToolType.Pointer);
            foreach (var state in new[] { select, select.ZoomBy(1).HoldHand(true), Drawing.PickTool(ToolType.Eraser), Hidden })
            {
                var updated = state.SelectCustomColor(Avalonia.Media.Colors.Blue);
                Assert.Equal(state.Tool, updated.Tool);
                Assert.Equal(state.Hidden, updated.Hidden);
                Assert.Equal(state.Hand, updated.Hand);
                Assert.Equal(Avalonia.Media.Colors.Blue, updated.DrawingColor);
            }
        }

        [Fact]
        public void Colour_or_size_keeps_a_drawing_tool_in_hand()
        {
            var text = Drawing.PickTool(ToolType.Text);
            Assert.Equal(text with { ColorIndex = 4 }, text.SelectColor(4));
            Assert.Equal(text with { SizeIndex = 3 }, text.SelectSize(3));
        }

        [Fact]
        public void IsToolLit_only_for_the_picked_tool_while_drawing()
        {
            Assert.True(Drawing.IsToolLit(ToolType.Brush));
            Assert.False(Drawing.IsToolLit(ToolType.Arrow));
            Assert.False(Hidden.IsToolLit(ToolType.Brush));
            Assert.False((Drawing with { Hidden = true }).IsToolLit(ToolType.Brush));
        }

        [Fact]
        public void Freeze_opens_with_the_remembered_drawing_tool()
        {
            Assert.Equal(DrawPalette.DefaultTool, DrawOnScreenState.Initial.Freeze().Tool);
            Assert.Equal(ToolType.Arrow, DrawOnScreenState.Initial.Freeze(ToolType.Arrow).Tool);
            Assert.True(DrawOnScreenState.Initial.Freeze(ToolType.Arrow).IsToolLit(ToolType.Arrow));
            // nothing to select or erase yet, and a tool the toolbar does not offer cannot be lit
            Assert.Equal(DrawPalette.DefaultTool, DrawOnScreenState.Initial.Freeze(ToolType.Pointer).Tool);
            Assert.Equal(DrawPalette.DefaultTool, DrawOnScreenState.Initial.Freeze(ToolType.Eraser).Tool);
            Assert.Equal(DrawPalette.DefaultTool, DrawOnScreenState.Initial.Freeze(ToolType.Highlighter).Tool);
        }

        [Fact]
        public void Freeze_preserves_the_remembered_color_and_size()
        {
            var s = DrawOnScreenState.Restored(4, 3).Freeze();
            Assert.True(s.Frozen);
            Assert.Equal(4, s.ColorIndex);
            Assert.Equal(3, s.SizeIndex);
        }

        [Fact]
        public void Frozen_never_leaves_drawing()
        {
            var frozen = DrawOnScreenState.Initial.Freeze();
            foreach (var s in new[]
            {
                frozen.ToggleHide(), frozen.AfterClear(),
                frozen.PickTool(ToolType.Eraser), frozen.SelectColor(2), frozen.SelectSize(0),
            })
            {
                Assert.True(s.Frozen);
                Assert.False(s.Hidden);
            }
        }

        [Fact]
        public void Wheel_zoom_moves_in_small_steps_and_returns_to_live_at_one_hundred_percent()
        {
            var live = DrawOnScreenState.Initial.PickTool(ToolType.Arrow);
            var zoomed = live.ZoomBy(1);
            Assert.True(zoomed.Zoomed);
            Assert.Equal(1.10, zoomed.ZoomScale, precision: 2);
            Assert.Equal(1.21, zoomed.ZoomBy(1).ZoomScale, precision: 2);
            Assert.True(zoomed.IsToolLit(ToolType.Arrow));
            Assert.Equal(live, zoomed.ZoomBy(-1));
            Assert.Same(live, live.ZoomBy(-1));
            Assert.InRange(live.ZoomBy(0.5).ZoomScale, 1.01, 1.09);
            Assert.Equal(DrawOnScreenState.MaxLiveZoom, live.ZoomBy(1000).ZoomScale);
        }

        [Fact]
        public void Zooming_out_from_the_limit_skips_the_near_one_hundred_percent_step()
        {
            var live = DrawOnScreenState.Initial;
            var zoom = live.ZoomBy(1000);
            var previousScale = zoom.ZoomScale;
            for (var i = 0; i < 28; i++)
            {
                zoom = zoom.ZoomBy(-1);
                Assert.True(zoom.ZoomScale < previousScale);
                previousScale = zoom.ZoomScale;
            }

            Assert.Equal(live.ZoomBy(1).ZoomScale, zoom.ZoomScale);
            Assert.Equal(live, zoom.ZoomBy(-1));
            Assert.Equal(live, live.ZoomBy(0.75).ZoomBy(-0.65));
        }

        [Fact]
        public void Picking_a_tool_puts_the_hand_down_and_the_pan_key_holds_it_up()
        {
            var drawing = DrawOnScreenState.Initial.ZoomBy(1).PickTool(ToolType.Rectangle);
            Assert.False(drawing.Hand);
            Assert.True(drawing.IsToolLit(ToolType.Rectangle));

            var held = drawing.HoldHand(held: true);
            Assert.True(held.Hand);
            Assert.DoesNotContain(DrawPalette.Tools, held.IsToolLit);

            var released = held.HoldHand(held: false);
            Assert.False(released.Hand);
            Assert.True(released.IsToolLit(ToolType.Rectangle));

            Assert.False(released.ResetZoom().Hand);
        }

        [Fact]
        public void The_pan_key_does_nothing_off_zoom()
        {
            var s = DrawOnScreenState.Initial.PickTool(ToolType.Brush);
            Assert.Same(s, s.HoldHand(held: true));
        }

        [Fact]
        public void Hiding_leaves_zoom_and_drops_the_pan_hand()
        {
            var hidden = DrawOnScreenState.Initial.ZoomBy(1).ToggleHide();
            Assert.True(hidden.Hidden);
            Assert.False(hidden.Zoomed);
            Assert.False(hidden.Hand);
        }

        [Fact]
        public void Zoomed_keeps_drawing_through_tool_colour_undo_and_clear()
        {
            var zoomed = DrawOnScreenState.Initial.ZoomBy(1);
            foreach (var s in new[] { zoomed.AfterClear(), zoomed.PickTool(ToolType.MeasureArea), zoomed.SelectColor(3), zoomed.SelectSize(2) })
            {
                Assert.True(s.Zoomed);
            }
        }

        [Fact]
        public void Frozen_ignores_live_wheel_zoom()
        {
            var frozen = DrawOnScreenState.Initial.Freeze();
            Assert.Same(frozen, frozen.ZoomBy(1));
        }
    }
}
