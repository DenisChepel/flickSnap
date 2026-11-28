# C# / Rust contracts: how to find and check both sides

Read this before changing anything that crosses the C#/Rust boundary: an export in
`crates/flicksnap_interop/src/ffi/`, a wrapper in `ui/flickSnap.Interop/`, a helper message
catalog, a shared contract table, or a helper's command line. The binding rules themselves live
in [`crates/flicksnap_interop/README.md`](../crates/flicksnap_interop/README.md) (sections A–F);
this file covers how to locate the matching code and what to verify. It does not repeat those
rules.

## The boundaries

| Boundary | Rust source of truth | C# side | Generated? |
|---|---|---|---|
| In-process C ABI | `crates/flicksnap_interop/src/ffi/*.rs` (exports, `#[repr(C)]` types, `pub const FLICKSNAP_*`) and `crates/flicksnap_hotkeys/src/key.rs` (key enum) | `ui/flickSnap.Interop/Generated/{NativeMethods,InteropTypes,InteropAbi}.g.cs` + hand-written wrapper `ui/flickSnap.Interop/<Area>.cs` | yes (`cargo codegen`, csbindgen) |
| Helper processes (stdio JSONL) | `crates/flicksnap_ipc/src/catalog.rs` (`ipc_catalog!`), framing in `framing.rs`, exit codes | `ui/flickSnap.Ipc/Generated/HelperMessages.g.cs`, `ui/flickSnap.Ipc.Tests/Generated/FramingVectors.g.cs` | yes |
| Shared env var names | `crates/flicksnap_ipc/src/env.rs` | `ui/flickSnap.Interop/Generated/EnvVars.g.cs` | yes |
| Shared constants (defaults, accent, telemetry, tray tokens, share-region geometry, H.264, click highlight, matte) | `crates/flicksnap_shared/src/contract/` | the matching `*.g.cs` under `ui/` (the header of each names its source) | yes |
| `session.json` written by the Rust helpers | `crates/flicksnap_shared/src/session.rs` (`contract_sample()`) | `ui/flickSnap.Ui/Session/SessionContract.g.cs` (test sample); readers in `ui/flickSnap.Ui/Session/` | sample only |
| Hotkey tables and status codes | `crates/flicksnap_hotkeys/src/{key,error}.rs` | `ui/flickSnap.Shared/Hotkeys/*.g.cs`, `ui/flickSnap.VideoSDK/Composition/KeycapLegends.g.cs` | yes |
| Capture overlay CLI flags | `CliArgs` (clap) in `crates/flicksnap_capture/src/settings.rs` | argument strings built in `ui/flickSnap.Ui/Services/ScreenCaptureService.cs` | **no, hand-mirrored** |
| Session-dir marker files, OCR request | `crates/flicksnap_capture/CAPTURE_PROTOCOL.md` | `ScreenCaptureService.cs` and the session readers | **no, documented** |
| Recording CLI and JSONL sidecars | `crates/flicksnap_recorder/src/{cli,input_capture,cursor_sprite,window_capture}.rs` | `RecorderArguments.cs` and VideoSDK `Composition/{InputCapture,WindowCapture}.cs` | **no, hand-mirrored** |
| Binary AI requests and responses | `crates/flicksnap_ai/src/` and the OCR protocol in `CAPTURE_PROTOCOL.md` | Rust overlay OCR client and managed callers in VideoSDK / Ui | **no** |

`cargo codegen --list` prints every Rust-generated C# output, and its header identifies the source
it comes from. Other native boundaries that do not involve our Rust: FFmpeg 8.1 from C# through
`FFmpeg.AutoGen` (`ui/flickSnap.VideoSDK`, loaded by `FFmpegLoader`), and libobs from Rust
through `bindgen` (`crates/flicksnap_obs_sys`, recorder process only). C# never calls libobs.

## Generation and the gates

