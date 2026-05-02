#!/usr/bin/env bash
set -euo pipefail

TOOL_HOME="${VEEROS_VM_HOME:-${HOME}/VeerOS-VMs}"
DEFAULT_KIT="${TOOL_HOME}/VeerOS-AArch64-HVF"
INSTANCES_DIR="${TOOL_HOME}/instances"
DEFAULT_NAME="veer-a64"
DEFAULT_TARGET="aarch64-hvf"
DEFAULT_MODE="normal"
DEFAULT_MEMORY="512"
DEFAULT_VMNET="shared"
DEFAULT_GUEST_IP="192.168.2.100"

usage() {
    cat <<'USAGE'
Usage: veeros-vm <command> [options]

Commands:
  create      Create or update a VM instance record
  start       Start a normal or folded VeerOS VM instance
  stop        Stop a VM instance
  list        List VM instances
  status      Show one VM instance status
  logs        Show VM logs
  connect     Open VSC shell with veer-connect
  ssh         Open SSH as root
  fold        Pass through to the bundled fold CLI
  help        Show this help

Common options:
  --name NAME                 Instance name (default: veer-a64)
  --target TARGET             aarch64-hvf | raspi5 | esp32c6 | qemu-esp32c6 | qemu-pc
  --mode MODE                 normal | folded
  --memory MIB                Guest memory in MiB (default: 512)
  --vmnet shared|host         macOS vmnet mode for aarch64-hvf (default: shared)
  --guest-ip IP               Guest service IP for connect/ssh (default: 192.168.2.100)
  --kit PATH                  VM kit path (default: ~/VeerOS-VMs/VeerOS-AArch64-HVF)
  --kernel PATH               Override kernel image
  --disk PATH                 Override disk image path

Examples:
  veeros-vm create --name dev1 --target aarch64-hvf --mode normal
  veeros-vm start --name dev1
  veeros-vm create --name fold1 --target aarch64-hvf --mode folded
  veeros-vm start --name fold1
  veeros-vm connect --name dev1
  veeros-vm ssh --name dev1
  veeros-vm list

Notes:
  aarch64-hvf can run locally on Apple Silicon via veer-vm + vmnet.
  raspi5 and esp32c6 records stage build artifacts for deployment/hardware use.
  qemu-* targets require QEMU tools installed before they can be started.
USAGE
}

say() { printf '[veeros-vm] %s\n' "$*"; }
die() { printf '[veeros-vm] ERROR: %s\n' "$*" >&2; exit 1; }

host_target() {
    case "$(uname -m)" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64|amd64) echo "x86_64-apple-darwin" ;;
        *) die "unsupported host architecture: $(uname -m)" ;;
    esac
}

format_disk_if_needed() {
    local path="$1"
    if /usr/sbin/fsck_msdos -n "${path}" >/dev/null 2>&1; then
        return
    fi
    command -v hdiutil >/dev/null 2>&1 || die "hdiutil is required to format ${path}"
    command -v newfs_msdos >/dev/null 2>&1 || die "newfs_msdos is required to format ${path}"

    local dev
    dev="$(hdiutil attach -nomount -readwrite -imagekey diskimage-class=CRawDiskImage "${path}" | awk '/^\/dev\// { print $1; exit }')"
    [[ -n "${dev}" ]] || die "hdiutil did not attach ${path}"
    local rawdev="/dev/r$(basename "${dev}")"
    if ! newfs_msdos -F 32 -v VEEROS "${rawdev}" >/dev/null; then
        hdiutil detach "${dev}" >/dev/null 2>&1 || true
        die "newfs_msdos failed for ${path}"
    fi
    hdiutil detach "${dev}" >/dev/null
}

instance_dir() { printf '%s/%s\n' "${INSTANCES_DIR}" "$1"; }
config_path() { printf '%s/vm.conf\n' "$(instance_dir "$1")"; }

load_config() {
    local name="$1"
    local cfg
    cfg="$(config_path "${name}")"
    [[ -f "${cfg}" ]] || die "VM '${name}' does not exist; run: veeros-vm create --name ${name}"
    # shellcheck disable=SC1090
    source "${cfg}"
}

