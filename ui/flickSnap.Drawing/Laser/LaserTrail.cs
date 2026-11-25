using System;
using System.Collections.Generic;
using Avalonia;

namespace FlickSnap.Drawing.Laser
{
    /// <summary>
    /// A temporary, timestamped stroke. Like Excalidraw, the trail narrows by sample age and
    /// by how many newer samples follow it. Spacing the samples farther apart makes a fast
    /// flick span more of the screen.
    /// </summary>
    public sealed class LaserTrail
    {
        /// <summary>How long a point lives, in milliseconds.</summary>
        public const double DecayMs = 1000;

        /// <summary>How many points back from the tip the trail tapers to nothing.</summary>
        public const int DecayLength = 50;

        private const double Streamline = 0.4;

        private readonly List<(Point P, double T)> _points = new();
        private Point? _lastInput;

        /// <summary>True until <see cref="End"/>: the pointer is still down on this trail.</summary>
        public bool IsDrawing { get; private set; } = true;

        public int Count => _points.Count;

        /// <summary>Samples the pointer and smooths both its position and timestamp.</summary>
        public void Add(Point p, double tMs)
        {
            if (!IsDrawing || _lastInput == p)
                return;

            _lastInput = p;
            if (_points.Count > 0)
            {
                var (last, lastT) = _points[^1];
                p = last + (p - last) * (1 - Streamline);
                tMs = lastT + (tMs - lastT) * (1 - Streamline);
            }

            _points.Add((p, tMs));
            // Keep one extra point so a full tail can taper to zero at its first visible sample.
            if (_points.Count > DecayLength + 1)
                _points.RemoveAt(0);
        }

        /// <summary>The pointer is up: no more points; what is there fades out.</summary>
        public void End() => IsDrawing = false;

        public bool HasRecentPoints(double nowMs) => _points.Count > 0 && nowMs - _points[^1].T < DecayMs;

        public bool IsAlive(double nowMs) => IsDrawing || HasRecentPoints(nowMs);

        /// <summary>The visible stroke's centreline and radii, oldest first.</summary>
        public void GetShape(double nowMs, double maxRadius, List<(Point P, double R)> output)
        {
            output.Clear();
            int n = _points.Count;
            if (!HasRecentPoints(nowMs))
                return;

            // A click is a dot. The two-point case draws a round-capped line even for a sparse flick.
            if (n <= 2)
            {
                double start = Radius(0, 0, nowMs, maxRadius);
                double end = Radius(n - 1, 0, nowMs, maxRadius);
                if (Math.Min(start, end) < maxRadius / 4)
                    return;
                output.Add((_points[0].P, start));
                if (n == 2)
                    output.Add((_points[1].P, end));
                return;
            }

            int first = 0;
            for (int i = 1; i < n - 1; i++)
            {
                if (Radius(i, i, nowMs, maxRadius) == 0)
                    first = i + 1;
            }
            if (first >= n - 2)
                return;

            output.Add((_points[first].P, Radius(first, 0, nowMs, maxRadius)));
            for (int i = first + 1; i < n - 1; i++)
                output.Add((_points[i].P, Radius(i, i, nowMs, maxRadius)));

            // A round tip uses the radius of the last segment.
            output.Add((_points[^1].P, Radius(n - 2, n - 2, nowMs, maxRadius)));
        }

        private double Radius(int point, int widthIndex, double nowMs, double maxRadius)
        {
            double age = (nowMs - _points[point].T) / DecayMs;
            double tail = (_points.Count - widthIndex) / (double)DecayLength;
            double decay = Math.Clamp(Math.Max(age, tail), 0, 1);
            // Excalidraw uses a quartic ease-out for both limits.
            double squared = decay * decay;
            return maxRadius * (1 - squared * squared);
        }
    }
}
