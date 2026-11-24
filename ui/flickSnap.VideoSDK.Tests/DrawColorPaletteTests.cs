using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Headless.XUnit;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Avalonia.Input.Raw;
using Avalonia.Markup.Xaml.Styling;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.Threading;
using Avalonia.VisualTree;
using FlickSnap.Config;
using FlickSnap.UI.Controls.Tray;
using FlickSnap.UI.Dialogs.ColorPicker;
using FlickSnap.UI.DrawOnScreen;
using Xunit;

namespace FlickSnap.VideoSDK.Tests;

[Collection(HeadlessApp.Collection)]
public class DrawColorPaletteTests
{
    [Theory]
    [InlineData(100, 110, 38, 175)]
    [InlineData(30, 40, 0, 105)]
    [InlineData(195, 40, 76, 105)]
    [InlineData(30, 285, 0, 196)]
    [InlineData(195, 285, 76, 196)]
    public void Eyedropper_hex_label_stays_on_screen_without_moving_the_cursor(int x, int y, int left, int top)
    {
        var position = DrawEyedropperWindow.PlaceLabel(new Point(x, y), new Size(200, 300), new Size(124, 24));
        Assert.Equal(new Point(left, top), position);
    }

    [Theory]
    [InlineData("#05aBf0")]
    [InlineData("05ABF0")]
    public void Hex_accepts_pasted_rgb_and_formats_it_canonically(string text)
    {
        Assert.True(DrawColorPaletteWindow.TryParseHex(text, out var color));
        Assert.Equal(Color.FromRgb(5, 171, 240), color);
        Assert.Equal("#05ABF0", DrawColorPaletteWindow.FormatHex(color));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("#")]
    [InlineData("#ABC")]
    [InlineData("##123456")]
    [InlineData("#FF123456")]
    [InlineData("#05AGF0")]
    [InlineData(" 05ABF0 ")]
    [InlineData("#123 45")]
    [InlineData("123456\n")]
    [InlineData("123456\nAB12CD")]
    [InlineData("#１２３４５６")]
    [InlineData("0x123456")]
    [InlineData("+12345")]
    [InlineData("12345\0")]
    public void Hex_rejects_incomplete_and_invalid_rgb(string text) =>
        Assert.False(DrawColorPaletteWindow.TryParseHex(text, out _));

    [AvaloniaFact]
    public void Enter_commits_only_valid_hex_and_restores_the_last_color_on_invalid_input()
    {
        var palette = CreatePalette();
        var changes = new List<Color>();
        palette.ColorPicked += changes.Add;
        palette.Show();
        try
        {
            Dispatcher.UIThread.RunJobs();
            var hex = Assert.Single(palette.GetVisualDescendants().OfType<TextBox>());
            var slider = Assert.Single(palette.GetVisualDescendants().OfType<ChannelSlider>());
            Assert.Contains(palette.GetVisualDescendants().OfType<TextBlock>(), text => text.Text == "Hex");
            Assert.Equal(slider.TranslatePoint(default, palette).Value.X, hex.TranslatePoint(default, palette).Value.X);
            Assert.Equal(slider.Bounds.Width, hex.Bounds.Width);
            Assert.Equal("#FF0000", hex.Text);
            Assert.True(hex.Focus());
            hex.Text = "321abc";
            Assert.Empty(changes);
            Assert.Equal(0, slider.Value);

            SendKey(hex, Key.Enter);
            Assert.Equal(Color.FromRgb(50, 26, 188), Assert.Single(changes));
            Assert.Equal("#321ABC", hex.Text);
            Assert.NotEqual(0, slider.Value);

            hex.Text = "#1F";
            SendKey(hex, Key.Enter);
            Assert.Equal("#321ABC", hex.Text);
            Assert.Single(changes);

            SendKey(hex, Key.Enter);
            Assert.Equal(2, changes.Count);
            Assert.Equal(changes[0], changes[1]);
        }
        finally
        {
            palette.Close();
        }
    }

    [AvaloniaFact]
    public void Eyedropper_applies_the_sample_even_when_it_matches_the_palettes_color()
    {
        var palette = CreatePalette();
        var changes = new List<Color>();
        palette.ColorPicked += changes.Add;

        palette.ApplyEyedropperColor(Colors.Red);

        Assert.Equal(Colors.Red, Assert.Single(changes));
    }

