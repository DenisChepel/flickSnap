using System;
using System.Collections.Generic;
using Avalonia.Input;
using FlickSnap.Config;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>
    /// The draw-on-screen hotkeys (issue #110), live while the session has the keyboard. Every
    /// tile on the toolbar has one: the primary lane's are single keys hinted on the tiles (and named
    /// in their tooltips), matching the image editor's tool letters where the tools are shared; the
    /// secondary lane's are named in the tooltips only. Delete removes the select tool's selection,
    /// which has no tile.
    /// <para>
    /// The gestures are written the Windows way; on macOS Control is read as ⌘ (<see cref="SimpleKeyGesture.ForApp"/>).
    /// An instance routes one window's key edges onto an <see cref="IDrawOnScreenController"/>; it
    /// tracks the keys held so an auto-repeat only repeats what is safe to (Undo and Redo), never a toggle.
    /// </para>
    /// </summary>
    public sealed class DrawHotkeys
    {
        public static SimpleKeyGesture Escape { get; } = new(Key.Escape);
        public static SimpleKeyGesture Hide { get; } = new(Key.H);
        public static SimpleKeyGesture Undo { get; } = new(Key.Z, KeyModifiers.Control);
        public static SimpleKeyGesture Redo { get; } = new(Key.Z, KeyModifiers.Control | KeyModifiers.Shift);
        public static SimpleKeyGesture ClearAll { get; } = new(Key.Delete, KeyModifiers.Control);
        public static SimpleKeyGesture Close { get; } = new(Key.W, KeyModifiers.Control);

        /// <summary>The tool letters. Shared tools take the image editor's (its ToolRegistry); the select
        /// tool is S, and the eraser and the laser, which the editor lacks, are X and K.</summary>
        public static SimpleKeyGesture Tool(ToolType tool) => new(tool switch
        {
            ToolType.Pointer => Key.S,
            ToolType.Brush => Key.B,
            ToolType.Arrow => Key.A,
            ToolType.Line => Key.L,
            ToolType.Rectangle => Key.R,
            ToolType.Ellipse => Key.E,
            ToolType.Text => Key.T,
            ToolType.Count => Key.C,
            ToolType.Eraser => Key.X,
            ToolType.MeasureArea => Key.M,
            ToolType.Laser => Key.K, // Excalidraw's laser key
            _ => throw new ArgumentOutOfRangeException(nameof(tool), tool, "Not a draw-on-screen tool."),
        });

        /// <summary>1–4, thinnest first.</summary>
        public static SimpleKeyGesture Size(int index) => new(Key.D1 + index);

        /// <summary>Shift+1–8, in palette order.</summary>
        public static SimpleKeyGesture Color(int index) => new(Key.D1 + index, KeyModifiers.Shift);

        /// <summary>Every binding, aliases included: Delete and Backspace both delete the selection (a Mac's
        /// delete key is Backspace), and on macOS ⌘Backspace also clears all, as ⌘Delete is a key few
        /// Mac keyboards have.</summary>
        public static IReadOnlyList<(SimpleKeyGesture Gesture, Action<IDrawOnScreenController> Run)> Bindings { get; } = BuildBindings();

        private readonly IDrawOnScreenController _controller;
        private readonly HashSet<Key> _held = new();

        private readonly Func<GestureList> _readGridShortcuts;
        private readonly Func<GestureList> _readEyedropperShortcuts;

        public DrawHotkeys(IDrawOnScreenController controller, Func<GestureList> readGridShortcuts = null,
            Func<GestureList> readEyedropperShortcuts = null)
        {
            _controller = controller ?? throw new ArgumentNullException(nameof(controller));
            _readGridShortcuts = readGridShortcuts ?? (() => SettingsRoot.Current?.Hotkeys?.ZoomGridShortcut
                ?? SettingsHotkey.DefaultZoomGridShortcuts);
            _readEyedropperShortcuts = readEyedropperShortcuts ?? ReadEyedropperShortcuts;
        }

        /// <summary>A key went down. Returns whether a hotkey took it. An auto-repeat of a hotkey is
        /// taken (and swallowed) without running again, except for Undo and Redo.</summary>
        public bool KeyDown(Key key, KeyModifiers modifiers)
        {
            var repeat = !_held.Add(key);

            // the pan key is held, not pressed: its down and up both matter, its repeats do not
            if (key == PanKey)
            {
                if (!repeat)
                    _controller.SetPanKey(true);
                return true;
            }

            if (Find(key, modifiers) is not { } run)
            {
                if ((_controller.State.Zoomed || _controller.State.Frozen) &&
                    MatchesConfigured(_readGridShortcuts(), key, modifiers))
                {
                    if (!repeat)
                        _controller.ToggleGrid();
                    return true;
                }

                if (!MatchesConfigured(_readEyedropperShortcuts(), key, modifiers))
                    return false;

                if (!repeat)
                    _controller.ActivateEyedropper();
                return true;
            }

            if (!repeat || Matches(Undo, key, modifiers) || Matches(Redo, key, modifiers))
                run(_controller);
            return true;
        }

        public void KeyUp(Key key)
        {
            // the pan key's up counts wherever its down went: the keyboard can move between the
            // session's windows (toolbar, canvases) while it is held
            _held.Remove(key);
            if (key == PanKey)
                _controller.SetPanKey(false);
        }

        /// <summary>Forgets the held keys, for when the window loses the keyboard: their key-ups go
        /// elsewhere. A held pan key is not let go here: the keyboard may only have moved to another of
        /// the session's windows, which keeps the hand up; the session lets it go when the keyboard
        /// leaves the session.</summary>
        public void Reset() => _held.Clear();

        /// <summary>Held while zoomed in, the picture moves with the mouse.</summary>
        public static Key PanKey => Key.Space;

        /// <summary>The command bound to <paramref name="key"/> with exactly <paramref name="modifiers"/>, or null.</summary>
        public static Action<IDrawOnScreenController> Find(Key key, KeyModifiers modifiers)
        {
            foreach (var (gesture, run) in Bindings)
            {
                if (Matches(gesture, key, modifiers))
                    return run;
            }

            return null;
        }

        internal static bool CanUseForLocalShortcut(HotkeyGesture gesture) =>
            gesture.ToKeyGestureOrNull() is { } local && local.Key != PanKey &&
            Find(local.Key, local.KeyModifiers) == null;

        internal static GestureList ReadEyedropperShortcuts() =>
            SettingsRoot.Current?.Hotkeys?.EyedropperShortcut ?? SettingsHotkey.DefaultEyedropperShortcuts;

        internal static bool IsEyedropperGesture(Key key, KeyModifiers modifiers) =>
            key != PanKey && Find(key, modifiers) == null && MatchesConfigured(ReadEyedropperShortcuts(), key, modifiers);

        private static bool MatchesConfigured(GestureList shortcuts, Key key, KeyModifiers modifiers)
        {
            // Settings record literal modifiers, including Control as distinct from Command.
            foreach (var shortcut in shortcuts)
            {
                if (shortcut.ToKeyGestureOrNull() is { } local && local.Key == key && local.KeyModifiers == modifiers)
                    return true;
            }

            return false;
        }

        /// <summary>A gesture as a tooltip names it: "Ctrl+Z" (⌘ as "Cmd" on macOS), "Shift+1".</summary>
        public static string Label(SimpleKeyGesture gesture) => gesture.ForApp().ToString();

        /// <summary>A gesture's key alone, as a tile's hint shows it: "B", "Esc".</summary>
        public static string Hint(SimpleKeyGesture gesture) => Label(new SimpleKeyGesture(gesture.Key));

        private static bool Matches(SimpleKeyGesture gesture, Key key, KeyModifiers modifiers)
        {
            var app = gesture.ForApp();
            const KeyModifiers relevant = KeyModifiers.Control | KeyModifiers.Shift | KeyModifiers.Alt | KeyModifiers.Meta;
            return app.Key == key && app.Modifiers == (modifiers & relevant);
        }

        private static List<(SimpleKeyGesture, Action<IDrawOnScreenController>)> BuildBindings()
        {
            var bindings = new List<(SimpleKeyGesture, Action<IDrawOnScreenController>)>
            {
                (Escape, c => c.HandleEscape()),
                (Hide, c => c.ToggleHide()),
                (Undo, c => c.Undo()),
                (Redo, c => c.Redo()),
                (ClearAll, c => c.ClearAll()),
                (Close, c => c.Close()),
                (new SimpleKeyGesture(Key.Delete), c => c.DeleteSelection()),
                (new SimpleKeyGesture(Key.Back), c => c.DeleteSelection()),
            };

            if (OperatingSystem.IsMacOS())
                bindings.Add((new SimpleKeyGesture(Key.Back, KeyModifiers.Control), c => c.ClearAll()));

            foreach (var tool in DrawPalette.Tools)
                bindings.Add((Tool(tool), c => c.PickTool(tool)));

            for (var i = 0; i < DrawPalette.Sizes.Count; i++)
            {
                var index = i;
                bindings.Add((Size(index), c => c.SelectSize(index)));
            }

            for (var i = 0; i < DrawPalette.Colors.Count; i++)
            {
                var index = i;
                bindings.Add((Color(index), c => c.SelectColor(index)));
            }

            return bindings;
        }
    }
}
