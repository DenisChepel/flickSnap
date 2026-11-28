# Shared helpers for scripts/ai/setup.ps1 and scripts/ai/doctor.ps1.
# Dot-sourced; defines functions and constants only and changes nothing by itself.
# Windows PowerShell 5.1 compatible. Keep this file ASCII-only.

Set-StrictMode -Version 2.0

# Versions this repository's AI setup was verified with. setup.ps1 installs exactly these
# when a tool is missing and never upgrades an installed tool; doctor.ps1 warns on a mismatch.
$script:AiTested = @{
    CodeGraph = '1.6.4'
    Rtk       = '1.2.1'
}

$script:CodeGraphNpmPackage   = '@colbymchenry/codegraph'
$script:CodeGraphInstallPs1   = "https://raw.githubusercontent.com/colbymchenry/codegraph/v$($AiTested.CodeGraph)/install.ps1"
$script:RtkWingetId           = 'rtk-ai.rtk'

# The build toolchain BUILDING.md requires on Windows. setup.ps1 offers these official winget
# packages only for what is missing, after confirmation, and never upgrades what is installed.
$script:BuildWinget = @{
    Git    = 'Git.Git'
    DotNet = 'Microsoft.DotNet.SDK.10'
    Rustup = 'Rustlang.Rustup'
    Msvc   = 'Microsoft.VisualStudio.2022.BuildTools'
}
# The C++ build tools workload (MSVC linker + Windows SDK) that Rust's MSVC target links with.
$script:MsvcInstallerOverride = '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'

# ---------------------------------------------------------------- output

function Write-AiHeader([string]$Text) {
    Write-Host ''
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Write-AiLine([string]$Level, [string]$Text) {
    $color = switch ($Level) {
        'OK'     { 'Green' }
        'WARN'   { 'Yellow' }
        'FAIL'   { 'Red' }
        'TODO'   { 'Magenta' }
        'PLAN'   { 'Cyan' }
        'DO'     { 'Cyan' }
        default  { 'Gray' }
    }
    Write-Host ('  [{0,-4}] {1}' -f $Level, $Text) -ForegroundColor $color
}

# ---------------------------------------------------------------- native commands

# Runs a native command and returns exit code + merged output lines, without letting
# stderr text turn into a terminating error under $ErrorActionPreference = 'Stop'.
function Invoke-AiNative {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$Arguments = @(),
        [string]$StdIn = $null
    )
    $prev = $ErrorActionPreference
    $previousExit = Get-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue
    $previousCode = if ($previousExit) { $previousExit.Value } else { $null }
    $global:LASTEXITCODE = $null
    $launchError = $null
    $ErrorActionPreference = 'Continue'
    try {
        if ($null -ne $StdIn -and $StdIn -ne '') {
            $out = $StdIn | & $FilePath @Arguments 2>&1
        } else {
            $out = & $FilePath @Arguments 2>&1
        }
        $code = $global:LASTEXITCODE
        if ($null -eq $code) {
            $launchError = $out | Where-Object { $_ -is [System.Management.Automation.ErrorRecord] } | Select-Object -First 1
        }
    } catch {
        $launchError = $_
        $out = @($_)
        $code = $null
    } finally {
        $ErrorActionPreference = $prev
        if ($previousExit) { $global:LASTEXITCODE = $previousCode }
        else { Remove-Variable -Name LASTEXITCODE -Scope Global -ErrorAction SilentlyContinue }
    }
    $lines = @($out | ForEach-Object { "$_" })
    $denied = $false
    if ($launchError) {
        for ($e = $launchError.Exception; $e; $e = $e.InnerException) {
            if ($e -is [UnauthorizedAccessException] -or
                ($e -is [ComponentModel.Win32Exception] -and $e.NativeErrorCode -eq 5) -or
                $e.HResult -eq -2147024891) { $denied = $true; break }
        }
    }
    [pscustomobject]@{
        ExitCode = if ($null -eq $code) { -1 } else { $code }
        AccessDenied = $denied
        Output = $lines
        Text = ($lines -join "`n")
    }
}

function Get-AiVersionFromText([string]$Text) {
    $m = [regex]::Match($Text, '(\d+\.\d+\.\d+(?:[-.][0-9A-Za-z.]+)?)')
    if ($m.Success) { return $m.Groups[1].Value }
    return $null
}

# ---------------------------------------------------------------- project root

