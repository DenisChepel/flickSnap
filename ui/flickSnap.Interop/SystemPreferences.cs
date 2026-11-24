namespace FlickSnap.Interop
{
    /// <summary>The user's system-wide preferences that FlickSnap honours.</summary>
    public static unsafe class SystemPreferences
    {
        /// <summary>
        /// Whether the user wants UI animations: Windows' "Animation effects" (a failed query reads
        /// as enabled), the inverse of macOS' Reduce Motion, always true elsewhere. Any thread.
        /// </summary>
        public static bool AnimationsEnabled()
        {
            InteropLibrary.Require();
            byte enabled;
            Native.Check(NativeMethods.flicksnap_system_animations_enabled(&enabled), "flicksnap_system_animations_enabled");
            return enabled != 0;
        }
    }
}
