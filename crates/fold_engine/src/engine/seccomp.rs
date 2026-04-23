//! Minimal seccomp BPF filter installer.
//!
//! Default-allow + denylist model. Callers pass a list of syscall numbers to
//! deny; the installed BPF program returns `EPERM` for any match and allows
//! everything else. We also set `PR_SET_NO_NEW_PRIVS` so the kernel accepts
//! the filter without `CAP_SYS_ADMIN`.
//!
//! This is intentionally small — a single filter, one arch check at the top,
//! linear jump-eq chain. It is *not* a sandbox on its own; it is meant to
//! stop the most destructive syscalls from leaking out of a fold in case
//! namespaces fail to contain them.

use anyhow::{bail, Context, Result};
use std::mem::size_of;

// ─── libc / kernel constants not exposed by the `libc` crate ────────────────

const PR_SET_NO_NEW_PRIVS: libc::c_int = 38;
const PR_SET_SECCOMP: libc::c_int = 22;
const SECCOMP_MODE_FILTER: libc::c_ulong = 2;

const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

// BPF instruction classes / fields (linux/bpf_common.h).
const BPF_LD: u16 = 0x00;
const BPF_JMP: u16 = 0x05;
const BPF_RET: u16 = 0x06;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;

// Audit arch constants for the filter's arch gate.
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xc000_003e;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xc000_00b7;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("fold_engine seccomp: unsupported host architecture");

// `seccomp_data` layout: { int nr; __u32 arch; __u64 instruction_pointer; __u64 args[6]; }
const OFFSET_NR: u32 = 0;
const OFFSET_ARCH: u32 = 4;

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

fn stmt(code: u16, k: u32) -> SockFilter { SockFilter { code, jt: 0, jf: 0, k } }
fn jump(code: u16, k: u32, jt: u8, jf: u8) -> SockFilter { SockFilter { code, jt, jf, k } }

/// Install a BPF filter that denies each syscall in `deny_nrs` with `EPERM`
/// and allows everything else. Also enables `NO_NEW_PRIVS`.
///
/// Must be called after privileges are set and before `execve`. Once set,
/// the filter is inherited across `execve` and cannot be removed.
pub fn install_denylist(deny_nrs: &[u32]) -> Result<()> {
    unsafe {
        if libc::prctl(PR_SET_NO_NEW_PRIVS, 1u64, 0u64, 0u64, 0u64) != 0 {
            let e = std::io::Error::last_os_error();
            bail!("prctl(PR_SET_NO_NEW_PRIVS): {e}");
        }
    }

    let mut prog: Vec<SockFilter> = Vec::with_capacity(4 + deny_nrs.len() * 2);

    // 1. Load arch; if it doesn't match build arch, kill the process. This
    //    stops x32/ia32 compat gadgetry on x86_64 and similar tricks.
    prog.push(stmt(BPF_LD | BPF_W | BPF_ABS, OFFSET_ARCH));
    prog.push(jump(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH, 1, 0));
    prog.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));

    // 2. Load syscall nr.
    prog.push(stmt(BPF_LD | BPF_W | BPF_ABS, OFFSET_NR));

    // 3. For each deny: if nr == deny_nr, return EPERM; else fall through.
    for &nr in deny_nrs {
        // if equal → jump over 1 insn → return EPERM
        // else     → skip the RET, continue
        prog.push(jump(BPF_JMP | BPF_JEQ | BPF_K, nr, 0, 1));
        prog.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xffff)));
    }

    // 4. Default: allow.
    prog.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));

    if prog.len() > u16::MAX as usize {
        bail!("seccomp filter too long ({} > {})", prog.len(), u16::MAX);
    }
    let fprog = SockFprog {
        len: prog.len() as u16,
        filter: prog.as_ptr(),
    };

    unsafe {
        if libc::prctl(
            PR_SET_SECCOMP,
            SECCOMP_MODE_FILTER,
            &fprog as *const _ as libc::c_ulong,
            0u64,
            0u64,
        ) != 0
        {
            let e = std::io::Error::last_os_error();
            bail!("prctl(PR_SET_SECCOMP, MODE_FILTER): {e} (filter insns={})", prog.len());
        }
    }

    // Keep `prog` alive until after the prctl call.
    drop(prog);
    let _ = size_of::<SockFilter>(); // ensure layout module stays referenced
    Ok(())
}