function Get-AiProjectRoot([string]$StartDir) {
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($git) {
        $r = Invoke-AiNative -FilePath $git.Source -Arguments @('-C', $StartDir, 'rev-parse', '--show-toplevel')
        if ($r.ExitCode -eq 0 -and $r.Text) {
            $p = ($r.Output | Select-Object -Last 1).Trim() -replace '/', '\'
            if (Test-Path -LiteralPath (Join-Path $p 'Cargo.toml')) { return $p }
        }
    }
    # Fallback without git: walk up to the directory holding the workspace markers.
    $dir = (Resolve-Path -LiteralPath $StartDir).ProviderPath
    while ($dir) {
        if ((Test-Path -LiteralPath (Join-Path $dir 'Cargo.toml')) -and
            (Test-Path -LiteralPath (Join-Path $dir 'flickSnap.slnx'))) { return $dir }
        $parent = Split-Path -Parent $dir
        if (-not $parent -or $parent -eq $dir) { break }
        $dir = $parent
    }
    throw "Could not find the flickSnap repository root above '$StartDir'."
}

# ---------------------------------------------------------------- PATH

function Get-AiPersistentPathDirs {
    $m = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $u = [Environment]::GetEnvironmentVariable('Path', 'User')
    @(("$m;$u") -split ';' | Where-Object { $_ -and $_.Trim() } | ForEach-Object { [Environment]::ExpandEnvironmentVariables($_.Trim().Trim('"')) })
}

# Extra directories official installers use. Added to this session only, so a tool installed
# a moment ago is usable before a new terminal is opened.
function Get-AiKnownToolDirs {
    $dirs = @()
    if ($env:APPDATA)      { $dirs += (Join-Path $env:APPDATA 'npm') }
    $dirs += (Join-Path $HOME '.cargo\bin')
    if ($env:ProgramFiles) {
        $dirs += (Join-Path $env:ProgramFiles 'dotnet')
        $dirs += (Join-Path $env:ProgramFiles 'Git\cmd')
    }
    if ($env:LOCALAPPDATA) {
        $dirs += (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links')
        $dirs += (Join-Path $env:LOCALAPPDATA 'codegraph\current\bin')
        $pkgs = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages'
        if (Test-Path -LiteralPath $pkgs) {
            $dirs += @(Get-ChildItem -LiteralPath $pkgs -Directory -Filter 'rtk-ai.rtk_*' -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName })
        }
    }
    @($dirs | Where-Object { Test-Path -LiteralPath $_ })
}

# Rebuilds $env:Path from the registry (what a new terminal or agent process would get),
# then appends the known installer directories. Session-only; nothing is persisted.
function Update-AiSessionPath {
    $seen = @{}
    $result = New-Object System.Collections.Generic.List[string]
    foreach ($d in @($env:Path -split ';') + @(Get-AiPersistentPathDirs) + @(Get-AiKnownToolDirs)) {
        if (-not $d) { continue }
        $k = $d.TrimEnd('\').ToLowerInvariant()
        if ($seen.ContainsKey($k)) { continue }
        $seen[$k] = $true
        $result.Add($d)
    }
    $env:Path = ($result -join ';')
}

# Resolve with the same rules as the current session, using only registry PATH entries.
function Find-AiOnPersistentPath([string]$Name) {
    $sessionPath = $env:Path
    try {
        $env:Path = (Get-AiPersistentPathDirs) -join ';'
        $cmd = Find-AiCommand $Name
        if ($cmd) { return $cmd.Source }
    } finally {
        $env:Path = $sessionPath
    }
}

# ---------------------------------------------------------------- tools

# Prefers a real executable (.exe/.cmd) over an npm .ps1 shim, which a restrictive
# execution policy would block.
function Find-AiCommand([string]$Name) {
    $app = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($app) { return $app }
    return (Get-Command $Name -CommandType ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1)
}

function Get-AiCodeGraph {
    $cmd = Find-AiCommand 'codegraph'
    if (-not $cmd) { return [pscustomobject]@{ Present = $false; Works = $false; Path = $null; Version = $null } }
    $r = Invoke-AiNative -FilePath $cmd.Source -Arguments @('--version')
    $v = Get-AiVersionFromText $r.Text
    [pscustomobject]@{ Present = $true; Works = ($r.ExitCode -eq 0 -and $v); Path = $cmd.Source; Version = $v }
}

function Get-AiRtk {
    $cmd = Find-AiCommand 'rtk'
    $result = [pscustomobject]@{ Present = [bool]$cmd; Works = $false; Path = $null; Version = $null; State = 'Missing'; Detail = '' }
    if (-not $cmd) { return $result }
    $result.Path = $cmd.Source
    $result.State = 'ProbeFailed'
    $r = Invoke-AiNative -FilePath $cmd.Source -Arguments @('--version')
    if ($r.ExitCode -eq 0) { $result.Version = Get-AiVersionFromText $r.Text }
    $probe = '--version'
    if ($result.Version) {
        $probe = 'rewrite'
        $r = Invoke-AiNative -FilePath $cmd.Source -Arguments @($probe, 'git status')
        # `rtk rewrite` prints a rewrite with exit 0 (allow rules cover it) or 3 (defer approval
        # to the host, which is also what it answers with no permission rules at all).
        if (($r.ExitCode -eq 0 -or $r.ExitCode -eq 3) -and $r.Text -match '(?m)^\s*(?:rtk\s+)?git status\s*$') {
            $result.State = 'Ready'
            $result.Works = $true
            return $result
        }
        if ($r.ExitCode -gt 0 -and $r.Text -match '(?i)(?:unrecognized|unknown|invalid) (?:subcommand|command)\s+[''"]?rewrite\b') {
            $result.State = 'WrongExecutable'
            $result.Detail = 'The executable does not support the RTK rewrite command.'
            return $result
        }
    }
    if ($r.AccessDenied) {
        $result.State = 'AccessDenied'
        $result.Detail = "$probe could not start: access denied (check sandbox or OS permissions)."
    } else {
        $result.Detail = "$probe failed or returned unrecognized output (exit $($r.ExitCode))."
    }
    return $result
}

# ---------------------------------------------------------------- build toolchain (read-only)

function Get-AiVswhere {
    $base = [Environment]::GetFolderPath('ProgramFilesX86')
    if (-not $base) { return $null }
    $p = Join-Path $base 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $p) { return $p }
    return $null
}

