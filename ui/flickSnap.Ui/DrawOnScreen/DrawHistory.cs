using System;
using System.Collections.Generic;

namespace FlickSnap.UI.DrawOnScreen
{
    /// <summary>Something with its own history that <see cref="DrawHistory"/> can replay;
    /// in practice one screen's drawing canvas.</summary>
    public interface IDrawUndoTarget
    {
        bool CanUndo { get; }
        void Undo();
        bool CanRedo { get; }
        void Redo();
    }

    /// <summary>
    /// One undo and redo order across several independent canvas histories. Each screen has its own canvas and so
    /// its own history; this records which canvas each change landed on, so the toolbar's single
    /// Undo steps back the most recent change wherever it was drawn. A group (clear all, which
    /// touches every canvas) replays as one step.
    /// </summary>
    public sealed class DrawHistory
    {
        private readonly List<IReadOnlyList<IDrawUndoTarget>> _entries = new();
        private readonly List<IReadOnlyList<IDrawUndoTarget>> _redo = new();
        private List<IDrawUndoTarget> _group;

        // set while history replays targets: a canvas that reported its own undo as a change must not
        // push a fresh entry (the canvas does not today, but the history must not depend on that)
        private bool _replaying;

        /// <summary>Raised after every change to the recorded entries.</summary>
        public event EventHandler Changed;

        public bool CanUndo => _entries.Count > 0;

        /// <summary>Records a change on <paramref name="target"/>: its own entry, or a member of the
        /// open group. Ignored while an entry is being replayed.</summary>
        public void Record(IDrawUndoTarget target)
        {
            if (target == null || _replaying)
                return;

            if (_group != null)
            {
                if (!_group.Contains(target))
                    _group.Add(target);
                return;
            }

            Append(new[] { target });
        }

        /// <summary>Opens a group: every <see cref="Record"/> until <see cref="EndGroup"/> joins one entry.</summary>
        public void BeginGroup() => _group ??= new List<IDrawUndoTarget>();

        /// <summary>Closes the group, pushing it only if something was recorded into it.</summary>
        public void EndGroup()
        {
            var group = _group;
            _group = null;
            if (group == null || group.Count == 0)
                return;

            Append(group);
        }

        private void Append(IReadOnlyList<IDrawUndoTarget> entry)
        {
            _entries.Add(entry);
            _redo.Clear();
            OnChanged();
        }

        public bool Undo() => Replay(_entries, _redo, forward: false);

        public bool Redo() => Replay(_redo, _entries, forward: true);

        private bool Replay(List<IReadOnlyList<IDrawUndoTarget>> source, List<IReadOnlyList<IDrawUndoTarget>> destination, bool forward)
        {
            if (_replaying)
                return false;

            var changed = false;
            _replaying = true;
            try
            {
                // A canvas may have trimmed its oldest steps. Skip stale entries and
                // transfer only the members that actually took part in this replay.
                while (!changed && source.Count > 0)
                {
                    var entry = source[^1];
                    source.RemoveAt(source.Count - 1);
                    var replayed = new List<IDrawUndoTarget>(entry.Count);
                    foreach (var target in entry)
                    {
                        if (forward ? !target.CanRedo : !target.CanUndo)
                            continue;
                        if (forward)
                            target.Redo();
                        else
                            target.Undo();
                        replayed.Add(target);
                    }
                    if (replayed.Count > 0)
                    {
                        destination.Add(replayed);
                        changed = true;
                    }
                }
            }
            finally
            {
                _replaying = false;
            }

            OnChanged();
            return changed;
        }

        /// <summary>Removes <paramref name="target"/> from every entry (its screen went away),
        /// dropping entries left empty.</summary>
        public void Forget(IDrawUndoTarget target)
        {
            _group?.Remove(target);
            Forget(_entries, target);
            Forget(_redo, target);
            OnChanged();
        }

        private static void Forget(List<IReadOnlyList<IDrawUndoTarget>> entries, IDrawUndoTarget target)
        {
            for (int i = entries.Count - 1; i >= 0; i--)
            {
                var entry = entries[i];
                if (!Contains(entry, target))
                    continue;

                var kept = new List<IDrawUndoTarget>(entry.Count);
                foreach (var t in entry)
                {
                    if (!ReferenceEquals(t, target))
                        kept.Add(t);
                }

                if (kept.Count == 0)
                    entries.RemoveAt(i);
                else
                    entries[i] = kept;
            }

        }

        public void Clear()
        {
            _entries.Clear();
            _redo.Clear();
            _group = null;
            OnChanged();
        }

        private static bool Contains(IReadOnlyList<IDrawUndoTarget> entry, IDrawUndoTarget target)
        {
            foreach (var t in entry)
            {
                if (ReferenceEquals(t, target))
                    return true;
            }

            return false;
        }

        private void OnChanged() => Changed?.Invoke(this, EventArgs.Empty);
    }
}
