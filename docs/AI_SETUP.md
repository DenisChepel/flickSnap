# AI development environment (CodeGraph + RTK)

flickSnap can be worked on with AI coding agents (Claude Code and Codex) assisted by two
optional tools:

| Tool | What it does for the agent | Tested version |
|---|---|---|
| [CodeGraph](https://github.com/colbymchenry/codegraph) | A local code graph of the repository (symbols, calls, references) served over MCP as `codegraph_explore`, so the agent can find code and its callers in one call instead of many grep/read rounds. | 1.6.4 |
| [RTK](https://github.com/rtk-ai/rtk) | A hook that rewrites common shell commands (`git status`, `cargo test`, `ls`, `grep`, `dotnet build`, ...) to `rtk <cmd>`, which condenses their output before the agent reads it. | 1.2.1 |

**Both are optional.** Nothing in the build, the tests or the app depends on them. No build
step, CI job or agent start installs them. Without them, agents fall back to their normal
tools (search, reading files, running commands as is).

The setup and diagnosis scripts target Windows PowerShell 5.1. The tested versions are
pinned in [`scripts/ai/common.ps1`](../scripts/ai/common.ps1) (`$AiTested`).

## What lives in the repository and what stays on your machine

| Path | In git | Purpose |
|---|---|---|
| `codegraph.json` | yes | Shared CodeGraph project config. Keeps the third-party OBS submodule (`crates/obs-studio/`, ~15k files) out of the index. Relative paths only. |
| `.gitignore` (CodeGraph block) | yes | Ignores `.codegraph/` and `.codegraph-*/` (for example `.codegraph-wsl/`). |
| `scripts/ai/setup.ps1`, `doctor.ps1`, `common.ps1` | yes | Setup and diagnostics. |
| `.codegraph/` | **no** | The local index (SQLite, ~140 MB). Built per machine. |
| `~/.claude.json`, `~/.claude/settings.json`, `~/.claude/CLAUDE.md`, `~/.claude/RTK.md` | no | Claude Code user-level config written by the tools' own installers. |
| `~/.codex/config.toml`, `~/.codex/hooks.json`, `~/.codex/AGENTS.md`, `~/.codex/RTK.md` | no | Codex user-level config written by the tools' own installers. |

`CLAUDE_CONFIG_DIR` and `CODEX_HOME` are honoured if you set them.

## Setting up a new machine

Prerequisites: Windows 10/11, Windows PowerShell 5.1, git, and at least one agent installed
(Claude Code CLI or VS Code extension, and/or Codex CLI or the VS Code "Codex" extension).
To install the tools, the script uses `npm` for CodeGraph (or CodeGraph's official
PowerShell installer when npm is missing) and `winget` for RTK.

```powershell
git clone <repo-url> flickSnap
cd flickSnap

# 1. Preview. Nothing is installed, written or indexed.
powershell -NoProfile -File scripts\ai\setup.ps1 -DryRun

# 2. Apply. Asks before installing anything that is missing.
powershell -NoProfile -File scripts\ai\setup.ps1

# 3. Restart your agents (and VS Code if they run inside it).
# 4. Codex only: approve the RTK hook once (see "Manual trust steps").

# 5. Check.
powershell -NoProfile -File scripts\ai\doctor.ps1 -Live
```

If your execution policy is `Restricted`, start the scripts with
`powershell -NoProfile -ExecutionPolicy RemoteSigned -File ...`. That relaxes the policy for
that one process only.

### What `setup.ps1` does

It can run from any directory. It finds the repository root itself (via git, or via
`Cargo.toml` + `flickSnap.slnx`). Each step first checks what is already there and only acts
on what is missing, so running it again is safe.

1. **Tools.** Checks `codegraph` and `rtk` and their versions. It also confirms that `rtk`
   really is rtk-ai/rtk: crates.io has an unrelated crate with the same name.
   - Missing tool: after you confirm, it installs the pinned version from the official
     source (`npm install -g @colbymchenry/codegraph@1.6.4`, or the official
     [Windows installer at tag v1.6.4](https://github.com/colbymchenry/codegraph/blob/v1.6.4/install.ps1)
     with `CODEGRAPH_VERSION=v1.6.4`; `winget install --id rtk-ai.rtk --exact --version 1.2.1`).
   - Installed tool: never reinstalled or upgraded. A different version is only reported.
   - Setup preserves the current process PATH order and appends registry entries and known
     installer directories. It refreshes the lookup after an install. The persistent PATH
     check compares the executable selected from registry entries with the one selected
     in the current session, so a missing entry and a shadowed executable are distinguishable.
2. **Agents.** Detects Claude Code (`claude` on PATH, or the VS Code-family extension's
   bundled `claude.exe`) and Codex (`codex` on PATH, or the extension's bundled `codex.exe`
   under `~/.vscode*/extensions/openai.chatgpt-*`). For each detected agent:
   - CodeGraph MCP: `codegraph install --target=<claude|codex> --location=global --yes` if
     setup finds no existing integration. It checks Claude's user and checkout entries in
     `.claude.json` and project `.mcp.json`, and Codex's user and project `config.toml`.
     It recognizes the standard server name and aliases that launch CodeGraph directly.
   - RTK: `rtk init -g --auto-patch` (Claude Code) / `rtk init -g --codex` (Codex), only if
     no recognized RTK hook exists. It checks user and project `PreToolUse` hooks, including
     Claude's `.claude/settings.local.json` and Codex's inline hook tables. An existing hook
     is preserved even if the RTK instruction file is absent.
   - These checks report configuration presence. Agents determine trust and effective
     settings. Wrapper commands, plugin and managed configurations require manual inspection.
   - Before the first agent change, setup backs up the user files listed below.
3. **Index.** Setup runs `codegraph init --yes --local-only <root>` only when a valid status
   reports an uninitialized index with an absent index directory. Failed or invalid
   status, a worktree mismatch or an existing uninitialized directory stops the index step.
   That uses `codegraph.json`, `.gitignore` and CodeGraph's built-in ignores (`target/`,
   `bin/`, `obj/`, `node_modules/` and similar). If the index exists and CodeGraph reports
   changed files, it runs `codegraph sync <root>`. Otherwise it leaves the index alone.
4. **Summary.** Lists what to restart and what you must confirm by hand.

| Switch | Effect |
|---|---|
| `-DryRun` | Preview only. Includes RTK's own `--dry-run` output for agent changes. |
| `-Yes` | Accept the install prompts (pinned versions, missing tools only). Needed when there is no console, e.g. in a script. |
| `-Agents claude` / `-Agents codex` | Configure only that agent. |
| `-SkipInstall`, `-SkipAgents`, `-SkipIndex` | Skip a phase. |
| `-NoCodeGraphAutoAllow` | Passes `--no-permissions` to `codegraph install`, so Claude Code keeps asking before each CodeGraph MCP call. |

The script never disables a sandbox, never trusts hooks for you, never edits application
source, and never commits.

### What `doctor.ps1` checks

`doctor.ps1` is read-only. It checks:

- the tools, their versions and whether they are on the persistent PATH;
- the per-agent MCP entries, RTK hooks and instruction blocks;
- whether Codex's `hooks` feature is enabled;
- the index (counts, pending changes, `codegraph status --strict`);
- git hygiene: `.codegraph*/` is ignored and untracked, `codegraph.json` is not ignored and
  contains only relative paths and no credentials, and the OBS exclusion is present;
- the presence of these docs and scripts.

It separates "configured" from "works":

- **Default.** Reads configuration and runs version, feature and index-status commands.
  It checks the expected user-level config locations. A configured hook or MCP entry
  can still fail in an agent session.
- **`-Live`.** Adds probes. Each line says what it proves:
  - CodeGraph's MCP server answers `initialize` + `tools/list` over stdio.
  - `claude mcp get codegraph` reports `Connected` (Claude Code's own check).
  - `codex mcp get codegraph` lists the server as enabled.
  - `rtk hook claude` / `rtk hook codex` rewrite a simulated `git status` payload.
- **In-agent confirmation.** Not done by the script. Ask the agent to run `git status` in
  its shell tool, then look for `rtk git status` in `rtk gain --history`. For MCP, ask it to
  use `codegraph_explore`.

`-Json` prints machine-readable results. The exit code is 1 if any check fails.

RTK diagnostics distinguish a missing executable, a launch denied by the sandbox or OS,
a command that explicitly rejects the `rewrite` subcommand, and a failed or unexpected
version or rewrite probe. A successful rewrite probe that returns the original command
still identifies RTK. `-Live` separately tests whether the hook rewrites a sample.

### Backup and manual restoration

Before changing agent integration, setup records the original paths in `manifest.json`
under `~/.config-backups/flicksnap-ai-setup-<timestamp>-<suffix>/`. It copies existing
Claude files `.claude.json`, `settings.json`, `CLAUDE.md`, and `RTK.md`, and Codex files
`config.toml`, `hooks.json`, `AGENTS.md`, and `RTK.md`. A null `Backup` entry means the
file was absent before setup. Review the diff before copying originals back or removing
new files, since the agent may have written other settings afterwards.

The backup covers those user files. Installed packages, persistent PATH changes, local
indexes and any additional files created by external installers require separate review
and manual cleanup.

## Using it with Claude Code and Codex

- **CodeGraph.** When this checkout has `.codegraph/`, agents should call the MCP tool
  `codegraph_explore` before grepping or reading files. Pass `projectPath` = the repository
  root if the agent was started elsewhere. The same output is available in a terminal:
  `codegraph explore "CompositionPlayer VideoPreviewControl"`. Other useful commands:
  `codegraph callers <symbol>`, `codegraph impact <symbol>`, `codegraph query <name>`.
  The installer adds a short instruction block (between `CODEGRAPH_START` and
  `CODEGRAPH_END` markers) to the global `CLAUDE.md` / `AGENTS.md`. It tells agents to skip
  CodeGraph entirely in a repository without `.codegraph/`.
- **RTK.** Works by itself through the hook. Output is condensed, not truncated silently:
  truncated results say how to recover. If a result looks wrong, re-run it raw with
  `rtk bypass <cmd>`. To see the savings: `rtk gain`, `rtk gain --history`. For commands
  the agent ran without RTK: `rtk discover`.

## Manual trust steps

| Agent | What you must do |
|---|---|
| Codex | Codex runs a hook only after you trust it. Open Codex interactively once (CLI or VS Code extension) and approve `rtk hook codex` when asked. Until then Codex silently runs commands without RTK. `doctor.ps1` cannot read the trust state and reports this as `TODO`. Do not use `--dangerously-bypass-hook-trust` as a workaround. |
| Claude Code | User-level hooks from `~/.claude/settings.json` need no approval. Review them with `/hooks`. By default `codegraph install` also auto-allows `mcp__codegraph__*` (read-only code queries). Use `-NoCodeGraphAutoAllow` to keep the prompts. |
| Both | Restart the agent (and VS Code) after setup, so it picks up the MCP server, the hooks and the new `PATH`. |

## Keeping the index fresh

- **Automatic.** A server with an active project watcher catches up after file changes.
  A project selected via `projectPath` may lack a live watcher. Pay attention to stale
  relationship warnings even when `codegraph_explore` shows current source from disk.
- **On demand.** `codegraph sync` (incremental), or re-run `setup.ps1`, which syncs when
  CodeGraph reports pending changes.
- **Status.** `codegraph status` (add `--json`), or `doctor.ps1`.

CodeGraph status lists the indexed languages. Inspect shell scripts, AXAML and Markdown
directly when their contents matter.

### When the index is out of sync

1. `codegraph status`. Look at the pending changes and `worktreeMismatch`.
2. `codegraph sync`.
3. Still wrong (for example after a large branch switch, or an interrupted index):
   `codegraph index` rebuilds from scratch.
4. Indexing blocked by a stale lock: `codegraph unlock`.
5. Last resort: `codegraph uninit -f`, then `scripts\ai\setup.ps1`.

## Windows, PowerShell and sandbox notes

- **The RTK hook only sees "Bash" tool calls.** In Claude Code on Windows, commands run
  through the PowerShell tool and the built-in Read/Grep/Glob tools are not rewritten.
  Codex on Windows runs shell commands through `powershell.exe`. Its hook still receives
  supported shell calls as `Bash`. Successful rewriting still requires the shell to be
  able to execute `rtk.exe`.
- **Blocked RTK binary.** If a rewritten command fails with `Access is denied`, run the
  original executable within the same sandbox. For example, in PowerShell use
  `& (Get-Command git.exe -CommandType Application).Source status`. This call form
  worked in Codex when bare `git status` was rewritten. Use `rtk bypass <cmd>` for raw
  output when RTK itself can run. Leave hook trust and sandbox settings intact.
- **Codex approvals see `rtk`.** Codex's command safety classifier looks at the rewritten
  command. Known-safe commands may prompt more often.
- **Codex sandbox and RTK history.** In Codex's `workspace-write` sandbox, RTK may be unable
  to write its history database (`SQLITE_CANTOPEN`). Command rewriting still works.
  `rtk init --codex` prints an optional `writable_roots` snippet for `config.toml`. The setup
  script leaves sandbox settings intact.
- **PATH.** `winget` and `npm` change the user PATH in the registry. Terminals, VS Code and
  agents started before that need a restart.
- **npm shims.** npm installs `codegraph.ps1` next to `codegraph.cmd`. The scripts call the
  `.cmd`/`.exe`, so a restrictive execution policy does not break them. The agent configs
  call `codegraph` / `codegraph.cmd` / `rtk` through PATH.
- **WSL.** Do not share one `.codegraph/` between Windows and WSL. Under WSL, CodeGraph
  creates `.codegraph-wsl/` on its own, or set `CODEGRAPH_DIR`. `.codegraph-*/` is ignored.
- **Telemetry.** CodeGraph sends anonymous usage stats unless you run
  `codegraph telemetry off` (or set `CODEGRAPH_TELEMETRY=0`). RTK telemetry is off by default.

## Limits of the code graph: C# and Rust

CodeGraph resolves calls and references on a best-effort basis. It does not connect
the two sides of the native boundary:

- **FFI.** The Rust exports (`#[no_mangle] extern "C"` functions in
  `crates/flicksnap_interop/src/ffi/*.rs`) and their C# P/Invoke declarations
  (`ui/flickSnap.Interop/Generated/NativeMethods.g.cs`, generated by
  `crates/flicksnap_codegen`) are two unrelated symbols with the same name.
  `codegraph callers flicksnap_share_region_apply_drag` lists both definitions separately.
  To follow a call from C# into Rust, query the FFI function name and read both sides.
- **IPC and other runtime boundaries.** Messages between processes (`flicksnap_ipc` /
  `flickSnap.Ipc`) are not modelled as calls.
- **XAML.** `.axaml` files are not indexed, so bindings and event handlers declared in
  markup do not show up as references.

Check manifests for project and crate dependencies. Verify critical symbol edges and
runtime wiring against source and configuration.

## Updating the tools

Nothing updates automatically. To move to a new version:

1. Upgrade one tool:
   - `codegraph upgrade <version>` (or `npm install -g @colbymchenry/codegraph@<version>`).
     `codegraph upgrade` refreshes what it configured in each agent.
   - `winget upgrade --id rtk-ai.rtk --exact --version <version>`, then `rtk init -g` and
     `rtk init -g --codex` to migrate the hook format if the release notes say so.
2. Run `powershell -NoProfile -File scripts\ai\doctor.ps1 -Live`, then check in a real agent
   session (see above).
3. Update `$AiTested` in `scripts/ai/common.ps1` and the table at the top of this file, then
   commit, so other machines get the same version.

## Disabling and removing

| Goal | Command |
|---|---|
| Full output for one command | `rtk bypass <cmd>` when RTK can run. See the PowerShell example above when it cannot. |
| Never rewrite some commands | `[hooks] exclude_commands = ["git rebase"]` in `%APPDATA%\rtk\config.toml` |
| Remove RTK from Claude Code | `rtk init -g --uninstall` |
| Remove RTK from Codex | `rtk init -g --codex --uninstall` |
| Remove CodeGraph from agents | `codegraph uninstall --target=claude --location=global --yes` (or `--target=codex`); add `--keep-cli` to keep the CLI |
| Delete this repo's index | `codegraph uninit -f` |
| Uninstall the binaries | `npm uninstall -g @colbymchenry/codegraph` (or `codegraph uninstall`), `winget uninstall --id rtk-ai.rtk` |
| Restore backed-up agent files | use the paths in `manifest.json` as described in Backup and manual restoration above |

Remove the agent hooks before you uninstall the `rtk` binary. A hook that points to a
missing command makes every shell call show a hook error.
