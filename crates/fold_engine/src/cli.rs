//! `fold` CLI definition (clap derive).

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "fold",
    version,
    about = "VeerOS Fold — secure compute envelope",
    long_about = "Manage VeerOS Folds: lightweight, secure, namespace-isolated \
                  compute envelopes. Not a container, not a VM.",
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Launch a new fold from a TOML manifest.
    Spawn {
        /// Path to a fold manifest (TOML).
        #[arg(short, long)]
        manifest: PathBuf,
    },

    /// List all known folds (running and stopped).
    List,

    /// Print the captured stdout+stderr log for a fold.
    Logs {
        name: String,
        /// Follow the log as it grows (tail -f).
        #[arg(short, long)]
        follow: bool,
    },

    /// Send SIGTERM to a fold's init process.
    Stop {
        name: String,
    },

    /// Delete a fold's state record and log. Fails if the fold is still alive.
    Rm {
        name: String,
        /// Force removal even if the fold is still running (SIGKILL first).
        #[arg(short, long)]
        force: bool,
    },

    /// VeerOS microVM operations (runs `veer-vm` inside a Fold envelope).
    Vm {
        #[command(subcommand)]
        cmd: VmCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum VmCmd {
    /// Spawn a VeerOS microVM inside a fresh Fold (namespaces + seccomp +
    /// cgroups) and return immediately. Stream output with `fold logs <name>`.
    Spawn {
        /// Path to the guest kernel ELF (e.g. the qemu_pc build output).
        #[arg(long)]
        kernel: PathBuf,

        /// Guest memory, in MiB.
        #[arg(long, default_value_t = 128)]
        memory: usize,

        /// Fold name. Defaults to `veeros-vm-<6-hex>`.
        #[arg(long, short)]
        name: Option<String>,

        /// Path to the `veer-vm` binary. Defaults to workspace
        /// `target/{release,debug}/veer-vm` or `$PATH`.
        #[arg(long)]
        vmm: Option<PathBuf>,

        /// Enable user namespace (rootless mode — fold spawns without
        /// needing root or CAP_SYS_ADMIN on kernels that allow
        /// unprivileged_userns_clone).
        #[arg(long)]
        user_ns: bool,

        /// cgroup `memory.max` in bytes. If unset, cgroup memory is not
        /// capped (rootless-friendly: avoids cgroup v2 "no internal
        /// processes" conflicts when the memory controller isn't
        /// delegated in `cgroup.subtree_control`).
        #[arg(long)]
        memory_cap: Option<u64>,

        /// cgroup `pids.max`. If unset, no pid cap. When any cgroup
        /// limit is set, the caller must be inside a delegated cgroup
        /// whose `cgroup.subtree_control` advertises the relevant
        /// controllers (typical helper: `systemd-run --user --scope
        /// --property=Delegate=yes`).
        #[arg(long)]
        pids_max: Option<u64>,
    },
}
