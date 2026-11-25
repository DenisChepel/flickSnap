using System;
using System.Collections.Generic;
using System.Diagnostics;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace FlickSnap.Drawing.Laser
{
    /// <summary>
    /// A transparent, click-through layer that shows laser-pointer trails over whatever is beneath
    /// it and fades them out by itself. It owns nothing else: no document, no history, no export —
    /// a host feeds it pointer positions (<see cref="BeginTrail"/>, <see cref="AddPoint"/>,
    /// <see cref="EndTrail"/>) in its own coordinate space and puts it on top of the thing being
    /// pointed at. The drawing canvas hosts one over its artwork (Draw on Screen's laser tool);
    /// anything else that wants a laser over it — a video preview — can host another.
    ///
    /// It repaints itself once per frame only while some trail is still visible, then stops.
    /// </summary>
    public sealed class LaserLayer : Control
    {
        internal const double DefaultTrailWidth = 4;

        /// <summary>The trail's widest diameter, in this layer's units.</summary>
        public static readonly StyledProperty<double> TrailWidthProperty =
            AvaloniaProperty.Register<LaserLayer, double>(nameof(TrailWidth), DefaultTrailWidth);

        // declared ABOVE ColorProperty on purpose: static fields initialise in textual order, and a
        // property registered before this is set would default to transparent — an invisible laser
        public static readonly Color DefaultColor = Colors.Red;

        /// <summary>The laser's colour: Excalidraw's red by default.</summary>
        public static readonly StyledProperty<Color> ColorProperty =
            AvaloniaProperty.Register<LaserLayer, Color>(nameof(Color), DefaultColor);

        private readonly List<LaserTrail> _trails = new();
        private readonly List<(Point P, double R)> _scratch = new();
        private readonly Stopwatch _clock = Stopwatch.StartNew();
        private LaserTrail _current;
        private Point? _pendingPoint;
        private bool _frameRequested;

        static LaserLayer()
        {
            AffectsRender<LaserLayer>(TrailWidthProperty, ColorProperty);
        }

        public LaserLayer()
        {
            IsHitTestVisible = false;
        }

        public double TrailWidth
        {
            get => GetValue(TrailWidthProperty);
            set => SetValue(TrailWidthProperty, value);
        }

        public Color Color
        {
            get => GetValue(ColorProperty);
            set => SetValue(ColorProperty, value);
        }

        /// <summary>Whether a trail is being drawn right now (the pointer is down).</summary>
        public bool IsDrawing => _current != null;

        /// <summary>Whether anything is still on screen or fading.</summary>
        public bool HasTrails => _trails.Count > 0;

        private double Now => _clock.Elapsed.TotalMilliseconds;

        /// <summary>The pointer went down at <paramref name="p"/>: a new trail starts there.</summary>
        public void BeginTrail(Point p)
        {
            EndTrail();
            _current = new LaserTrail();
            _current.Add(p, Now);
            _trails.Add(_current);
            Animate();
        }

        /// <summary>The pointer moved to <paramref name="p"/> while down.</summary>
        public void AddPoint(Point p)
        {
            if (_current == null)
                return;
            // Like the browser, use the latest position per frame. The tail
            // then doesn't get shorter with a high-polling mouse.
            _pendingPoint = p;
            Animate();
        }

        /// <summary>The pointer is up: the trail fades out on its own.</summary>
        public void EndTrail()
        {
            if (_current == null)
                return;

            FlushPoint(Now);
            _current.End();
            _current = null;
            Animate();
        }

        /// <summary>Drops every trail at once (the host is going away or hiding).</summary>
        public void Clear()
        {
            _current = null;
            _pendingPoint = null;
            _trails.Clear();
            InvalidateVisual();
        }

        public override void Render(DrawingContext context)
        {
            double now = Now;
            _trails.RemoveAll(t => !t.IsAlive(now));
            if (_trails.Count == 0)
                return;

            double maxRadius = TrailWidth / 2;
            var shapes = new List<(Point P, double R)[]>(_trails.Count);
            foreach (var trail in _trails)
            {
                trail.GetShape(now, maxRadius, _scratch);
                if (_scratch.Count > 0)
                    shapes.Add(_scratch.ToArray());
            }

            if (shapes.Count > 0)
            {
                // DrawingCanvas scales the trail width to keep it constant on screen.
                // Use the same ratio for the one-DIP antialias margin when zoomed out.
                double padding = Math.Max(1, TrailWidth / DefaultTrailWidth);
                context.Custom(new LaserDrawOperation(shapes.ToArray(), Color, padding));
            }
        }

        private void FlushPoint(double now)
        {
            if (_pendingPoint is { } point)
            {
                _current?.Add(point, now);
                _pendingPoint = null;
            }
        }

        /// <summary>Repaints on the next frame, and keeps doing so while anything is left to fade.</summary>
        private void Animate()
        {
            if (_frameRequested)
                return;

            var top = TopLevel.GetTopLevel(this);
            if (top == null)
                return;

            _frameRequested = true;
            top.RequestAnimationFrame(_ =>
            {
                _frameRequested = false;
                double now = Now;
                FlushPoint(now);
                _trails.RemoveAll(t => !t.IsAlive(now));
                InvalidateVisual();
                // Keep the gesture armed after a pause. Movement restarts the frames.
                if (_trails.Exists(t => t.HasRecentPoints(now)))
                    Animate();
            });
        }
    }
}
