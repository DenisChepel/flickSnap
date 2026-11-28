# flickSnap architecture

Orientation for changes that cross modules or processes. Read it on demand: it describes the
structure that CodeGraph cannot show (processes, stdio and file contracts, generated mirrors).
For a symbol's callers and callees, ask CodeGraph or read the source.

## Processes

flickSnap is a tray app (.NET 10, Avalonia 12) with screenshot and video editors. Most
child processes use the JSONL helper protocol. `flicksnap_ai` uses a separate binary protocol.

| Process | Built from | Started by (C#) | Talks over |
|---|---|---|---|
| `flickSnap` (tray app, editors) | `ui/flickSnap.Ui` | user / autostart (`Program.Main`, `App.OnFrameworkInitializationCompleted`) | loads `flicksnap_interop` in-process |
| `flicksnap_capture` (capture overlay, kept in standby) | `crates/flicksnap_capture` (winit + egui) | `Services/CaptureStandbySupervisor.cs`, `Services/ScreenCaptureService.cs` | `capture` catalog; CLI flags; session dir files |
| `flicksnap_scroll_driver` | `crates/flicksnap_scroll_driver` | `Scroll/ScrollCapturePage.cs` | `scroll` catalog; `session.json` |
| `flicksnap_recorder` (libobs) | `crates/flicksnap_recorder` | `Video/VideoCapturePage.cs`, `Services/CameraDeviceManager.cs` | `recorder` catalog |
| `flicksnap_share_region` | `crates/flicksnap_share_region` | `Video/ShareRegionPage.cs` | `share_region` catalog |
| `flicksnap_vid2gif` | `crates/flicksnap_vid2gif` | GIF conversion (`Services/GifConversionManager.cs`) | `job` catalog |
| `flickSnap.VideoRender` (the only C# child) | `ui/flickSnap.VideoRender` | `Services/JobRunner.cs` | `job` catalog |
| `flicksnap_ai` (ONNX: OCR, matting, denoise) | `crates/flicksnap_ai` | `flicksnap_capture` (OCR), `ui/flickSnap.VideoSDK/Ai/*` (effects), `Services/AiAnalysisManager.cs` | binary payload, no catalog |
| `flicksnap_shell_ext` (COM dll, Windows 11 menu) | `crates/flicksnap_shell_ext` | Explorer, registered by `SparsePackageManager` | COM |

The C# paths above are relative to `ui/flickSnap.Ui/` unless they say otherwise. Managed
callers resolve helper names through `HelperLocator` in `ui/flickSnap.Ipc`. The capture
overlay also resolves Rust helpers as siblings. See `tools/staging/shipped.sh` for which
binaries ship beside the app and which ship under `recorder/`.

Under the [helper protocol](../crates/flicksnap_capture/HELPER_PROTOCOL.md), a JSONL child
sends `hello` with its protocol version and catalog fingerprint, then emits exactly one
terminal event or fatal error on controlled exit. Its shutdown paths converge on the same
callback for `quit`, stdin EOF, stdout EPIPE and supported signals. For the binary AI
protocol, stdin EOF can mean end of input and there is no `hello`.

## Static dependency graph

C# (`ProjectReference`):

```
flickSnap.Ui  -> Drawing, Interop, Ipc, Shared, Upload, VideoSDK
VideoRender   -> Ipc, VideoSDK
VideoSDK      -> Geometry, Ipc, Interop            (no Avalonia: never reference Shared)
Drawing       -> Geometry, Shared
Shared        -> Interop                           (pulls in Avalonia)
Upload        -> Shared
Ipc           -> Interop                           (no package references)
Interop, Geometry -> nothing
VideoSDK.Tests -> VideoSDK, VideoRender, Ui        (so Ui changes can break it)
```

`IpcChild` in VideoRender uses Interop's `StdoutGuard` to protect JSONL stdout and prevent
pipe inheritance. VideoSDK calls `ProcessNative.PrepareFfmpegDirectory` before AutoGen
loads the libraries. Changes to native process or loader code can therefore affect render
startup even when the IPC catalog and FFmpeg bindings stay the same.

Rust (selected path dependencies). Root-level Cargo defaults exclude the recorder stack.
`cargo rec` explicitly builds the recorder, share-region and GIF packages.

```
flicksnap_ipc (leaf) <- flicksnap_shared <- flicksnap_desktop, flicksnap_telemetry
flicksnap_platform (leaf) <- flicksnap_hotkeys
flicksnap_interop (cdylib) -> desktop, hotkeys, ipc, platform, shared   (never telemetry/sentry/ai/recorder stack: CI gate)
flicksnap_codegen          -> hotkeys, ipc, shared                       (never ai/recorder stack: CI gate)
flicksnap_capture          -> desktop, hotkeys, ipc, platform, shared, telemetry
flicksnap_scroll_driver    -> desktop, ipc, platform, shared, telemetry
flicksnap_ai               -> ipc, shared
flicksnap_recorder         -> obs, obs_platform, obs_sys, hotkeys, ipc, platform, shared, telemetry
flicksnap_share_region     -> obs, obs_platform, obs_sys, desktop, ipc, shared, telemetry
flicksnap_vid2gif          -> ffmpeg_sys, ipc, telemetry
flicksnap_obs_sys          -> libobs built from the crates/obs-studio submodule (pinned 33.0.1)
```

## Main flows

- **Screenshot.**
  1. A hotkey (via `flicksnap_interop` → `flicksnap_hotkeys`) or the tray asks
     `ScreenCaptureService`.
  2. The standby `flicksnap_capture` receives a `Settings` command with CLI-style args and a
     capture request.
  3. It creates a session directory under the session root, freezes the screen and draws the
     overlay.
  4. It writes its outputs and an action marker (`action.txt`) into the session dir.
  5. The app reads them and opens the editor or uploads.

  Sessions are listed by `ui/flickSnap.Ui/Session/SessionManager.cs` (a FileSystemWatcher on
  the session root); `SessionInfo` maps `session.json`.
- **Recording and editing.**
  1. `VideoCapturePage` drives `flicksnap_recorder` (libobs) through the `recorder` catalog.
  2. The result opens in the video editor (`ui/flickSnap.Ui/VideoEditor`) over `VideoSDK`:
     - `Model/Project.cs` defines the project. `RecordingProject.Build` creates it from
       recording inputs for both the UI and legacy render arguments. `Editing/EditorSession` edits it;
     - `Playback/CompositionPlayer` plays it, with decode in `VideoDecodeWorker`, FFmpeg 8.1
       through `FFmpeg.AutoGen`, D3D11VA or VideoToolbox and a software fallback;
     - the frame compositor and readback rings live in `Composition/`, on D3D12/Metal through
       `flicksnap_interop`'s `gpu` exports, with a CPU fallback;
     - the audio master clock and mic capture live in `Audio/`.
  3. Export runs `flickSnap.VideoRender` as a job (`RenderJob`). The child reads project JSON
     or legacy render arguments via `RenderArgsCompat`. GIF export runs `flicksnap_vid2gif`.
- **Upload.** `ui/flickSnap.Upload`: one `*UploadProvider` per service (S3, Azure,
  Cloudflare R2, Backblaze, Imgur, the flickSnap relay, ...) over `UploadProviderBase`, routed by
  `UploadRouting`. The shell extension adds the Explorer entry. The relay server is not in this
  repository.

## Sources of truth

| Concern | Source of truth | Mirrors / consumers |
|---|---|---|
| C ABI, interop types, ABI hash | `crates/flicksnap_interop/src/ffi/` | generated `ui/flickSnap.Interop/Generated/*` |
| Helper messages, framing, exit codes, helper names | `crates/flicksnap_ipc/src/` (`catalog.rs`, `framing.rs`, `env.rs`) | generated `ui/flickSnap.Ipc/Generated/*`, `EnvVars.g.cs` |
| Shared defaults and contract constants | `crates/flicksnap_shared/src/contract/` | generated `*.g.cs` across Shared/Ui/VideoSDK |
| `session.json` schema | `crates/flicksnap_shared/src/session.rs` + `CAPTURE_PROTOCOL.md` §1.3 | `SessionInfo`, `SessionContract.g.cs` test |
| Hotkey keys and status codes | `crates/flicksnap_hotkeys/src/{key,error}.rs` | generated hotkey and keycap tables |
| Capture overlay options | `CliArgs` in `crates/flicksnap_capture/src/settings.rs` | hand-written flags in `ScreenCaptureService.cs` |
| Recording options | `crates/flicksnap_recorder/src/cli.rs` | hand-written flags in `ui/flickSnap.Ui/Services/RecorderArguments.cs` |
| Recording JSONL sidecars | `input_capture.rs`, `cursor_sprite.rs`, `window_capture.rs` in `crates/flicksnap_recorder/src/` | `InputCapture` and `WindowCapture` in VideoSDK `Composition/`, used in preview and export |
| Package contents by RID | `tools/staging/shipped.sh` | local staging, CI build and package verification |
| User settings | `SettingsRoot` / `SettingsService` in `ui/flickSnap.Shared/Config/` | the app; relevant values reach helpers via CLI or IPC (for example standby capture `Settings`) |
| UI strings | `ui/flickSnap.Shared/Localization/Strings.resx` (see `TRANSLATING.md`) | `Strings.<culture>.resx`, `{loc:T Key}` in AXAML |
| SkiaSharp version | `ui/Directory.Build.props` (`SkiaSharpVersion`; must equal what Avalonia resolves) | every `ui/` project |
| App version | `version.json` (nbgv) | C# assemblies; the Rust workspace version is a placeholder |
| Toolbar and panel icons | design sources in `assets/icons/` | Rust overlay SVGs `crates/flicksnap_capture/assets/icons/` (`include_bytes!`) **and** Avalonia geometries in `ui/flickSnap.Ui/Assets/VectorIcons.axaml`, kept in step by hand |
| Generated-file list | `crates/flicksnap_codegen` (`cargo codegen --list`) | |

## Links CodeGraph does not see

When you trace an impact, check these by search:
- Helper spawns by name (`HelperNames.*`) and the JSON catalogs over stdio.
- C# P/Invoke versus Rust export (identical names, two unrelated symbols): see
  [`FFI_CONTRACTS.md`](FFI_CONTRACTS.md).
- CLI flag strings, session-dir marker files, and environment variables (`FLICKSNAP_*`,
  generated in `EnvVars.g.cs`).
- `.axaml` markup: bindings, `x:Name` fields, styles and resources, `{loc:T}` keys. It is not
  indexed; edit it together with its code-behind.
- Embedded assets (`include_bytes!`, EmbeddedResource), ONNX models (`crates/flicksnap_ai`), and
  cursor packs (`ui/flickSnap.VideoSDK/Composition/CursorPacks`).
- `flicksnap_obs_sys` against libobs. The OBS sources are excluded from the index; for OBS
  behaviour read the pinned submodule `crates/obs-studio`. Never edit it.
- Reflection or source-generated JSON contexts (`FlickSnapUiJsonContext`) and Velopack hooks in
  `Program.Main`.

## Platforms

- **The app:** Windows 10+ and macOS 13.4+. Every project interop export exists on every
  target. An off-OS arm returns the truthful result or `FLICKSNAP_E_UNSUPPORTED`.
- **The recorder stack:** builds on Windows, macOS and Linux (Linux in `tools/linux-build`).
  The Linux payload includes the recorder and GIF helper.
- **`flicksnap_ai`:** not built for osx-x64.
- **`flicksnap_shell_ext`:** Windows only, an empty crate elsewhere. Bump its crate version
  whenever the extension changes (BUILDING.md).
