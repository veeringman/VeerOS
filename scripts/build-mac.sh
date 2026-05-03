#!/usr/bin/env bash
set -euo pipefail

# Build macOS host artifacts used for HVF bring-up with Hypervisor.framework support.
#
# Critical: veer-vm must be code-signed with com.apple.security.hypervisor entitlement.
# See docs/macos-hypervisor.md for details.

MANIFEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VEER_VM_CRATE="${MANIFEST_DIR}/crates/veer_vm"
cd "${MANIFEST_DIR}"

PROFILE="${VEER_VM_MAC_PROFILE:-debug}"
TARGET_DIR_RAW="${CARGO_TARGET_DIR:-${MANIFEST_DIR}/target}"
if [[ "${TARGET_DIR_RAW}" = /* ]]; then
    TARGET_DIR="${TARGET_DIR_RAW}"
else
    TARGET_DIR="${MANIFEST_DIR}/${TARGET_DIR_RAW}"
fi

ENTITLEMENTS="${VEER_VM_CRATE}/veer-vm.entitlements"
# Use the vmnet entitlements file when VEER_VM_VMNET=1 is set AND SIP is
# disabled.  With SIP enabled, com.apple.vm.networking requires a provisioning
# profile; embedding it without one causes an immediate SIGKILL.
# veer-vm-vmnet.entitlements adds com.apple.vm.networking on top of the base.
if [[ "${VEER_VM_VMNET:-0}" == "1" ]]; then
    if csrutil status 2>/dev/null | grep -q "disabled"; then
        ENTITLEMENTS="${VEER_VM_CRATE}/veer-vm-vmnet.entitlements"
    else
        echo "WARN: VEER_VM_VMNET=1 requested but SIP is enabled — using base entitlements."
        echo "      vmnet modes (shared/host/bridged) will be rejected at runtime."
        echo "      Disable SIP or provide a provisioning profile to use vmnet."
    fi
fi
DEFAULT_TARGET_DIR="${MANIFEST_DIR}/target"

native_macos_target() {
    case "$(uname -m)" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64|amd64) echo "x86_64-apple-darwin" ;;
        *)
            echo "ERROR: unsupported macOS host architecture: $(uname -m)" >&2
            return 1
            ;;
    esac
}

resolve_targets() {
    local spec="${VEER_VM_MAC_TARGET:-native}"
    local native
    native="$(native_macos_target)"

    case "${spec}" in
        native) echo "${native}" ;;
        all) echo "x86_64-apple-darwin aarch64-apple-darwin" ;;
        *) echo "${spec//,/ }" ;;
    esac
}

ensure_rust_target() {
    local target="$1"
    if command -v rustup >/dev/null 2>&1; then
        rustup target add "${target}"
    fi
}

build_pkg_for_target() {
    local pkg="$1"
    local target="$2"
    if [[ "${PROFILE}" == "release" ]]; then
        cargo build -p "${pkg}" --target "${target}" --release
    else
        cargo build -p "${pkg}" --target "${target}"
    fi
}

require_tool() {
    local tool="$1"
    if ! command -v "${tool}" >/dev/null 2>&1; then
        echo "ERROR: required tool not found: ${tool}"
        exit 1
    fi
}

verify_entitlement() {
    local bin="$1"
    local out
    local tmp_plist
    local parsed
    local normalized
    out="$(codesign --display --entitlements - "${bin}" 2>&1 || true)"
    if ! grep -q "com.apple.security.hypervisor" <<<"${out}"; then
        echo "ERROR: hypervisor entitlement missing from ${bin}"
        return 1
    fi

    # Prefer native plist parsing on macOS when available.
    if [[ -x /usr/libexec/PlistBuddy ]]; then
        tmp_plist="$(mktemp)"
        printf '%s\n' "${out}" > "${tmp_plist}"
        parsed="$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.hypervisor' "${tmp_plist}" 2>/dev/null || true)"
        rm -f "${tmp_plist}"
        if [[ "${parsed}" == "true" || "${parsed}" == "1" ]]; then
            return 0
        fi
        if [[ "${parsed}" == "false" || "${parsed}" == "0" ]]; then
            echo "ERROR: hypervisor entitlement explicitly false in ${bin}"
            return 1
        fi
    fi

    # Fallback parser: tolerate whitespace and XML formatting variants.
    normalized="$(tr -d '[:space:]' <<<"${out}")"
    if grep -q "<key>com.apple.security.hypervisor</key><true/>" <<<"${normalized}" \
        || grep -q "<key>com.apple.security.hypervisor</key><true></true>" <<<"${normalized}"; then
        return 0
    fi
    if grep -q "<key>com.apple.security.hypervisor</key><false/>" <<<"${normalized}" \
        || grep -q "<key>com.apple.security.hypervisor</key><false></false>" <<<"${normalized}"; then
        echo "ERROR: hypervisor entitlement explicitly false in ${bin}"
        return 1
    fi

    if awk '
        BEGIN { seen = 0; ok = 0 }
        /com\.apple\.security\.hypervisor/ {
            seen = 1
            if ($0 ~ /<true[[:space:]]*\/>/ || $0 ~ /<true><\/true>/) {
                ok = 1
                exit
            }
            next
        }
        seen && ($0 ~ /<true[[:space:]]*\/>/ || $0 ~ /<true><\/true>/) {
            ok = 1
            exit
        }
        seen && ($0 ~ /<false[[:space:]]*\/>/ || $0 ~ /<false><\/false>/) {
            exit
        }
        END {
            exit(ok ? 0 : 1)
        }
    ' <<<"${out}"; then
        return 0
    fi

    # Key exists but parser could not prove true/false due formatting noise.
    # Prefer liveness checks (`--probe`) over blocking on ambiguous formatting.
    echo "WARN: could not parse hypervisor entitlement value exactly for ${bin}; key is present"
    return 0
}

sign_and_verify() {
    local bin="$1"
    local entitlements="$2"

    if [[ ! -f "${bin}" ]]; then
        echo "ERROR: veer-vm binary not found: ${bin}"
        return 1
    fi
    if [[ ! -f "${entitlements}" ]]; then
        echo "ERROR: Entitlements file not found: ${entitlements}"
        return 1
    fi

    echo "▶ Signing veer-vm: ${bin}"
    # Remove any stale signature before re-signing to make result deterministic.
    codesign --remove-signature "${bin}" >/dev/null 2>&1 || true
    # Use Apple Development cert if available; fall back to ad-hoc (-).
    # NOTE: com.apple.vm.networking (vmnet.framework) additionally requires a
    # provisioning profile approved by the Apple Developer portal — ad-hoc
    # signing is sufficient only when SIP is disabled. See SIP_DISABLE_GUIDE.md.
    local SIGN_IDENTITY="-"
    if security find-identity -v -p codesigning 2>/dev/null | grep -q "Apple Development:"; then
        SIGN_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
            | grep 'Apple Development:' | head -1 | awk '{print $2}')"
    fi
    codesign --force --sign "${SIGN_IDENTITY}" --timestamp=none --entitlements "${entitlements}" "${bin}"

    echo "▶ Verifying signature and hypervisor entitlement..."
    codesign --verify --verbose=4 "${bin}"
    verify_entitlement "${bin}"
    echo "✓ veer-vm is signed and entitled"
}

sign_existing_binary_if_present() {
    local bin="$1"
    if [[ -f "${bin}" ]]; then
        sign_and_verify "${bin}" "${ENTITLEMENTS}"
    fi
}

require_binary() {
    local bin="$1"
    local label="$2"
    if [[ ! -f "${bin}" ]]; then
        echo "ERROR: expected ${label} binary not found: ${bin}"
        echo "Hint: check VEER_VM_MAC_PROFILE/VEER_VM_MAC_TARGET and CARGO_TARGET_DIR"
        exit 1
    fi
}

require_tool cargo
require_tool codesign

if [[ "${PROFILE}" != "debug" && "${PROFILE}" != "release" ]]; then
    echo "ERROR: unsupported VEER_VM_MAC_PROFILE=${PROFILE} (expected debug or release)"
    exit 1
fi

TARGET_TRIPLES=()
for target in $(resolve_targets); do
    case "${target}" in
        x86_64-apple-darwin|aarch64-apple-darwin) ;;
        *)
            echo "ERROR: unsupported macOS target: ${target}"
            echo "Supported targets: native, all, x86_64-apple-darwin, aarch64-apple-darwin"
            exit 1
            ;;
    esac
    TARGET_TRIPLES+=("${target}")
done

BUILT_TARGETS=()

for TARGET_TRIPLE in "${TARGET_TRIPLES[@]}"; do
    VEER_VM_BIN="${TARGET_DIR}/${TARGET_TRIPLE}/${PROFILE}/veer-vm"
    FOLD_BIN="${TARGET_DIR}/${TARGET_TRIPLE}/${PROFILE}/fold"
    VEER_CONNECT_BIN="${TARGET_DIR}/${TARGET_TRIPLE}/${PROFILE}/veer-connect"

    echo "▶ Building macOS host tools for ${TARGET_TRIPLE} (${PROFILE})"
    ensure_rust_target "${TARGET_TRIPLE}"

    # Build veer-vm for macOS
    build_pkg_for_target veer_vm "${TARGET_TRIPLE}"
    sign_and_verify "${VEER_VM_BIN}" "${ENTITLEMENTS}"

    # Also sign common default-output paths if they exist, so verification against
    # ./target/... does not accidentally hit a stale unsigned artifact.
    for candidate in \
        "${DEFAULT_TARGET_DIR}/${TARGET_TRIPLE}/debug/veer-vm" \
        "${DEFAULT_TARGET_DIR}/${TARGET_TRIPLE}/release/veer-vm"; do
        if [[ "${candidate}" != "${VEER_VM_BIN}" ]]; then
            sign_existing_binary_if_present "${candidate}"
        fi
    done

    # Build fold_engine for macOS
    build_pkg_for_target fold_engine "${TARGET_TRIPLE}"
    require_binary "${FOLD_BIN}" "fold"

    # Build veer-connect for macOS
    build_pkg_for_target veer-connect "${TARGET_TRIPLE}"
    require_binary "${VEER_CONNECT_BIN}" "veer-connect"

    # Final check: ensure veer-vm stayed signed after the complete build pipeline.
    if ! codesign --verify --verbose=4 "${VEER_VM_BIN}" >/dev/null 2>&1 || ! verify_entitlement "${VEER_VM_BIN}"; then
        echo "⚠ veer-vm signature changed after workspace build; re-signing..."
        sign_and_verify "${VEER_VM_BIN}" "${ENTITLEMENTS}"
    fi

    # Re-check default output paths too, in case a later build step touched them.
    for candidate in \
        "${DEFAULT_TARGET_DIR}/${TARGET_TRIPLE}/debug/veer-vm" \
        "${DEFAULT_TARGET_DIR}/${TARGET_TRIPLE}/release/veer-vm"; do
        if [[ "${candidate}" != "${VEER_VM_BIN}" && -f "${candidate}" ]]; then
            if ! codesign --verify --verbose=4 "${candidate}" >/dev/null 2>&1 || ! verify_entitlement "${candidate}"; then
                echo "⚠ ${candidate} is unsigned or missing entitlement; re-signing..."
                sign_and_verify "${candidate}" "${ENTITLEMENTS}"
            fi
        fi
    done

    BUILT_TARGETS+=("${TARGET_TRIPLE}|${VEER_VM_BIN}|${FOLD_BIN}|${VEER_CONNECT_BIN}")
done

echo "✓ macOS build complete"
echo "  profile     : ${PROFILE}"
for entry in "${BUILT_TARGETS[@]}"; do
    IFS='|' read -r target veer_vm_bin fold_bin veer_connect_bin <<<"${entry}"
    echo "  target      : ${target}"
    echo "  veer-vm bin : ${veer_vm_bin}"
    echo "  fold bin    : ${fold_bin}"
    echo "  veer-connect: ${veer_connect_bin}"
    echo "  verify      : codesign --verify --verbose=4 ${veer_vm_bin}"
done
