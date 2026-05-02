#!/usr/bin/env pwsh
param(
    [ValidateSet('auto', 'qemu', 'hyperv', 'custom')]
    [string]$Backend = 'qemu',

    [ValidateSet('debug', 'release')]
    [string]$Profile = 'debug',

    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$Kernel = 'build/veeros.iso',

    [ValidateSet('x86_64')]
    [string]$Arch = 'x86_64',

    [int]$Memory = 256,
    [int]$Cpus = 1,

    [string]$Disk,
    [switch]$DiskReadOnly,

    [string]$CustomRunner,
    [string[]]$CustomArg = @(),
    [string]$HypervSwitch,
    [string]$HypervVmName,
    [switch]$HypervKeepVm,
    [switch]$HypervHealthCheck,
    [switch]$HypervCreateDiskIfMissing,
    [int]$HypervDiskSizeGiB = 16,
    [string]$HypervProfileName,
    [switch]$SaveHypervProfile,
    [switch]$UseHypervProfile,
    [switch]$ListHypervProfiles,
    [bool]$AutoDetectQemu = $true,
    [switch]$PersistQemuPath,

    [switch]$Build,

    [switch]$AutoElevate,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$PassThroughArgs
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-RelaunchArgs {
    $relaunch = @()

    foreach ($kv in $PSBoundParameters.GetEnumerator()) {
        $name = $kv.Key
        $value = $kv.Value

        if ($name -eq 'AutoElevate') {
            continue
        }

        if ($name -in @('Build', 'DiskReadOnly', 'PersistQemuPath', 'HypervKeepVm', 'HypervHealthCheck', 'HypervCreateDiskIfMissing', 'SaveHypervProfile', 'UseHypervProfile', 'ListHypervProfiles')) {
            if ($value) {
                $relaunch += "-$name"
            }
            continue
        }

        if ($name -in @('CustomArg', 'PassThroughArgs')) {
            foreach ($item in @($value)) {
                $relaunch += "-$name"
                $relaunch += [string]$item
            }
            continue
        }

        $relaunch += "-$name"
        $relaunch += [string]$value
    }

    return $relaunch
}

function Get-HypervProfilesDir {
    return Join-Path $Root 'build/windows-hyperv-profiles'
}

function Get-HypervProfilePath([string]$Name) {
    return Join-Path (Get-HypervProfilesDir) ("$Name.json")
}

function Set-FromProfile([object]$ProfileData, [string]$ParamName) {
    if ($PSBoundParameters.ContainsKey($ParamName)) {
        return
    }
    $prop = $ProfileData.PSObject.Properties[$ParamName]
    if ($null -ne $prop) {
        Set-Variable -Name $ParamName -Value $prop.Value -Scope Script
    }
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$VeerVm = Join-Path $Root ("target/{0}/{1}/veer-vm.exe" -f $Target, $Profile)

if ($ListHypervProfiles) {
    $profilesDir = Get-HypervProfilesDir
    if (-not (Test-Path $profilesDir)) {
        Write-Host 'No Hyper-V profiles found.'
        exit 0
    }

    $profiles = Get-ChildItem -Path $profilesDir -Filter '*.json' | Select-Object -ExpandProperty BaseName
    if (-not $profiles) {
        Write-Host 'No Hyper-V profiles found.'
        exit 0
    }

    Write-Host 'Hyper-V profiles:'
    foreach ($name in $profiles) {
        Write-Host "  - $name"
    }
    exit 0
}

if ($UseHypervProfile -or $SaveHypervProfile) {
    if (-not $HypervProfileName) {
        $HypervProfileName = 'default'
    }
}

if ($UseHypervProfile) {
    $profilePath = Get-HypervProfilePath -Name $HypervProfileName
    if (-not (Test-Path $profilePath)) {
        throw "Hyper-V profile not found: $profilePath"
    }

    $profileData = Get-Content -Raw -Path $profilePath | ConvertFrom-Json
    Set-FromProfile -ProfileData $profileData -ParamName 'Backend'
    Set-FromProfile -ProfileData $profileData -ParamName 'Kernel'
    Set-FromProfile -ProfileData $profileData -ParamName 'Memory'
    Set-FromProfile -ProfileData $profileData -ParamName 'Cpus'
    Set-FromProfile -ProfileData $profileData -ParamName 'Disk'
    Set-FromProfile -ProfileData $profileData -ParamName 'DiskReadOnly'
    Set-FromProfile -ProfileData $profileData -ParamName 'HypervSwitch'
    Set-FromProfile -ProfileData $profileData -ParamName 'HypervVmName'
    Set-FromProfile -ProfileData $profileData -ParamName 'HypervKeepVm'
    Set-FromProfile -ProfileData $profileData -ParamName 'AutoElevate'
    Set-FromProfile -ProfileData $profileData -ParamName 'HypervCreateDiskIfMissing'
    Set-FromProfile -ProfileData $profileData -ParamName 'HypervDiskSizeGiB'
}

if ($SaveHypervProfile) {
    $profilesDir = Get-HypervProfilesDir
    New-Item -ItemType Directory -Force -Path $profilesDir | Out-Null

    $profilePath = Get-HypervProfilePath -Name $HypervProfileName
    $profileObject = [ordered]@{
        Backend                     = $Backend
        Kernel                      = $Kernel
        Memory                      = $Memory
        Cpus                        = $Cpus
        Disk                        = $Disk
        DiskReadOnly                = [bool]$DiskReadOnly
        HypervSwitch                = $HypervSwitch
        HypervVmName                = $HypervVmName
        HypervKeepVm                = [bool]$HypervKeepVm
        AutoElevate                 = [bool]$AutoElevate
        HypervCreateDiskIfMissing   = [bool]$HypervCreateDiskIfMissing
        HypervDiskSizeGiB           = $HypervDiskSizeGiB
    }

    $profileObject | ConvertTo-Json | Set-Content -Path $profilePath
    Write-Host "Saved Hyper-V profile: $HypervProfileName"
    Write-Host "  path: $profilePath"
    exit 0
}

if ($Build) {
    $oldTarget = $env:VEER_VM_WIN_TARGET
    $env:VEER_VM_WIN_TARGET = $Target
    try {
        & (Join-Path $Root 'scripts/build-windows-host-tools.ps1') $Profile
        if ($LASTEXITCODE -ne 0) {
            throw "Build failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        if ($null -eq $oldTarget) {
            Remove-Item Env:VEER_VM_WIN_TARGET -ErrorAction SilentlyContinue
        }
        else {
            $env:VEER_VM_WIN_TARGET = $oldTarget
        }
    }
}

if (-not (Test-Path $VeerVm)) {
    throw "veer-vm binary not found at $VeerVm. Run: ./scripts/build-windows-host-tools.ps1 $Profile"
}

if (($Backend -eq 'qemu' -or $Backend -eq 'auto') -and -not $env:VEER_VM_QEMU -and $AutoDetectQemu) {
    $finder = Join-Path $Root 'scripts/find-qemu-windows.ps1'
    if (Test-Path $finder) {
        $finderArgs = @('-SetProcessEnv', '-Quiet')
        if ($PersistQemuPath) {
            $finderArgs += '-PersistUserEnv'
        }
        & $finder @finderArgs
        if ($LASTEXITCODE -eq 0 -and $env:VEER_VM_QEMU) {
            Write-Host "Detected QEMU: $($env:VEER_VM_QEMU)"
        }
    }
}

$KernelPath = if ([System.IO.Path]::IsPathRooted($Kernel)) {
    $Kernel
}
else {
    (Join-Path $Root $Kernel)
}

if (-not (Test-Path $KernelPath)) {
    throw "Kernel/ISO not found at $KernelPath. Build ISO first with scripts/build-qemu-pc.sh"
}

$args = @(
    '--backend', $Backend,
    '--kernel', $KernelPath,
    '--arch', $Arch,
    '--memory', $Memory.ToString(),
    '--cpus', $Cpus.ToString()
)

if ($Disk) {
    $DiskPath = if ([System.IO.Path]::IsPathRooted($Disk)) {
        $Disk
    }
    else {
        (Join-Path $Root $Disk)
    }

    if (-not (Test-Path $DiskPath) -and -not ($Backend -eq 'hyperv' -and $HypervCreateDiskIfMissing)) {
        throw "Disk image not found at $DiskPath"
    }

    $args += @('--disk', $DiskPath)
    if ($DiskReadOnly) {
        $args += '--disk-ro'
    }
}

if ($CustomRunner) {
    $RunnerPath = if ([System.IO.Path]::IsPathRooted($CustomRunner)) {
        $CustomRunner
    }
    else {
        $candidate = Join-Path $Root $CustomRunner
        if (Test-Path $candidate) { $candidate } else { $CustomRunner }
    }
    $args += @('--custom-runner', $RunnerPath)
}

if ($HypervSwitch) {
    $args += @('--hyperv-switch', $HypervSwitch)
}

if ($HypervVmName) {
    $args += @('--hyperv-vm-name', $HypervVmName)
}

if ($HypervKeepVm) {
    $args += '--hyperv-keep-vm'
}

if ($HypervHealthCheck) {
    $args += '--hyperv-health-check'
}

if ($HypervCreateDiskIfMissing) {
    $args += '--hyperv-create-disk-if-missing'
}

if ($HypervDiskSizeGiB -ne 16) {
    $args += @('--hyperv-disk-size-gib', $HypervDiskSizeGiB.ToString())
}

foreach ($item in $CustomArg) {
    $args += @('--custom-arg', $item)
}

if ($PassThroughArgs) {
    $args += $PassThroughArgs
}

if ($Backend -eq 'hyperv' -and -not $HypervHealthCheck -and -not (Test-IsAdministrator)) {
    if ($AutoElevate) {
        Write-Host "Hyper-V backend requires admin; requesting elevation..."
        $relaunchArgs = @(
            '-NoProfile',
            '-ExecutionPolicy', 'Bypass',
            '-File', $PSCommandPath
        ) + (Get-RelaunchArgs)

        $proc = Start-Process -FilePath 'pwsh' -Verb RunAs -ArgumentList $relaunchArgs -PassThru -Wait
        exit $proc.ExitCode
    }

    throw "Hyper-V backend requires elevation. Re-run this script in an Administrator PowerShell, or pass -AutoElevate."
}

# Add default QEMU install location to PATH if not already there
$defaultQemuDir = "C:\Program Files\qemu"
if ((Test-Path $defaultQemuDir) -and ($env:PATH -notlike "*$defaultQemuDir*")) {
    $env:PATH += ";$defaultQemuDir"
}

Write-Host "Launching VeerOS guest via veer-vm"
Write-Host "  binary : $VeerVm"
Write-Host "  backend: $Backend"
Write-Host "  kernel : $KernelPath"
if (($Backend -eq 'qemu' -or $Backend -eq 'auto') -and $env:VEER_VM_QEMU) {
    Write-Host "  qemu   : $($env:VEER_VM_QEMU)"
}

& $VeerVm @args
exit $LASTEXITCODE