# One entry per toolchain piece: Id, Name, Present, Detail. Never installs anything.
function Get-AiBuildPrereqs {
    $items = New-Object System.Collections.Generic.List[object]

    $git = Find-AiCommand 'git'
    $gv = if ($git) { Get-AiVersionFromText (Invoke-AiNative -FilePath $git.Source -Arguments @('--version')).Text } else { $null }
    $items.Add([pscustomobject]@{ Id = 'git'; Name = 'git'; Present = [bool]$gv
        Detail = $(if ($gv) { "git $gv" } else { 'not found' }) })

    $dotnet = Find-AiCommand 'dotnet'
    $sdk = $null
    if ($dotnet) {
        $r = Invoke-AiNative -FilePath $dotnet.Source -Arguments @('--list-sdks')
        $sdk = $r.Output | Where-Object { $_ -match '^\s*10\.\d+\.\d+' } | Select-Object -First 1
    }
    $items.Add([pscustomobject]@{ Id = 'dotnet'; Name = '.NET 10 SDK'; Present = [bool]$sdk
        Detail = $(if ($sdk) { ($sdk -split '\s+')[0] } elseif ($dotnet) { 'dotnet found, but no 10.x SDK' } else { 'not found' }) })

    $cargo = Find-AiCommand 'cargo'
    $rustup = Find-AiCommand 'rustup'
    $cv = if ($cargo) { Get-AiVersionFromText (Invoke-AiNative -FilePath $cargo.Source -Arguments @('--version')).Text } else { $null }
    $rustDetail = if ($cv) { "cargo $cv" } elseif ($rustup) { 'rustup found, but no default toolchain' } else { 'not found' }
    $items.Add([pscustomobject]@{ Id = 'rust'; Name = 'Rust (rustup, stable)'; Present = [bool]$cv; Detail = $rustDetail
        HasRustup = [bool]$rustup; RustupPath = $(if ($rustup) { $rustup.Source } else { $null }) })

    $msvc = $null
    $vswhere = Get-AiVswhere
    if ($vswhere) {
        $r = Invoke-AiNative -FilePath $vswhere -Arguments @('-latest', '-products', '*', '-requires',
            'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'displayName')
        if ($r.ExitCode -eq 0) { $msvc = $r.Output | Where-Object { $_ -and $_.Trim() } | Select-Object -First 1 }
    }
    $items.Add([pscustomobject]@{ Id = 'msvc'; Name = 'MSVC C++ build tools'; Present = [bool]$msvc
        Detail = $(if ($msvc) { $msvc } else { 'no Visual Studio / Build Tools with the C++ workload' }) })

    return $items
}

# ---------------------------------------------------------------- agents

function Get-AiClaudePaths {
    if ($env:CLAUDE_CONFIG_DIR) {
        $dir = $env:CLAUDE_CONFIG_DIR
        $json = Join-Path $env:CLAUDE_CONFIG_DIR '.claude.json'
    } else {
        $dir = Join-Path $HOME '.claude'
        $json = Join-Path $HOME '.claude.json'
    }
    [pscustomobject]@{
        Dir          = $dir
        Json         = $json
        Settings     = Join-Path $dir 'settings.json'
        Instructions = Join-Path $dir 'CLAUDE.md'
        RtkMd        = Join-Path $dir 'RTK.md'
    }
}

function Get-AiCodexPaths {
    $dir = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Join-Path $HOME '.codex' }
    [pscustomobject]@{
        Dir          = $dir
        Config       = Join-Path $dir 'config.toml'
        Hooks        = Join-Path $dir 'hooks.json'
        Instructions = Join-Path $dir 'AGENTS.md'
        RtkMd        = Join-Path $dir 'RTK.md'
    }
}

