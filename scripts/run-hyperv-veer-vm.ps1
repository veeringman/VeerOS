#!/usr/bin/env pwsh
param(
    [Parameter(Mandatory = $true)]
    [string]$Kernel,

    [int]$MemoryMiB = 256,
    [int]$Cpus = 1,

    [string]$VmName = "VeerOS-HyperV",
    [string]$SwitchName,

    [string]$Disk,
    [switch]$DiskReadOnly,
    [switch]$CreateDiskIfMissing,
    [int]$DiskSizeGiB = 16,

    [switch]$KeepVm,
    [switch]$HealthCheck
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($id)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Hyper-V backend requires an elevated shell (Run as Administrator)."
    }
}

function Test-IsAdmin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Assert-HyperV {
    if (-not (Get-Command Get-VM -ErrorAction SilentlyContinue)) {
        throw "Hyper-V PowerShell cmdlets are unavailable. Enable Hyper-V and import the Hyper-V module."
    }
}

function Resolve-SwitchName([string]$Requested) {
    if ($Requested) {
        $sw = Get-VMSwitch -Name $Requested -ErrorAction SilentlyContinue
        if ($null -eq $sw) {
            throw "Hyper-V switch not found: $Requested"
        }
        return $Requested
    }

    $default = Get-VMSwitch -Name 'Default Switch' -ErrorAction SilentlyContinue
    if ($null -ne $default) {
        return 'Default Switch'
    }

    $candidate = Get-VMSwitch -ErrorAction SilentlyContinue |
        Where-Object { $_.SwitchType -eq 'External' -or $_.SwitchType -eq 'Internal' } |
        Select-Object -First 1
    if ($null -ne $candidate) {
        return $candidate.Name
    }

    return $null
}

function Remove-ExistingVm([string]$Name) {
    $vm = Get-VM -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $vm) {
        return
    }

    if ($vm.State -ne 'Off') {
        Stop-VM -Name $Name -TurnOff -Force -Confirm:$false | Out-Null
    }

    $hardDisks = @(Get-VMHardDiskDrive -VMName $Name -ErrorAction SilentlyContinue)
    foreach ($d in $hardDisks) {
        if ($d.Path) {
            Set-ItemProperty -Path $d.Path -Name IsReadOnly -Value $false -ErrorAction SilentlyContinue
        }
    }

    Remove-VM -Name $Name -Force -Confirm:$false
}

function Resolve-DiskPath([string]$DiskInput, [bool]$CreateIfMissing, [int]$SizeGiB) {
    if (-not $DiskInput) {
        return $null
    }

    $resolved = $null
    if (Test-Path $DiskInput) {
        $resolved = (Resolve-Path $DiskInput).Path
    }
    else {
        $resolved = [System.IO.Path]::GetFullPath($DiskInput)
    }

    $ext = [System.IO.Path]::GetExtension($resolved).ToLowerInvariant()
    if ($ext -ne '.vhd' -and $ext -ne '.vhdx') {
        throw "Hyper-V backend supports only .vhd/.vhdx for -Disk"
    }

    if (-not (Test-Path $resolved)) {
        if (-not $CreateIfMissing) {
            throw "Disk not found: $DiskInput"
        }
        if ($SizeGiB -lt 1) {
            throw "DiskSizeGiB must be >= 1"
        }

        $diskDir = Split-Path -Path $resolved -Parent
        if ($diskDir -and -not (Test-Path $diskDir)) {
            New-Item -ItemType Directory -Force -Path $diskDir | Out-Null
        }

        $sizeBytes = "${SizeGiB}GB"
        New-VHD -Path $resolved -Dynamic -SizeBytes $sizeBytes | Out-Null
        Write-Host "[veer-vm] created disk: $resolved (${SizeGiB} GiB dynamic)"
    }

    return $resolved
}

