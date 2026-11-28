# flickSnap: instructions for AI agents (Claude Code and Codex)

flickSnap is a tray app for screenshots, screen recording and video editing. The UI is C#
(.NET 10, Avalonia 12, under `ui/`). The capture overlay, the recorder and the other helpers are
Rust (under `crates/`). Windows and macOS ship the app. Linux builds the recorder payload.
This file is shared by both agents
(`CLAUDE.md` imports it).

Prioritize correctness, security and existing behavior. Match dependency analysis and verification to the scope and risk of the change.

## First session on a machine

This section applies on Windows. Skip it when the `codegraph_explore` MCP tool is available
and `.codegraph/` exists, when `build/ai-setup-declined` or the legacy
`.git/ai-setup-declined` exists, or when the user already answered it in this session.
Otherwise, before the first task, even if the user only asked you to get to know the project:

1. Run `powershell -NoProfile -File scripts/ai/doctor.ps1` (read-only) and read its final
   `Setup would help:` line. If a sandbox refuses it with `DotSourceNotSupported`, run
   `& ./scripts/ai/doctor.ps1` in the current PowerShell instead. Check reported failures as
   well as the summary. `Setup would help: none` means there is nothing to offer through
   setup. Report failures that affect the task and proceed with the available tools.
   If the diagnostic cannot run, explain the limitation and continue with the task.
2. Otherwise ask the user one question. List what is missing, in plain words:
   - `build-toolchain`: git, .NET 10 SDK, Rust, MSVC build tools;
   - `ai-tools`: CodeGraph, RTK;
   - `agent-integration`: MCP server and hooks for Claude Code / Codex;
   - `index`: the local `.codegraph/`.

   Offer: set up everything / only some parts / not now / never ask in this clone. Install
   nothing before the answer.
3. **Yes (all or some):** run `scripts/ai/setup.ps1 -DryRun` with the switches that match the
   answer, show the plan, then run the same command with `-Yes` instead of `-DryRun`:
   - only the index: `-SkipBuildTools -SkipInstall -SkipAgents`;
   - no build toolchain: `-SkipBuildTools`;
   - only the build toolchain: `-SkipInstall -SkipAgents -SkipIndex`.

   The script installs software and writes agent configs outside the repository. Request
   that through your normal approval or escalation prompt. If the sandbox cannot run it, give
   the user the one command to run. Warn the user that the .NET SDK and MSVC installers may
   show an administrator (UAC) prompt.
4. **Afterwards:** run `doctor.ps1` again and report the result.
   - Tell the user to restart the agent (and VS Code): the MCP server and hooks load only at
     agent start. The `codegraph explore` CLI works at once.
   - In Codex, the user must approve the `rtk hook codex` trust prompt once.
5. **Not now:** continue with normal search, reading and plain commands, and do not ask again
   in this session.
6. **Never:** create an empty `build/ai-setup-declined` in the ignored `build/` directory
   and do not ask again in this copy. If it cannot be written, honor the refusal for
   this session and explain that it could not be saved. Removing the marker brings the offer back.

## Read on demand

| Task touches | Read first |
|---|---|
| C#/Rust or shared contract changes | `docs/FFI_CONTRACTS.md`; for the native ABI also read `crates/flicksnap_interop/README.md` |
| Several modules or processes, or first orientation | `docs/ARCHITECTURE.md` |
| Helper processes, stdio messages, exit codes, `HelperLocator` | `crates/flicksnap_capture/HELPER_PROTOCOL.md` |
| Capture overlay CLI, session dir, `session.json`, OCR request | `crates/flicksnap_capture/CAPTURE_PROTOCOL.md` |
| Recorder / libobs / FFmpeg staging | `crates/flicksnap_recorder/README.md`, `BUILDING.md` (Recording stack) |
| Build setup, env vars, helper lookup, FFmpeg loading | `BUILDING.md` |
| UI strings / translations | `ui/flickSnap.Shared/Localization/TRANSLATING.md` |
| AI models, cursor packs, share region, vid2gif | `crates/flicksnap_ai/assets/models/README.md`, `ui/flickSnap.VideoSDK/Composition/CursorPacks/README.md`, `crates/flicksnap_share_region/README.md`, `crates/flicksnap_vid2gif/README.md`, respectively |
| CodeGraph / RTK setup or failures | `docs/AI_SETUP.md` |

