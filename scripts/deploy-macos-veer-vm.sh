#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${VEEROS_PROFILE:-debug}"
VM_NAME="${VEEROS_VM_NAME:-VeerOS-AArch64-HVF}"
DEST_ROOT="${VEEROS_VM_HOME:-${HOME}/VeerOS-VMs}"
DEST="${DEST_ROOT}/${VM_NAME}"
DISK_SIZE="${VEEROS_DISK_SIZE:-512m}"
MEMORY_MIB="${VEEROS_MEMORY_MIB:-512}"

case "${PROFILE}" in
    debug|release) ;;
    *)
        echo "ERROR: VEEROS_PROFILE must be debug or release" >&2
        exit 2
        ;;
esac

host_target() {
    case "$(uname -m)" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64|amd64) echo "x86_64-apple-darwin" ;;
        *)
            echo "ERROR: unsupported macOS host architecture: $(uname -m)" >&2
            exit 1
            ;;
    esac
}

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required tool not found: $1" >&2
        exit 1
    fi
}

hvf_only_entitlements() {
    local path="${TMPDIR:-/tmp}/veer-hvf-only.entitlements"
    cat > "${path}" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
"https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.hypervisor</key>
    <true/>
</dict>
</plist>
PLIST
    echo "${path}"
}

create_disk() {
    local path="$1"
    local size="$2"
    if [[ -f "${path}" ]]; then
        return
    fi
    mkdir -p "$(dirname "${path}")"
    if command -v mkfile >/dev/null 2>&1; then
        mkfile -n "${size}" "${path}"
    else
        truncate -s "${size}" "${path}"
    fi
}

format_disk_if_needed() {
    local path="$1"
    if /usr/sbin/fsck_msdos -n "${path}" >/dev/null 2>&1; then
        return
    fi
    require_tool hdiutil
    require_tool newfs_msdos

    local dev=""
    dev="$(hdiutil attach -nomount -readwrite -imagekey diskimage-class=CRawDiskImage "${path}" | awk '/^\/dev\// { print $1; exit }')"
    if [[ -z "${dev}" ]]; then
        echo "ERROR: hdiutil did not attach ${path}" >&2
        exit 1
    fi
    local rawdev="/dev/r$(basename "${dev}")"
    if ! newfs_msdos -F 32 -v VEEROS "${rawdev}" >/dev/null; then
        hdiutil detach "${dev}" >/dev/null 2>&1 || true
        exit 1
    fi
    hdiutil detach "${dev}" >/dev/null
}

write_launcher() {
    local path="$1"
    cat > "${path}" <<'LAUNCHER'
#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MEMORY_MIB="${VEEROS_MEMORY_MIB:-512}"
VMNET_MODE="${VEEROS_VMNET_MODE:-shared}"

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    cat <<'HELP'
Usage: ./veer-vm-macos [extra veer-vm args]

Starts VeerOS AArch64 on Apple Silicon HVF with vmnet networking.
Guest services:
  SSH: ssh root@192.168.2.100    password: toor
  VSC: ./veer-connect-shell       then login root / toor

The FAT32 disk image is at disks/veeros-root.raw and is attached as virtio-blk.
Inside VeerOS it is mounted at /disk when present.
HELP
    exit 0
fi

exec sudo "${HERE}/bin/veer-vm" \
    --arch aarch64 \
    --kernel "${HERE}/images/kernel-aarch64-debug.elf" \
    --memory "${MEMORY_MIB}" \
    --disk "${HERE}/disks/veeros-root.raw" \
    --vmnet "${VMNET_MODE}" \
    "$@"
LAUNCHER
    chmod +x "${path}"
}

write_connect() {
    local path="$1"
    cat > "${path}" <<'CONNECT'
#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUEST_IP="${VEEROS_GUEST_IP:-192.168.2.100}"
exec "${HERE}/bin/veer-connect" shell "${GUEST_IP}" 2323
CONNECT
    chmod +x "${path}"
}

write_readme() {
    local path="$1"
    cat > "${path}" <<'README'
# VeerOS AArch64 HVF VM Kit

This folder is a self-contained macOS VM bundle for the current VeerOS AArch64 virtual target.

## Run

```sh
./veer-vm-macos
```

The VM uses vmnet shared networking. macOS will ask for sudo because vmnet needs elevated privileges.

## Login

- SSH: `ssh root@192.168.2.100`, password `toor`
- VSC shell: `./veer-connect-shell`, then login `root` / `toor`

## Included Artifacts

- `bin/veer-vm`: signed macOS HVF VM runner
- `bin/veer-connect`: VSC client
- `bin/fold`: Fold engine CLI for folded VM launch
- `images/kernel-aarch64-debug.elf`: VeerOS kernel image
- `disks/veeros-root.raw`: FAT32 disk image mounted at `/disk`

## Manager

Use the installed manager from the parent folder:

```sh
../veeros-vm create --name dev1 --target aarch64-hvf --mode normal
../veeros-vm start --name dev1
../veeros-vm create --name fold1 --target aarch64-hvf --mode folded
../veeros-vm start --name fold1
../veeros-vm connect --name dev1
../veeros-vm ssh --name dev1
```

## Disk Note

The raw disk image is formatted FAT32 and passed to the Apple Silicon HVF backend as virtio-blk. VeerOS keeps boot-critical files in RamFS and mounts persistent disk files at `/disk`.

Useful in-guest commands include `ps`, `tasks`, `vi`, `vim`, `ls`, `cat`, `write`, `mkdir`, `touch`, `rm`, `mv`, `pwd`, `cd`, `tree`, `mount`, `lsblk`, `df`, `ifconfig`, `netstat`, `hwinfo`, and `dmesg`.
README
}

