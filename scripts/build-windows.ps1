#!/usr/bin/env pwsh
param(
    [ValidateSet('debug', 'release')]
    [string]$Profile = $(if ($env:VEER_VM_WIN_PROFILE) { $env:VEER_VM_WIN_PROFILE } else { 'debug' })
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Build Windows host artifacts used for local bring-up.
# Builds:
# - veer-vm
# - fold
# - veer-connect
#
# Usage:
#   ./scripts/build-windows.ps1
#   ./scripts/build-windows.ps1 -Profile release
#
# Environment overrides:
#   VEER_VM_WIN_TARGET=native|all|x86_64-pc-windows-msvc|aarch64-pc-windows-msvc
#   VEER_VM_WIN_PROFILE=debug|release

$ManifestDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $ManifestDir

function Resolve-HostTarget {
    $rustcInfo = & rustc -vV
    foreach ($line in $rustcInfo) {
        if ($line.StartsWith('host: ')) {
            return $line.Substring(6).Trim()
        }
    }
    throw 'Failed to detect rustc host target.'
}

function Resolve-Targets {
    $spec = if ($env:VEER_VM_WIN_TARGET) { $env:VEER_VM_WIN_TARGET } else { 'native' }
    $native = Resolve-HostTarget

    switch ($spec) {
        'native' { return @($native) }
        'all' { return @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc') }
        default {
            return $spec.Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne '' }
        }
    }
}

function Ensure-RustTarget([string]$Target) {
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        & rustup target add $Target | Out-Null
    }
}

function Assert-TargetSupported([string]$Target) {
    if ($Target -notin @('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')) {
        throw "Unsupported Windows target: $Target"
    }
}

function Build-Package([string]$Package, [string]$Target, [string]$ProfileArg) {
    if ($ProfileArg -eq 'release') {
        & cargo build -p $Package --target $Target --release
    } else {
        & cargo build -p $Package --target $Target
    }
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed for package '$Package' target '$Target' profile '$ProfileArg'"
    }
}

function Require-Binary([string]$Path, [string]$Label) {
    if (-not (Test-Path $Path)) {
        throw "Expected $Label binary not found: $Path"
    }
}

$targets = Resolve-Targets
$built = @()

foreach ($target in $targets) {
    Assert-TargetSupported $target
    Ensure-RustTarget $target

    Write-Host "==> Building Windows host tools for $target ($Profile)"

    Build-Package 'veer_vm' $target $Profile
    Build-Package 'fold_engine' $target $Profile
    Build-Package 'veer-connect' $target $Profile

    $base = Join-Path $ManifestDir "target/$target/$Profile"
    $veerVm = Join-Path $base 'veer-vm.exe'
    $fold = Join-Path $base 'fold.exe'
    $veerConnect = Join-Path $base 'veer-connect.exe'

    Require-Binary $veerVm 'veer-vm'
    Require-Binary $fold 'fold'
    Require-Binary $veerConnect 'veer-connect'

    $built += [PSCustomObject]@{
        Target      = $target
        VeerVm      = $veerVm
        Fold        = $fold
        VeerConnect = $veerConnect
    }
}

Write-Host 'Build complete.'
Write-Host "  profile: $Profile"
foreach ($entry in $built) {
    Write-Host "  target      : $($entry.Target)"
    Write-Host "  veer-vm bin : $($entry.VeerVm)"
    Write-Host "  fold bin    : $($entry.Fold)"
    Write-Host "  veer-connect: $($entry.VeerConnect)"
}

Pop-Location
