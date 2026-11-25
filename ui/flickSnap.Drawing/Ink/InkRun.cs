using SkiaSharp;

namespace FlickSnap.Drawing.Ink
{
    /// <summary>
    /// One piece of a stroke's ink as a Skia path, origin-local and immutable once built: what the
    /// Skia draw operations (<see cref="HighlighterDrawOperation"/>, <see cref="InkOutlineDrawOperation"/>)
    /// paint from. The path is made on first use and kept; the lock is there because a scene may be
    /// drawn from more than one thread (the render thread, and an export on the UI thread).
    /// </summary>
    internal abstract class InkRun
    {
        private readonly object _lock = new object();
        private SKPath _path;

        public SKPath GetPath()
        {
            lock (_lock)
                return _path ??= BuildPath();
        }

        protected abstract SKPath BuildPath();
    }
}