## Project rules

- **Native interop:**
  - Access project Rust exports through `ui/flickSnap.Interop`. The native-interop gate
    checks hand-written entry points. VideoSDK uses FFmpeg.AutoGen for FFmpeg.
  - Never hand-edit Rust-generated `*.g.cs`. Edit the Rust source, run `cargo codegen`, then
    `cargo test -p flicksnap_codegen`.
  - The generator's sources of truth are listed in `docs/ARCHITECTURE.md`.
- **Changes affecting C#/Rust interop or shared contracts** (see `docs/FFI_CONTRACTS.md`):
  - match FFI by exact export / EntryPoint and other contracts by their wire names or formats;
  - trace relevant callers and consumers separately in C# and Rust;
  - verify ABI, calling conventions, layout, ownership, lifetime, thread contracts, error
    handling and handle leases;
  - build and test both affected sides when the change crosses the managed/native boundary;
  - CodeGraph does not reliably resolve cross-language FFI edges: a missing edge is not
    evidence of independence.
  - Local changes that do not affect interop or shared contracts need only the relevant
    dependency analysis and verification.
- **Hand-mirrored contracts (no generator):**
  - capture and recorder CLI flags, session markers and recording sidecars are mapped in
    `docs/FFI_CONTRACTS.md`;
  - icons, which exist in three places: `assets/icons/`, `crates/flicksnap_capture/assets/icons/`
    and `ui/flickSnap.Ui/Assets/VectorIcons.axaml`.
  - Change every side together.
- **Crate and project boundaries:**
  - `flicksnap_interop` never depends on Sentry, `flicksnap_telemetry`, `flicksnap_ai` or the
    recorder stack, and `flicksnap_codegen` never on the last two (CI gate);
  - `flicksnap_ipc` stays a leaf;
  - VideoSDK and VideoRender must not reference `flickSnap.Shared` (Avalonia);
  - check `Cargo.toml` / `.csproj` / `ui/Directory.Build.props` when you change dependencies.
- **UI:**
  - check the `.axaml` (bindings, `x:Name`, styles, resources, `{loc:T}` keys) together with
    its code-behind. AXAML is not in the code graph;
  - new strings go into `Strings.resx` first.
- **VideoSDK changes:** check the consumers in `flickSnap.Ui` (`VideoEditor/`, `Video/`,
  `Services/`), `flickSnap.VideoRender` and `flickSnap.VideoSDK.Tests`, which also references
  `flickSnap.Ui`.
- **GPU, video, audio, capture and threading code:**
  - verify thread affinity, fence and slot order, ownership and release order of native
    resources, and leaks;
  - a successful build proves none of these. Run the relevant runtime tests, or state that
    they were not run.
- **Pinned versions:**
  - SkiaSharp lives in `ui/Directory.Build.props`;
  - FFmpeg 8.1 library majors are asserted by `FFmpegLoader`;
  - OBS is the `crates/obs-studio` submodule (33.0.1). Never edit the submodule.
  - Bump the `flicksnap_shell_ext` crate version whenever the shell extension changes.
- **New dependencies:** add none (NuGet, crates, tools) without the user's agreement.

## Workflow

- **Before:**
  - define the scope;
  - find the existing implementation and reuse it; never build a parallel one or a second
    source of truth;
  - trace affected callers and contracts. For boundary changes, check the dynamic links in
    `docs/ARCHITECTURE.md`;
  - for risky or cross-module work, write a short plan and record the baseline test state, so
    old failures are not mistaken for regressions.
