using System;
using System.ComponentModel;
using System.ComponentModel.DataAnnotations;
using System.Text.Json.Serialization;

namespace FlickSnap.Config
{
    /// <summary>
    /// Whether Draw on Screen exists as a feature. Rendered by the Draw on Screen page as the tile
    /// selector at its top (<see cref="ModeSelectorAttribute"/>). Off removes every way to open it —
    /// the tray item, the hotkey and the recording and share strips' draw buttons — and hides the
    /// rest of the page. Persisted by name.
    /// </summary>
    public enum DrawOnScreenMode
    {
        [Description("On")]
        [ModeCaption("Draw over your screen from the tray menu, a hotkey, or while recording or sharing a region.")]
        [ModeIcon("IconToolPen")]
        On,

        [Description("Off")]
        [ModeCaption("Turns Draw on Screen off entirely.")]
        [ModeIcon("IconPower")]
        Off,
    }

    /// <summary>Settings for Draw on Screen and Draw on Instant Screenshot.</summary>
    public class SettingsDrawOnScreen : SimpleNotifyObject
    {
        /// <summary>The mode banner at the top of the page; Off hides the rest of the page and
        /// every Draw on Screen entry point in the app (see <see cref="DrawOnScreenMode"/>).</summary>
        [ModeSelector(Feature = "Draw on Screen")]
        public DrawOnScreenMode Mode
        {
            get => _mode;
            set => Set(ref _mode, value, nameof(Mode), nameof(IsEnabled));
        }

        /// <summary>Whether Draw on Screen is offered anywhere (<see cref="Mode"/> is On).</summary>
        [Browsable(false), JsonIgnore]
        public bool IsEnabled => Mode == DrawOnScreenMode.On;

        [Category("Zoom")]
        [VisibleWhen(nameof(Mode), DrawOnScreenMode.On)]
        [DisplayName("Show pixel grid by default")]
        [Description("Show the pixel grid from 500% zoom. The shortcut in Hotkeys temporarily toggles it.")]
        public bool ShowPixelGridByDefault
        {
            get => _showPixelGridByDefault;
            set => Set(ref _showPixelGridByDefault, value);
        }

        [Category("Instant screenshot")]
        [VisibleWhen(nameof(Mode), DrawOnScreenMode.On)]
        [DisplayName("Initial zoom")]
        [Description("How far Draw on Instant Screenshot zooms into the picture when it opens, centered on the mouse, in percent. Scroll the mouse wheel to zoom in or out.")]
        [Range(MinScreenshotZoom, MaxScreenshotZoom)]
        [SpinnerStep(ScreenshotZoomStep)]
        public int ScreenshotZoom
        {
            get => _screenshotZoom;
            set
            {
                // kept on the spinner's 10% grid; a typed value that had to move is re-announced so
                // the field shows where it landed
                var snapped = Math.Clamp((int)Math.Round(value / (double)ScreenshotZoomStep) * ScreenshotZoomStep,
                    MinScreenshotZoom, MaxScreenshotZoom);
                if (!Set(ref _screenshotZoom, snapped) && snapped != value)
                    OnPropertyChanged();
            }
        }

        private const int MinScreenshotZoom = 100;
        private const int MaxScreenshotZoom = 1600;
        private const int ScreenshotZoomStep = 10;

        private DrawOnScreenMode _mode = DrawOnScreenMode.On;
        private bool _showPixelGridByDefault;
        private int _screenshotZoom = 100;
    }
}
