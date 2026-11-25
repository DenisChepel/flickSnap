using System;
using System.Collections.Generic;
using System.Linq;
using Avalonia.Input;
using FlickSnap;
using FlickSnap.Config;
using FlickSnap.Interop.Testing;
using FlickSnap.UI.DrawOnScreen;
using Xunit;

namespace FlickSnap.VideoSDK.Tests
{
    /// <summary>
    /// The draw-on-screen hotkey table (issue #110): every tile has a key, no two bindings collide, the
    /// shared tools keep the image editor's letters, and an auto-repeat never re-runs a toggle.
    /// </summary>
    public class DrawHotkeysTests
    {
        [Fact]
        public void Tools_take_the_image_editor_letters_and_S_X_K_for_select_eraser_laser()
        {
            var expected = new Dictionary<ToolType, Key>
            {
                [ToolType.Pointer] = Key.S,
                [ToolType.Brush] = Key.B,
                [ToolType.Arrow] = Key.A,
                [ToolType.Line] = Key.L,
                [ToolType.Rectangle] = Key.R,
                [ToolType.Ellipse] = Key.E,
                [ToolType.Text] = Key.T,
                [ToolType.Count] = Key.C,
                [ToolType.Eraser] = Key.X,
                [ToolType.MeasureArea] = Key.M,
                [ToolType.Laser] = Key.K,
            };

            foreach (var tool in DrawPalette.Tools)
                Assert.Equal(new SimpleKeyGesture(expected[tool]), DrawHotkeys.Tool(tool));
        }

        [Fact]
        public void No_two_bindings_share_a_gesture()
        {
            var gestures = DrawHotkeys.Bindings.Select(b => b.Gesture.ForApp()).ToList();
            Assert.Equal(gestures.Count, gestures.Distinct().Count());
        }

        [Fact]
        public void Every_tile_runs_its_command()
        {
            AssertRuns(DrawHotkeys.Hide, "ToggleHide");
            AssertRuns(DrawHotkeys.Undo, "Undo");
            AssertRuns(DrawHotkeys.Redo, "Redo");
            AssertRuns(DrawHotkeys.ClearAll, "ClearAll");
            AssertRuns(DrawHotkeys.Close, "Close");
            AssertRuns(new SimpleKeyGesture(Key.Delete), "DeleteSelection");
            AssertRuns(new SimpleKeyGesture(Key.Back), "DeleteSelection");

            foreach (var tool in DrawPalette.Tools)
                AssertRuns(DrawHotkeys.Tool(tool), "PickTool " + tool);
            for (var i = 0; i < DrawPalette.Sizes.Count; i++)
                AssertRuns(DrawHotkeys.Size(i), "SelectSize " + i);
            for (var i = 0; i < DrawPalette.Colors.Count; i++)
                AssertRuns(DrawHotkeys.Color(i), "SelectColor " + i);
        }

        [Fact]
        public void Modifiers_must_match_exactly()
        {
            var controller = new RecordingController();
            var hotkeys = new DrawHotkeys(controller);

            // Ctrl+B is nobody's, and neither is Alt+1
            Assert.False(hotkeys.KeyDown(Key.B, AppControl));
            Assert.False(hotkeys.KeyDown(Key.D1, KeyModifiers.Alt));
            Assert.Empty(controller.Calls);
        }

        [Fact]
        public void A_repeat_reruns_undo_but_not_a_toggle()
        {
            var controller = new RecordingController();
            var hotkeys = new DrawHotkeys(controller);

            Assert.True(hotkeys.KeyDown(Key.H, KeyModifiers.None));
            Assert.True(hotkeys.KeyDown(Key.H, KeyModifiers.None)); // swallowed, not run
            hotkeys.KeyUp(Key.H);
            Assert.True(hotkeys.KeyDown(Key.H, KeyModifiers.None));
            Assert.Equal(new[] { "ToggleHide", "ToggleHide" }, controller.Calls);

            controller.Calls.Clear();
            Assert.True(hotkeys.KeyDown(Key.Z, AppControl));
            Assert.True(hotkeys.KeyDown(Key.Z, AppControl));
            Assert.Equal(new[] { "Undo", "Undo" }, controller.Calls);
        }

        [Fact]
        public void Reset_forgets_held_keys()
        {
            var controller = new RecordingController();
            var hotkeys = new DrawHotkeys(controller);

            hotkeys.KeyDown(Key.H, KeyModifiers.None);
            hotkeys.Reset(); // the key-up went to another window
            hotkeys.KeyDown(Key.H, KeyModifiers.None);
            Assert.Equal(new[] { "ToggleHide", "ToggleHide" }, controller.Calls);
        }

