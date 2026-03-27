#Requires -Version 5.1
<#
.SYNOPSIS
    MCPViews - Agent Integration Setup (Windows)
.DESCRIPTION
    Detects installed MCP-compatible platforms and configures them to connect
    to the local MCPViews gateway at http://localhost:4200/mcp.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# Platform definitions
# ---------------------------------------------------------------------------

$MCPVIEWS_URL = "http://localhost:4200/mcp"

$Platforms = @(
    @{
        Name       = "Claude Desktop"
        Binary     = "claude-desktop"
        ConfigPath = Join-Path $env:APPDATA "Claude\claude_desktop_config.json"
        JsonKey    = "mcpServers"
        Format     = "json"
    },
    @{
        Name       = "Claude Code CLI"
        Binary     = "claude"
        ConfigPath = Join-Path $env:USERPROFILE ".claude.json"
        JsonKey    = "mcpServers"
        Format     = "json"
    },
    @{
        Name       = "Cursor IDE"
        Binary     = "cursor"
        ConfigPath = Join-Path $env:USERPROFILE ".cursor\mcp.json"
        JsonKey    = "mcpServers"
        Format     = "json"
    },
    @{
        Name       = "Windsurf"
        Binary     = "windsurf"
        ConfigPath = Join-Path $env:USERPROFILE ".codeium\windsurf\mcp_config.json"
        JsonKey    = "mcpServers"
        Format     = "json"
    },
    @{
        Name       = "Codex CLI"
        Binary     = "codex"
        ConfigPath = Join-Path $env:USERPROFILE ".codex\config.toml"
        JsonKey    = "mcp_servers"
        Format     = "toml"
    },
    @{
        Name       = "OpenCode"
        Binary     = "opencode"
        ConfigPath = Join-Path $env:USERPROFILE ".config\opencode\opencode.json"
        JsonKey    = "mcp"
        Format     = "json"
    },
    @{
        Name       = "Antigravity"
        Binary     = "antigravity"
        ConfigPath = Join-Path $env:USERPROFILE ".gemini\antigravity\mcp_config.json"
        JsonKey    = "mcpServers"
        Format     = "json"
    }
)

# ---------------------------------------------------------------------------
# Helper functions
# ---------------------------------------------------------------------------

function Test-PlatformDetected {
    param([hashtable]$Platform)

    # Check if the config directory exists
    $configDir = Split-Path $Platform.ConfigPath -Parent
    if (Test-Path $configDir) { return $true }

    # Check if binary is on PATH
    if (Get-Command $Platform.Binary -ErrorAction SilentlyContinue) { return $true }

    return $false
}

function Test-AlreadyConfigured {
    param([hashtable]$Platform)

    if (-not (Test-Path $Platform.ConfigPath)) { return $false }

    try {
        if ($Platform.Format -eq "toml") {
            $content = Get-Content $Platform.ConfigPath -Raw -ErrorAction SilentlyContinue
            if ($content -and $content -match '\[mcp_servers\.mcpviews\]') { return $true }
            return $false
        }

        # JSON
        $content = Get-Content $Platform.ConfigPath -Raw -ErrorAction SilentlyContinue
        if (-not $content) { return $false }

        $json = $content | ConvertFrom-Json
        $key = $Platform.JsonKey

        if ($json.PSObject.Properties.Name -contains $key) {
            $section = $json.$key
            if ($section.PSObject.Properties.Name -contains "mcpviews") {
                return $true
            }
        }
    }
    catch {
        # If we can't parse, treat as not configured
    }

    return $false
}

function Backup-ConfigFile {
    param([string]$Path)

    if (Test-Path $Path) {
        $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
        $backupPath = "$Path.bak.$timestamp"
        Copy-Item -Path $Path -Destination $backupPath -Force
        Write-Host "  Backup: $backupPath" -ForegroundColor DarkGray
    }
}