- **During:**
  - stay inside the task: no unrelated edits, no unrequested refactoring;
  - change no public API, schema or shared contract without an impact analysis;
  - do not suppress errors; fix root causes, or explain a workaround;
  - keep the user's uncommitted changes and other agents' work;
  - no destructive git (`reset --hard`, `checkout --`, `clean`, force push, history rewrite)
    and no commit or push without explicit permission.
- **After:**
  - review the full diff and keep it to the task;
  - re-check affected modules;
  - run the checks the change calls for (below), wider for critical code;
  - never delete or weaken tests to get green;
  - report what was verified, what was not, and the remaining risks. Never claim a check you
    did not run.

## Commands

- **Build** the touched target. See `BUILDING.md` for target commands and native prerequisites.
- **Rust gate** (for affected crates, one crate per call):
  - `cargo fmt --check`;
  - `cargo clippy -p <crate> --all-targets -- -D warnings` (for `flicksnap_capture`, without
    `--all-targets`, as for `flicksnap_shell_ext` in CI);
  - `cargo test -p <crate>`;
  - for interop or codegen work, also `cargo test -p flicksnap_codegen`.
  - For off-OS arms (from Windows): `cargo clippy -p <crate> --target x86_64-unknown-linux-gnu -- -D warnings`.
- **C# tests:**
  - `dotnet test ui/flickSnap.<Interop|Shared|Drawing|Ipc|VideoSDK>.Tests/flickSnap.<...>.Tests.csproj`;
  - for native-dependent tests, build `flicksnap_interop` separately and set `FLICKSNAP_INTEROP_REQUIRED=1` to fail on a missing or stale library;
  - check that the relevant tests executed. FFmpeg, hardware, helper and platform gates can cause skips.
  - There is no C# formatter or linter gate: follow `.editorconfig`, and analyzer warnings
    count.
- **Recorder stack:** `cargo rec` (needs the submodule, CMake and LLVM, and takes long), then
  the tests listed in `BUILDING.md`. Do not start it unless the task needs it.
- **Generated code:** `cargo codegen`, or `cargo codegen --check` to only report.
- **Interop probe:** `flickSnap --interop-probe <out.json>` on a Release build.
- **Locked DLL on Windows:** a running flickSnap, VideoRender, a test host or the standby
  `flicksnap_capture` locks `flicksnap_interop.dll` (LNK1104). Ask the user to close it.
- **Local test build** (existing `build/current/` only): after executable or runtime-resource
  changes, follow the Debug copy and compatibility rules in `BUILDING.md`.

## CodeGraph and RTK

- With an existing `.codegraph/`, use `codegraph_explore` first for structural questions
  (`projectPath` = repo root), or `codegraph explore`. Reuse its impact analysis. Query callers,
  callees or impact separately when needed. Verify critical edges in source and check the
  dynamic links in `docs/ARCHITECTURE.md` for affected boundaries.
- If the graph is stale, check `codegraph status` and sync when needed. Avoid routine rebuilds
  or new indexes. If unavailable, use targeted search and reads.
- Where the RTK hook applies, write normal commands. In an unhooked shell use RTK if available.
  For raw output or a blocked RTK binary, see `docs/AI_SETUP.md`. Keep sandbox, permissions
  and hook trust intact.
- Use `scripts/ai/doctor.ps1` to diagnose tooling issues. Run setup only with the user's
  consent. After MCP or hook changes, tell the user to restart the agent and verify in session.

## Working across agents

- No conversation history carries over between Claude Code and Codex. Resume from the code,
  `git status` / `git diff` and these docs.
- Shared rules live only here. Keep agent-specific notes minimal and consistent with this file.
- Do not edit another agent's global configuration unless the task requires it.

## Keeping docs current

- **When to update:** in the same task as an architectural change, a change to a shared
  contract or source of truth, or a module move. That covers this file,
  `docs/ARCHITECTURE.md`, `docs/FFI_CONTRACTS.md`, and the protocol and crate READMEs.
- **What not to do:**
  - no updates for minor edits;
  - no task logs or history;
  - no planned features described as existing;
  - no volatile statistics or local absolute paths.
- **Stale facts:** delete them when you find them.
