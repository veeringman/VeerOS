//! Linux fold engine — namespace-based isolation.
//!
//! Pipeline:
//!   parent `fold spawn`
//!     └── fork → launcher
//!           (optional) unshare(USER) + write uid_map/gid_map  [rootless mode]
//!           unshare(PID|NS|UTS|IPC|NET)
//!           └── fork → init  (PID 1 inside the new PID namespace)
//!                 sethostname → pivot_root → mount /proc → chdir → [seccomp] → execve
//!           launcher writes init PID
//!           parent reads the report pipe: OK tag → success, ERR tag → bail.
//!
//! Error pipe:
//!   A single pipe, `O_CLOEXEC` on both ends so that init's successful
//!   `execve` produces EOF on the parent. Messages are framed as one byte
//!   tag + payload:
//!     0x00 : 4-byte LE i32 init-pid
//!     0x01 : 4-byte LE u32 length + UTF-8 error message (from launcher or
//!            init; init messages follow the OK tag if launcher succeeded)

use anyhow::{bail, Context, Result};
use chrono::Utc;
use nix::fcntl::OFlag;
use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{
    chdir, dup2, execve, fork, getegid, geteuid, pipe2, pivot_root, read, sethostname, setsid,
    write, ForkResult, Gid, Pid, Uid,
};
use std::ffi::CString;
use std::io::Write as _;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::{cgroup, seccomp, Engine};
use crate::manifest::Manifest;
use crate::state::{FoldRecord, StateDir};

pub struct LinuxEngine;

impl LinuxEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Engine for LinuxEngine {
    fn spawn(&self, manifest: Manifest) -> Result<FoldRecord> {
        let state = StateDir::open()?;
        anyhow::ensure!(
            !state.record_path(&manifest.name).exists(),
            "fold {:?} already exists; use `fold rm` first",
            manifest.name
        );

        let log_path = state.log_path(&manifest.name);
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&log_path)
            .with_context(|| format!("opening log {}", log_path.display()))?;
        let log_fd: OwnedFd = log_file.into();

        // Capture parent's (outer) uid/gid so the launcher can write uid_map
        // after entering the user namespace.
        let outer_uid = geteuid();
        let outer_gid = getegid();

        // Report pipe — `O_CLOEXEC` so `execve` in init closes the child end.
        let (rd, wr) = pipe2(OFlag::O_CLOEXEC).context("pipe2")?;

        // SAFETY: single-threaded CLI; fork() here is safe.
        match unsafe { fork() }.context("fork launcher")? {
            ForkResult::Parent { child: launcher } => {
                drop(wr);
                drop(log_fd);
                let (init_pid_opt, err) = read_report(&rd)?;
                drop(rd);
                if let Some(msg) = err {
                    let _ = waitpid(launcher, None);
                    bail!("fold spawn failed: {msg}");
                }
                let init_pid =
                    init_pid_opt.context("launcher exited without reporting init pid")?;

                // Attach to cgroup from the parent (we still have write perms
                // on the parent cgroup dir). Done *after* init exists so the
                // pid is writable into cgroup.procs. If anything fails we
                // kill the orphaned init so we don't leak it.
                if let Some(limits) = manifest.limits.clone() {
                    match cgroup::create(&manifest.name, &limits).and_then(|cg| {
                        cgroup::attach_pid(&cg, init_pid)?;
                        // Leak: cgroup persists until `fold rm`.
                        std::mem::forget(cg);
                        Ok(())
                    }) {
                        Ok(()) => {}
                        Err(e) => {
                            let _ = kill(Pid::from_raw(init_pid), Signal::SIGKILL);
                            let _ = waitpid(launcher, None);
                            // Also clean the half-created cgroup dir, if any.
                            let _ = cgroup::remove(&manifest.name, &limits);
                            bail!(
                                "fold spawn failed during cgroup setup: {e:#}\n\
                                 Hint: rootless users need a delegated cgroup — try \
                                 wrapping: `systemd-run --user --scope --property=Delegate=yes \
                                 -- fold spawn …`, or omit the [limits] section."
                            );
                        }
                    }
                }

                let rec = FoldRecord {
                    name: manifest.name.clone(),
                    pid: init_pid,
                    started_at: Utc::now(),
                    manifest,
                    log_path,
                    exit_status: None,
                };
                state.save(&rec)?;
                Ok(rec)
            }
            ForkResult::Child => {
                drop(rd);
                let rc = launcher_main(manifest, log_fd, wr, outer_uid, outer_gid);
                if let Err(e) = rc {
                    eprintln!("fold launcher: {e:#}");
                }
                std::process::exit(127);
            }
        }
    }

    fn stop(&self, rec: &FoldRecord) -> Result<()> {
        match kill(Pid::from_raw(rec.pid), Signal::SIGTERM) {
            Ok(()) => Ok(()),
            Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(e) => bail!("kill({}): {e}", rec.pid),
        }
    }

    fn is_alive(&self, rec: &FoldRecord) -> bool {
        kill(Pid::from_raw(rec.pid), None).is_ok()
    }

    fn cleanup(&self, rec: &FoldRecord) -> Result<()> {
        if let Some(limits) = &rec.manifest.limits {
            // Best-effort: failure to rmdir (e.g. non-empty) is not fatal.
            let _ = cgroup::remove(&rec.name, limits);
        }
        Ok(())
    }
}

