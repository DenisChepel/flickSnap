using System;
using System.IO;
using System.Runtime.Versioning;
using System.Security;
using Microsoft.Win32;

namespace FlickSnap.Util
{
    /// <summary>Reads the Windows shell theme independently of the application theme.</summary>
    [SupportedOSPlatform("windows")]
    internal static class WindowsTaskbarTheme
    {
        private const string Personalize = @"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

        public static bool IsLight()
        {
            try
            {
                return Registry.GetValue(Personalize, "SystemUsesLightTheme", 0) is 1;
            }
            catch (Exception ex) when (ex is IOException or SecurityException or UnauthorizedAccessException)
            {
                return false;
            }
        }
    }
}
