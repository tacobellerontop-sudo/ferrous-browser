<#
.SYNOPSIS
    Applies our patches in patches\servo\ to the pinned Servo checkout.

.DESCRIPTION
    Servo is a path dependency on an unmodified upstream checkout (see
    docs/servo-version.md), except for the few fixes kept in patches\servo\.
    Each one is listed in docs/servo-version.md under "Local patches", with
    why it exists and when it can be deleted.

    Safe to run repeatedly: a patch that is already applied is skipped, and one
    that neither applies nor is already applied stops the script, because that
    means the checkout is not the pinned revision or has other edits.

.EXAMPLE
    .\scripts\apply-servo-patches.ps1
#>
[CmdletBinding()]
param(
    # The Servo checkout. Defaults to servo\ beside browser\.
    [string]$ServoDir
)

$ErrorActionPreference = 'Stop'

$repo = Join-Path $PSScriptRoot '..'
if (-not $ServoDir) { $ServoDir = Join-Path $repo 'servo' }
$ServoDir = (Resolve-Path -LiteralPath $ServoDir).Path

# Whether `git apply` accepts the patch with these flags. git reports a failed
# check on stderr, which Windows PowerShell turns into a terminating error under
# 'Stop', so the check runs with errors allowed and is judged by its exit code.
function Test-Apply([string]$patch, [string[]]$flags) {
    $ErrorActionPreference = 'Continue'
    git -C $ServoDir apply --check @flags $patch 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

$patches = Get-ChildItem (Join-Path $repo 'patches\servo') -Filter *.patch | Sort-Object Name
foreach ($patch in $patches) {
    if (Test-Apply $patch.FullName @()) {
        git -C $ServoDir apply $patch.FullName
        if ($LASTEXITCODE -ne 0) { throw "failed to apply $($patch.Name)" }
        Write-Host "applied $($patch.Name)"
        continue
    }
    if (Test-Apply $patch.FullName @('--reverse')) {
        Write-Host "already applied: $($patch.Name)"
        continue
    }
    throw "$($patch.Name) neither applies nor is already applied to $ServoDir"
}