function Install-JsonConfig {
    param([hashtable]$Platform)

    # Skip if already configured
    if (Test-AlreadyConfigured $Platform) {
        return $false
    }

    $configPath = $Platform.ConfigPath
    $jsonKey = $Platform.JsonKey

    # Ensure parent directory exists
    $configDir = Split-Path $configPath -Parent
    if (-not (Test-Path $configDir)) {
        New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    }

    # Backup existing file
    Backup-ConfigFile -Path $configPath

    # Read or create JSON object
    $json = $null
    if (Test-Path $configPath) {
        $raw = Get-Content $configPath -Raw
        if ($raw -and $raw.Trim().Length -gt 0) {
            try {
                $json = $raw | ConvertFrom-Json
            }
            catch {
                Write-Host "  WARNING: Could not parse $configPath - creating fresh config" -ForegroundColor Yellow
            }
        }
    }

    if (-not $json) {
        $json = [PSCustomObject]@{}
    }

    # Ensure the top-level key exists
    if (-not ($json.PSObject.Properties.Name -contains $jsonKey)) {
        $json | Add-Member -NotePropertyName $jsonKey -NotePropertyValue ([PSCustomObject]@{})
    }

    # Add mcpviews entry
    # Claude Desktop requires stdio transport via mcp-remote bridge
    if ($Platform.Name -eq "Claude Desktop") {
        $mcpMuxEntry = [PSCustomObject]@{
            command = "npx"
            args    = @("-y", "mcp-remote", $MCPVIEWS_URL)
        }
    }
    else {
        $mcpMuxEntry = [PSCustomObject]@{
            url = $MCPVIEWS_URL
        }
    }
    $json.$jsonKey | Add-Member -NotePropertyName "mcpviews" -NotePropertyValue $mcpMuxEntry -Force

    # Write back
    $json | ConvertTo-Json -Depth 10 | Set-Content -Path $configPath -Encoding UTF8

    return $true
}