        [Fact]
        public void Labels_read_as_the_keys_are_printed()
        {
            InteropTestGate.Require();
            var ctrl = OperatingSystem.IsMacOS() ? "Cmd" : "Ctrl";
            Assert.Equal("Esc", DrawHotkeys.Hint(DrawHotkeys.Escape));
            Assert.Equal("B", DrawHotkeys.Hint(DrawHotkeys.Tool(ToolType.Brush)));
            Assert.Equal("1", DrawHotkeys.Label(DrawHotkeys.Size(0)));
            Assert.Equal("Shift+8", DrawHotkeys.Label(DrawHotkeys.Color(7)));
            Assert.Equal(ctrl + "+Z", DrawHotkeys.Label(DrawHotkeys.Undo));
            Assert.Equal(ctrl + "+Del", DrawHotkeys.Label(DrawHotkeys.ClearAll));
        }

        // Control, or ⌘ on macOS, where the table reads Control as Meta
        private static KeyModifiers AppControl => OperatingSystem.IsMacOS() ? KeyModifiers.Meta : KeyModifiers.Control;

        [Fact]
        public void Z_toggles_the_zoom()
        {
        }

        [Fact]
        public void Space_is_held_to_pan_its_repeats_ignored_and_a_reset_keeps_it_held()
        {
            var controller = new RecordingController();
            var hotkeys = new DrawHotkeys(controller);

            Assert.True(hotkeys.KeyDown(Key.Space, KeyModifiers.None));
            Assert.True(hotkeys.KeyDown(Key.Space, KeyModifiers.None)); // auto-repeat
            hotkeys.KeyUp(Key.Space);
            hotkeys.KeyDown(Key.Space, KeyModifiers.None);
            hotkeys.Reset(); // the keyboard moved to another of the session's windows: still held
            Assert.Equal(new[] { "SetPanKey True", "SetPanKey False", "SetPanKey True" }, controller.Calls);

            // the up lands on whichever window has the keyboard, which never saw the down
            new DrawHotkeys(controller).KeyUp(Key.Space);
            Assert.Equal("SetPanKey False", controller.Calls[^1]);
        }

        [Fact]
        public void The_grid_uses_the_setting_at_each_zoom_and_G_is_temporary()
        {
            var defaults = new FlickSnap.Config.SettingsDrawOnScreen();
            Assert.False(defaults.ShowPixelGridByDefault);
            var zoomed = DrawOnScreenState.Initial.ZoomBy(1, defaults.ShowPixelGridByDefault);
            Assert.False(zoomed.ShowGrid);

            var controller = new RecordingController { State = zoomed };
            new DrawHotkeys(controller).KeyDown(Key.G, KeyModifiers.None);
            Assert.Equal(new[] { "ToggleGrid" }, controller.Calls);

            var withGrid = zoomed.ToggleGrid();
            Assert.True(withGrid.ShowGrid);
            Assert.False(withGrid.ResetZoom().ZoomBy(1).ShowGrid);
            Assert.True(zoomed.ResetZoom().ZoomBy(1, showGridByDefault: true).ShowGrid);
        }