write_manual() {
    local path="$1"
    local source_manual="${ROOT}/docs/macos-vm-manual.txt"
    if [[ -f "${source_manual}" ]]; then
        cp "${source_manual}" "${path}"
        return
    fi
        cat > "${path}" <<'MANUAL'
VeerOS VM Manual for macOS
==========================

Location
--------
The macOS VM toolkit is deployed under:

    ~/VeerOS-VMs

The default Apple Silicon kit is:

    ~/VeerOS-VMs/VeerOS-AArch64-HVF

The VM manager is:

    ~/VeerOS-VMs/veeros-vm


Quick Start
-----------
Create one normal VM instance:

    ~/VeerOS-VMs/veeros-vm create --name dev1 --target aarch64-hvf --mode normal

Start it:

    ~/VeerOS-VMs/veeros-vm start --name dev1

Connect to the VeerOS secure console:

    ~/VeerOS-VMs/veeros-vm connect --name dev1

Connect over SSH:

    ~/VeerOS-VMs/veeros-vm ssh --name dev1

Login credentials:

    user: root
    password: toor


Folded VM Quick Start
---------------------
Create one folded VM instance:

    ~/VeerOS-VMs/veeros-vm create --name fold1 --target aarch64-hvf --mode folded

Start it:

    ~/VeerOS-VMs/veeros-vm start --name fold1

Connect to the folded VM secure console:

    ~/VeerOS-VMs/veeros-vm connect --name fold1

Connect to the folded VM over SSH:

    ~/VeerOS-VMs/veeros-vm ssh --name fold1

Folded and normal VMs use the same guest service contract by default:

    guest IP: 192.168.2.100
    SSH port: 22
    VSC port: 2323

Do not run two instances with the same guest IP at the same time unless the
manager records have been configured with different networking.


VM Manager Commands
-------------------
Show help:

    ~/VeerOS-VMs/veeros-vm help

Create or update an instance record:

    ~/VeerOS-VMs/veeros-vm create --name NAME --target TARGET --mode MODE

Start an instance:

    ~/VeerOS-VMs/veeros-vm start --name NAME

Stop an instance:

    ~/VeerOS-VMs/veeros-vm stop --name NAME

List instances:

    ~/VeerOS-VMs/veeros-vm list

Show instance status:

    ~/VeerOS-VMs/veeros-vm status --name NAME

Show logs:

    ~/VeerOS-VMs/veeros-vm logs --name NAME

Follow logs:

    ~/VeerOS-VMs/veeros-vm logs --name NAME --follow

Open the secure console:

    ~/VeerOS-VMs/veeros-vm connect --name NAME

Open SSH:

    ~/VeerOS-VMs/veeros-vm ssh --name NAME

Pass commands to the bundled Fold CLI:

    ~/VeerOS-VMs/veeros-vm fold --help


Targets and Modes
-----------------
Supported manager targets:

    aarch64-hvf
    raspi5
    esp32c6
    qemu-esp32c6
    qemu-pc

Supported modes:

    normal
    folded

Local start support is currently implemented for aarch64-hvf on Apple Silicon.
The raspi5 and esp32c6 targets are hardware deployment records. The qemu-*
targets are staged records until QEMU launch integration is completed.


Useful Create Options
---------------------
Set guest memory in MiB:

    --memory 512

Set macOS vmnet mode:

    --vmnet shared
    --vmnet host

Set the guest service IP used by connect and SSH:

    --guest-ip 192.168.2.100

Use a specific kit folder:

    --kit ~/VeerOS-VMs/VeerOS-AArch64-HVF

Override the kernel image:

    --kernel /path/to/kernel.elf

Override the disk image path:

    --disk /path/to/root.raw


Direct Kit Commands
-------------------
You can also run the default kit launcher directly:

    cd ~/VeerOS-VMs/VeerOS-AArch64-HVF
    ./veer-vm-macos

Connect with the kit-local console helper:

    cd ~/VeerOS-VMs/VeerOS-AArch64-HVF
    ./veer-connect-shell


In-Guest Shell
--------------
The guest starts VeerOS Shell v0.1.0 after login.

Useful commands include:

    help
    ps
    tasks
    pwd
    ls
    tree
    cd
    mkdir
    touch
    write
    cat
    rm
    mv
    vi
    vim
    df
    mount
    lsblk
    ifconfig
    netstat
    hwinfo
    dmesg
    exit

The built-in vi/vim editor can be opened with:

    vi FILE
    vim FILE

Exit vi/vim with:

    :q


Filesystem and Disk Status
--------------------------
The deployed kit creates this FAT32 disk image:

    ~/VeerOS-VMs/VeerOS-AArch64-HVF/disks/veeros-root.raw

The Apple Silicon HVF backend exposes this image as virtio-blk. VeerOS mounts
it at /disk when present. Boot-critical files remain in RamFS under /etc, /dev,
and /tmp, while regular files under /disk persist across VM restarts.

Inside the guest, use these commands to inspect storage:

    df
    mount
    lsblk


Networking
----------
The default guest address is:

    192.168.2.100

The default macOS vmnet mode is:

    shared

Service ports:

    SSH: 22
    Veer secure console: 2323

If SSH or the secure console cannot connect, first check that only one VM is
using the default IP, then inspect logs:

    ~/VeerOS-VMs/veeros-vm list
    ~/VeerOS-VMs/veeros-vm status --name NAME
    ~/VeerOS-VMs/veeros-vm logs --name NAME


macOS Notes
-----------
Starting an aarch64-hvf VM uses Hypervisor.framework and vmnet. The start
command may ask for sudo. If status or folded logs say sudo is required, refresh
sudo first:

    sudo -v

The deployed veer-vm binary is ad-hoc signed with the Hypervisor entitlement.
For local ad-hoc runs, the expected entitlement is:

    com.apple.security.hypervisor


Rebuild and Redeploy
--------------------
From the VeerOS source checkout:

    cd ~/VeerOS
    ./scripts/deploy-macos-veer-vm.sh

This rebuilds the kernel and macOS host tools, refreshes the deployed kit, signs
the deployed VM runner, installs the VM manager, and regenerates this manual.


Current Limitations
-------------------
1. Persistent FAT32 files are mounted at /disk. The root filesystem still uses
    RamFS for boot-critical files.
2. FAT32 long filenames and directory creation are not implemented yet; use
    regular 8.3-style filenames for the disk-backed path.
3. Local start is implemented for the Apple Silicon aarch64-hvf target.
4. Hardware targets are represented by manager records and staged images, but
     flashing and board-specific deployment remain separate workflows.
5. QEMU target records exist, but QEMU launch integration still needs to be
     completed.
MANUAL
}

