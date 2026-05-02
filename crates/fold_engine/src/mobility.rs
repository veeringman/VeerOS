use anyhow::{bail, Result};
use serde::Serialize;

use crate::cli::{MigrationStrategyArg, ReplicationConsistencyArg};
use crate::state::FoldRecord;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStrategy {
    Live,
    Cold,
}

impl From<MigrationStrategyArg> for MigrationStrategy {
    fn from(value: MigrationStrategyArg) -> Self {
        match value {
            MigrationStrategyArg::Live => MigrationStrategy::Live,
            MigrationStrategyArg::Cold => MigrationStrategy::Cold,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MigrationPlan {
    pub fold: String,
    pub source_pid: i32,
    pub source_auras: Vec<String>,
    pub target_zone: String,
    pub target_device: Option<String>,
    pub strategy: MigrationStrategy,
    pub requires_checkpoint: bool,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationConsistency {
    Eventual,
    Strong,
}

impl From<ReplicationConsistencyArg> for ReplicationConsistency {
    fn from(value: ReplicationConsistencyArg) -> Self {
        match value {
            ReplicationConsistencyArg::Eventual => ReplicationConsistency::Eventual,
            ReplicationConsistencyArg::Strong => ReplicationConsistency::Strong,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReplicationTarget {
    pub zone: String,
    pub device: Option<String>,
    pub replica_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReplicationPlan {
    pub fold: String,
    pub source_pid: i32,
    pub source_auras: Vec<String>,
    pub consistency: ReplicationConsistency,
    pub targets: Vec<ReplicationTarget>,
    pub steps: Vec<String>,
}

pub fn build_migration_plan(
    rec: &FoldRecord,
    target_zone: &str,
    target_device: Option<&str>,
    strategy: MigrationStrategy,
) -> Result<MigrationPlan> {
    let zone = normalize_token(target_zone, "target_zone")?;
    let device = target_device
        .map(|d| normalize_token(d, "target_device"))
        .transpose()?;

    let requires_checkpoint = matches!(strategy, MigrationStrategy::Live);

    let mut steps = vec![
        format!("validate fold '{}' source state", rec.name),
        "capture current manifest and runtime metadata".to_string(),
    ];
    if requires_checkpoint {
        steps.push("create incremental memory+io checkpoint".to_string());
        steps.push("stream checkpoint to destination and resume".to_string());
    } else {
        steps.push("stop source fold and persist final state snapshot".to_string());
        steps.push("restore fold from snapshot on destination".to_string());
    }
    steps.push("verify aura memberships and health probes".to_string());

    Ok(MigrationPlan {
        fold: rec.name.clone(),
        source_pid: rec.pid,
        source_auras: rec.manifest.auras.clone(),
        target_zone: zone,
        target_device: device,
        strategy,
        requires_checkpoint,
        steps,
    })
}

pub fn build_replication_plan(
    rec: &FoldRecord,
    raw_targets: &[String],
    consistency: ReplicationConsistency,
) -> Result<ReplicationPlan> {
    if raw_targets.is_empty() {
        bail!("at least one replication target is required");
    }

    let mut targets = Vec::new();
    for raw in raw_targets {
        let (zone, device) = parse_target(raw)?;
        let replica_name = if let Some(dev) = &device {
            format!("{}-replica-{}", rec.name, sanitize_name(dev))
        } else {
            format!("{}-replica-{}", rec.name, sanitize_name(&zone))
        };
        targets.push(ReplicationTarget {
            zone,
            device,
            replica_name,
        });
    }

    let mut steps = vec![
        format!("snapshot fold '{}' baseline image", rec.name),
        "ship immutable image + config artifacts to targets".to_string(),
    ];
    match consistency {
        ReplicationConsistency::Eventual => {
            steps.push("enable async state fanout with background reconciliation".to_string())
        }
        ReplicationConsistency::Strong => {
            steps.push("enable quorum commit and synchronous state propagation".to_string())
        }
    }
    steps.push("start replicas and register placement metadata".to_string());

    Ok(ReplicationPlan {
        fold: rec.name.clone(),
        source_pid: rec.pid,
        source_auras: rec.manifest.auras.clone(),
        consistency,
        targets,
        steps,
    })
}

fn parse_target(raw: &str) -> Result<(String, Option<String>)> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("replication target must be non-empty")
    }

    if let Some((zone, device)) = raw.split_once(':') {
        let zone = normalize_token(zone, "target zone")?;
        let device = Some(normalize_token(device, "target device")?);
        return Ok((zone, device));
    }

    Ok((normalize_token(raw, "target zone")?, None))
}

fn normalize_token(input: &str, field: &str) -> Result<String> {
    let out = input.trim();
    if out.is_empty() {
        bail!("{} must be non-empty", field);
    }
    Ok(out.to_string())
}

fn sanitize_name(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use chrono::Utc;

    use crate::manifest::{Manifest, Namespaces};

    use super::*;

    fn sample_record() -> FoldRecord {
        FoldRecord {
            name: "worker-a".to_string(),
            pid: 4242,
            started_at: Utc::now(),
            manifest: Manifest {
                name: "worker-a".to_string(),
                cmd: "/bin/true".to_string(),
                args: vec![],
                auras: vec!["aur{ops,private,open}".to_string()],
                env: BTreeMap::new(),
                rootfs: None,
                hostname: None,
                workdir: None,
                namespaces: Namespaces::default(),
                limits: None,
                seccomp: None,
            },
            log_path: PathBuf::from("/tmp/worker-a.log"),
            exit_status: None,
        }
    }

    #[test]
    fn migration_plan_live_requires_checkpoint() {
        let rec = sample_record();
        let plan = build_migration_plan(
            &rec,
            "zone-east",
            Some("dev{edge,home,active}"),
            MigrationStrategy::Live,
        )
        .unwrap();
        assert!(plan.requires_checkpoint);
        assert_eq!(plan.target_zone, "zone-east");
        assert!(plan.steps.iter().any(|s| s.contains("checkpoint")));
    }

    #[test]
    fn replication_plan_parses_targets_and_names() {
        let rec = sample_record();
        let plan = build_replication_plan(
            &rec,
            &[
                "zone-east:dev{edge-a,corp,active}".to_string(),
                "zone-west".to_string(),
            ],
            ReplicationConsistency::Strong,
        )
        .unwrap();

        assert_eq!(plan.targets.len(), 2);
        assert_eq!(plan.targets[0].zone, "zone-east");
        assert_eq!(plan.targets[1].zone, "zone-west");
        assert!(plan.steps.iter().any(|s| s.contains("quorum")));
    }
}
