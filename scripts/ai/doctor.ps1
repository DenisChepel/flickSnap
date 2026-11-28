<#
.SYNOPSIS
    Diagnoses the optional AI tooling (CodeGraph + RTK) for flickSnap. Read-only by default.

.DESCRIPTION
    Reports, per check, one of:
      OK    the thing is present / configured as expected
      WARN  works, but something is off (untested version, stale index, ...)
      FAIL  broken or missing in a way that defeats the integration
      INFO  neutral fact
      TODO  needs a person (for example trusting a hook inside Codex)

    Configuration being present is not the same as the integration working inside an agent.
    Lines say "configured" for file-level checks. With -Live the script adds probes that run
    the tools themselves, and labels exactly what each probe proves:
      - CodeGraph MCP server answers an MCP initialize + tools/list over stdio
      - `claude mcp get codegraph` (Claude Code's own connection check)
      - RTK hook binaries answer a simulated PreToolUse payload
    None of these is a live agent session. To confirm the RTK hook end to end, ask the agent
    to run `git status` and look for it in `rtk gain --history`.

    Without -Live the script only reads files and runs version/status commands.

.PARAMETER Live
    Also run the probes above. They start short-lived tool processes but change no settings.

.PARAMETER Json
    Print the results as JSON (for CI or scripts) instead of the coloured list.

.EXAMPLE
    powershell -NoProfile -File scripts\ai\doctor.ps1

.EXAMPLE
    powershell -NoProfile -File scripts\ai\doctor.ps1 -Live
#>
[CmdletBinding()]
param(
    [switch]$Live,
    [switch]$Json
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')

$script:Results = New-Object System.Collections.Generic.List[object]
$script:Section = ''

function Section([string]$Name) {
    $script:Section = $Name
    if (-not $Json) { Write-AiHeader $Name }
}

function Report([string]$Level, [string]$Check, [string]$Detail) {
    $script:Results.Add([pscustomobject]@{ Section = $script:Section; Level = $Level; Check = $Check; Detail = $Detail })
    if (-not $Json) { Write-AiLine $Level ("{0}: {1}" -f $Check, $Detail) }
}

# Sends initialize + tools/list to `codegraph serve --mcp` and waits for the tools list.
function Test-AiCodeGraphMcpServer([string]$Exe, [string]$WorkDir, [int]$TimeoutSec = 30) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    if ($Exe -match '\.(cmd|bat)$') {
        $psi.FileName = $env:ComSpec
        $psi.Arguments = "/d /c `"`"$Exe`" serve --mcp`""
    } else {
        $psi.FileName = $Exe
        $psi.Arguments = 'serve --mcp'
    }
    $psi.WorkingDirectory = $WorkDir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    try {
        $p.StandardInput.WriteLine('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"flicksnap-doctor","version":"1"}}}')
        $p.StandardInput.WriteLine('{"jsonrpc":"2.0","method":"notifications/initialized"}')
        $p.StandardInput.WriteLine('{"jsonrpc":"2.0","id":2,"method":"tools/list"}')
        $p.StandardInput.Flush()
        $deadline = (Get-Date).AddSeconds($TimeoutSec)
        while ((Get-Date) -lt $deadline) {
            $task = $p.StandardOutput.ReadLineAsync()
            $left = [int][Math]::Max(1, ($deadline - (Get-Date)).TotalMilliseconds)
            if (-not $task.Wait($left)) { break }
            $line = $task.Result
            if ($null -eq $line) { break }
            if ($line -match '"id"\s*:\s*2\b') {
                $names = [regex]::Matches($line, '"name"\s*:\s*"(codegraph_[a-z_]+)"') | ForEach-Object { $_.Groups[1].Value }
                return @($names | Select-Object -Unique)
            }
        }
        return $null
    } finally {
        try { $p.StandardInput.Close() } catch { }
        if (-not $p.WaitForExit(3000)) { try { $p.Kill() } catch { } }
        $p.Dispose()
    }
}

function Test-AiRtkHookProbe([string]$RtkExe, [string]$Agent) {
    $payload = '{"session_id":"doctor","turn_id":"doctor","tool_use_id":"doctor","hook_event_name":"PreToolUse","tool_name":"Bash","permission_mode":"default","tool_input":{"command":"git status"}}'
    $r = Invoke-AiNative -FilePath $RtkExe -Arguments @('hook', $Agent) -StdIn $payload
    return ($r.Text -match '"updatedInput"\s*:\s*\{\s*"command"\s*:\s*"rtk git status"')
}

# ================================================================ project

Section 'Project'
$Root = Get-AiProjectRoot $PSScriptRoot
Report 'OK' 'Repository root' $Root
Update-AiSessionPath
$git = Find-AiCommand 'git'

# ================================================================ tools

Section 'Tools'
$cg = Get-AiCodeGraph
$rtk = Get-AiRtk
foreach ($t in @(
        @{ Name = 'CodeGraph'; Exe = 'codegraph'; Tool = $cg;  Tested = $AiTested.CodeGraph },
        @{ Name = 'RTK';       Exe = 'rtk';       Tool = $rtk; Tested = $AiTested.Rtk })) {
    $tool = $t.Tool
    if (-not $tool.Present) { Report 'WARN' $t.Name 'not installed (optional; agents fall back to normal search/output)'; continue }
    if ($t.Name -eq 'RTK' -and -not $tool.Works) { Report 'FAIL' $t.Name "$($tool.State) at $($tool.Path). $($tool.Detail)"; continue }
    if (-not $tool.Works) { Report 'FAIL' $t.Name "found at $($tool.Path) but does not run"; continue }
    if ($tool.Version -eq $t.Tested) { Report 'OK' $t.Name "$($tool.Version) at $($tool.Path)" }
    else { Report 'WARN' $t.Name "$($tool.Version) at $($tool.Path) (repository tested with $($t.Tested))" }
    $persistent = Find-AiOnPersistentPath $t.Exe
    if ($persistent -eq $tool.Path) { Report 'OK' "$($t.Name) on persistent PATH" 'resolves to the same executable' }
    elseif ($persistent) { Report 'WARN' "$($t.Name) on persistent PATH" "resolves to a different executable: $persistent" }
    else { Report 'WARN' "$($t.Name) on persistent PATH" 'does not resolve through the registry PATH' }
}

# ================================================================ agents

$claude = Find-AiClaudeCode
$codex = Find-AiCodex

Section 'Claude Code'
if (-not $claude.Detected) {
    Report 'INFO' 'Claude Code' 'not detected'
} else {
    Report 'OK' 'Detected' ((@($claude.CliPath, $claude.ExtExe) | Where-Object { $_ }) -join '; ')
    if (Test-AiClaudeCodeGraphMcp $claude $Root) { Report 'OK' 'CodeGraph MCP' 'found in user or project configuration' }
    else { Report 'WARN' 'CodeGraph MCP' 'not configured (run scripts\ai\setup.ps1)' }
    if (Test-AiClaudeCodeGraphNotes $claude) { Report 'OK' 'CodeGraph instructions' "block present in $($claude.Paths.Instructions)" }
    else { Report 'INFO' 'CodeGraph instructions' 'no CodeGraph block in global CLAUDE.md' }
    if (Test-AiClaudeRtkHook $claude $Root) { Report 'OK' 'RTK hook' 'found in user or project PreToolUse configuration' }
    else { Report 'WARN' 'RTK hook' 'not configured (run scripts\ai\setup.ps1)' }
    if (Test-AiClaudeRtkNotes $claude) { Report 'OK' 'RTK instructions' '@RTK.md referenced from global CLAUDE.md' }
    else { Report 'INFO' 'RTK instructions' 'RTK.md / @RTK.md not present' }
    Report 'INFO' 'RTK hook scope' 'rewrites Bash tool calls only; PowerShell tool and built-in Read/Grep/Glob are not rewritten'
}

Section 'Codex'
if (-not $codex.Detected) {
    Report 'INFO' 'Codex' 'not detected'
} else {
    Report 'OK' 'Detected' ((@($codex.CliPath, $codex.ExtExe) | Where-Object { $_ }) -join '; ')
    if (Test-AiCodexCodeGraphMcp $codex $Root) { Report 'OK' 'CodeGraph MCP' 'found in user or project configuration' }
    else { Report 'WARN' 'CodeGraph MCP' 'not configured (run scripts\ai\setup.ps1)' }
    if (Test-AiCodexCodeGraphNotes $codex) { Report 'OK' 'CodeGraph instructions' "block present in $($codex.Paths.Instructions)" }
    else { Report 'INFO' 'CodeGraph instructions' 'no CodeGraph block in global AGENTS.md' }
    if (Test-AiCodexRtkHook $codex $Root) { Report 'OK' 'RTK hook' 'found in user or project PreToolUse configuration' }
    else { Report 'WARN' 'RTK hook' 'not configured (run scripts\ai\setup.ps1)' }
    if (Test-AiCodexRtkNotes $codex) { Report 'OK' 'RTK instructions' 'RTK.md referenced from global AGENTS.md' }
    else { Report 'INFO' 'RTK instructions' 'RTK.md reference not present' }
    $feature = Get-AiCodexHooksFeature $codex
    if ($feature -eq $true) { Report 'OK' 'Codex hooks feature' 'enabled' }
    elseif ($feature -eq $false) { Report 'WARN' 'Codex hooks feature' 'disabled - the RTK hook cannot run' }
    else { Report 'INFO' 'Codex hooks feature' 'unknown (no Codex executable found to ask)' }
    if (Test-AiCodexRtkHook $codex $Root) {
        Report 'TODO' 'Codex hook trust' "cannot be read from the CLI; Codex runs the hook only after you approve it in an interactive Codex session"
    }
}

# ================================================================ index

Section 'CodeGraph index'
$indexDir = Join-Path $Root '.codegraph'
if (-not $cg.Works) {
    Report 'INFO' 'Index' 'CodeGraph not available - not checked'
} else {
    $st = Get-AiIndexStatus $cg.Path $Root
    if ($null -eq $st) {
        Report 'FAIL' 'Index status' 'CodeGraph status failed or returned invalid JSON. Check codegraph status --json.'
    } elseif (-not $st.initialized) {
        $reportedPath = Get-AiProp $st 'indexPath'
        if (-not $reportedPath -or (Test-Path -LiteralPath $reportedPath) -or (Test-Path -LiteralPath $indexDir)) {
            Report 'FAIL' 'Index' 'CodeGraph reports an uninitialized index but its absence could not be confirmed'
        } else {
            Report 'WARN' 'Index' 'no index yet (run scripts\ai\setup.ps1, or codegraph init --local-only)'
        }
    } else {
        Report 'OK' 'Index' ("{0} files, {1} nodes, {2} edges, last indexed {3}" -f (Get-AiProp $st 'fileCount'), (Get-AiProp $st 'nodeCount'), (Get-AiProp $st 'edgeCount'), (Get-AiProp $st 'lastIndexed'))
        $pending = Get-AiPendingCount $st
        if ($pending -gt 0) { Report 'WARN' 'Index freshness' "$pending changed files not yet synced (auto-sync catches up; or run codegraph sync)" }
        else { Report 'OK' 'Index freshness' 'no pending changes' }
        if (Get-AiProp $st 'rebuildInProgress') { Report 'INFO' 'Rebuild' 'a rebuild is in progress' }
        $wm = Get-AiProp $st 'worktreeMismatch'
        if ($wm) { Report 'WARN' 'Worktree' "index belongs to another worktree: $wm" }
        $strict = Invoke-AiNative -FilePath $cg.Path -Arguments @('status', '--strict', $Root)
        if ($strict.ExitCode -eq 0) { Report 'OK' 'codegraph status --strict' 'finished, up to date, every language yielded symbols' }
        else { Report 'WARN' 'codegraph status --strict' "exit $($strict.ExitCode)" }
    }
}
$alt = @(Get-ChildItem -LiteralPath $Root -Directory -Force -Filter '.codegraph-*' -ErrorAction SilentlyContinue)
foreach ($d in $alt) { Report 'INFO' 'Alternative index dir' "$($d.Name) present (for example WSL); ignored by git" }

# ================================================================ git hygiene

Section 'Git exclusions'
if (-not $git) {
    Report 'WARN' 'git' 'not found - exclusion checks skipped'
} else {
    function Test-Ignored([string]$Rel) {
        (Invoke-AiNative -FilePath $git.Source -Arguments @('-C', $Root, 'check-ignore', '-q', '--no-index', $Rel)).ExitCode -eq 0
    }
    foreach ($rel in '.codegraph/codegraph.db', '.codegraph-wsl/codegraph.db') {
        if (Test-Ignored $rel) { Report 'OK' "Ignored: $rel" 'yes' } else { Report 'FAIL' "Ignored: $rel" 'NO - the local index could be committed' }
    }
    if (Test-Ignored 'codegraph.json') { Report 'FAIL' 'codegraph.json' 'is ignored by git, so the shared config cannot be committed' }
    else {
        $tracked = (Invoke-AiNative -FilePath $git.Source -Arguments @('-C', $Root, 'ls-files', '--error-unmatch', 'codegraph.json')).ExitCode -eq 0
        if ($tracked) { Report 'OK' 'codegraph.json' 'tracked by git' } else { Report 'INFO' 'codegraph.json' 'not ignored, not yet committed' }
    }
    $trackedIndex = (Invoke-AiNative -FilePath $git.Source -Arguments @('-C', $Root, 'ls-files', '--', '.codegraph', ':(glob).codegraph-*/**')).Output | Where-Object { $_ }
    if ($trackedIndex) { Report 'FAIL' 'Tracked index files' ("{0} file(s) under .codegraph*/ are tracked: {1}" -f @($trackedIndex).Count, (@($trackedIndex)[0])) }
    else { Report 'OK' 'Tracked index files' 'none' }
    $infoExclude = Join-Path $Root '.git\info\exclude'
    if ((Get-AiFileText $infoExclude) -match '(?m)^\s*/?codegraph\.json\s*$') {
        Report 'WARN' '.git/info/exclude' 'still hides codegraph.json locally; remove that line'
    }
}

$cgConfig = Join-Path $Root 'codegraph.json'
if (Test-Path -LiteralPath $cgConfig) {
    $raw = Get-Content -LiteralPath $cgConfig -Raw
    $cfg = $null
    try { $cfg = $raw | ConvertFrom-Json } catch { Report 'FAIL' 'codegraph.json' "invalid JSON: $_" }
    if ($cfg) {
        $ex = @(Get-AiProp $cfg 'exclude')
        if ($ex -contains 'crates/obs-studio/') { Report 'OK' 'OBS exclusion' 'crates/obs-studio/ excluded from the index' }
        else { Report 'WARN' 'OBS exclusion' 'crates/obs-studio/ is not in codegraph.json exclude (about 15k third-party files)' }
        if ($raw -match '([A-Za-z]:[\\/])|(^|[\s"])/(Users|home)/|\\\\') { Report 'FAIL' 'codegraph.json paths' 'contains an absolute or machine-specific path' }
        else { Report 'OK' 'codegraph.json paths' 'relative only' }
        if ($raw -match '(?i)(token|secret|password|apikey|api_key)') { Report 'FAIL' 'codegraph.json secrets' 'contains a credential-like key' }
        else { Report 'OK' 'codegraph.json secrets' 'none found' }
    }
} else {
    Report 'WARN' 'codegraph.json' 'missing at repository root'
}

# ================================================================ docs

Section 'Shared files'
foreach ($rel in 'docs\AI_SETUP.md', 'scripts\ai\setup.ps1', 'scripts\ai\doctor.ps1', 'scripts\ai\common.ps1') {
    if (Test-Path -LiteralPath (Join-Path $Root $rel)) { Report 'OK' $rel 'present' } else { Report 'WARN' $rel 'missing' }
}
foreach ($rel in 'AGENTS.md', 'CLAUDE.md') {
    if (Test-Path -LiteralPath (Join-Path $Root $rel)) { Report 'INFO' $rel 'repository-level agent instructions present' }
}

# ================================================================ live probes

if ($Live) {
    Section 'Live probes (not a live agent session)'
    if ($cg.Works) {
        $tools = Test-AiCodeGraphMcpServer $cg.Path $Root
        if ($tools -and ($tools -contains 'codegraph_explore')) { Report 'OK' 'CodeGraph MCP server' ("answers over stdio; tools: {0}" -f ($tools -join ', ')) }
        else { Report 'FAIL' 'CodeGraph MCP server' 'no tools/list answer from codegraph serve --mcp' }
    }
    if ($claude.Detected -and $claude.Exe) {
        $r = Invoke-AiNative -FilePath $claude.Exe -Arguments @('mcp', 'get', 'codegraph')
        if ($r.Text -match 'Connected') { Report 'OK' 'Claude Code MCP check' "'claude mcp get codegraph' reports Connected" }
        elseif ($r.ExitCode -ne 0) { Report 'WARN' 'Claude Code MCP check' "claude mcp get codegraph failed (exit $($r.ExitCode))" }
        else { Report 'WARN' 'Claude Code MCP check' ("not connected: " + (($r.Output | Where-Object { $_ -match 'Status' }) -join ' ')) }
    }
    if ($codex.Detected -and $codex.Exe) {
        $r = Invoke-AiNative -FilePath $codex.Exe -Arguments @('mcp', 'get', 'codegraph')
        if ($r.ExitCode -eq 0 -and $r.Text -match 'enabled:\s*true') { Report 'OK' 'Codex MCP entry' "'codex mcp get codegraph' lists it as enabled (Codex starts it per session; not probed here)" }
        else { Report 'WARN' 'Codex MCP entry' "codex mcp get codegraph: exit $($r.ExitCode)" }
    }
    if ($rtk.Works) {
        foreach ($agent in 'claude', 'codex') {
            if (Test-AiRtkHookProbe $rtk.Path $agent) { Report 'OK' "rtk hook $agent" 'rewrites a simulated git status payload to rtk git status' }
            else { Report 'FAIL' "rtk hook $agent" 'did not rewrite the simulated payload' }
        }
    }
    Report 'INFO' 'In-agent confirmation' "ask the agent to run 'git status' in its Bash tool, then check 'rtk gain --history'"
}

# ================================================================ summary

$fails = @($script:Results | Where-Object { $_.Level -eq 'FAIL' }).Count
$warns = @($script:Results | Where-Object { $_.Level -eq 'WARN' }).Count
$todos = @($script:Results | Where-Object { $_.Level -eq 'TODO' }).Count
if ($Json) {
    [pscustomobject]@{ root = $Root; live = [bool]$Live; fail = $fails; warn = $warns; todo = $todos; results = $script:Results } | ConvertTo-Json -Depth 4
} else {
    Write-AiHeader 'Summary'
    Write-Host ("  {0} FAIL, {1} WARN, {2} TODO{3}" -f $fails, $warns, $todos, $(if ($Live) { '' } else { '  (config checks only; add -Live for tool probes)' }))
}
if ($fails -gt 0) { exit 1 }
exit 0