    [AvaloniaTheory]
    [InlineData("A1B2C3")]
    [InlineData("#Z11222")]
    public void Unconfirmed_hex_is_restored_on_escape_focus_change_and_clicking_away(string draft)
    {
        var palette = CreatePalette();
        var changes = new List<Color>();
        palette.ColorPicked += changes.Add;
        palette.Show();
        try
        {
            Dispatcher.UIThread.RunJobs();
            var hex = Assert.Single(palette.GetVisualDescendants().OfType<TextBox>());
            var label = palette.GetVisualDescendants().OfType<TextBlock>().Single(text => text.Text == "Hex");
            var slider = Assert.Single(palette.GetVisualDescendants().OfType<ChannelSlider>());

            Assert.True(hex.Focus());
            hex.Text = draft;
            SendKey(hex, Key.Escape);
            Assert.Equal("#FF0000", hex.Text);
            Assert.False(hex.IsFocused);
            Assert.True(palette.IsVisible);

            Assert.True(hex.Focus());
            hex.Text = draft;
            Assert.True(slider.Focus());
            Assert.Equal("#FF0000", hex.Text);

            Assert.True(hex.Focus());
            hex.Text = draft;
            var labelCenter = new Point(label.Bounds.Width / 2, label.Bounds.Height / 2);
            var labelPosition = label.TranslatePoint(labelCenter, palette).Value;
            palette.MouseDown(labelPosition, MouseButton.Left, RawInputModifiers.None);
            palette.MouseUp(labelPosition, MouseButton.Left, RawInputModifiers.None);
            Assert.Equal("#FF0000", hex.Text);
            Assert.False(hex.IsFocused);
            Assert.Empty(changes);
        }
        finally
        {
            palette.Close();
        }
    }

    [AvaloniaFact]
    public async Task Clipboard_drafts_preserve_extra_text_and_copy_the_confirmed_hex()
    {
        var palette = CreatePalette();
        var changes = new List<Color>();
        palette.ColorPicked += changes.Add;
        palette.Show();
        try
        {
            Dispatcher.UIThread.RunJobs();
            var hex = Assert.Single(palette.GetVisualDescendants().OfType<TextBox>());
            Assert.True(hex.Focus());
            hex.SelectAll();
            await palette.Clipboard.SetTextAsync("ABCDEF\n123456");
            hex.Paste();
            Dispatcher.UIThread.RunJobs();
            Assert.Equal("ABCDEF\n123456", hex.Text.Replace("\r\n", "\n"));
            SendKey(hex, Key.Enter);
            Assert.Equal("#FF0000", hex.Text);
            Assert.Empty(changes);

            await palette.Clipboard.SetTextAsync("05aBf0");
            hex.Paste();
            Dispatcher.UIThread.RunJobs();
            Assert.Equal("05aBf0", hex.Text);
            Assert.Empty(changes);
            SendKey(hex, Key.Enter);
            Assert.Equal(Color.FromRgb(5, 171, 240), Assert.Single(changes));
            hex.Copy();
            Dispatcher.UIThread.RunJobs();
            Assert.Equal("#05ABF0", await palette.Clipboard.TryGetTextAsync());
        }
        finally
        {
            palette.Close();
        }
    }

    private static void SendKey(Control control, Key key) =>
        control.RaiseEvent(new KeyEventArgs { RoutedEvent = InputElement.KeyDownEvent, Key = key });

    private static DrawColorPaletteWindow CreatePalette()
    {
        var app = Application.Current;
        app.Resources["IconDrawColorDropper"] = StreamGeometry.Parse("M0 0L12 12L0 12Z");
        // Headless gives each test a fresh application; the control's static constructor runs once.
        if (!app.TryGetResource(typeof(FloatingTray), ThemeVariant.Dark, out _))
        {
            var uri = new Uri("avares://flickSnap/Controls/Tray/FloatingTray.axaml");
            app.Resources.MergedDictionaries.Add(new ResourceInclude(uri) { Source = uri });
        }
        return new DrawColorPaletteWindow(Colors.Red, new Size(800, 600));
    }

    [AvaloniaFact]
    public void Palette_and_toolbar_frames_follow_the_same_live_accent()
    {
        TrayTokens.SetAccentResources(Application.Current.Resources, Colors.Blue, Colors.LightBlue);
        var palette = CreatePalette();
        var tray = new FloatingTray { IsEngaged = true, Width = 240, Height = 48 };
        var toolbar = new Window { Content = tray };
        palette.Show();
        toolbar.Show();
        try
        {
            Dispatcher.UIThread.RunJobs();
            var frame = Assert.IsType<FloatingTray>(palette.Content);
            var inset = frame.GetVisualDescendants().OfType<Border>().Single(b => b.Name == "_panel");
            Assert.Equal(new Thickness(TrayTokens.EngagedRing), inset.Margin);
            Check(Colors.Blue, Colors.LightBlue);
            TrayTokens.SetAccentResources(Application.Current.Resources, Colors.Purple, Colors.Pink);
            Dispatcher.UIThread.RunJobs();
            Check(Colors.Purple, Colors.Pink);

            void Check(Color fill, Color glow)
            {
                var paletteFrame = EngagedBorders(frame);
                var toolbarFrame = EngagedBorders(tray);
                Assert.Same(toolbarFrame[1].Background, paletteFrame[1].Background);
                Assert.Equal(fill, Assert.IsAssignableFrom<ISolidColorBrush>(paletteFrame[1].Background).Color);
                Assert.Equal(TrayTokens.EngagedGlow(glow), paletteFrame[0].BoxShadow);
                Assert.Equal(toolbarFrame[0].BoxShadow, paletteFrame[0].BoxShadow);
            }
        }
        finally
        {
            toolbar.Close();
            palette.Close();
        }

        static Border[] EngagedBorders(FloatingTray tray) =>
            tray.GetVisualDescendants().OfType<Panel>().Single(p => p.Name == "_engaged").Children.OfType<Border>().ToArray();
    }