// ─── launcher (child of `fold spawn`) ────────────────────────────────────────

fn launcher_main(
    manifest: Manifest,
    log_fd: OwnedFd,
    pipe_wr: OwnedFd,
    outer_uid: Uid,
    outer_gid: Gid,
) -> Result<()> {
    let report = ReportWriter::new(pipe_wr);
    if let Err(e) = launcher_body(manifest, log_fd, &report, outer_uid, outer_gid) {
        report.fail(format!("launcher: {e:#}"));
        std::process::exit(127);
    }
    Ok(())
}

fn launcher_body(
    manifest: Manifest,
    log_fd: OwnedFd,
    report: &ReportWriter,
    outer_uid: Uid,
    outer_gid: Gid,
) -> Result<()> {
    let _ = setsid();

    // Open /dev/null before stdio redirect so we catch the host's, not the
    // fold's post-pivot /dev/null.
    let devnull = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(OFlag::O_CLOEXEC.bits())
        .open("/dev/null")
        .context("open /dev/null")?;
    let devnull_fd: OwnedFd = devnull.into();
    dup2(devnull_fd.as_raw_fd(), 0).context("dup2 stdin")?;
    dup2(log_fd.as_raw_fd(), 1).context("dup2 stdout")?;
    dup2(log_fd.as_raw_fd(), 2).context("dup2 stderr")?;
    drop(devnull_fd);
    drop(log_fd);

    // ── User namespace (rootless path) ─────────────────────────────────────
    let ns = &manifest.namespaces;
    if ns.user {
        unshare(CloneFlags::CLONE_NEWUSER).context(
            "unshare(CLONE_NEWUSER) — kernel may have user-ns disabled \
             (sysctl kernel.unprivileged_userns_clone=1)",
        )?;
        write_userns_maps(outer_uid, outer_gid)
            .context("writing uid_map/gid_map for user namespace")?;
    }

    // ── Remaining namespaces ──────────────────────────────────────────────
    let mut flags = CloneFlags::empty();
    if ns.pid {
        flags |= CloneFlags::CLONE_NEWPID;
    }
    if ns.mount {
        flags |= CloneFlags::CLONE_NEWNS;
    }
    if ns.uts {
        flags |= CloneFlags::CLONE_NEWUTS;
    }
    if ns.ipc {
        flags |= CloneFlags::CLONE_NEWIPC;
    }
    if ns.net {
        flags |= CloneFlags::CLONE_NEWNET;
    }

    if !flags.is_empty() {
        unshare(flags).with_context(|| {
            format!(
                "unshare({flags:?}) — non-root? enable `user = true` in manifest \
             or run as root / with CAP_SYS_ADMIN"
            )
        })?;
    }

    // Fork so the child becomes PID 1 inside the new PID namespace.
    match unsafe { fork() }.context("fork init")? {
        ForkResult::Parent { child: init } => {
            report.ok(init.as_raw());
            // Close our writer so that parent can detect init's exec via EOF
            // (init still holds its own copy of the fd).
            report.close();
            let code = match waitpid(init, None) {
                Ok(WaitStatus::Exited(_, c)) => c,
                Ok(WaitStatus::Signaled(_, sig, _)) => 128 + (sig as i32),
                Ok(_) => 0,
                Err(_) => 127,
            };
            std::process::exit(code);
        }
        ForkResult::Child => {
            if let Err(e) = init_main(manifest) {
                report.fail(format!("init: {e:#}"));
                std::process::exit(127);
            }
            unreachable!()
        }
    }
}

