<#
.SYNOPSIS
    Copies the GStreamer DLLs and plugins next to the browser executable.

.DESCRIPTION
    With the `media` feature the executable links against GStreamer, so Windows
    refuses to start it unless the GStreamer DLLs can be found. Servo also loads
    a fixed list of GStreamer plugins from the executable's own directory, and
    exits the process if any fails to load (servo/components/servo/servo.rs,
    `media_platform::init`).

    Upstream does this copy in `mach build` (package_gstreamer_dlls in
    servo/python/servo/build_commands.py). We build with plain cargo, so this is
    our version of it, like scripts/copy-angle-dlls.ps1.

    The file lists are read from the pinned Servo checkout rather than copied
    here, so they cannot drift from what that revision of Servo loads:
      * libraries: GSTREAMER_WIN_DEPENDENCY_LIBS and GSTREAMER_BASE_LIBS in
        servo/python/servo/gstreamer.py
      * plugins: servo/components/servo/gstreamer_plugin_lists/{common,windows}.rs.in

.EXAMPLE
    .\scripts\copy-gstreamer-dlls.ps1
#>
[CmdletBinding()]
param(
    # Directory holding the built executable. Defaults to the medium profile dir.
    [string]$BinDir,
    # GStreamer root (the folder with bin\ and lib\). Defaults to deps\gstreamer.
    [string]$GstRoot
)

$ErrorActionPreference = 'Stop'

$repo = Join-Path $PSScriptRoot '..'
if (-not $BinDir) { $BinDir = Join-Path $repo 'browser\target\medium' }
if (-not $GstRoot) { $GstRoot = Join-Path $repo 'deps\gstreamer\1.0\msvc_x86_64' }
$BinDir = (Resolve-Path -LiteralPath $BinDir).Path
if (-not (Test-Path (Join-Path $GstRoot 'bin\ffi-7.dll'))) {
    throw "GStreamer not found at $GstRoot. Run scripts\fetch-gstreamer.ps1 first."
}
$GstRoot = (Resolve-Path -LiteralPath $GstRoot).Path

# Quoted strings inside `NAME = [ ... ]` in a Python file.
function Get-PythonList([string]$file, [string]$name) {
    $text = Get-Content -Raw -LiteralPath $file
    $m = [regex]::Match($text, "(?s)$name\s*=\s*\[(.*?)\]")
    if (-not $m.Success) { throw "$name not found in $file" }
    [regex]::Matches(($m.Groups[1].Value -replace '#[^\n]*', ''), '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }
}

# Quoted strings in a Rust `[&str]` list file, ignoring `//` comment lines.
function Get-RustList([string]$file) {
    $text = (Get-Content -LiteralPath $file | Where-Object { $_.Trim() -notlike '//*' }) -join "`n"
    [regex]::Matches($text, '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }
}

$servo = Join-Path $repo 'servo'
$py = Join-Path $servo 'python\servo\gstreamer.py'
$lists = Join-Path $servo 'components\servo\gstreamer_plugin_lists'

$libraries = @(Get-PythonList $py 'GSTREAMER_WIN_DEPENDENCY_LIBS') +
             @(Get-PythonList $py 'GSTREAMER_BASE_LIBS' | ForEach-Object { "$_-1.0-0.dll" })
$plugins = @(Get-RustList (Join-Path $lists 'common.rs.in')) +
           @(Get-RustList (Join-Path $lists 'windows.rs.in')) | ForEach-Object { "$_.dll" }

$missing = @()
foreach ($dll in $libraries) {
    $src = Join-Path $GstRoot "bin\$dll"
    if (Test-Path $src) { Copy-Item $src $BinDir -Force } else { $missing += $src }
}
foreach ($dll in $plugins) {
    $src = Join-Path $GstRoot "lib\gstreamer-1.0\$dll"
    if (Test-Path $src) { Copy-Item $src $BinDir -Force } else { $missing += $src }
}
if ($missing) {
    $missing | ForEach-Object { Write-Host "missing: $_" -ForegroundColor Red }
    throw "$($missing.Count) GStreamer files not found"
}

Write-Host "Copied $($libraries.Count) GStreamer libraries and $($plugins.Count) plugins to $BinDir" -ForegroundColor Green
