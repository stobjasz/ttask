# Installs or updates ttask from GitHub releases, no admin rights needed:
#
#   irm https://raw.githubusercontent.com/stobjasz/ttask/main/scripts/install.ps1 | iex
#
# Copies ttask.exe to %LOCALAPPDATA%\Programs\ttask and adds that folder to the
# user PATH (open a new terminal afterwards). $env:TTASK_VERSION = '0.3.1' picks
# a version, $env:TTASK_INSTALL_DIR another folder. To remove ttask again:
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/stobjasz/ttask/main/scripts/install.ps1))) -Uninstall
#
# Runs on Windows PowerShell 5.1 and PowerShell 7. Your tasks live in
# %APPDATA%\ttask and are never touched.

param(
    [string]$Version = $env:TTASK_VERSION,
    [string]$InstallDir = $env:TTASK_INSTALL_DIR,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest's progress bar slows downloads to a crawl on PowerShell 5.1.
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol =
    [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = 'stobjasz/ttask'
$target = 'x86_64-pc-windows-msvc'
if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\ttask' }
$InstallDir = $InstallDir.TrimEnd('\')
$exe = Join-Path $InstallDir 'ttask.exe'

# The user PATH is read and written raw (REG_EXPAND_SZ), so entries such as
# %USERPROFILE%\bin keep working; [Environment]::SetEnvironmentVariable would
# expand them for good.
function Get-UserPath {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
    try { $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames') } finally { $key.Close() }
}

function Set-UserPath([string]$value) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try { $key.SetValue('Path', $value, 'ExpandString') } finally { $key.Close() }
    # Setting a variable through .NET broadcasts WM_SETTINGCHANGE, so new
    # terminals see the change without signing out.
    [Environment]::SetEnvironmentVariable('TTASK_INSTALL_NOTIFY', '1', 'User')
    [Environment]::SetEnvironmentVariable('TTASK_INSTALL_NOTIFY', $null, 'User')
}

function Get-PathEntries([string]$path) {
    @($path -split ';' | Where-Object { $_ })
}

function Test-SameDir([string]$a, [string]$b) {
    [Environment]::ExpandEnvironmentVariables($a).TrimEnd('\') -eq $b
}

if ($Uninstall) {
    if (Test-Path $exe) {
        # Removes the Task Scheduler entry from `ttask remind --install`, if any.
        try { & $exe remind --uninstall *> $null } catch { }
        Remove-Item -Recurse -Force $InstallDir
    }
    $entries = Get-PathEntries (Get-UserPath)
    $kept = @($entries | Where-Object { -not (Test-SameDir $_ $InstallDir) })
    if ($kept.Count -ne $entries.Count) { Set-UserPath ($kept -join ';') }
    Write-Host "Removed ttask from $InstallDir. Your tasks in $env:APPDATA\ttask were kept."
    return
}

$api = "https://api.github.com/repos/$repo/releases"
$release = if ($Version) {
    Invoke-RestMethod "$api/tags/v$($Version.TrimStart('v'))"
} else {
    Invoke-RestMethod "$api/latest"
}
$tag = $release.tag_name
$asset = $release.assets | Where-Object { $_.name -like "ttask-*-$target.exe" } | Select-Object -First 1
$sums = $release.assets | Where-Object { $_.name -eq "$($asset.name).sha256" } | Select-Object -First 1
if (-not $asset -or -not $sums) { throw "Release $tag has no $target build" }

$tmp = Join-Path ([IO.Path]::GetTempPath()) "ttask-install-$PID"
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    $download = Join-Path $tmp $asset.name
    Write-Host "Downloading ttask $tag..."
    Invoke-WebRequest $asset.browser_download_url -OutFile $download -UseBasicParsing
    # "<sha256>  <file name>"; PowerShell 7 returns binary assets as bytes.
    $sumsText = (Invoke-WebRequest $sums.browser_download_url -UseBasicParsing).Content
    if ($sumsText -is [byte[]]) { $sumsText = [Text.Encoding]::ASCII.GetString($sumsText) }
    $expected = ($sumsText.Trim() -split '\s+')[0]
    if ((Get-FileHash $download -Algorithm SHA256).Hash -ne $expected) {
        throw "Checksum mismatch for $($asset.name)"
    }

    New-Item -ItemType Directory -Force $InstallDir | Out-Null
    try {
        Copy-Item $download $exe -Force
    } catch {
        throw "Could not replace $exe; close any running ttask (the TUI or a reminder run) and try again"
    }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$entries = Get-PathEntries (Get-UserPath)
if (-not ($entries | Where-Object { Test-SameDir $_ $InstallDir })) {
    Set-UserPath ((@($entries) + $InstallDir) -join ';')
    Write-Host "Added $InstallDir to your PATH; open a new terminal to use it."
}
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -eq $InstallDir })) {
    $env:Path = "$env:Path;$InstallDir"
}

Write-Host "Installed $(& $exe --version) to $exe"
