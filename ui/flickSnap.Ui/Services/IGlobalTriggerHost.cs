using System;

namespace FlickSnap.UI
{
    /// <summary>
    /// OS hotkey backend used by <see cref="HotkeyManager"/>. The settings layer (FlickSnap.Shared)
    /// only stores gestures — all registration machinery lives in the UI layer.
    /// </summary>
    public interface IGlobalTriggerHost
    {
        /// <summary>
        /// Begin listening for <paramref name="gesture"/>. The returned handle reports live
        /// registration status (raising <see cref="IGlobalTriggerRegistration.StatusChanged"/> on the
        /// UI thread when it changes, e.g. when the hook fails to start asynchronously) and invokes
        /// <paramref name="executed"/> on the UI thread each time the gesture is pressed.
        /// Dispose the handle to remove the registration. With <paramref name="fireOnRepeat"/> the
        /// action also runs on the key's auto-repeats (the media keys); otherwise once per press.
        /// </summary>
        IGlobalTriggerRegistration RegisterTrigger(HotkeyGesture gesture, Action executed, bool fireOnRepeat = false);

        /// <summary>
        /// Records one gesture through the hook, from whichever app has the focus: the next key
        /// press that is not a modifier is swallowed and handed to <paramref name="recorded"/>,
        /// with the modifiers held. Registered hotkeys do not fire meanwhile.
        /// <paramref name="modifiersChanged"/> follows the modifiers held as they change, for a
        /// preview. <paramref name="failed"/> fires instead of <paramref name="recorded"/> when
        /// the hook dies mid-recording, with why — no gesture will ever come. All three run on the
        /// UI thread; <paramref name="recorded"/> or <paramref name="failed"/> fires at most once and
        /// ends the session, and disposing the handle cancels it. Null when the hook is unavailable
        /// (<paramref name="error"/> says why, e.g. the Accessibility message) or another recording
        /// is already active (<paramref name="error"/> is null).
        /// </summary>
        IDisposable StartRecording(Action<HotkeyModifiers> modifiersChanged, Action<HotkeyGesture> recorded,
            Action<string> failed, out string error);

        /// <summary>
        /// Raised on the UI thread when a hook that was down is running again (macOS, once the
        /// Accessibility permission is granted), after the registrations it took along report
        /// registered: <see cref="StartRecording"/> works from now on, so whatever said why it did
        /// not is out of date.
        /// </summary>
        event EventHandler RecordingAvailable;
    }

    /// <summary>Live handle for a single hotkey registration created by <see cref="IGlobalTriggerHost"/>.</summary>
    public interface IGlobalTriggerRegistration : IDisposable
    {
        bool IsRegistered { get; }

        string Error { get; }

        event EventHandler StatusChanged;
    }
}
