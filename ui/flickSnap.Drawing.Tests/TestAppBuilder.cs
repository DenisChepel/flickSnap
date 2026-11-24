using Avalonia;
using Avalonia.Headless;
using FlickSnap.Drawing.Tests;

[assembly: AvaloniaTestApplication(typeof(TestAppBuilder))]

namespace FlickSnap.Drawing.Tests
{
    public class TestAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp()
        {
            // DrawingCanvas reads tool settings from SettingsRoot.Current, which the app assigns at
            // startup. Assign it here, before any Avalonia test runs, so a test class that builds a
            // canvas without its own setup does not depend on another class having run first.
            FlickSnap.Config.SettingsRoot.Current ??= new FlickSnap.Config.SettingsRoot();
            return AppBuilder.Configure<Application>()
                             .UseSkia()
                             .UseHeadless(new AvaloniaHeadlessPlatformOptions { UseHeadlessDrawing = false });
        }
    }
}