- Edit the Rust source, then run `cargo codegen`.
- `cargo codegen --check` lists stale, missing or orphaned outputs and changes nothing.
- `cargo test -p flicksnap_codegen` is the gate: every output is current, and every
  flicksnap_interop crate rule (no `cfg` on ffi items, the `Thread:` doc line, `ffi_enum!`
  lists, no telemetry, no threads, no foreign `#[no_mangle]`, no em-dash) holds.
- ABI hash: `flicksnap_interop`'s `build.rs` and the codegen hash the same `src/ffi` surface.
  The C# loader (`InteropLibrary.cs`) rejects a mismatched hash. Check the path it actually
  loaded as well as regeneration and rebuilds.
- `tools/check-native-interop.sh` checks `DllImport`, `LibraryImport`, `extern`,
  `UnmanagedCallersOnly`, selected `Marshal` APIs, `NativeLibrary.` and other patterns under
  `ui/` against `tools/native-interop-allowlist/*.txt`. Existing exceptions include FFmpeg callbacks.

## Matching the two sides

CodeGraph indexes each language separately, so an export and its P/Invoke appear as two
unrelated symbols with the same name. It finds no edge between them, and an empty "callers"
result on one side proves nothing about the other. Match by exact name:

1. Take the export name `flicksnap_<area>_<verb>[_<noun>]`. It is the Rust `fn` name, the
   `EntryPoint` in `[DllImport(..., EntryPoint = "...", CallingConvention = CallingConvention.Cdecl)]` in
   `NativeMethods.g.cs`, and the C# method name.
2. Rust side: the definition in `src/ffi/<area>.rs`, then the implementation it calls
   (`src/platform/{windows,macos}`, `src/*_host.rs`, or a shared crate such as
   `flicksnap_shared`). Use `codegraph callees` / `codegraph impact` within Rust.
3. C# side: search `NativeMethods.<name>` in `ui/`. That is the wrapper in
   `ui/flickSnap.Interop/<Area>.cs`. Then run `codegraph callers` / `codegraph impact` on the
   wrapper type or method to reach consumers in Ui, VideoSDK, VideoRender and the tests.
4. The same applies to helper messages (a catalog variant name becomes a C# record
   `<Catalog><Name>`), to contract constants, and, by hand, to CLI flag strings.

## What to verify on a change

- **Signature and layout:**
  - only `i32`/`u32`/`u8` for enums and bools; no struct passed by value;
  - `_reserved` padding;
  - the Rust `offset_of!` test and the C# `Marshal.OffsetOf`/`sizeof` test for every struct;
  - enum values only appended, never renumbered.
- **Status and errors:**
  - exports return `i32` status except the two last-error accessors (README A1);
  - read the thread-local last error immediately on a negative status. Positive statuses
    such as `None`, `Timeout` and `Closed` are successful outcomes;
  - C# checks the status (`Native.Check`, `GpuStatus.Check`, ...) instead of ignoring it;
  - new status members only appended.
- **Ownership and lifetime:**
  - the string mode (owned `FlickSnapString` freed by `flicksnap_string_free`, array-owned,
    or handle-borrowed) is named in the export doc and honoured by the wrapper;
  - owned arrays have their `_free`;
  - borrowed OS pointers are not retained;
  - check the explicit transfer exceptions in README C2.
- **Handles:**
  - exactly one `_create`/`_open` and one `_destroy`;
  - managed handle calls take a lease (`InteropHandle` /
    `HandleLease`), so `_destroy` runs only after in-flight calls return;
  - a handle that can sit in a blocking call is stopped before `Dispose`;
  - non-finalizable, thread-affine handles (the Windows foreground watch) are destroyed on
    their owning thread.
- **Threads and callbacks:**
  - each export's `Thread:` line states its thread contract (macOS main thread,
    window-procedure thread, no concurrent calls on one handle);
  - callbacks are static `[UnmanagedCallersOnly(Cdecl)]` thunks with a `GCHandle` user pointer
    and a catch-all body (an escaping exception is a fail-fast);
  - Rust trampolines use `panic::callback_guard`;
  - the log sink and audio callbacks must not call back into any export;
  - Rust releases locks before invoking host callbacks.