function Get-AiEditorExtensionRoots {
    @('.vscode', '.vscode-insiders', '.cursor', '.windsurf') |
        ForEach-Object { Join-Path $HOME "$_\extensions" } |
        Where-Object { Test-Path -LiteralPath $_ }
}

function Find-AiClaudeCode {
    $paths = Get-AiClaudePaths
    $cli = Find-AiCommand 'claude'
    $ext = $null
    foreach ($root in Get-AiEditorExtensionRoots) {
        $hit = Get-ChildItem -Path (Join-Path $root 'anthropic.claude-code-*\resources\native-binary\claude.exe') -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($hit) { $ext = $hit.FullName; break }
    }
    $exe = if ($cli) { $cli.Source } elseif ($ext) { $ext } else { $null }
    [pscustomobject]@{
        Name      = 'Claude Code'
        Id        = 'claude'
        Detected  = [bool]($cli -or $ext -or (Test-Path -LiteralPath $paths.Dir))
        CliPath   = if ($cli) { $cli.Source } else { $null }
        ExtExe    = $ext
        Exe       = $exe
        Paths     = $paths
    }
}

function Find-AiCodex {
    $paths = Get-AiCodexPaths
    $cli = Find-AiCommand 'codex'
    $ext = $null
    foreach ($root in Get-AiEditorExtensionRoots) {
        $hit = Get-ChildItem -Path (Join-Path $root 'openai.chatgpt-*\bin\windows-*\codex.exe') -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($hit) { $ext = $hit.FullName; break }
    }
    $exe = if ($cli) { $cli.Source } elseif ($ext) { $ext } else { $null }
    [pscustomobject]@{
        Name      = 'Codex'
        Id        = 'codex'
        Detected  = [bool]($cli -or $ext -or (Test-Path -LiteralPath $paths.Dir))
        CliPath   = if ($cli) { $cli.Source } else { $null }
        ExtExe    = $ext
        Exe       = $exe
        Paths     = $paths
    }
}

function Get-AiFileText([string]$Path) {
    if (Test-Path -LiteralPath $Path -PathType Leaf) { return [IO.File]::ReadAllText($Path) }
    return ''
}

# ---------------------------------------------------------------- config state (read-only)

function Test-AiJsonCodeGraphMcp($Config) {
    $servers = Get-AiProp $Config 'mcpServers'
    if (-not $servers) { return $false }
    foreach ($server in $servers.PSObject.Properties) {
        $command = Get-AiProp $server.Value 'command'
        if ($server.Name -eq 'codegraph' -or $command -match '(?:^|[\\/])codegraph(?:\.cmd|\.exe)?$') {
            return $true
        }
    }
    return $false
}

