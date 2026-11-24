# Building flickSnap

## Prerequisites

- [.NET 10 SDK](https://dotnet.microsoft.com/download/dotnet/10.0)
- A stable [Rust toolchain](https://rustup.rs)
- For video recording, the video editor, `flickSnap.VideoRender` and the FFmpeg-gated tests: the native
  toolchain in [Recording stack (libobs)](#recording-stack-libobs) below (`cargo rec` stages FFmpeg
  8.1 beside `flicksnap_recorder`). Without it, set `FLICKSNAP_FFMPEG_PATH` to a directory holding the FFmpeg
  8.1 shared libraries, or `FLICKSNAP_HELPERS_DIR` to the directory an unpacked CI `recorder-<rid>`
  artifact sits in; see [The video editor's FFmpeg](#the-video-editors-ffmpeg).

## Build

```cmd
git clone https://github.com/DenisChepel/flickSnap.git
cargo build -p flicksnap_capture
cargo build -p flicksnap_interop
dotnet build ui/flickSnap.Ui/flickSnap.Ui.csproj
```

`flicksnap_interop` is the native library flickSnap and flickSnap.VideoRender load in-process; neither starts
without it (see [Native interop (flicksnap_interop)](#native-interop-flicksnap_interop)).

A plain `cargo build` / `cargo test` covers the app crates (the workspace's `default-members`) and
stays fast. The recorder stack builds libobs with CMake and only builds when asked (`cargo rec`, see
below).

### Editors

rust-analyzer checks and runs build scripts with `--workspace` by default, which includes the
recorder stack: on a checkout without the `crates/obs-studio` submodule every check fails in the
`flicksnap_obs_sys` build script, and with it the editor starts the 20–40 minute libobs build in the
background and holds the `target/debug` lock while it runs. `.zed/settings.json` turns that off for
Zed. In VS Code, set the same two options in the workspace settings:

```json
{
  "rust-analyzer.check.workspace": false,
  "rust-analyzer.cargo.buildScripts.overrideCommand": [
    "cargo", "check", "--quiet", "--message-format=json", "--all-targets", "--keep-going"
  ]
}
```

Without `--workspace`, cargo at the workspace root covers only the app crates, so rust-analyzer has
no build-script output (the libobs bindings) for the recorder crates. When working on those, remove
both options (or add `"rust-analyzer.cargo.targetDir": true`, which gives the editor its own
target dir so it never blocks your own builds).

### Finding the helpers

flickSnap spawns its helpers (`flicksnap_capture`, `flicksnap_scroll_driver`, `flicksnap_recorder`,
`flicksnap_share_region`, `flicksnap_vid2gif`, `flicksnap_ai`, `flickSnap.VideoRender`) through one lookup,
`HelperLocator` (`ui/flickSnap.Ipc`), with one override; the `flicksnap_interop` loader uses the same probe
(`NativeProbe`, `ui/flickSnap.Interop`). For each file, the first hit wins:

1. each directory of `FLICKSNAP_HELPERS_DIR`, then its `recorder/` subdirectory. It is a path list,
   separated like `PATH` (`;` on Windows, `:` elsewhere),
2. the app directory, then its `recorder/` subdirectory (the release layout),
3. walking up to the workspace `Cargo.toml`, the newer of `target/debug` and `target/release` (the
   dev layout); `flickSnap.VideoRender` is looked for in `ui/flickSnap.VideoRender/bin/{Debug,Release}/net10.0`.

A flickSnap built inside the checkout needs no variable. One built to an out-of-tree `OutDir` finds no
`Cargo.toml` on the way up and no single directory that holds both the Rust helpers and
`flickSnap.VideoRender`, so set the list to both:

```sh
FLICKSNAP_HELPERS_DIR="<repo>/target/debug:<repo>/ui/flickSnap.VideoRender/bin/Debug/net10.0"
```

(`;` instead of `:` on Windows.) The only other variable is `FLICKSNAP_FFMPEG_PATH`, which names the
FFmpeg library directory rather than a helper (below). The stdio contract every helper speaks is
`crates/flicksnap_capture/HELPER_PROTOCOL.md`.

### The video editor's FFmpeg

The editor, the playback engine and `flickSnap.VideoRender` all load FFmpeg 8.1 natively (avcodec 62 /
avformat 62 / avutil 60 / swscale 9 / swresample 6) and refuse to start without it:
`FFmpegLoader.TryInitialize` asserts those majors. A release build takes them from the bundled
recorder payload; `FFmpegLoader.FindBesideRecorder` is the one resolver every caller goes through,
probing in order:

1. `FLICKSNAP_FFMPEG_PATH`, if set: always wins,
2. the recorder binary's own directory (the Windows layout, and every dev layout),
3. its `Frameworks/` subdirectory (the macOS bundle layout).

The recorder binary itself (`flicksnap_recorder`) is found like every other helper (see
[Finding the helpers](#finding-the-helpers)). Building the recorder with `cargo rec` (or `cargo recr`
for release) is therefore all a dev checkout needs on either platform: its build scripts stage the
FFmpeg libraries into the profile dir beside the binary, so the dev FFmpeg directory is
`target/<profile>`. Failing that, point `FLICKSNAP_HELPERS_DIR` at the directory an unpacked CI
`recorder-<rid>` artifact sits in (the one holding `recorder/`).

Roughly a hundred tests — every encode, render, composition-player, filmstrip, waveform and AI
sidecar test — skip rather than fail when FFmpeg is missing, so check for `Skipped: 0` before
believing a green run.

The screen capture overlay is a separate Rust binary (`flicksnap_capture`); the tray app / editor is the Avalonia project `flickSnap`. For video recording, flickSnap runs the recorder (`flicksnap_recorder`, built from `crates/flicksnap_recorder` in this repository) from a `recorder/` directory alongside the flickSnap binary — see `.github/workflows/ci.yml` (which merges the outputs of `build-ui.yml`, `build-rust.yml` and `build-recorder.yml` in the same folder) for how release packages are assembled.

The video editor runs on Windows and macOS alike. Platform-specific pieces: the frame compositor
renders on Direct3D 12 or Metal (`GpuSurfaceFactory`, with a CPU fallback either way), video decode
uses D3D11VA or VideoToolbox and falls back to software, and the preview's audio master clock is
driven by WASAPI or a CoreAudio default-output AudioUnit (`AudioOutputFactory`) — the clock backs
off by whatever latency the device actually reports, which is ~100 ms on WASAPI and ~14 ms on
CoreAudio. The voice recorder's microphone capture is the same split: WASAPI shared mode or a
CoreAudio input AudioQueue behind one `MicCapture` abstraction, with every take normalized to a
48 kHz mono 16-bit WAV by `MicRecorder` whatever the device delivers.

On-device AI — text recognition for the capture overlay, plus the video/audio effects (background matting, speech denoising) — runs in another Rust binary, `flicksnap_ai`: the overlay spawns it per OCR press (`flicksnap_ai ocr`, see `crates/flicksnap_capture/CAPTURE_PROTOCOL.md` §3) and `flickSnap.VideoSDK` spawns it per effect job. It embeds its ONNX models (see `crates/flicksnap_ai/assets/models/README.md`; release packages ship its `NOTICE.txt` of model attributions beside the exe). ONNX Runtime is linked statically by the [ort](https://ort.pyke.io) crate, whose build downloads pyke's prebuilt runtime for the target (first build needs the network; verified against the hashes in `ort-sys`'s `dist.tsv`). Hardware execution providers are on by default per platform — DirectML on Windows (any DX12 GPU, no user-installed runtime; deliberately not CUDA, which would require the user's own CUDA toolkit), CoreML on Apple Silicon — and fall back to the CPU at runtime. One exception, spelled out at `CoreMl::Unsupported` in `crates/flicksnap_ai/src/main.rs`: the 48 kHz speech enhancer stays off CoreML, which miscompiles DPDFNet's recurrent state to the wrong rank and then fails every inference. That is a provider getting a graph wrong rather than a machine lacking one, so there is nothing to detect at registration and nothing to fall back from; DirectML compiles the same model correctly, so it costs Windows nothing. On Windows on Windows `DirectML.dll` must sit beside the exe, where cargo stages it and CI packages it from. On macOS the binary targets macOS 13.4, the minimum of the ONNX Runtime build it links, and its build script links compiler-rt for the runtime's `@available` checks (see `crates/flicksnap_ai/build.rs`). Upstream ONNX Runtime no longer supports macOS x86_64, so the crate is not built on the osx-x64 leg; the app grays the AI effects out on Intel Macs and the overlay reports OCR as unavailable there.

On Windows there is also `flicksnap_shell_ext`, a small COM dll (`IExplorerCommand`) that adds the "Upload with flickSnap" entry to the Windows 11 context menu. It is registered as a *sparse* MSIX package: CI substitutes version/arch into `crates/flicksnap_shell_ext/msix/AppxManifest.template.xml`, packs it with `makeappx` into `flickSnapShellExt.msix` (manifest + logos only — the dll stays outside the package as external content), and release.yml signs the msix with Azure Trusted Signing before Velopack bundles it. At install time the app copies the dll to the install root and registers the msix with `Add-AppxPackage -ExternalLocation` (see `SparsePackageManager`). The installed dll is named for the crate's own version (`flickSnapShellExt_<version>.dll`, e.g. `flickSnapShellExt_1.0.4.dll`, independent of the app version) — **bump the `flicksnap_shell_ext` crate version whenever the extension changes**, so the update lands under a fresh filename that Explorer's lock on the old copy cannot block; unchanged versions are never re-copied. The crate compiles to an empty library on non-Windows so workspace-wide builds still work on macOS.

### Global hotkeys (flicksnap_hotkeys)

Global hotkeys, gesture recording in the hotkey settings and the Draw on Screen keystroke overlay all
go through one keyboard hook in the `flicksnap_hotkeys` crate (`WH_KEYBOARD_LL` on Windows, a
`CGEventTap` on macOS, which needs the Accessibility permission; there is no Linux backend yet). It
is a plain Rust library: the capturer links it, and flickSnap reaches it through `flicksnap_interop`'s
`flicksnap_hotkeys_*` and `flicksnap_gesture_*` exports. The C# key enum
(`ui/flickSnap.Shared/Hotkeys/HotkeyKey.g.cs`), the status codes (`HotkeyErrorCodes.g.cs`) and the
keystroke overlay's keycap legends are generated from the crate's tables by `cargo codegen`.

### Native interop (flicksnap_interop)

C# reaches native code only through `crates/flicksnap_interop`, one library loaded in-process by
flickSnap, flickSnap.VideoRender and the test hosts (`flicksnap_interop.dll` / `libflicksnap_interop.dylib`,
shipped beside the flickSnap executable). Its C ABI is a thin layer over the shared Rust crates
(`flicksnap_platform`, `flicksnap_desktop`, `flicksnap_shared`, `flicksnap_hotkeys`, `flicksnap_ipc`); the rules it keeps are in
`crates/flicksnap_interop/README.md`. The C# side, `ui/flickSnap.Interop`, is generated plus a thin
hand-written wrapper per area.

Build it before running flickSnap or any test project from a checkout:

```sh
cargo build -p flicksnap_interop
```

The loader finds it like a helper (above), with two additions: `FLICKSNAP_INTEROP_PATH` names the file
outright (set but not an existing file means unavailable, no fallback), and the dev walk-up also
looks in `target/{debug,release}/build/flicksnap_interop/*/out/`, where building a dependent crate leaves it. A Debug build of
`ui/flickSnap.Interop` warns when no library is found. A library built from different `src/ffi` sources
than the C# bindings is refused at load (the ABI hash, README section E): run `cargo codegen`, then
build again.

Everything generated from Rust into C# comes from one command in `crates/flicksnap_codegen` (the
`DllImport`s, structs and enums of `ui/flickSnap.Interop/Generated`, the ABI hash, the shared
environment variable names, the helper message catalogs and names, the hotkey tables, and the
contract constants shared with the Rust helpers):

```sh
cargo codegen            # rewrite every generated file
cargo codegen --check    # list what is stale, change nothing
cargo test -p flicksnap_codegen   # the gate: fails on a stale or orphaned file and on any crate rule
```

Edit the Rust source and regenerate; never edit a `.g.cs` file by hand. Hand-written native interop
anywhere else in `ui/` fails `tools/check-native-interop.sh`; the few places that keep it (the loader,
the callback thunks, FFmpeg's buffer callbacks) are listed in
`tools/native-interop-allowlist/permanent.txt`.

While flickSnap, flickSnap.VideoRender or a test host runs it holds the library open, so relinking it
fails on Windows (LNK1104). Close them (and the standby `flicksnap_capture`) before rebuilding.

The Windows half compiles on any machine with the target installed, which is how to check a change
to it from a Mac:

```sh
rustup target add x86_64-pc-windows-msvc
cargo clippy -p flicksnap_interop --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

The Avalonia previewer loads flickSnap's views in its own process, which needs the library too for
any view that formats a gesture or reads an OS value, so build it before previewing those.

## Recording stack (libobs)

Screen recording runs in a separate process, `flicksnap_recorder`, a Rust front end over libobs.
libobs is compiled from the `crates/obs-studio` submodule (pinned to **33.0.1**),
so this part needs a full native toolchain:

- the submodule, recursively: `git submodule update --init --recursive` (or clone with
  `--recursive`). The disabled `obs-browser` / `obs-websocket` submodule dirs must exist, or their
  CMakeLists fail.
- `cmake` ≥ 3.28
- **Windows** — Visual Studio 2022 or 2026 with the C++ workload (the newest installed one picks the
  CMake generator; set `OBS_CMAKE_GENERATOR` to override), and LLVM/`libclang` for `bindgen` (set
  `LIBCLANG_PATH`, e.g. `C:\Program Files\LLVM\bin`, if it is not on `PATH`)
- **macOS** — full **Xcode 26.5+** (not just the Command Line Tools — the Metal renderer and Swift
  are required)
- **Linux** (x86_64 and aarch64) — build in the reference image, `tools/linux-build/Dockerfile`. It
  is a manylinux_2_34 (AlmaLinux 9, glibc 2.34) base with clang/`libclang` (for `bindgen`), `ninja`,
  `nasm`, `patchelf`, Rust, and the development packages of the system libraries the bundle links.
  CI builds in the same image, and building there is what keeps the result portable to every glibc
  2.34+ distribution:

  ```sh
  docker build -t flicksnap-recorder-linux tools/linux-build
  tools/linux-build/run-in-image.sh cargo build --release -p flicksnap_recorder -p flicksnap_vid2gif
  ```

  A native build on another distribution also works, given the same tools and `-dev` packages; the
  Dockerfile is the list. Its output then needs that distribution's glibc or newer. Do **not**
  install the system FFmpeg development packages (`libav*-dev`). The build downloads a pinned FFmpeg
  8.1 shared build (BtbN FFmpeg-Builds, SHA-256 verified), builds pinned x264 and Mbed TLS from
  source, and fetches a pinned SIMDe, all into `crates/obs-studio/.deps`, so the first build needs access
  to github.com. libobs, every plugin and `flicksnap_vid2gif` share that one FFmpeg.
  `flicksnap_share_region` is not built on Linux (there is no Linux UI), so Linux builds name
  `-p flicksnap_recorder -p flicksnap_vid2gif` rather than using `cargo rec`.

Then, from the repository root:

```sh
cargo rec                         # = cargo build -p flicksnap_recorder -p flicksnap_share_region -p flicksnap_vid2gif
cargo recr                        # the same, --release
```

The first build compiles libobs and takes 20–40 minutes; later builds reuse it. OBS derives its
version from `git describe --tags`, and a submodule checkout usually has no tags, so the build
passes the version itself (`OBS_VERSION_OVERRIDE`, default the pinned 33.0.1, sets it). On
Windows, OBS's child configures for the 32-bit helpers ignore that and still run `git describe`,
so before configuring, the build script gives HEAD a local lightweight tag of that version when it
has none. No manual tagging is needed, in CI or locally.

The build tree records a successful configure, together with the checkout and the submodule commit
it was for, so a failed configure, a moved clone or a submodule bump configures again on the next
build. If a libobs build still fails after you fixed the cause, delete `target/obs-<arch>` and
`crates/obs-studio/build_x86` (plus `build_x64` on Windows ARM64) and build again.

`OBS_BUILD_DIR` moves the libobs build tree (default `target/obs-<arch>` on Windows and Linux; on
macOS it lives in the `flicksnap_obs_sys` build script's `OUT_DIR`). Several worktrees can share one
tree only with two caveats. The tree keeps compiling the `crates/obs-studio` sources of the
worktree that configured it, so that worktree has to stay in place and on the same OBS commit. And
only that configure downloads the OBS deps, so every other worktree needs its
`crates/obs-studio/.deps` pointed at the first one's before its first build (a directory junction
on Windows, `mklink /J crates\obs-studio\.deps <first>\crates\obs-studio\.deps`, or a symlink
elsewhere). A shared tree that does not match the checkout is never wiped: the build stops and
names it instead.

The build scripts stage the runtime next to the binaries:

- **Windows** — `flicksnap_recorder.exe`, `flicksnap_share_region.exe` and `flicksnap_vid2gif.exe` plus
  `obs.dll`, the graphics/plugin DLLs, `obs-ffmpeg-mux.exe`, the FFmpeg & x264 runtime DLLs,
  `core/` (OBS's per-module directories) and `data/` end up in `target/<profile>/`.
- **Linux** — like Windows: `libobs.so.30`, `libobs-opengl.so.30`, the FFmpeg `.so` files,
  `obs-ffmpeg-mux`, `obs-plugins/*.so` and `data/` are copied into `target/<profile>/`, each with an
  `$ORIGIN`-relative RUNPATH. The executables also carry absolute RUNPATHs into the build tree, so
  cargo's test binaries resolve. `tools/linux-build/stage-bundle.sh` removes those for the bundle and
  checks it, and `tools/linux-build/verify-bundle.sh` runs it end to end under Xvfb; CI runs both.
  `build.sh` stages through `stage-bundle.sh` too, so a local Linux package build needs `patchelf`
  and binutils (`readelf`, `objdump`) and is held to the same rules, except that the glibc ceiling
  is the build machine's own. A native build that links `libstdc++` or a system FFmpeg, x264 or
  Mbed TLS fails there rather than producing a bundle.
- **macOS** — the binaries link `libobs.framework`, the graphics modules and the plugins straight
  out of the OBS build tree (absolute rpaths), and the FFmpeg/x264 dependency dylibs are copied into
  `target/<profile>/` (symlinked aliases preserved, each given an `@loader_path` rpath and ad-hoc
  re-signed), so that, as on Windows, the profile dir holds a loadable FFmpeg runtime. The
  self-contained, relocatable `recorder/` folder (framework + graphics modules + those dylibs +
  `.plugin` bundles, with `@executable_path/Frameworks` rpaths) is assembled by the stage step in
  `.github/workflows/build-recorder.yml`.

The bundled layout is discovered automatically, but libobs's paths can be overridden:
`OBS_PLUGIN_PATH`, `OBS_PLUGIN_DATA_PATH` and `OBS_DATA_PATH` point it at plugin binaries, plugin
data and core data respectively.

Tests (on Windows, put `target/release` on `PATH` first: the test executables need the staged
runtime DLLs):

```sh
cargo test --release -p flicksnap_recorder -p flicksnap_obs_platform -p flicksnap_share_region
cargo test --release -p flicksnap_vid2gif
cargo test --release -p flicksnap_ffmpeg_sys
cargo test --release -p flicksnap_recorder --test smoke -- --ignored   # end-to-end: records ~3 s and validates the MP4
```

The smoke test is `--ignored` by default because it needs a real display and the assembled OBS
runtime next to the binary. On Linux it needs an X server, e.g.
`xvfb-run -a -s "-screen 0 1280x720x24 +extension RANDR" cargo test --release -p flicksnap_recorder --test smoke -- --ignored`
(on a blank Xvfb screen the recordings compress below the tests' size threshold, so put something
moving on it, such as `glxgears`).

| Crate | Role |
| --- | --- |
| `crates/flicksnap_obs_sys` | Raw FFI bindings to libobs (via `bindgen`); its build script compiles OBS from the submodule with CMake (`links = "obs"`). |
| `crates/flicksnap_obs` | Safe, RAII Rust wrappers over the libobs FFI (context, sources, scenes, encoders, output, signals). |
| `crates/flicksnap_obs_platform` | Shared platform layer: monitor enumeration, OBS paths, display-capture settings and region math, used by the recorder and the share-region helper. |
| `crates/flicksnap_ffmpeg_sys` | Raw FFI bindings to the FFmpeg libraries the recorder ships (`links = "ffmpeg"`). |
| `crates/flicksnap_build_support` | Build-script helpers (the pinned Linux FFmpeg/x264/Mbed TLS runtime). |
| `crates/flicksnap_recorder` | The recorder binary: CLI, region planning, encoder configuration, the command run loop, and the platform capture back-ends. |
| `crates/flicksnap_share_region` | Mirrors a screen region into an ordinary window for meeting apps that can only share a whole screen or one window. Windows and macOS only. |
| `crates/flicksnap_vid2gif` | Converts a video into an optimized GIF by linking the bundled FFmpeg libraries. |

## Tests

```cmd
cargo test -p flicksnap_platform
cargo test -p flicksnap_interop
cargo test -p flicksnap_codegen
cargo test -p flicksnap_ipc
cargo test -p flicksnap_shared
cargo test -p flicksnap_desktop
cargo test -p flicksnap_telemetry
cargo test -p flicksnap_hotkeys
cargo test -p flicksnap_capture
cargo test -p flicksnap_scroll_driver
cargo test -p flicksnap_shell_ext
cargo test -p flicksnap_ai
cargo build -p flicksnap_interop
dotnet test ui/flickSnap.Interop.Tests/flickSnap.Interop.Tests.csproj
dotnet test ui/flickSnap.Ipc.Tests/flickSnap.Ipc.Tests.csproj
dotnet test ui/flickSnap.VideoSDK.Tests/flickSnap.VideoSDK.Tests.csproj
dotnet test ui/flickSnap.Shared.Tests/flickSnap.Shared.Tests.csproj
dotnet test ui/flickSnap.Drawing.Tests/flickSnap.Drawing.Tests.csproj
```

Tests that need `flicksnap_interop` skip when it is missing; set `FLICKSNAP_INTEROP_REQUIRED=1` (CI does)
to make them fail instead. flickSnap has a probe of every interop wrapper and kept facade,
`flickSnap --interop-probe <out.json>`, which CI runs on both OSes.

The recorder crates' tests are listed under [Recording stack (libobs)](#recording-stack-libobs).

The upload relay server lives in a separate repository.