- **GPU (Windows D3D12/D3D11, macOS Metal):**
  - readback-ring slot state machine: begin, then submit or submit_shared, then wait or
    release_shared;
  - fence values returned by submit are what the consumer waits on. On the zero-copy path the
    consumer must signal the consumed fence. `GpuNv12Converter.Convert` reports
    `signalQueued`: whether to abandon or release the slot;
  - mapped readback memory is valid only until the slot's next begin;
  - textures from `AcquireTexture` are owned by the crate until `ReleaseTexture`, and FFmpeg
    never releases them;
  - `flicksnap_gpu_d3d11_device_acquire_windows` returns an AddRef'd device that FFmpeg
    releases.
- **Platform arms:** every export exists on every target. The off-OS arm returns
  `FLICKSNAP_E_UNSUPPORTED` or the truthful answer, written as a cfg'd block inside the body,
  never as a cfg on the item.
- **Loading:**
  - `InteropLibrary` checks `FLICKSNAP_INTEROP_PATH`, then uses `NativeProbe`. `HelperLocator`
    uses the same probe for helpers. See `BUILDING.md` for precedence;
  - `FFmpegLoader.TryInitialize` prepares the directory through `ProcessNative.PrepareFfmpegDirectory`
    and verifies the avcodec and avformat majors for FFmpeg 8.1. The normal fallback resolver is
    `FindBesideRecorder`, with the `FLICKSNAP_FFMPEG_PATH` override.

## Tests on each side

| What | Command |
|---|---|
| Codegen current + crate rules | `cargo test -p flicksnap_codegen` |
| Rust exports (null args, off-OS, layout, GPU ring, main-thread targets) | `cargo test -p flicksnap_interop` (tests in `crates/flicksnap_interop/tests/`) |
| Lint, incl. the Linux off-OS arms (from Windows) | `cargo clippy -p flicksnap_interop --all-targets -- -D warnings`; `cargo clippy -p flicksnap_interop --target x86_64-unknown-linux-gnu -- -D warnings` |
| C# wrappers against the real library | `cargo build -p flicksnap_interop`, then `dotnet test ui/flickSnap.Interop.Tests/flickSnap.Interop.Tests.csproj` with `FLICKSNAP_INTEROP_REQUIRED=1` (otherwise tests skip when the library is missing) |
| GPU bridge (real GPU) | `ui/flickSnap.VideoSDK.Tests` (`GpuInteropTests`, `ReadbackRingTests`, `D3D11EncodeBridgeTests`) |
| Helper protocol | `cargo test -p flicksnap_ipc`; run the affected helper's tests and `dotnet test ui/flickSnap.Ipc.Tests/flickSnap.Ipc.Tests.csproj`. For a catalog or lifecycle change, exercise the rebuilt parent and child together. |
| Every wrapper in the real app | `dotnet build ui/flickSnap.Ui/flickSnap.Ui.csproj -c Release`, then `flickSnap --interop-probe <out.json>` (no `"status": "error"` entries) |

A green build or test run is not proof that GPU, FFI or threading code behaves correctly:
- the library may be missing or stale, and the interop tests then skip;
- FFmpeg, optional helpers, platform and GPU availability may affect skips. Check the
  reasons and confirm the scenarios relevant to the change actually ran;
- CI has a filtered Windows WARP baseline. Use physical hardware to validate paths that
  WARP cannot exercise, including applicable hardware encode and device interop paths;
- macOS paths cannot be built on Windows.

When a needed runtime check cannot be run, say so explicitly.

On Windows, a running flickSnap, `flickSnap.VideoRender`, a test host or the standby
`flicksnap_capture` keeps the DLL open, and relinking fails with LNK1104. Close them first.

## Keeping this file current

Update this file when a boundary row, a generator, a gate or a verification rule changes. Do not
list individual exports here: the README's "Exports by area" table and `cargo codegen --list`
are the inventories.