        [Fact]
        public void A_reassigned_grid_key_only_runs_while_zoomed()
        {
            var controller = new RecordingController();
            var shortcuts = GestureList.Of(new HotkeyGesture(HotkeyKey.F7, HotkeyModifiers.Shift));
            var hotkeys = new DrawHotkeys(controller, () => shortcuts);
            Assert.False(hotkeys.KeyDown(Key.F7, KeyModifiers.Shift));
            hotkeys.KeyUp(Key.F7);

            controller.State = controller.State.ZoomBy(1);
            Assert.False(hotkeys.KeyDown(Key.G, KeyModifiers.None));
            Assert.True(hotkeys.KeyDown(Key.F7, KeyModifiers.Shift));
            Assert.True(hotkeys.KeyDown(Key.F7, KeyModifiers.Shift));
            Assert.Equal(new[] { "ToggleGrid" }, controller.Calls);

            shortcuts = GestureList.Empty;
            hotkeys.KeyUp(Key.F7);
            Assert.False(hotkeys.KeyDown(Key.F7, KeyModifiers.Shift));
        }

        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public void The_eyedropper_runs_from_default_and_reassigned_shortcuts(bool frozen)
        {
            var state = DrawOnScreenState.Initial;
            var controller = new RecordingController { State = frozen ? state.Freeze() : state };
            var shortcuts = new SettingsHotkey().EyedropperShortcut;
            var hotkeys = new DrawHotkeys(controller, readEyedropperShortcuts: () => shortcuts);

            Assert.True(hotkeys.KeyDown(Key.F, KeyModifiers.None));
            Assert.True(hotkeys.KeyDown(Key.F, KeyModifiers.None));
            hotkeys.KeyUp(Key.F);
            Assert.Equal("ActivateEyedropper", Assert.Single(controller.Calls));

            shortcuts = GestureList.Of(new HotkeyGesture(HotkeyKey.F7, HotkeyModifiers.Shift));
            Assert.False(hotkeys.KeyDown(Key.F, KeyModifiers.None));
            Assert.False(hotkeys.KeyDown(Key.F7, KeyModifiers.None));
            hotkeys.KeyUp(Key.F7);
            Assert.True(hotkeys.KeyDown(Key.F7, KeyModifiers.Shift));
            Assert.Equal(2, controller.Calls.Count);

            shortcuts = GestureList.Empty;
            hotkeys.KeyUp(Key.F7);
            Assert.False(hotkeys.KeyDown(Key.F7, KeyModifiers.Shift));
        }

        [Fact]
        public void A_saved_grid_binding_to_f_keeps_precedence_in_zoom()
        {
            var controller = new RecordingController { State = DrawOnScreenState.Initial.ZoomBy(1) };
            var hotkeys = new DrawHotkeys(controller, () => SettingsHotkey.DefaultEyedropperShortcuts,
                () => SettingsHotkey.DefaultEyedropperShortcuts);

            Assert.True(hotkeys.KeyDown(Key.F, KeyModifiers.None));
            Assert.Equal("ToggleGrid", Assert.Single(controller.Calls));
        }

        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public void Escape_routes_each_press_to_the_session_and_ignores_auto_repeat(bool frozen)
        {
            var initial = DrawOnScreenState.Initial;
            var controller = new RecordingController { State = frozen ? initial.Freeze() : initial };
            var hotkeys = new DrawHotkeys(controller);
            Assert.True(hotkeys.KeyDown(Key.Escape, KeyModifiers.None));
            Assert.True(hotkeys.KeyDown(Key.Escape, KeyModifiers.None));
            hotkeys.KeyUp(Key.Escape);
            Assert.True(hotkeys.KeyDown(Key.Escape, KeyModifiers.None));
            Assert.Equal(new[] { "HandleEscape", "HandleEscape" }, controller.Calls);
        }

        private static void AssertRuns(SimpleKeyGesture gesture, string call)
        {
            var controller = new RecordingController();
            var app = gesture.ForApp();
            Assert.True(new DrawHotkeys(controller).KeyDown(app.Key, app.Modifiers), $"{gesture} is not bound");
            Assert.Equal(new[] { call }, controller.Calls);
        }

        private sealed class RecordingController : IDrawOnScreenController
        {
            public List<string> Calls { get; } = new();
            public DrawOnScreenState State { get; set; } = DrawOnScreenState.Initial;
            public Avalonia.Media.Color PaletteColor => State.DrawingColor;
            public bool IsEngaged => true;
            public bool CanUndo => true;
            public bool CanClear => true;
            public event EventHandler Changed { add { } remove { } }
            public void PickTool(ToolType tool) => Calls.Add("PickTool " + tool);
            public void HandleEscape() => Calls.Add("HandleEscape");
            public void ToggleHide() => Calls.Add("ToggleHide");
            public void SetPanKey(bool held) => Calls.Add("SetPanKey " + held);
            public void ToggleGrid() => Calls.Add("ToggleGrid");
            public void ActivateEyedropper() => Calls.Add("ActivateEyedropper");
            public void SelectColor(int index) => Calls.Add("SelectColor " + index);
            public void SelectCustomColor(Avalonia.Media.Color color) => Calls.Add("SelectCustomColor " + color);
            public void SelectSize(int index) => Calls.Add("SelectSize " + index);
            public void Undo() => Calls.Add("Undo");

            public void Redo() => Calls.Add("Redo");
            public void ClearAll() => Calls.Add("ClearAll");
            public void DeleteSelection() => Calls.Add("DeleteSelection");
            public void Close() => Calls.Add("Close");
        }
    }
}
