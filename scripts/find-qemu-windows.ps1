#!/usr/bin/env pwsh
param(
    [switch]$SetProcessEnv,
    [switch]$PersistUserEnv,
    [switch]$Quiet
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-QemuCandidates {
    $candidates = @()

    try {
        $cmd = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
        if ($cmd -and $cmd.Source) {
            $candidates += $cmd.Source
        }
    }
    catch {
    }

    $candidates += @(
        (Join-Path $env:ProgramFiles 'qemu/qemu-system-x86_64.exe'),
        (Join-Path $env:ProgramFiles 'QEMU/qemu-system-x86_64.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs/qemu/qemu-system-x86_64.exe'),
        (Join-Path $env:USERPROFILE 'scoop/apps/qemu/current/qemu-system-x86_64.exe'),
        'C:/msys64/mingw64/bin/qemu-system-x86_64.exe',
        'C:/tools/qemu/qemu-system-x86_64.exe'
    )

    if ($env:ChocolateyInstall) {
        $candidates += (Join-Path $env:ChocolateyInstall 'bin/qemu-system-x86_64.exe')
    }

    $seen = @{}
    foreach ($path in $candidates) {
        if ([string]::IsNullOrWhiteSpace($path)) {
            continue
        }
        $key = $path.ToLowerInvariant()
        if ($seen.ContainsKey($key)) {
            continue
        }
        $seen[$key] = $true
        if (Test-Path $path) {
            return (Resolve-Path $path).Path
        }
    }

    return $null
}

$found = Get-QemuCandidates
if (-not $found) {
    if (-not $Quiet) {
        Write-Warning 'qemu-system-x86_64.exe was not found in PATH or common install locations.'
    }
    exit 1
}

if ($SetProcessEnv) {
    $env:VEER_VM_QEMU = $found
}

if ($PersistUserEnv) {
    [Environment]::SetEnvironmentVariable('VEER_VM_QEMU', $found, 'User')
}

if (-not $Quiet) {
    Write-Host $found
}

exit 0
