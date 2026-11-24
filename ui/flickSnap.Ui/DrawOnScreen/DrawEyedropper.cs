using System;
using System.Collections.Generic;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using FlickSnap.UI.Helpers;
using FlickSnap.Util;

namespace FlickSnap.UI.DrawOnScreen;

internal sealed class DrawEyedropper(Action<Color> picked, Action<double, PixelPoint> zoom, Action<bool> pan)
{
    private readonly List<DrawEyedropperWindow> _windows = new();
    private Window _owner;

    public bool IsPicking => _windows.Count != 0;

    public void Start(Window owner)
    {
        if (IsPicking || !ScreenPixels.IsAvailable)
            return;

        _owner = owner;
        try
        {
            foreach (var screen in DesktopScreens.All(owner))
            {
                var window = new DrawEyedropperWindow(screen, Finish, zoom, pan);
                _windows.Add(window);
                window.Show(owner);
            }
        }
        catch
        {
            Stop();
            throw;
        }
    }

    public void Stop(Window owner = null)
    {
        if (!IsPicking || (owner != null && owner != _owner))
            return;

        foreach (var window in _windows)
        {
            if (window.IsVisible)
                window.Close();
        }
        _windows.Clear();
        _owner = null;
        pan?.Invoke(false);
    }

    private void Finish(Color? color)
    {
        Stop();
        if (color is { } selected)
            picked(selected);
    }

    public void SetPanTarget(DrawOnScreenWindow canvas)
    {
        foreach (var window in _windows)
            window.SetPanTarget(canvas);
    }

    public void RefreshSample()
    {
        foreach (var window in _windows)
            window.RefreshSample();
    }

    public bool RoutePanKey(KeyEventArgs e, bool held) =>
        IsPicking && DrawEyedropperWindow.RoutePanKey(e, pan, held);

    public void RouteZoomWheel(Window owner, PointerWheelEventArgs e)
    {
        if (IsPicking && !ScreenCapturePage.IsCaptureActive)
            DrawEyedropperWindow.RouteZoomWheel(owner, e, zoom);
    }

    public Action IncludeEyedropperInScreenshot(bool passMouse)
    {
        var windows = _windows.ToArray();
        try
        {
            foreach (var window in windows)
                window.SetScreenshotPreview(true, passMouse);
            return Restore;
        }
        catch
        {
            Restore();
            throw;
        }

        void Restore()
        {
            foreach (var window in windows)
                window.SetScreenshotPreview(false, passMouse);
        }
    }
}
