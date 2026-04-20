<#
.SYNOPSIS
    knotch uninstaller for Windows.

.PARAMETER InstallDir
    Binary directory (default: $env:USERPROFILE\.local\bin).

.PARAMETER KeepPlugin
    Do not remove user-level plugin bundle.

.PARAMETER KeepBackup
    Do not back up plugin before removal.

.PARAMETER Yes
    Non-interactive mode.

.EXAMPLE
    .\scripts\uninstall.ps1
#>

[CmdletBinding()]
param(
    [string]$InstallDir = $(if ($env:KNOTCH_INSTALL_DIR) { $env:KNOTCH_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }),
    [switch]$KeepPlugin = ($env:KNOTCH_KEEP_PLUGIN -eq "1"),
    [switch]$KeepBackup = ($env:KNOTCH_KEEP_BACKUP -eq "1"),
    [switch]$Yes        = ($env:KNOTCH_YES         -eq "1")
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Script:BinaryName = "knotch"
$Script:PluginName = "knotch"

function Write-Step { param([string]$Message) Write-Host "▸  $Message" -ForegroundColor Blue }
function Write-Ok   { param([string]$Message) Write-Host "✓  $Message" -ForegroundColor Green }
function Write-Warn { param([string]$Message) Write-Host "!  $Message" -ForegroundColor Yellow }
function Write-Info { param([string]$Message) Write-Host "   $Message" -ForegroundColor DarkGray }

function Test-Interactive {
    if ($Yes) { return $false }
    try { return [Environment]::UserInteractive -and -not [Console]::IsInputRedirected } catch { return $true }
}

function Read-YesNo {
    param([string]$Question, [bool]$DefaultYes = $false)
    if (-not (Test-Interactive)) { return $DefaultYes }
    $default = if ($DefaultYes) { 0 } else { 1 }
    $choices = @(
        [System.Management.Automation.Host.ChoiceDescription]::new("&Yes", "Yes"),
        [System.Management.Automation.Host.ChoiceDescription]::new("&No", "No")
    )
    $idx = $Host.UI.PromptForChoice($Question, $null, [System.Management.Automation.Host.ChoiceDescription[]]$choices, $default)
    return ($idx -eq 0)
}

function Backup-Path {
    param([string]$Target)
    if (-not (Test-Path $Target)) { return }
    $stamp = Get-Date -Format "yyyyMMdd_HHmmss"
    $backup = "$Target.backup_${stamp}_$PID"
    Copy-Item -Path $Target -Destination $backup -Recurse -Force
    Write-Info "Backup: $backup"
}

function Uninstall-Binary {
    $dest = Join-Path $InstallDir "$Script:BinaryName.exe"
    Write-Step "Removing binary"
    if (Test-Path $dest) {
        Remove-Item -Path $dest -Force
        Write-Ok "Removed $dest"
    } else {
        Write-Info "Binary not found at $dest"
    }
}

function Uninstall-Plugin {
    $target = Join-Path $env:USERPROFILE ".claude\plugins\$Script:PluginName"
    if (-not (Test-Path $target)) { Write-Info "No user-level plugin at $target"; return }
    if ($KeepPlugin) { Write-Info "Keeping plugin (-KeepPlugin)"; return }
    # -Yes means non-interactive full cleanup. Only interactive runs prompt
    # (default No) since plugin bundles can outlive the binary across projects.
    if (-not $Yes -and -not (Read-YesNo "Remove plugin at $target?" $false)) {
        Write-Info "Plugin kept"; return
    }

    Write-Step "Removing plugin"
    if (-not $KeepBackup) { Backup-Path $target }
    Remove-Item -Path $target -Recurse -Force
    Write-Ok "Removed $target"

    $parent = Join-Path $env:USERPROFILE ".claude\plugins"
    if ((Test-Path $parent) -and -not (Get-ChildItem $parent -Force)) {
        Remove-Item -Path $parent -Force
        Write-Info "Cleaned empty $parent"
    }
}

function Start-Uninstall {
    Write-Host ""
    Write-Host "knotch uninstaller" -ForegroundColor White
    Write-Host ""
    Uninstall-Binary
    Uninstall-Plugin
    Write-Host ""
    Write-Host "✅ Uninstall complete" -ForegroundColor Green
    Write-Info "Project-level plugins (.claude\plugins\$Script:PluginName\) are managed by git"
}

Start-Uninstall