function Test-AiClaudeCodeGraphMcp($Claude, [string]$Root) {
    $text = Get-AiFileText $Claude.Paths.Json
    if ($text) {
        $config = $text | ConvertFrom-Json
        if (Test-AiJsonCodeGraphMcp $config) { return $true }
        $projects = Get-AiProp $config 'projects'
        if ($projects) {
            foreach ($project in $projects.PSObject.Properties) {
                if (($project.Name -replace '/', '\').TrimEnd('\') -eq $Root.TrimEnd('\') -and
                    (Test-AiJsonCodeGraphMcp $project.Value)) { return $true }
            }
        }
    }
    $text = Get-AiFileText (Join-Path $Root '.mcp.json')
    return [bool]($text -and (Test-AiJsonCodeGraphMcp ($text | ConvertFrom-Json)))
}

function Test-AiCodexCodeGraphMcp($Codex, [string]$Root) {
    foreach ($path in $Codex.Paths.Config, (Join-Path $Root '.codex\config.toml')) {
        $text = Get-AiFileText $path
        if ($text -match '(?m)^\s*\[mcp_servers\s*\.\s*["'']?codegraph["'']?\]\s*(?:#.*)?$') { return $true }
        $tables = [regex]::Matches($text, '(?ms)^\s*\[mcp_servers\.[^\]\r\n]+\][^\r\n]*\r?\n(.*?)(?=^\s*\[|\z)')
        foreach ($table in $tables) {
            if ($table.Groups[1].Value -match '(?m)^\s*command\s*=\s*["''](?:[^"''\r\n]*[\\/])?codegraph(?:\.cmd|\.exe)?["'']') { return $true }
        }
    }
    return $false
}

function Test-AiJsonRtkHook([string]$Path, [string]$Agent) {
    $text = Get-AiFileText $Path
    if (-not $text) { return $false }
    $hooks = Get-AiProp ($text | ConvertFrom-Json) 'hooks'
    foreach ($group in (Get-AiProp $hooks 'PreToolUse')) {
        foreach ($hook in (Get-AiProp $group 'hooks')) {
            if ((Get-AiProp $hook 'command') -match "(?:^|[\\/])rtk(?:\.exe)?[`"']?\s+hook\s+$Agent(?:\s|$)") { return $true }
        }
    }
    return $false
}

function Test-AiClaudeRtkHook($Claude, [string]$Root) {
    foreach ($path in $Claude.Paths.Settings, (Join-Path $Root '.claude\settings.json'), (Join-Path $Root '.claude\settings.local.json')) {
        if (Test-AiJsonRtkHook $path 'claude') { return $true }
    }
    return $false
}

function Test-AiCodexRtkHook($Codex, [string]$Root) {
    foreach ($path in $Codex.Paths.Hooks, (Join-Path $Root '.codex\hooks.json')) {
        if (Test-AiJsonRtkHook $path 'codex') { return $true }
    }
    foreach ($path in $Codex.Paths.Config, (Join-Path $Root '.codex\config.toml')) {
        $tables = [regex]::Matches((Get-AiFileText $path), '(?ms)^\s*\[\[hooks\.PreToolUse\.hooks\]\][^\r\n]*\r?\n(.*?)(?=^\s*\[|\z)')
        foreach ($table in $tables) {
            if ($table.Groups[1].Value -match '(?m)^\s*command\s*=.*(?:^|[\\/"''])rtk(?:\.exe)?\s+hook\s+codex(?:\s|["''])') { return $true }
        }
    }
    return $false
}

function Test-AiClaudeCodeGraphNotes($Claude) { (Get-AiFileText $Claude.Paths.Instructions) -match 'CODEGRAPH_START' }
function Test-AiCodexCodeGraphNotes($Codex)   { (Get-AiFileText $Codex.Paths.Instructions) -match 'CODEGRAPH_START' }
function Test-AiClaudeRtkNotes($Claude) {
    ((Get-AiFileText $Claude.Paths.Instructions) -match '(?m)^@RTK\.md\s*$') -and (Test-Path -LiteralPath $Claude.Paths.RtkMd)
}
function Test-AiCodexRtkNotes($Codex) {
    ((Get-AiFileText $Codex.Paths.Instructions) -match 'RTK\.md') -and (Test-Path -LiteralPath $Codex.Paths.RtkMd)
}

# Codex runs hooks only when its "hooks" feature is on. Returns $true/$false, or $null if unknown.
function Get-AiCodexHooksFeature($Codex) {
    if (-not $Codex.Exe) { return $null }
    $r = Invoke-AiNative -FilePath $Codex.Exe -Arguments @('features', 'list')
    if ($r.ExitCode -ne 0) { return $null }
    $line = $r.Output | Where-Object { $_ -match '^\s*hooks\s' } | Select-Object -First 1
    if (-not $line) { return $null }
    return ($line -match '\btrue\s*$')
}

# ---------------------------------------------------------------- index

function Get-AiIndexStatus([string]$CodeGraphExe, [string]$Root) {
    $r = Invoke-AiNative -FilePath $CodeGraphExe -Arguments @('status', '--json', $Root)
    if ($r.ExitCode -ne 0) { return $null }
    try { $status = $r.Text | ConvertFrom-Json -ErrorAction Stop } catch { return $null }
    if ((Get-AiProp $status 'initialized') -is [bool]) { return $status }
    return $null
}

function Get-AiPendingCount($Status) {
    $p = $Status.PSObject.Properties['pendingChanges']
    if (-not $p -or -not $p.Value) { return 0 }
    $n = 0
    foreach ($k in 'added', 'modified', 'removed') {
        $v = $p.Value.PSObject.Properties[$k]
        if ($v -and $v.Value) { $n += [int]$v.Value }
    }
    return $n
}

function Get-AiProp($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    $p = $Object.PSObject.Properties[$Name]
    if ($p) { return $p.Value }
    return $null
}
