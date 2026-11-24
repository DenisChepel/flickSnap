using System.Text.Json.Serialization;

namespace FlickSnap.Util;

[JsonSerializable(typeof(SessionInfo))]
[JsonSerializable(typeof(SendArgsRequestModel))]
[JsonSerializable(typeof(FlickSnap.UI.Pages.CreditsData))]
[JsonSerializable(typeof(FlickSnap.UI.RecorderSettingsJson))]
internal partial class FlickSnapUiJsonContext : JsonSerializerContext { }
