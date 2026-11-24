using System;
using System.Linq;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless.XUnit;
using Avalonia.Media;
using Avalonia.Threading;
using FlickSnap.Config;
using FlickSnap.Drawing.Graphics;
using Xunit;

namespace FlickSnap.Drawing.Tests;

public class GraphicTextWidthTests
{
    [AvaloniaFact]
    public void Pasted_text_wraps_at_the_default_width_in_edit_mode_and_after_commit()
    {
        SettingsRoot.Current ??= new SettingsRoot();
        var canvas = new DrawingCanvas { IsOverlayMode = true, Tool = ToolType.Text };
        var window = new Window { Width = 1200, Height = 600, Content = canvas };
        window.Show();
        try
        {
            var graphic = new GraphicText(canvas, new Point(20, 20)) { Body = "Short text" };
            canvas.GraphicsList.Add(graphic);
            var singleLineHeight = graphic.Bottom - graphic.Top;

            canvas.ToolText.CreateTextBox(graphic, canvas);
            Dispatcher.UIThread.RunJobs();
            var editor = Assert.Single(canvas.Children.OfType<TextBox>());
            editor.Text = string.Join(' ', Enumerable.Repeat("text wraps at a word boundary", 80));
            Dispatcher.UIThread.RunJobs();

            Assert.Equal(TextWrapping.Wrap, editor.TextWrapping);
            Assert.InRange(editor.Bounds.Width, 1, GraphicText.DefaultBoxWidth - graphic.Padding * 2);
            canvas.CommitTextEdit();
            Assert.False(graphic.Editing);
            Assert.InRange(graphic.Right - graphic.Left, 1, GraphicText.DefaultBoxWidth);
            Assert.True(graphic.Bottom - graphic.Top > singleLineHeight);
        }
        finally
        {
            window.Close();
        }
    }

    [AvaloniaTheory]
    [InlineData(2)]
    [InlineData(3)]
    public void Side_handle_respects_the_widest_word_keeps_its_opposite_edge_and_saves_width(int handle)
    {
        const string word = "uninterrupted";
        var text = new GraphicText(Colors.Transparent, 1, new Point(100, 100), 25, $"one {word} two three");
        var wordOnly = new GraphicText(Colors.Transparent, 1, new Point(), body: word);
        var fromLeft = handle == 2;
        Point Anchor() => text.ApplyRotation(new Point(fromLeft ? text.Right : text.Left, text.Top));
        var anchor = Anchor();

        text.MoveHandleTo(anchor, handle);
        Assert.Equal(Math.Ceiling(wordOnly.Right - wordOnly.Left), text.Right - text.Left, 6);
        Assert.Equal(anchor.X, Anchor().X, 6);
        Assert.Equal(anchor.Y, Anchor().Y, 6);
        var narrowHeight = text.Bottom - text.Top;

        var targetX = fromLeft ? text.Right - 1100 : text.Left + 1100;
        text.MoveHandleTo(text.ApplyRotation(new Point(targetX, text.Top)), handle);
        Assert.Equal(1100, text.Right - text.Left, 6);
        Assert.True(text.Bottom - text.Top < narrowHeight);

        var saved = GraphicsSerializer.SerializeToUtf8Bytes(new GraphicBase[] { text });
        var restored = Assert.IsType<GraphicText>(Assert.Single(GraphicsSerializer.DeserializeFromUtf8Bytes(saved)));
        restored.Normalize();
        Assert.Equal(1100, restored.Right - restored.Left, 6);
    }
}
