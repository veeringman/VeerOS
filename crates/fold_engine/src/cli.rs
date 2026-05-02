//! `fold` CLI definition (clap derive).

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "fold",
    version,
    about = "VeerOS Fold — secure compute envelope",
    long_about = "Manage VeerOS Folds: lightweight, secure, namespace-isolated \
                  compute envelopes. Not a container, not a VM."
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
    Stop { name: String },

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

    /// Mobility planning for fold migration and replication.
    Mobility {
        #[command(subcommand)]
        cmd: MobilityCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum VmCmd {
    /// Spawn a VeerOS microVM inside a fresh Fold (namespaces + seccomp +
    /// cgroups) and return immediately. Stream output with `fold logs <name>`.
    Spawn {
        /// Path to the guest boot image.
        ///
        /// Accepts either a Multiboot ELF or a VeerOS ISO containing
        /// `/boot/kernel.elf`.
        #[arg(long)]
        kernel: PathBuf,

        /// Guest memory, in MiB.
        #[arg(long, default_value_t = 128)]
        memory: usize,

        /// Disk image passed through to `veer-vm --disk`.
        #[arg(long)]
        disk: Option<PathBuf>,

        /// Guest architecture passed through to `veer-vm --arch`.
        #[arg(long, value_enum, default_value_t = VmArchArg::X8664)]
        arch: VmArchArg,

        /// Host TAP interface passed through to `veer-vm --tap`.
        #[arg(long)]
        tap: Option<String>,

        /// macOS vmnet mode passed through to `veer-vm --vmnet`.
        #[arg(long, value_parser = ["shared", "host"])]
        vmnet: Option<String>,

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum VmArchArg {
    #[value(name = "x86_64")]
    X8664,
    #[value(name = "aarch64")]
    Aarch64,
    #[value(name = "riscv32")]
    Riscv32,
}

impl VmArchArg {
    pub fn as_cli_value(self) -> &'static str {
        match self {
            VmArchArg::X8664 => "x86_64",
            VmArchArg::Aarch64 => "aarch64",
            VmArchArg::Riscv32 => "riscv32",
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum MobilityCmd {
    /// Build a migration plan for moving a fold to another zone/device.
    MigratePlan {
        /// Fold name from local state registry.
        name: String,
        /// Destination zone (VAS atom style, free-form string).
        #[arg(long)]
        target_zone: String,
        /// Optional destination device identity.
        #[arg(long)]
        target_device: Option<String>,
        /// Migration strategy.
        #[arg(long, value_enum, default_value_t = MigrationStrategyArg::Live)]
        strategy: MigrationStrategyArg,
        /// Emit JSON plan.
        #[arg(long)]
        json: bool,
    },

    /// Build a replication plan to fan out a fold into additional zones/devices.
    ReplicatePlan {
        /// Fold name from local state registry.
        name: String,
        /// Replication targets as `<zone>:<device>` or `<zone>`.
        #[arg(long = "target", required = true)]
        targets: Vec<String>,
        /// Replication consistency model.
        #[arg(long, value_enum, default_value_t = ReplicationConsistencyArg::Eventual)]
        consistency: ReplicationConsistencyArg,
        /// Emit JSON plan.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum MigrationStrategyArg {
    Live,
    Cold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ReplicationConsistencyArg {
    Eventual,
    Strong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_spawn_parses_tap_flag() {
        let cli = Cli::try_parse_from([
            "fold",
            "vm",
            "spawn",
            "--kernel",
            "/tmp/kernel.elf",
            "--arch",
            "riscv32",
            "--memory",
            "256",
            "--tap",
            "tap0",
        ])
        .unwrap();

        match cli.cmd {
            Cmd::Vm {
                cmd: VmCmd::Spawn {
                    tap, arch, memory, ..
                },
            } => {
                assert_eq!(tap.as_deref(), Some("tap0"));
                assert_eq!(arch, VmArchArg::Riscv32);
                assert_eq!(memory, 256);
            }
            _ => panic!("parsed wrong command variant"),
        }
    }

    #[test]
    fn vm_spawn_defaults_tap_to_none() {
        let cli =
            Cli::try_parse_from(["fold", "vm", "spawn", "--kernel", "/tmp/kernel.elf"]).unwrap();

        match cli.cmd {
            Cmd::Vm {
                cmd: VmCmd::Spawn { tap, .. },
            } => {
                assert!(tap.is_none());
            }
            _ => panic!("parsed wrong command variant"),
        }
    }

    #[test]
    fn mobility_migrate_plan_parses() {
        let cli = Cli::try_parse_from([
            "fold",
            "mobility",
            "migrate-plan",
            "worker-a",
            "--target-zone",
            "zone-east",
            "--target-device",
            "dev{host,edge,active}",
            "--strategy",
            "cold",
        ])
        .unwrap();

        match cli.cmd {
            Cmd::Mobility {
                cmd:
                    MobilityCmd::MigratePlan {
                        name,
                        target_zone,
                        target_device,
                        strategy,
                        ..
                    },
            } => {
                assert_eq!(name, "worker-a");
                assert_eq!(target_zone, "zone-east");
                assert_eq!(target_device.as_deref(), Some("dev{host,edge,active}"));
                assert_eq!(strategy, MigrationStrategyArg::Cold);
            }
            _ => panic!("parsed wrong command variant"),
        }
    }

    #[test]
    fn mobility_replicate_plan_parses_multi_targets() {
        let cli = Cli::try_parse_from([
            "fold",
            "mobility",
            "replicate-plan",
            "worker-a",
            "--target",
            "zone-east:dev{edge-a,edge,active}",
            "--target",
            "zone-west",
            "--consistency",
            "strong",
        ])
        .unwrap();

        match cli.cmd {
            Cmd::Mobility {
                cmd:
                    MobilityCmd::ReplicatePlan {
                        name,
                        targets,
                        consistency,
                        ..
                    },
            } => {
                assert_eq!(name, "worker-a");
                assert_eq!(targets.len(), 2);
                assert_eq!(consistency, ReplicationConsistencyArg::Strong);
            }
            _ => panic!("parsed wrong command variant"),
        }
    }
}
