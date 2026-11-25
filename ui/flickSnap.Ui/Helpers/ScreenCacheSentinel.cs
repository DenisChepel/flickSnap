using System;
using System.Runtime.InteropServices;
using Avalonia.Controls;
using Avalonia.Threading;

namespace FlickSnap.UI.Helpers
{
    /// <summary>
    /// Keeps Avalonia's Win32 screens cache from going stale while FlickSnap is tray-resident.
    ///
    /// Avalonia invalidates that cache only from a live window's WM_DISPLAYCHANGE handler (the
    /// hidden message window ignores WM_DISPLAYCHANGE), so a display change while no window is
    /// open — sleep/wake, monitor hotplug, or the logon-time 1024x768 fallback monitor — leaves
    /// the cache stale for the rest of the process. Avalonia then mis-places everything that
    /// consults it internally, most visibly the tray context menu, whose flip-above-the-cursor
    /// constraint never fits the stale bounds and leaves the menu running off screen.
    /// Upstream: https://github.com/AvaloniaUI/Avalonia/issues/22217.
    ///
    /// The fix is one never-shown Window held for the life of the process: Avalonia creates its
    /// HWND in the constructor, WM_DISPLAYCHANGE is broadcast to all top-level windows visible or
    /// not, and the handler refreshes the cache through Avalonia's own per-window path. Unshown,
    /// it starts no renderer and never appears in the desktop lifetime's window list.
    ///
    /// This complements <see cref="DesktopScreens"/>: that keeps FlickSnap's own placement math off
    /// the cache entirely; this keeps the cache itself fresh for the consumers inside Avalonia
    /// that FlickSnap cannot redirect.
    /// </summary>
    internal static class ScreenCacheSentinel
    {
        private const uint WM_SETTINGCHANGE = 0x001A;
        private const uint WM_THEMECHANGED = 0x031A;
        private static Window _sentinel;

        public static void Initialize(Action onShellThemeChanged)
        {
            if (!OperatingSystem.IsWindows() || _sentinel != null)
                return;

            _sentinel = new Window
            {
                Title = "flickSnap screen-change sentinel",
                ShowInTaskbar = false,
                ShowActivated = false,
            };
            Win32Properties.AddWndProcHookCallback(_sentinel, OnSystemSettingsChanged);

            IntPtr OnSystemSettingsChanged(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam, ref bool handled)
            {
                // The shell and app color modes can change independently. Observe the
                // broadcast even if Avalonia sees no change in the app's colors.
                if (message == WM_THEMECHANGED ||
                    (message == WM_SETTINGCHANGE && Marshal.PtrToStringUni(lParam) == "ImmersiveColorSet"))
                {
                    Dispatcher.UIThread.Post(onShellThemeChanged);
                }
                return IntPtr.Zero;
            }
        }
    }
}