save_config() {
    local name="$1"
    local dir cfg
    dir="$(instance_dir "${name}")"
    cfg="${dir}/vm.conf"
    mkdir -p "${dir}"
    cat > "${cfg}" <<CFG
NAME='${NAME}'
TARGET='${TARGET}'
MODE='${MODE}'
MEMORY='${MEMORY}'
VMNET='${VMNET}'
GUEST_IP='${GUEST_IP}'
KIT='${KIT}'
KERNEL='${KERNEL}'
DISK='${DISK}'
PID_FILE='${dir}/vm.pid'
LOG_FILE='${dir}/vm.log'
FOLD_NAME='${NAME}'
FOLD_STATE='${dir}/fold-state'
CFG
}

parse_common() {
    NAME="${DEFAULT_NAME}"
    TARGET="${DEFAULT_TARGET}"
    MODE="${DEFAULT_MODE}"
    MEMORY="${DEFAULT_MEMORY}"
    VMNET="${DEFAULT_VMNET}"
    GUEST_IP="${DEFAULT_GUEST_IP}"
    KIT="${DEFAULT_KIT}"
    KERNEL=""
    DISK=""
    EXTRA=()

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --name) NAME="$2"; shift 2 ;;
            --target) TARGET="$2"; shift 2 ;;
            --mode) MODE="$2"; shift 2 ;;
            --memory) MEMORY="$2"; shift 2 ;;
            --vmnet) VMNET="$2"; shift 2 ;;
            --guest-ip) GUEST_IP="$2"; shift 2 ;;
            --kit) KIT="$2"; shift 2 ;;
            --kernel) KERNEL="$2"; shift 2 ;;
            --disk) DISK="$2"; shift 2 ;;
            --help|-h) usage; exit 0 ;;
            *) EXTRA+=("$1"); shift ;;
        esac
    done

    case "${MODE}" in normal|folded) ;; *) die "--mode must be normal or folded" ;; esac
    case "${TARGET}" in aarch64-hvf|raspi5|esp32c6|qemu-esp32c6|qemu-pc) ;; *) die "unsupported target: ${TARGET}" ;; esac
    case "${VMNET}" in shared|host) ;; *) die "--vmnet must be shared or host" ;; esac

    if [[ -z "${KERNEL}" ]]; then
        case "${TARGET}" in
            aarch64-hvf) KERNEL="${KIT}/images/kernel-aarch64-debug.elf" ;;
            raspi5) KERNEL="${KIT}/images/kernel-raspi5.elf" ;;
            esp32c6) KERNEL="${KIT}/images/kernel-xiao-esp32c6.elf" ;;
            qemu-esp32c6) KERNEL="${KIT}/images/kernel-qemu-esp32c6.elf" ;;
            qemu-pc) KERNEL="${KIT}/images/veeros.iso" ;;
        esac
    fi
    if [[ -z "${DISK}" ]]; then
        DISK="${KIT}/disks/${NAME}.raw"
    fi
}

cmd_create() {
    parse_common "$@"
    mkdir -p "${KIT}/disks"
    if [[ ! -f "${DISK}" ]]; then
        if command -v mkfile >/dev/null 2>&1; then
            mkfile -n 512m "${DISK}"
        else
            truncate -s 512m "${DISK}"
        fi
    fi
    format_disk_if_needed "${DISK}"
    save_config "${NAME}"
    say "created ${NAME}: target=${TARGET} mode=${MODE} memory=${MEMORY}MiB"
}

require_kit_bin() {
    local bin="$1"
    [[ -x "${KIT}/bin/${bin}" ]] || die "missing ${KIT}/bin/${bin}; run scripts/deploy-macos-veer-vm.sh"
}

ensure_kernel_for_start() {
    [[ -f "${KERNEL}" ]] || die "kernel/image not found for ${TARGET}: ${KERNEL}"
}