    [AvaloniaFact]
    public void Hue_drag_reaches_both_ends_and_agrees_with_the_inset_gradient()
    {
        var palette = CreatePalette();
        palette.Show();
        try
        {
            Dispatcher.UIThread.RunJobs();
            var slider = Assert.Single(palette.GetVisualDescendants().OfType<ChannelSlider>());
            var spectrum = Assert.IsType<LinearGradientBrush>(slider.SliderBrush);
            var ring = Assert.IsAssignableFrom<IConicGradientBrush>(DrawColorPaletteWindow.HueRingBrush);
            Assert.Equal(spectrum.GradientStops.Select(stop => (stop.Offset, stop.Color)),
                ring.GradientStops.Select(stop => (stop.Offset, stop.Color)));
            var width = slider.Bounds.Width;
            Assert.Equal(new RelativePoint(8, 0, RelativeUnit.Absolute), spectrum.StartPoint);
            Assert.Equal(new RelativePoint(width - 8, 0, RelativeUnit.Absolute), spectrum.EndPoint);

            Point At(double x) => slider.TranslatePoint(new Point(x, slider.Bounds.Height / 2), palette).Value;
            palette.MouseDown(At(1), MouseButton.Left, RawInputModifiers.None);
            Assert.Equal(0, slider.Value);
            palette.MouseMove(At(width / 2), RawInputModifiers.LeftMouseButton);
            Assert.Equal(180, slider.Value, 6);
            var hex = Assert.Single(palette.GetVisualDescendants().OfType<TextBox>());
            Assert.Equal("#00FFFF", hex.Text);
            palette.MouseMove(At(width - 1), RawInputModifiers.LeftMouseButton);
            Assert.Equal(360, slider.Value);
            Assert.Equal("#FF0000", hex.Text);
            palette.MouseUp(At(width - 1), MouseButton.Left, RawInputModifiers.None);
        }
        finally
        {
            palette.Close();
        }
    }

    [Fact]
    public void Custom_color_is_saved_and_a_preset_replaces_it()
    {
        var color = Color.FromRgb(20, 56, 91);
        var path = Path.GetTempFileName();
        try
        {
            var settings = new SettingsRoot();
            settings.General.DrawOnScreenCustomColor = color;
            SettingsService.Save(settings, path);
            var restored = SettingsService.Load(path);
            var state = DrawOnScreenState.Restored(1, 2, customColor: restored.General.DrawOnScreenCustomColor);
            Assert.Equal(color, state.DrawingColor);
            Assert.Equal(color, state.SelectSize(1).DrawingColor);
            var preset = state.SelectColor(3);
            Assert.Null(preset.CustomColor);
            Assert.Equal(DrawPalette.Colors[3], preset.DrawingColor);
        }
        finally
        {
            File.Delete(path);
        }
    }

    [Theory]
    [InlineData(-500, 100, 250, -740, 100)] // left, aligned to the toolbar's top
    [InlineData(-500, 500, 250, -740, 360)] // left, moved up to fit the monitor
    [InlineData(-980, 40, 650, -320, 40)] // right when the left edge is blocked
    [InlineData(-980, 40, 950, -960, 110)] // below when neither side fits
    [InlineData(-980, 500, 950, -960, 200)] // above near the bottom
    public void Palette_chooses_a_side_and_stays_in_the_work_area(int x, int y, int width, int expectedX, int expectedY)
    {
        var at = DrawColorPaletteWindow.Place(new PixelRect(x, y, width, 60), new PixelPoint(x + 20, y + 20),
            new PixelRect(-1000, -50, 1000, 700), new PixelSize(230, 290), (int)DrawColorPaletteWindow.ToolbarGap);
        Assert.Equal(new PixelPoint(expectedX, expectedY), at);
    }

    [Fact]
    public void Palette_position_is_clamped_when_there_is_no_room_on_any_side()
    {
        var at = DrawColorPaletteWindow.Place(new PixelRect(-950, 100, 840, 440), new PixelPoint(-950, 200),
            new PixelRect(-1000, -50, 1000, 700), new PixelSize(230, 290), (int)DrawColorPaletteWindow.ToolbarGap);
        Assert.Equal(new PixelPoint(-230, 100), at);
    }
}
