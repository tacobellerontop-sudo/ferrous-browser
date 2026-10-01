<#
.SYNOPSIS
    Downloads GStreamer 1.22.8 (MSVC x86_64) and unpacks it into deps\gstreamer.

.DESCRIPTION
    The `media` feature builds Servo with `media-gstreamer`, which links against
    GStreamer. These are the same two packages Servo's `mach bootstrap` uses
    (servo/python/servo/platform/windows.py:27-28), from Servo's own release of
    build dependencies. GStreamer itself no longer hosts 1.22.8.

    Unlike `mach bootstrap`, this does not elevate. `msiexec /a` is an
    administrative install: it only unpacks the files into a folder. Nothing is
    registered with Windows and PATH is not changed. .cargo/config.toml points
    pkg-config at the result.

    The packages are not code-signed, and GitHub records no digest for these
    assets, so the size and SHA-256 are pinned here. They were measured on the
    first download (2026-10-01); a mismatch means the file is not that one.

.EXAMPLE
    .\scripts\fetch-gstreamer.ps1
#>
[CmdletBinding()]
param(
    # Re-download and re-unpack even if GStreamer is already present.
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

$base = 'https://github.com/servo/servo-build-deps/releases/download/msvc-deps'
$packages = @(
    @{ Name = 'gstreamer-1.0-msvc-x86_64-1.22.8.msi'; Size = 127258624
       Sha256 = '37F9973FE5C720CE1F1602E7E599336384B9FF3E4878817987DD6B77265F17BB' },
    @{ Name = 'gstreamer-1.0-devel-msvc-x86_64-1.22.8.msi'; Size = 225861632
       Sha256 = '2D0CF6E89CF88D94E670CD81087C002408161D1C8843C00D3F27D33CE254C523' }
)

$deps = Join-Path $PSScriptRoot '..\deps\gstreamer'
$root = Join-Path $deps '1.0\msvc_x86_64'

# Runtime and development files: an interrupted unpack can leave one without
# the other, which is why mach checks for both.
$present = (Test-Path (Join-Path $root 'bin\ffi-7.dll')) -and
           (Test-Path (Join-Path $root 'lib\pkgconfig\gstreamer-1.0.pc'))
if ($present -and -not $Force) {
    Write-Host "GStreamer already present at $((Resolve-Path $root).Path)"
    return
}

$download = Join-Path ([IO.Path]::GetTempPath()) "ferrous-gstreamer-$PID"
$staging = Join-Path $deps '_staging'
New-Item -ItemType Directory -Force $download, $staging | Out-Null
try {
    foreach ($p in $packages) {
        $file = Join-Path $download $p.Name
        Write-Host "Downloading $($p.Name) ($([math]::Round($p.Size / 1MB)) MB) ..."
        Invoke-WebRequest -Uri "$base/$($p.Name)" -OutFile $file -UseBasicParsing

        $size = (Get-Item $file).Length
        if ($size -ne $p.Size) { throw "$($p.Name): expected $($p.Size) bytes, got $size" }
        $hash = (Get-FileHash $file -Algorithm SHA256).Hash
        if ($hash -ne $p.Sha256) { throw "$($p.Name): SHA-256 $hash does not match the pinned $($p.Sha256)" }

        Write-Host "Unpacking $($p.Name) ..."
        $log = Join-Path $download "$($p.Name).log"
        $proc = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @(
            '/a', "`"$file`"", "TARGETDIR=`"$((Resolve-Path $staging).Path)`"", '/qn', '/l*v', "`"$log`"")
        if ($proc.ExitCode -ne 0) { throw "msiexec failed for $($p.Name) (exit $($proc.ExitCode)); see $log" }
    }

    # The packages unpack to <target>\gstreamer\1.0\msvc_x86_64, plus a copy of
    # each .msi, which is not needed.
    $unpacked = Join-Path $staging 'gstreamer\1.0'
    if (-not (Test-Path (Join-Path $unpacked 'msvc_x86_64\bin\ffi-7.dll'))) {
        throw "unexpected package layout under $staging"
    }
    $final = Join-Path $deps '1.0'
    if (Test-Path $final) { Remove-Item -Recurse -Force $final }
    Move-Item $unpacked $final
}
finally {
    Remove-Item -Recurse -Force $staging, $download -ErrorAction SilentlyContinue
}

Write-Host "GStreamer unpacked to $((Resolve-Path $root).Path)" -ForegroundColor Green