start_normal_aarch64() {
    require_kit_bin veer-vm
    ensure_kernel_for_start
    mkdir -p "$(dirname "${LOG_FILE}")"
    sudo -v
    say "starting normal VM ${NAME} on vmnet ${VMNET}; log=${LOG_FILE}"
    nohup sudo -n "${KIT}/bin/veer-vm" \
        --arch aarch64 \
        --kernel "${KERNEL}" \
        --memory "${MEMORY}" \
        --disk "${DISK}" \
        --vmnet "${VMNET}" \
        >"${LOG_FILE}" 2>&1 &
    echo $! > "${PID_FILE}"
}

start_folded_aarch64() {
    require_kit_bin fold
    require_kit_bin veer-vm
    ensure_kernel_for_start
    mkdir -p "$(dirname "${LOG_FILE}")" "${FOLD_STATE}"
    sudo -v
    say "starting folded VM ${NAME} on vmnet ${VMNET}; fold state=${FOLD_STATE}"
    sudo -n env XDG_STATE_HOME="${FOLD_STATE}" "${KIT}/bin/fold" vm spawn \
        --name "${FOLD_NAME}" \
        --arch aarch64 \
        --kernel "${KERNEL}" \
        --memory "${MEMORY}" \
        --disk "${DISK}" \
        --vmnet "${VMNET}" \
        --vmm "${KIT}/bin/veer-vm" \
        >"${LOG_FILE}" 2>&1
}

cmd_start() {
    local name="${DEFAULT_NAME}"
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --name) name="$2"; shift 2 ;;
            --help|-h) usage; exit 0 ;;
            *) die "unknown start option: $1" ;;
        esac
    done
    load_config "${name}"
    case "${TARGET}:${MODE}" in
        aarch64-hvf:normal) start_normal_aarch64 ;;
        aarch64-hvf:folded) start_folded_aarch64 ;;
        qemu-esp32c6:*) die "qemu-esp32c6 start needs qemu-system-riscv32 integration; artifact is staged at ${KERNEL}" ;;
        qemu-pc:*) die "qemu-pc start needs qemu-system-x86_64 integration; artifact is staged at ${KERNEL}" ;;
        raspi5:*) die "raspi5 is a hardware image target; flash/deploy the staged image instead of starting locally" ;;
        esp32c6:*) die "esp32c6 is a hardware image target; flash/deploy via ESP32 tooling instead of starting locally" ;;
        *) die "unsupported start combination: ${TARGET}:${MODE}" ;;
    esac
}

cmd_stop() {
    local name="${DEFAULT_NAME}"
    while [[ $# -gt 0 ]]; do
        case "$1" in --name) name="$2"; shift 2 ;; *) die "unknown stop option: $1" ;; esac
    done
    load_config "${name}"
    if [[ "${MODE}" == "folded" ]]; then
        require_kit_bin fold
        sudo -n env XDG_STATE_HOME="${FOLD_STATE}" "${KIT}/bin/fold" stop "${FOLD_NAME}" || true
        return
    fi
    if [[ -f "${PID_FILE}" ]]; then
        local pid
        pid="$(cat "${PID_FILE}")"
        sudo kill "${pid}" 2>/dev/null || kill "${pid}" 2>/dev/null || true
        rm -f "${PID_FILE}"
    else
        pkill -f "${KIT}/bin/veer-vm.*${KERNEL}" 2>/dev/null || true
    fi
}