main() {
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo "ERROR: this deploy tool is intended for macOS" >&2
        exit 1
    fi

    require_tool cargo
    require_tool codesign

    cd "${ROOT}"

    echo "==> Building VeerOS AArch64 kernel (${PROFILE})"
    ./scripts/build-aarch64-virt.sh "${PROFILE}"

    echo "==> Building macOS host tools (${PROFILE})"
    VEER_VM_MAC_PROFILE="${PROFILE}" VEER_VM_MAC_TARGET=native ./scripts/build-mac-host-tools.sh "${PROFILE}"

    local target
    target="$(host_target)"
    local profile_dir="${ROOT}/target/${target}/${PROFILE}"
    local kernel="${ROOT}/build/veer-vm/kernel-aarch64-${PROFILE}.elf"

    mkdir -p "${DEST}/bin" "${DEST}/images" "${DEST}/disks"
    cp "${profile_dir}/veer-vm" "${DEST}/bin/veer-vm"
    cp "${profile_dir}/veer-connect" "${DEST}/bin/veer-connect"
    cp "${profile_dir}/fold" "${DEST}/bin/fold"
    cp "${kernel}" "${DEST}/images/kernel-aarch64-debug.elf"
    cp "${ROOT}/scripts/veeros-vm-manager.sh" "${DEST_ROOT}/veeros-vm"
    chmod +x "${DEST_ROOT}/veeros-vm"
    xattr -dr com.apple.quarantine "${DEST}/bin" "${DEST_ROOT}/veeros-vm" 2>/dev/null || true
    codesign --remove-signature "${DEST}/bin/veer-vm" >/dev/null 2>&1 || true
    codesign --force --sign - --timestamp=none --entitlements "$(hvf_only_entitlements)" "${DEST}/bin/veer-vm"

    create_disk "${DEST}/disks/veeros-root.raw" "${DISK_SIZE}"
    format_disk_if_needed "${DEST}/disks/veeros-root.raw"
    write_launcher "${DEST}/veer-vm-macos"
    write_connect "${DEST}/veer-connect-shell"
    write_readme "${DEST}/README.md"
    write_manual "${DEST_ROOT}/VeerOS-VM-Manual.txt"

    echo ""
    echo "Deployed VeerOS VM kit: ${DEST}"
    echo "Run:"
    echo "  cd '${DEST}'"
    echo "  ./veer-vm-macos"
    echo "  ../veeros-vm create --name dev1 --target aarch64-hvf --mode normal"
    echo "  ../veeros-vm create --name fold1 --target aarch64-hvf --mode folded"
    echo "Connect:"
    echo "  ./veer-connect-shell"
    echo "  ../veeros-vm connect --name dev1"
    echo "  ssh root@192.168.2.100"
    echo "Manual:"
    echo "  ${DEST_ROOT}/VeerOS-VM-Manual.txt"
}

main "$@"