function Install-TomlConfig {
    param([hashtable]$Platform)

    $configPath = $Platform.ConfigPath

    # Ensure parent directory exists
    $configDir = Split-Path $configPath -Parent
    if (-not (Test-Path $configDir)) {
        New-Item -ItemType Directory -Path $configDir -Force | Out-Null
    }

    # Backup existing file
    Backup-ConfigFile -Path $configPath

    $tomlBlock = @"

[mcp_servers.mcpviews]
type = "sse"
url = "$MCPVIEWS_URL"
"@

    if (Test-Path $configPath) {
        $content = Get-Content $configPath -Raw
        if ($content -match '\[mcp_servers\.mcpviews\]') {
            Write-Host "  Already configured (skipped)" -ForegroundColor Yellow
            return $false
        }
        Add-Content -Path $configPath -Value $tomlBlock -Encoding UTF8
    }
    else {
        Set-Content -Path $configPath -Value $tomlBlock.TrimStart() -Encoding UTF8
    }

    return $true
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

Write-Host ""
Write-Host "MCPViews - Agent Integration Setup" -ForegroundColor Cyan
Write-Host "===================================" -ForegroundColor Cyan
Write-Host ""

# Detect platforms
$detected = @()
foreach ($p in $Platforms) {
    if (Test-PlatformDetected $p) {
        $configured = Test-AlreadyConfigured $p
        $detected += @{
            Platform   = $p
            Configured = $configured
        }
    }
}

if ($detected.Count -eq 0) {
    Write-Host "No supported MCP platforms detected on this system." -ForegroundColor Yellow
    Write-Host "Install one of: Claude Desktop, Claude Code CLI, Cursor, Windsurf, Codex, OpenCode, Antigravity"
    Write-Host ""
    Read-Host "Press Enter to close..."
    exit 0
}

# Display menu
Write-Host "Detected platforms:"
for ($i = 0; $i -lt $detected.Count; $i++) {
    $entry = $detected[$i]
    $num = $i + 1
    $name = $entry.Platform.Name
    if ($entry.Configured) {
        $status = "(already configured)"
        $color = "Green"
    }
    else {
        $status = "(not configured)"
        $color = "Yellow"
    }
    Write-Host "  [$num] " -NoNewline
    Write-Host ("{0,-22}" -f $name) -NoNewline
    Write-Host $status -ForegroundColor $color
}

Write-Host ""
$allConfigured = ($detected | Where-Object { -not $_.Configured }).Count -eq 0
if ($allConfigured) {
    Write-Host "All detected platforms are already configured." -ForegroundColor Green
    Write-Host ""
    Read-Host "Press Enter to close..."
    exit 0
}

$input = Read-Host "Enter numbers to install (e.g. 1 3), 'a' for all unconfigured, 'q' to quit"

if ($input -eq 'q' -or $input -eq 'Q') {
    Write-Host "Cancelled." -ForegroundColor DarkGray
    exit 0
}

# Determine which indices to install
$toInstall = @()
if ($input -eq 'a' -or $input -eq 'A') {
    for ($i = 0; $i -lt $detected.Count; $i++) {
        if (-not $detected[$i].Configured) {
            $toInstall += $i
        }
    }
}
else {
    $numbers = $input -split '\s+' | Where-Object { $_ -match '^\d+$' } | ForEach-Object { [int]$_ }
    foreach ($n in $numbers) {
        $idx = $n - 1
        if ($idx -ge 0 -and $idx -lt $detected.Count) {
            $toInstall += $idx
        }
        else {
            Write-Host "  Skipping invalid selection: $n" -ForegroundColor Yellow
        }
    }
}

if ($toInstall.Count -eq 0) {
    Write-Host "Nothing to install." -ForegroundColor DarkGray
    Read-Host "Press Enter to close..."
    exit 0
}

# Install
Write-Host ""
$successCount = 0
$configuredNames = @()

foreach ($idx in $toInstall) {
    $entry = $detected[$idx]
    $platform = $entry.Platform
    $name = $platform.Name

    Write-Host "Configuring $name..." -ForegroundColor White

    if ($entry.Configured) {
        Write-Host "  Already configured (skipped)" -ForegroundColor Yellow
        continue
    }

    try {
        $ok = $false
        if ($platform.Format -eq "toml") {
            $ok = Install-TomlConfig -Platform $platform
        }
        else {
            $ok = Install-JsonConfig -Platform $platform
        }

        if ($ok) {
            Write-Host "  Done." -ForegroundColor Green
            $successCount++
            $configuredNames += $name
        }
    }
    catch {
        Write-Host "  ERROR: $_" -ForegroundColor Red
    }
}

# Create sentinel file
$sentinelDir = Join-Path $env:USERPROFILE ".mcpviews"
if (-not (Test-Path $sentinelDir)) {
    New-Item -ItemType Directory -Path $sentinelDir -Force | Out-Null
}
$sentinelPath = Join-Path $sentinelDir ".setup-complete"
Get-Date -Format "o" | Set-Content -Path $sentinelPath -Encoding UTF8

# Summary
Write-Host ""
Write-Host "===================================" -ForegroundColor Cyan
Write-Host "Setup Complete" -ForegroundColor Cyan
Write-Host "===================================" -ForegroundColor Cyan
Write-Host ""

if ($successCount -gt 0) {
    Write-Host "Configured $successCount platform(s):" -ForegroundColor Green
    foreach ($n in $configuredNames) {
        Write-Host "  - $n" -ForegroundColor Green
    }
}
else {
    Write-Host "No new platforms were configured." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "MCPViews server runs on $MCPVIEWS_URL" -ForegroundColor Cyan
Write-Host "Make sure the MCPViews app is running (check your system tray)." -ForegroundColor Cyan
Write-Host ""
Read-Host "Press Enter to close..."