// ─── init (PID 1 inside new namespaces) ──────────────────────────────────────

fn init_main(manifest: Manifest) -> Result<()> {
    if manifest.namespaces.uts {
        if let Some(h) = &manifest.hostname {
            sethostname(h.as_str()).with_context(|| format!("sethostname {h}"))?;
        }
    }

    if manifest.namespaces.mount {
        mount::<str, str, str, str>(None, "/", None, MsFlags::MS_REC | MsFlags::MS_PRIVATE, None)
            .context("mount / rec-private")?;

        if let Some(rootfs) = manifest.rootfs.as_deref() {
            pivot_into_rootfs(rootfs)?;
        }

        if manifest.namespaces.pid {
            let _ = std::fs::create_dir_all("/proc");
            mount::<str, str, str, str>(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
                None,
            )
            .context("mount /proc")?;
        }
    }

    let workdir = manifest.workdir.as_deref().unwrap_or(Path::new("/"));
    chdir(workdir).with_context(|| format!("chdir {}", workdir.display()))?;

    // Install seccomp *after* all privileged setup (mount, pivot_root, …)
    // has finished, otherwise the filter would reject our own setup syscalls.
    if let Some(sc) = &manifest.seccomp {
        seccomp::install_profile(&sc.profile, &sc.deny).context("installing seccomp filter")?;
    }

    let cmd_c = CString::new(manifest.cmd.as_bytes()).context("cmd has NUL")?;
    let mut argv: Vec<CString> = Vec::with_capacity(manifest.args.len() + 1);
    argv.push(cmd_c.clone());
    for a in &manifest.args {
        argv.push(CString::new(a.as_bytes()).context("arg has NUL")?);
    }

    let envp: Vec<CString> = if manifest.env.is_empty() {
        vec![
            CString::new("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
                .unwrap(),
            CString::new(format!(
                "HOSTNAME={}",
                manifest.hostname.as_deref().unwrap_or("fold")
            ))
            .unwrap(),
        ]
    } else {
        manifest
            .env
            .iter()
            .map(|(k, v)| CString::new(format!("{k}={v}")).expect("env NUL"))
            .collect()
    };

    execve(&cmd_c, &argv, &envp).with_context(|| format!("execve {}", manifest.cmd))?;
    unreachable!()
}

// ─── user namespace uid/gid map ─────────────────────────────────────────────

fn write_userns_maps(outer_uid: Uid, outer_gid: Gid) -> Result<()> {
    // setgroups must be "deny" before gid_map can be written for an
    // unprivileged single-mapping user namespace.
    std::fs::write("/proc/self/setgroups", "deny\n").context("write /proc/self/setgroups=deny")?;
    std::fs::write(
        "/proc/self/uid_map",
        format!("0 {} 1\n", outer_uid.as_raw()),
    )
    .context("write /proc/self/uid_map")?;
    std::fs::write(
        "/proc/self/gid_map",
        format!("0 {} 1\n", outer_gid.as_raw()),
    )
    .context("write /proc/self/gid_map")?;
    Ok(())
}

// ─── rootfs pivot ───────────────────────────────────────────────────────────

fn pivot_into_rootfs(rootfs: &Path) -> Result<()> {
    mount::<Path, Path, str, str>(
        Some(rootfs),
        rootfs,
        None,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None,
    )
    .with_context(|| format!("bind-mount rootfs {}", rootfs.display()))?;

    let put_old = rootfs.join(".oldroot");
    std::fs::create_dir_all(&put_old).with_context(|| format!("mkdir {}", put_old.display()))?;

    pivot_root(rootfs, put_old.as_path())
        .with_context(|| format!("pivot_root {}", rootfs.display()))?;
    chdir("/").context("chdir / after pivot_root")?;

    nix::mount::umount2("/.oldroot", nix::mount::MntFlags::MNT_DETACH)
        .context("umount2 /.oldroot")?;
    let _ = std::fs::remove_dir("/.oldroot");
    Ok(())
}

// ─── report pipe framing ───────────────────────────────────────────────────

const TAG_OK: u8 = 0x00;
const TAG_ERR: u8 = 0x01;

struct ReportWriter {
    fd: std::cell::RefCell<Option<OwnedFd>>,
}

impl ReportWriter {
    fn new(fd: OwnedFd) -> Self {
        Self {
            fd: std::cell::RefCell::new(Some(fd)),
        }
    }

    fn ok(&self, init_pid: i32) {
        let borrow = self.fd.borrow();
        if let Some(fd) = borrow.as_ref() {
            let mut buf = [0u8; 5];
            buf[0] = TAG_OK;
            buf[1..].copy_from_slice(&init_pid.to_le_bytes());
            let _ = write(fd, &buf);
        }
    }

    fn fail(&self, msg: impl Into<String>) {
        let msg = msg.into();
        let bytes = msg.as_bytes();
        let borrow = self.fd.borrow();
        if let Some(fd) = borrow.as_ref() {
            let len = (bytes.len() as u32).to_le_bytes();
            let mut hdr = [0u8; 5];
            hdr[0] = TAG_ERR;
            hdr[1..].copy_from_slice(&len);
            let _ = write(fd, &hdr);
            let _ = write(fd, bytes);
        } else {
            let _ = std::io::stderr().write_all(msg.as_bytes());
        }
    }

    fn close(&self) {
        self.fd.borrow_mut().take();
    }
}

fn read_report(rd: &OwnedFd) -> Result<(Option<i32>, Option<String>)> {
    let mut init_pid: Option<i32> = None;
    let mut error: Option<String> = None;

    loop {
        let mut tag = [0u8; 1];
        match read_exact(rd, &mut tag)? {
            ReadExact::Eof => break,
            ReadExact::Short => bail!("short read on report pipe"),
            ReadExact::Ok => {}
        }
        match tag[0] {
            TAG_OK => {
                let mut buf = [0u8; 4];
                if !matches!(read_exact(rd, &mut buf)?, ReadExact::Ok) {
                    bail!("truncated OK frame on report pipe");
                }
                init_pid = Some(i32::from_le_bytes(buf));
            }
            TAG_ERR => {
                let mut len_buf = [0u8; 4];
                if !matches!(read_exact(rd, &mut len_buf)?, ReadExact::Ok) {
                    bail!("truncated ERR length on report pipe");
                }
                let len = u32::from_le_bytes(len_buf) as usize;
                let mut msg = vec![0u8; len];
                if len > 0 && !matches!(read_exact(rd, &mut msg)?, ReadExact::Ok) {
                    bail!("truncated ERR payload on report pipe");
                }
                let s = String::from_utf8_lossy(&msg).into_owned();
                // Keep the first error — later frames are usually noise.
                error.get_or_insert(s);
            }
            other => bail!("unknown report tag {:#x}", other),
        }
    }
    Ok((init_pid, error))
}

enum ReadExact {
    Ok,
    Short,
    Eof,
}

fn read_exact(fd: &OwnedFd, buf: &mut [u8]) -> Result<ReadExact> {
    let mut got = 0;
    while got < buf.len() {
        let n = read(fd.as_raw_fd(), &mut buf[got..])?;
        if n == 0 {
            return Ok(if got == 0 {
                ReadExact::Eof
            } else {
                ReadExact::Short
            });
        }
        got += n;
    }
    Ok(ReadExact::Ok)
}
