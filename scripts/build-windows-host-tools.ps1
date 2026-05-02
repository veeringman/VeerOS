#!/usr/bin/env pwsh
param(
    [ValidateSet('debug', 'release')]
    [string]$Profile = 'debug'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Build VeerOS Windows host tools:
# - veer-vm
# - fold
# - veer-connect
#
# This is a convenience wrapper around scripts/build-windows.ps1.
#
# Usage:
#   ./scripts/build-windows-host-tools.ps1
#   ./scripts/build-windows-host-tools.ps1 release
#
# Environment overrides:
#   VEER_VM_WIN_TARGET=native|all|x86_64-pc-windows-msvc|aarch64-pc-windows-msvc
#   VEER_VM_WIN_PROFILE=debug|release

$RootDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$env:VEER_VM_WIN_PROFILE = $Profile

& (Join-Path $RootDir 'scripts/build-windows.ps1') -Profile $Profile
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
