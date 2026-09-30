<#
.SYNOPSIS
    Copies the ANGLE runtime DLLs next to the browser executable.

.DESCRIPTION
    On Windows we enable Servo's `no-wgl` feature, which routes GL through ANGLE
    (mozangle). mozangle's build.rs links our binary against `libEGL` and emits
    `libEGL.dll` / `libGLESv2.dll` into its own OUT_DIR, but never places them
    next to the executable. Windows will not find them at load time, and the
    binary dies with a missing-DLL error before it can create a GL context.

    Upstream Servo handles this in `mach run-post-build-tasks`
    (servo/python/servo/build_commands.py:327-345, copy_windows_dlls_to_build_directory),
    but that only targets the servoshell binary. We build our own crate with a
    separate `cargo build`, so we need our own copy step.

    This is the gap that makes a bare `cargo build` "work" on a machine where
    someone previously ran `mach bootstrap`, and fail on a clean one.

.EXAMPLE
    .\scripts\copy-angle-dlls.ps1
#>
[CmdletBinding()]
param(
    # Directory holding the built executable. Defaults to the medium profile dir.
    [string]$BinDir
)

$ErrorActionPreference = 'Stop'

if (-not $BinDir) {
    $BinDir = Join-Path $PSScriptRoot '..\browser\target\medium'
}
$BinDir = (Resolve-Path -LiteralPath $BinDir).Path

$targetRoot = Join-Path $PSScriptRoot '..\browser\target'

Write-Host "Looking for ANGLE DLLs under $targetRoot ..."
$found = @{}
foreach ($dll in 'libEGL.dll', 'libGLESv2.dll') {
    $match = Get-ChildItem -Path $targetRoot -Recurse -Filter $dll -File -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $match) {
        throw "Could not find $dll under $targetRoot. Has 'cargo build' finished the mozangle build script?"
    }
    $found[$dll] = $match
    Write-Host ("  found {0} -> {1}" -f $dll, $match.FullName.Replace($targetRoot, '<target>'))
}

foreach ($dll in $found.Keys) {
    $dest = Join-Path $BinDir $dll
    Copy-Item -LiteralPath $found[$dll].FullName -Destination $dest -Force
    Write-Host "  copied $dll -> $dest"
}

Write-Host "ANGLE DLLs staged in $BinDir" -ForegroundColor Green