<#
.SYNOPSIS
    Prepares the optional AI tooling (CodeGraph + RTK) for flickSnap on this Windows machine.

.DESCRIPTION
    Safe to run repeatedly. Every step checks the current state first and only acts on
    what is missing:

      1. Finds the repository root wherever the script is started from.
      2. Checks prerequisites and the CodeGraph / RTK CLIs. A missing tool is offered for
         installation (pinned, tested version, official source) only after confirmation.
         Installed tools are never reinstalled or upgraded.
      3. Detects Claude Code and Codex (CLI on PATH or VS Code-family extension) and wires
         CodeGraph MCP and RTK hooks into each with the tools' own commands
         (`codegraph install`, `rtk init`). Agent config files are backed up first.
      4. Builds the local CodeGraph index from the shared codegraph.json if there is none,
         otherwise syncs it incrementally when files changed.

    It never disables a sandbox, never trusts a hook on your behalf, never edits
    application source and never runs as part of a build. CodeGraph and RTK are optional:
    flickSnap builds, tests and runs without them.

.PARAMETER DryRun
    Preview only. Reports what would be done and shows RTK's own --dry-run preview;
    nothing is installed, written or indexed.

.PARAMETER Yes
    Answer "yes" to the install prompts (non-interactive use). Only the pinned versions
    are installed, and only for tools that are missing.

.PARAMETER Agents
    Which agents to configure: claude, codex (default: both, when detected).

.PARAMETER SkipInstall
    Never offer to install missing tools.

.PARAMETER SkipAgents
    Do not touch any agent configuration.

.PARAMETER SkipIndex
    Do not build or sync the local CodeGraph index.

.PARAMETER NoCodeGraphAutoAllow
    Pass --no-permissions to `codegraph install`, so Claude Code keeps asking before each
    CodeGraph MCP call instead of auto-allowing mcp__codegraph__*.

.EXAMPLE
    powershell -NoProfile -File scripts\ai\setup.ps1 -DryRun

.EXAMPLE
    powershell -NoProfile -File scripts\ai\setup.ps1