cmd_list() {
    mkdir -p "${INSTANCES_DIR}"
    printf '%-18s %-14s %-8s %-8s %s\n' NAME TARGET MODE MEMORY KERNEL
    for cfg in "${INSTANCES_DIR}"/*/vm.conf; do
        [[ -f "${cfg}" ]] || continue
        # shellcheck disable=SC1090
        source "${cfg}"
        printf '%-18s %-14s %-8s %-8s %s\n' "${NAME}" "${TARGET}" "${MODE}" "${MEMORY}" "${KERNEL}"
    done
}

cmd_status() {
    local name="${DEFAULT_NAME}"
    while [[ $# -gt 0 ]]; do
        case "$1" in --name) name="$2"; shift 2 ;; *) die "unknown status option: $1" ;; esac
    done
    load_config "${name}"
    echo "name=${NAME}"
    echo "target=${TARGET}"
    echo "mode=${MODE}"
    echo "memory=${MEMORY}"
    echo "kernel=${KERNEL}"
    echo "disk=${DISK}"
    echo "guest_ip=${GUEST_IP}"
    echo "log=${LOG_FILE}"
    if [[ "${MODE}" == "folded" ]]; then
        require_kit_bin fold
        if sudo -n true 2>/dev/null; then
            sudo -n env XDG_STATE_HOME="${FOLD_STATE}" "${KIT}/bin/fold" list || true
        else
            echo "state=unknown (folded vmnet status needs sudo; run: sudo -v)"
        fi
    elif [[ -f "${PID_FILE}" ]]; then
        local pid
        pid="$(cat "${PID_FILE}")"
        if kill -0 "${pid}" 2>/dev/null; then
            echo "state=running pid=${pid}"
        else
            echo "state=stopped stale_pid=${pid}"
        fi
    else
        echo "state=unknown"
    fi
}

cmd_logs() {
    local name="${DEFAULT_NAME}"
    local follow=0
    while [[ $# -gt 0 ]]; do
        case "$1" in --name) name="$2"; shift 2 ;; --follow|-f) follow=1; shift ;; *) die "unknown logs option: $1" ;; esac
    done
    load_config "${name}"
    if [[ "${MODE}" == "folded" ]]; then
        require_kit_bin fold
        if ! sudo -n true 2>/dev/null; then
            die "folded VM logs need sudo for this instance; run sudo -v first"
        fi
        if [[ "${follow}" == "1" ]]; then
            sudo -n env XDG_STATE_HOME="${FOLD_STATE}" "${KIT}/bin/fold" logs "${FOLD_NAME}" --follow
        else
            sudo -n env XDG_STATE_HOME="${FOLD_STATE}" "${KIT}/bin/fold" logs "${FOLD_NAME}"
        fi
        return
    fi
    [[ -f "${LOG_FILE}" ]] || die "no log yet: ${LOG_FILE}"
    if [[ "${follow}" == "1" ]]; then tail -f "${LOG_FILE}"; else tail -100 "${LOG_FILE}"; fi
}

cmd_connect() {
    local name="${DEFAULT_NAME}"
    while [[ $# -gt 0 ]]; do
        case "$1" in --name) name="$2"; shift 2 ;; *) die "unknown connect option: $1" ;; esac
    done
    load_config "${name}"
    require_kit_bin veer-connect
    exec "${KIT}/bin/veer-connect" shell "${GUEST_IP}" 2323
}

cmd_ssh() {
    local name="${DEFAULT_NAME}"
    while [[ $# -gt 0 ]]; do
        case "$1" in --name) name="$2"; shift 2 ;; *) die "unknown ssh option: $1" ;; esac
    done
    load_config "${name}"
    exec ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@"${GUEST_IP}"
}

cmd_fold() {
    local kit="${DEFAULT_KIT}"
    if [[ "${1:-}" == "--kit" ]]; then
        kit="$2"
        shift 2
    fi
    [[ -x "${kit}/bin/fold" ]] || die "missing ${kit}/bin/fold"
    exec "${kit}/bin/fold" "$@"
}

main() {
    local cmd="${1:-help}"
    if [[ $# -gt 0 ]]; then shift; fi
    case "${cmd}" in
        create) cmd_create "$@" ;;
        start) cmd_start "$@" ;;
        stop) cmd_stop "$@" ;;
        list) cmd_list "$@" ;;
        status) cmd_status "$@" ;;
        logs) cmd_logs "$@" ;;
        connect) cmd_connect "$@" ;;
        ssh) cmd_ssh "$@" ;;
        fold) cmd_fold "$@" ;;
        help|--help|-h) usage ;;
        *) die "unknown command: ${cmd}" ;;
    esac
}

main "$@"
