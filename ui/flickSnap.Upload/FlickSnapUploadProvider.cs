using System;
using System.ComponentModel;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using System.Text.Json;

namespace FlickSnap.Upload
{
    /// <summary>The built-in text host running on clwd.app (hastebin-compatible API). Marked
    /// <see cref="IBuiltInProvider"/> so it sorts first and is enabled/defaulted for Text out of
    /// the box — see <see cref="FlickSnap.Config.SettingsUpload.DiscoverProviders"/>.</summary>
    public class FlickSnapUploadProvider : UploadProviderBase, IBuiltInProvider
    {
        public const string DefaultServerUrl = "https://clwd.app";

        public override string Name
        {
            get
            {
                // only qualify the name when pointed at a self-hosted server
                if (String.Equals(ServerUrl?.TrimEnd('/'), DefaultServerUrl, StringComparison.OrdinalIgnoreCase))
                    return "flickSnap";

                try
                {
                    return $"flickSnap ({new Uri(ServerUrl).Host})";
                }
                catch
                {
                    return "flickSnap";
                }
            }
        }

        public override string Description => "Fast text sharing on clwd.app with automatic syntax highlighting";

        public override SupportedUploadType SupportedUpload => SupportedUploadType.Text;

        public override Stream Icon => new Resource().FlickSnapIcon;

        [Description("The flickSnap server to store pastes on. Change this only if you are self-hosting.")]
        public string ServerUrl
        {
            get => _serverUrl;
            set => Set(ref _serverUrl, value, nameof(ServerUrl), nameof(Name));
        }

        private string _serverUrl = DefaultServerUrl;

        public override async Task<UploadResult> UploadAsync(Stream fileStream, UploadProgressHandler progress, string uploadName, CancellationToken cancelToken)
        {
            var url = ServerUrl.TrimEnd('/') + "/p";
            var result = await SendFileAsContent(url + "/documents", fileStream, progress);
            var resp = JsonSerializer.Deserialize(result, UploadJsonContext.Default.HasebinResponse);

            if (resp?.key == null)
                throw new Exception("Empty response");

            var fileUrl = url + "/" + resp.key;

            // the extension selects the syntax highlighting language on the viewer page
            var ext = Path.GetExtension(uploadName);
            if (!String.IsNullOrWhiteSpace(ext))
                fileUrl += ext;

            return new UploadResult()
            {
                Provider = this,
                PublicUrl = fileUrl,
                FileName = uploadName,
                UploadKey = resp.key,
            };
        }
    }
}