#>
[CmdletBinding()]
param(
    [switch]$DryRun,
    [switch]$Yes,
    [ValidateSet('claude', 'codex')]
    [string[]]$Agents = @('claude', 'codex'),
    [switch]$SkipInstall,
    [switch]$SkipAgents,
    [switch]$SkipIndex,
    [switch]$NoCodeGraphAutoAllow
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')

$script:Failures = 0
$script:Todo = New-Object System.Collections.Generic.List[string]
$script:Changed = $false
$script:BackupDir = $null
$script:RestartAgents = New-Object System.Collections.Generic.List[string]

function Add-Todo([string]$Text) { if (-not $script:Todo.Contains($Text)) { $script:Todo.Add($Text) } }
function Add-Restart([string]$Name) { if (-not $script:RestartAgents.Contains($Name)) { $script:RestartAgents.Add($Name) } }

function Confirm-Ai([string]$Question) {
    if ($DryRun) { return $false }
    if ($Yes) { return $true }
    if ([Console]::IsInputRedirected) {
        Write-AiLine 'WARN' "$Question -> no interactive console; re-run with -Yes to accept."
        return $false
    }
    $answer = Read-Host "  $Question [y/N]"
    return ($answer -match '^(y|yes)$')
}

# Copies every agent config file that exists into one timestamped folder, once per run,
# before the first change.
function Backup-AiAgentConfigs {
    if ($script:BackupDir) { return }
    $stamp = "$(Get-Date -Format 'yyyyMMdd-HHmmss')-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
    $dest = Join-Path $HOME ".config-backups\flicksnap-ai-setup-$stamp"
    $c = Get-AiClaudePaths
    $x = Get-AiCodexPaths
    $items = @(
        @{ Src = $c.Json;         Rel = 'claude\claude.json' },
        @{ Src = $c.Settings;     Rel = 'claude\settings.json' },
        @{ Src = $c.Instructions; Rel = 'claude\CLAUDE.md' },
        @{ Src = $c.RtkMd;        Rel = 'claude\RTK.md' },
        @{ Src = $x.Config;       Rel = 'codex\config.toml' },
        @{ Src = $x.Hooks;        Rel = 'codex\hooks.json' },
        @{ Src = $x.Instructions; Rel = 'codex\AGENTS.md' },
        @{ Src = $x.RtkMd;        Rel = 'codex\RTK.md' }
    )
    $manifest = foreach ($i in $items) {
        [pscustomobject]@{
            Path = $i.Src
            Backup = if (Test-Path -LiteralPath $i.Src -PathType Leaf) { $i.Rel } else { $null }
        }
    }
    $null = New-Item -ItemType Directory -Path $dest
    $copied = 0
    foreach ($i in $items) {
        if (Test-Path -LiteralPath $i.Src -PathType Leaf) {
            $target = Join-Path $dest $i.Rel
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
            Copy-Item -LiteralPath $i.Src -Destination $target
            $copied++
        }
    }
    ConvertTo-Json -InputObject @($manifest) | Set-Content -LiteralPath (Join-Path $dest 'manifest.json') -Encoding UTF8
    $script:BackupDir = $dest
    Write-AiLine 'INFO' "Backed up $copied agent config file(s) to $dest"
}

function Invoke-AiStep([string]$Description, [string]$FilePath, [string[]]$Arguments) {
    $shown = "$([IO.Path]::GetFileName($FilePath)) $($Arguments -join ' ')"
    if ($DryRun) { Write-AiLine 'PLAN' "$Description  ->  $shown"; return $true }
    Write-AiLine 'DO' "$Description  ->  $shown"
    $r = Invoke-AiNative -FilePath $FilePath -Arguments $Arguments
    $r.Output | Where-Object { $_ -and $_.Trim() } | ForEach-Object { Write-Host "         $_" -ForegroundColor DarkGray }
    if ($r.ExitCode -ne 0) {
        Write-AiLine 'FAIL' "$Description failed (exit $($r.ExitCode))."
        $script:Failures++
        return $false
    }
    $script:Changed = $true
    return $true
}

# ================================================================ 1. root

Write-AiHeader 'Project'
$Root = Get-AiProjectRoot $PSScriptRoot
Write-AiLine 'OK' "Repository root: $Root"
if ($DryRun) { Write-AiLine 'INFO' 'Dry run: nothing will be installed, written or indexed.' }

$cgConfig = Join-Path $Root 'codegraph.json'
if (Test-Path -LiteralPath $cgConfig) {
    try {
        $null = Get-Content -LiteralPath $cgConfig -Raw | ConvertFrom-Json
        Write-AiLine 'OK' 'codegraph.json found (shared index exclusions are used as-is).'
    } catch {
        Write-AiLine 'FAIL' "codegraph.json is not valid JSON: $_"
        $script:Failures++
    }
} else {
    Write-AiLine 'WARN' 'codegraph.json is missing; the index would include the third-party OBS submodule. Restore it from git.'
}

# ================================================================ 2. prerequisites + tools

Write-AiHeader 'Prerequisites'
Update-AiSessionPath
Write-AiLine 'INFO' "Windows PowerShell $($PSVersionTable.PSVersion)"
$gitCmd = Find-AiCommand 'git'
if ($gitCmd) { Write-AiLine 'OK' ("git: " + (Invoke-AiNative -FilePath $gitCmd.Source -Arguments @('--version')).Text) }
else { Write-AiLine 'WARN' 'git not found (root detection fell back to marker files).' }
$npmCmd = Find-AiCommand 'npm'
$wingetCmd = Find-AiCommand 'winget'
if ($npmCmd) { Write-AiLine 'OK' ("npm " + (Invoke-AiNative -FilePath $npmCmd.Source -Arguments @('--version')).Text + " (used only to install CodeGraph if missing)") }
else { Write-AiLine 'INFO' 'npm not found; a missing CodeGraph would be installed with its official PowerShell installer.' }
if ($wingetCmd) { Write-AiLine 'OK' 'winget available (used only to install RTK if missing).' }
else { Write-AiLine 'INFO' 'winget not found; a missing RTK has to be installed manually (see docs/AI_SETUP.md).' }

Write-AiHeader 'Tools'

function Resolve-AiTools {
    $script:CG = Get-AiCodeGraph
    $script:RTK = Get-AiRtk
}
Resolve-AiTools

function Report-AiTool([string]$Name, $Tool, [string]$Tested) {
    if (-not $Tool.Present) { Write-AiLine 'WARN' "$Name not found."; return }
    if (-not $Tool.Works) {
        $detail = Get-AiProp $Tool 'Detail'
        if (-not $detail) { $detail = 'Version probe failed.' }
        Write-AiLine 'FAIL' "$Name at $($Tool.Path): $detail"
        $script:Failures++
        return
    }
    if ($Tool.Version -ne $Tested) {
        Write-AiLine 'WARN' "$Name $($Tool.Version) ($($Tool.Path)); tested with $Tested. Left as is - see docs/AI_SETUP.md, 'Updating tools'."
    } else {
        Write-AiLine 'OK' "$Name $($Tool.Version) ($($Tool.Path))"
    }
}

Report-AiTool 'CodeGraph' $script:CG $AiTested.CodeGraph
Report-AiTool 'RTK' $script:RTK $AiTested.Rtk

if (-not $SkipInstall) {
    if (-not $script:CG.Present) {
        $cgv = $AiTested.CodeGraph
        if ($npmCmd) {
            $desc = "Install CodeGraph $cgv (npm, official package $CodeGraphNpmPackage)"
            if ($DryRun) { Write-AiLine 'PLAN' "$desc  ->  npm install -g $CodeGraphNpmPackage@$cgv" }
            elseif (Confirm-Ai "$desc?") {
                $null = Invoke-AiStep $desc $npmCmd.Source @('install', '-g', "$CodeGraphNpmPackage@$cgv")
            } else { Add-Todo "Install CodeGraph: npm install -g $CodeGraphNpmPackage@$cgv" }
        } else {
            $desc = "Install CodeGraph v$cgv (official install.ps1, pinned with CODEGRAPH_VERSION)"
            $inner = "`$ErrorActionPreference='Stop'; `$env:CODEGRAPH_VERSION='v$cgv'; irm '$CodeGraphInstallPs1' | iex"
            if ($DryRun) { Write-AiLine 'PLAN' "$desc  ->  powershell -NoProfile -Command `"$inner`"" }
            elseif (Confirm-Ai "$desc?") {
                $ps = (Get-Command powershell.exe).Source
                $null = Invoke-AiStep $desc $ps @('-NoProfile', '-Command', $inner)
            } else { Add-Todo "Install CodeGraph: $inner" }
        }
    }
    if (-not $script:RTK.Present) {
        $rv = $AiTested.Rtk
        if ($wingetCmd) {
            $desc = "Install RTK $rv (winget, official package $RtkWingetId)"
            $wargs = @('install', '--id', $RtkWingetId, '--exact', '--version', $rv, '--source', 'winget')
            if ($Yes) { $wargs += @('--accept-package-agreements', '--accept-source-agreements') }
            if ($DryRun) { Write-AiLine 'PLAN' "$desc  ->  winget $($wargs -join ' ')" }
            elseif (Confirm-Ai "$desc?") { $null = Invoke-AiStep $desc $wingetCmd.Source $wargs }
            else { Add-Todo "Install RTK: winget install --id $RtkWingetId --exact --version $rv" }
        } else {
            Add-Todo "Install RTK $rv manually from https://github.com/rtk-ai/rtk/releases (rtk-x86_64-pc-windows-msvc.zip) and put rtk.exe on PATH."
        }
    }
    if (-not $DryRun -and (-not $script:CG.Present -or -not $script:RTK.Present)) {
        $hadCodeGraph = $script:CG.Present
        $hadRtk = $script:RTK.Present
        Update-AiSessionPath   # installers change PATH in the registry, not in this process
        Resolve-AiTools
        if (-not $hadCodeGraph -and $script:CG.Present) { Report-AiTool 'CodeGraph' $script:CG $AiTested.CodeGraph }
        if (-not $hadRtk -and $script:RTK.Present) { Report-AiTool 'RTK' $script:RTK $AiTested.Rtk }
    }
}

foreach ($t in @(@{ N = 'codegraph'; Tool = $script:CG }, @{ N = 'rtk'; Tool = $script:RTK })) {
    if ($t.Tool.Present -and (Find-AiOnPersistentPath $t.N) -ne $t.Tool.Path) {
        Add-Todo "The persistent PATH does not resolve '$($t.N)' to the executable selected in this session."
    }
}

# ================================================================ 3. agents

$claude = Find-AiClaudeCode
$codex = Find-AiCodex

Write-AiHeader 'Agents'
foreach ($a in @($claude, $codex)) {
    if (-not $a.Detected) { Write-AiLine 'INFO' "$($a.Name): not detected"; continue }
    $where = @()
    if ($a.CliPath) { $where += "CLI $($a.CliPath)" }
    if ($a.ExtExe)  { $where += "editor extension $($a.ExtExe)" }
    if (-not $where) { $where += "config dir $($a.Paths.Dir) only" }
    Write-AiLine 'OK' "$($a.Name): $($where -join '; ')"
}

function Set-AiCodeGraphMcp($Agent) {
    $configured = if ($Agent.Id -eq 'claude') { Test-AiClaudeCodeGraphMcp $Agent $Root } else { Test-AiCodexCodeGraphMcp $Agent $Root }
    if ($configured) { Write-AiLine 'OK' "$($Agent.Name): CodeGraph MCP server already configured (left unchanged)."; return }
    if (-not $script:CG.Works) { Write-AiLine 'WARN' "$($Agent.Name): CodeGraph MCP not configured, and CodeGraph is unavailable - skipped."; return }
    $cgArgs = @('install', "--target=$($Agent.Id)", '--location=global', '--yes')
    if ($NoCodeGraphAutoAllow) { $cgArgs += '--no-permissions' }
    if ($DryRun) {
        Write-AiLine 'PLAN' "$($Agent.Name): add CodeGraph MCP server  ->  codegraph $($cgArgs -join ' ')"
        return
    }
    Backup-AiAgentConfigs
    if (Invoke-AiStep "$($Agent.Name): add CodeGraph MCP server" $script:CG.Path $cgArgs) { Add-Restart $Agent.Name }
}

function Set-AiRtkHook($Agent) {
    if ($Agent.Id -eq 'claude') {
        $hook = Test-AiClaudeRtkHook $Agent $Root
        $rtkArgs = @('init', '-g', '--auto-patch')
    } else {
        $hook = Test-AiCodexRtkHook $Agent $Root
        $rtkArgs = @('init', '-g', '--codex')   # rtk refuses --auto-patch together with --codex
    }
    if ($hook) { Write-AiLine 'OK' "$($Agent.Name): existing RTK hook detected (left unchanged)."; return }
    if (-not $script:RTK.Works) { Write-AiLine 'WARN' "$($Agent.Name): RTK not configured, and RTK is unavailable - skipped."; return }
    if ($DryRun) {
        Write-AiLine 'PLAN' "$($Agent.Name): configure RTK  ->  rtk $($rtkArgs -join ' ')  (RTK's own preview follows)"
        $r = Invoke-AiNative -FilePath $script:RTK.Path -Arguments ($rtkArgs + '--dry-run') -StdIn ' '
        $r.Output | Where-Object { $_ -match '^\[dry-run\]' } | ForEach-Object { Write-Host "         $_" -ForegroundColor DarkGray }
        return
    }
    Backup-AiAgentConfigs
    if (Invoke-AiStep "$($Agent.Name): configure RTK" $script:RTK.Path $rtkArgs) { Add-Restart $Agent.Name }
}

if (-not $SkipAgents) {
    Write-AiHeader 'Agent integration'
    $targets = @()
    if ($Agents -contains 'claude') { $targets += $claude }
    if ($Agents -contains 'codex')  { $targets += $codex }
    foreach ($a in $targets) {
        if (-not $a.Detected) { Write-AiLine 'INFO' "$($a.Name): not detected - skipped."; continue }
        Set-AiCodeGraphMcp $a
        Set-AiRtkHook $a
    }
    if ($targets -contains $codex -and $codex.Detected) {
        $feature = Get-AiCodexHooksFeature $codex
        if ($feature -eq $false) {
            Add-Todo "Codex: the 'hooks' feature is off, so the RTK hook cannot run. Enable it in Codex ($($codex.Paths.Config)) if you want RTK there."
        }
        if (Test-AiCodexRtkHook $codex $Root) {
            Add-Todo "Codex: open Codex interactively once and approve (trust) the 'rtk hook codex' hook when asked. Until then Codex skips it. This script never trusts hooks for you."
        }
    }
}

# ================================================================ 4. index

if (-not $SkipIndex) {
    Write-AiHeader 'CodeGraph index'
    if (-not $script:CG.Works) {
        Write-AiLine 'WARN' 'CodeGraph unavailable - index step skipped (agents fall back to normal search).'
    } else {
        $status = Get-AiIndexStatus $script:CG.Path $Root
        $indexPath = Get-AiProp $status 'indexPath'
        if ($null -eq $status -or (Get-AiProp $status 'worktreeMismatch')) {
            Write-AiLine 'FAIL' 'Cannot verify the CodeGraph index status for this worktree. Index left unchanged.'
            $script:Failures++
        } elseif (-not $status.initialized -and
            (-not $indexPath -or (Test-Path -LiteralPath $indexPath) -or (Test-Path -LiteralPath (Join-Path $Root '.codegraph')))) {
            Write-AiLine 'FAIL' 'CodeGraph reports an uninitialized index but its absence could not be confirmed. Index left unchanged.'
            $script:Failures++
        } elseif (-not $status.initialized) {
            # --local-only: .codegraph/ also ignores itself, a second guard next to .gitignore.
            $null = Invoke-AiStep 'Build the local index (first time)' $script:CG.Path @('init', '--yes', '--local-only', $Root)
        } elseif ([bool](Get-AiProp $status 'rebuildInProgress')) {
            Write-AiLine 'INFO' 'A rebuild is already in progress; leaving it alone.'
        } else {
            $pending = Get-AiPendingCount $status
            if ($pending -gt 0) {
                $null = Invoke-AiStep "Sync the index ($pending changed files)" $script:CG.Path @('sync', $Root)
            } else {
                Write-AiLine 'OK' ("Index up to date: {0} files, {1} nodes, {2} edges." -f (Get-AiProp $status 'fileCount'), (Get-AiProp $status 'nodeCount'), (Get-AiProp $status 'edgeCount'))
            }
        }
    }
}

# ================================================================ summary

Write-AiHeader 'Summary'
if ($DryRun) { Write-AiLine 'INFO' 'Dry run finished; nothing was changed.' }
elseif (-not $script:Changed) { Write-AiLine 'OK' 'Nothing to do - the environment was already set up.' }
if ($script:BackupDir) { Write-AiLine 'INFO' "Config backup: $($script:BackupDir)" }
if ($script:RestartAgents.Count -gt 0) {
    Add-Todo ("Restart " + ($script:RestartAgents -join ' and ') + " (and VS Code, if they run inside it) so they load the new MCP server / hooks and PATH.")
}
foreach ($t in $script:Todo) { Write-AiLine 'TODO' $t }
Write-AiLine 'INFO' 'Check the result with: powershell -NoProfile -File scripts\ai\doctor.ps1'
if ($script:Failures -gt 0) { exit 1 }
exit 0