function Invoke-HealthCheck {
    $ok = $true

    if (Test-IsAdmin) {
        Write-Host "[health] ok   elevation: administrator"
    }
    else {
        Write-Host "[health] fail elevation: not administrator"
        $ok = $false
    }

    if (Get-Command Get-VM -ErrorAction SilentlyContinue) {
        Write-Host "[health] ok   hyperv module: available"
    }
    else {
        Write-Host "[health] fail hyperv module: cmdlets unavailable"
        $ok = $false
    }

    if (Test-Path $Kernel) {
        $resolvedKernel = (Resolve-Path $Kernel).Path
        Write-Host "[health] ok   kernel iso: $resolvedKernel"
    }
    else {
        Write-Host "[health] fail kernel iso: not found ($Kernel)"
        $ok = $false
    }

    try {
        $sw = Resolve-SwitchName -Requested $SwitchName
        if ($sw) {
            Write-Host "[health] ok   switch: $sw"
        }
        else {
            Write-Host "[health] warn switch: none found (VM will run disconnected)"
        }
    }
    catch {
        Write-Host "[health] fail switch: $($_.Exception.Message)"
        $ok = $false
    }

    if ($Disk) {
        try {
            $diskPath = Resolve-DiskPath -DiskInput $Disk -CreateIfMissing:$false -SizeGiB $DiskSizeGiB
            Write-Host "[health] ok   disk: $diskPath"
        }
        catch {
            if ($CreateDiskIfMissing -and $_.Exception.Message -like 'Disk not found:*') {
                Write-Host "[health] ok   disk: missing now, will be created on launch"
            }
            else {
                Write-Host "[health] fail disk: $($_.Exception.Message)"
                $ok = $false
            }
        }
    }

    if ($ok) {
        Write-Host "[health] status: ready"
        exit 0
    }
    else {
        Write-Host "[health] status: blocked"
        exit 2
    }
}

if ($HealthCheck) {
    Invoke-HealthCheck
}

Assert-Admin
Assert-HyperV

$selectedSwitch = Resolve-SwitchName -Requested $SwitchName

$kernelPath = (Resolve-Path $Kernel).Path
if (-not (Test-Path $kernelPath)) {
    throw "ISO not found: $Kernel"
}

if ($MemoryMiB -lt 128) {
    throw "MemoryMiB must be >= 128"
}
if ($Cpus -lt 1 -or $Cpus -gt 64) {
    throw "Cpus must be between 1 and 64"
}

$isoDir = Split-Path -Path $kernelPath -Parent
$vmPath = Join-Path $isoDir ("hyperv-" + $VmName)
New-Item -ItemType Directory -Force -Path $vmPath | Out-Null

Remove-ExistingVm -Name $VmName

$memoryBytes = "${MemoryMiB}MB"
$vm = New-VM -Name $VmName -Generation 2 -MemoryStartupBytes $memoryBytes -Path $vmPath -NoVHD
Set-VMProcessor -VMName $VmName -Count $Cpus
Set-VM -VMName $VmName -AutomaticCheckpointsEnabled $false | Out-Null
Set-VMMemory -VMName $VmName -DynamicMemoryEnabled $false

if ($selectedSwitch) {
    Connect-VMNetworkAdapter -VMName $VmName -SwitchName $selectedSwitch
}

Add-VMDvdDrive -VMName $VmName -Path $kernelPath | Out-Null
$dvd = Get-VMDvdDrive -VMName $VmName
Set-VMFirmware -VMName $VmName -EnableSecureBoot Off -FirstBootDevice $dvd

if ($Disk) {
    $diskPath = Resolve-DiskPath -DiskInput $Disk -CreateIfMissing:$CreateDiskIfMissing -SizeGiB $DiskSizeGiB

    if ($DiskReadOnly) {
        Set-ItemProperty -Path $diskPath -Name IsReadOnly -Value $true
    }

    Add-VMHardDiskDrive -VMName $VmName -Path $diskPath | Out-Null
}

Write-Host "[veer-vm] Hyper-V VM: $VmName"
if ($selectedSwitch) {
    Write-Host "[veer-vm] Hyper-V switch: $selectedSwitch"
}
else {
    Write-Host "[veer-vm] Hyper-V switch: none (guest network disconnected)"
}
Write-Host "[veer-vm] Starting VM..."
Start-VM -Name $VmName | Out-Null
Write-Host "[veer-vm] VM is running. Connect with: vmconnect.exe localhost $VmName"

while ((Get-VM -Name $VmName).State -eq 'Running') {
    [System.Threading.Thread]::Sleep(1000)
}

Write-Host "[veer-vm] VM stopped"

if (-not $KeepVm) {
    Remove-ExistingVm -Name $VmName
    Write-Host "[veer-vm] VM definition removed"
}