// ─── Default denylist ───────────────────────────────────────────────────────

/// The "default" seccomp profile — a curated list of syscalls a fold should
/// almost never need. These mostly protect the host kernel from attacks that
/// rely on namespace/keyring/module loading primitives.
pub fn default_deny_names() -> &'static [&'static str] {
    &[
        // Keyring — destructive cross-namespace side channel.
        "add_key", "request_key", "keyctl",
        // Kernel module loading.
        "init_module", "finit_module", "delete_module",
        // Kernel tunables / debugging.
        "bpf",
        "kexec_load", "kexec_file_load",
        "reboot",
        // Creating more namespaces from inside a fold would let workloads
        // build their own sandboxes and confuse host accounting.
        "unshare", "setns",
        // Filesystem surgery.
        "mount", "umount", "umount2", "pivot_root",
        // Swap control.
        "swapon", "swapoff",
        // Clock fiddling.
        "settimeofday", "clock_settime", "clock_adjtime", "adjtimex",
        // Kernel tracing / perf.
        "perf_event_open",
        // Quota.
        "quotactl",
        // Obsolete / dangerous.
        "ptrace",
        "personality",
        "acct",
    ]
}

/// Resolve a list of syscall names into syscall numbers for the current host
/// architecture. Unknown names are silently skipped (they might simply not
/// exist on this arch / kernel).
pub fn resolve_names(names: &[&str]) -> Vec<u32> {
    names.iter().filter_map(|n| syscall_nr(n)).collect()
}

/// Ad-hoc syscall name → number table. We maintain a tiny local map so we
/// don't need `libseccomp` just to parse names. This is intentionally
/// limited to the default profile plus a handful of commonly-denied syscalls.
pub fn syscall_nr(name: &str) -> Option<u32> {
    #[cfg(target_arch = "x86_64")]
    #[rustfmt::skip]
    let table: &[(&str, u32)] = &[
        ("add_key", 248), ("request_key", 249), ("keyctl", 250),
        ("init_module", 175), ("finit_module", 313), ("delete_module", 176),
        ("bpf", 321),
        ("kexec_load", 246), ("kexec_file_load", 320),
        ("reboot", 169),
        ("unshare", 272), ("setns", 308),
        ("mount", 165), ("umount", 22 /* no umount on x86_64 */), ("umount2", 166),
        ("pivot_root", 155),
        ("swapon", 167), ("swapoff", 168),
        ("settimeofday", 164), ("clock_settime", 227), ("clock_adjtime", 305),
        ("adjtimex", 159),
        ("perf_event_open", 298),
        ("quotactl", 179),
        ("ptrace", 101),
        ("personality", 135),
        ("acct", 163),
    ];
    #[cfg(target_arch = "aarch64")]
    #[rustfmt::skip]
    let table: &[(&str, u32)] = &[
        ("add_key", 217), ("request_key", 218), ("keyctl", 219),
        ("init_module", 105), ("finit_module", 273), ("delete_module", 106),
        ("bpf", 280),
        ("kexec_load", 104), ("kexec_file_load", 294),
        ("reboot", 142),
        ("unshare", 97), ("setns", 268),
        ("mount", 40), ("umount2", 39),
        ("pivot_root", 41),
        ("swapon", 224), ("swapoff", 225),
        ("settimeofday", 170), ("clock_settime", 112), ("clock_adjtime", 266),
        ("adjtimex", 171),
        ("perf_event_open", 241),
        ("quotactl", 60),
        ("ptrace", 117),
        ("personality", 92),
        ("acct", 89),
    ];
    for &(n, nr) in table {
        if n == name { return Some(nr); }
    }
    None
}

/// Convenience: resolve + install a default-deny profile, extended with any
/// caller-supplied names.
pub fn install_profile(profile: &str, extra_deny: &[String]) -> Result<()> {
    if profile == "none" {
        return Ok(());
    }
    if profile != "default" {
        bail!("unknown seccomp profile: {profile:?} (known: default, none)");
    }
    let mut names: Vec<&str> = default_deny_names().to_vec();
    for e in extra_deny {
        names.push(e.as_str());
    }
    let nrs = resolve_names(&names);
    install_denylist(&nrs).context("installing seccomp filter")?;
    Ok(())
}
