//! VeerOS Aura Governor v2.
//!
//! Capabilities:
//! - Static policy load from TOML
//! - Dynamic policy updates (delta patches)
//! - Inherited policy overlays (admins/member-types/removal gate)
//! - Explainable decision traces
//! - Append-only JSONL audit records

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};
use veer_aura::Aura;

#[derive(Debug, Clone, Deserialize)]
pub struct GovernorPolicy {
    pub governor: String,
    pub managed_aura: String,
    #[serde(default = "default_policy_version")]
    pub policy_version: u64,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub inherited_admins: Vec<String>,
    #[serde(default = "default_member_types")]
    pub allow_member_types: Vec<String>,
    #[serde(default)]
    pub inherited_allow_member_types: Vec<String>,
    #[serde(default = "default_allow_removal")]
    pub allow_removal: bool,
    #[serde(default)]
    pub inherited_allow_removal: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GovernorPolicyDelta {
    #[serde(default)]
    pub add_admins: Vec<String>,
    #[serde(default)]
    pub remove_admins: Vec<String>,
    #[serde(default)]
    pub add_member_types: Vec<String>,
    #[serde(default)]
    pub remove_member_types: Vec<String>,
    pub allow_removal: Option<bool>,
    #[serde(default)]
    pub inherited_admins: Option<Vec<String>>,
    #[serde(default)]
    pub inherited_allow_member_types: Option<Vec<String>>,
    pub inherited_allow_removal: Option<bool>,
}

fn default_policy_version() -> u64 {
    1
}

fn default_member_types() -> Vec<String> {
    vec![
        "usr".to_string(),
        "dev".to_string(),
        "fld".to_string(),
        "agt".to_string(),
        "svc".to_string(),
        "nod".to_string(),
    ]
}

fn default_allow_removal() -> bool {
    true
}

impl GovernorPolicy {
    pub fn from_toml_str(input: &str) -> Result<Self, GovernorError> {
        let mut p: GovernorPolicy = toml::from_str(input).map_err(GovernorError::PolicyParse)?;
        p.normalize()?;
        Ok(p)
    }

    pub fn load(path: &Path) -> Result<Self, GovernorError> {
        let content = std::fs::read_to_string(path).map_err(GovernorError::Io)?;
        Self::from_toml_str(&content)
    }

    fn normalize(&mut self) -> Result<(), GovernorError> {
        self.governor = canonicalize(&self.governor)
            .map_err(|_| GovernorError::InvalidPolicy("governor must be valid VAS address".into()))?;
        self.managed_aura = canonicalize(&self.managed_aura)
            .map_err(|_| GovernorError::InvalidPolicy("managed_aura must be valid VAS address".into()))?;

        let managed = VasAddress::parse(&self.managed_aura)
            .map_err(|_| GovernorError::InvalidPolicy("managed_aura must be parseable".into()))?;
        if managed.kind != AddressType::Aura {
            return Err(GovernorError::InvalidPolicy(
                "managed_aura must be an aur{...} address".into(),
            ));
        }

        self.admins = normalize_address_list(&self.admins, "admin")?;
        self.inherited_admins = normalize_address_list(&self.inherited_admins, "inherited admin")?;

        self.allow_member_types = normalize_member_types(&self.allow_member_types)?;
        self.inherited_allow_member_types = normalize_member_types(&self.inherited_allow_member_types)?;

        if self.policy_version == 0 {
            self.policy_version = 1;
        }

        Ok(())
    }

    fn is_admin(&self, actor: &str) -> Result<bool, GovernorError> {
        let canonical_actor = canonicalize(actor).map_err(|_| GovernorError::InvalidAddress)?;
        let effective = self.effective_admins();
        Ok(effective.binary_search(&canonical_actor).is_ok())
    }

    fn member_type_allowed(&self, member: &str) -> Result<bool, GovernorError> {
        let canonical_member = canonicalize(member).map_err(|_| GovernorError::InvalidAddress)?;
        let addr = VasAddress::parse(&canonical_member).map_err(|_| GovernorError::InvalidAddress)?;
        Ok(self
            .effective_member_types()
            .iter()
            .any(|t| address_type_code(addr.kind) == t))
    }

    fn effective_admins(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        for a in &self.admins {
            out.insert(a.clone());
        }
        for a in &self.inherited_admins {
            out.insert(a.clone());
        }
        out.into_iter().collect()
    }

    fn effective_member_types(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        for t in &self.allow_member_types {
            out.insert(t.clone());
        }
        for t in &self.inherited_allow_member_types {
            out.insert(t.clone());
        }
        out.into_iter().collect()
    }

    fn effective_allow_removal(&self) -> bool {
        let inherited_gate = self.inherited_allow_removal.unwrap_or(true);
        self.allow_removal && inherited_gate
    }

    fn apply_delta(&mut self, delta: &GovernorPolicyDelta) -> Result<bool, GovernorError> {
        let mut changed = false;

        if !delta.add_admins.is_empty() || !delta.remove_admins.is_empty() {
            let mut admins = self.admins.clone();
            for a in &delta.add_admins {
                admins.push(a.clone());
            }
            let remove = normalize_address_list(&delta.remove_admins, "admin")?;
            let remove_set: BTreeSet<String> = remove.into_iter().collect();
            admins = normalize_address_list(&admins, "admin")?
                .into_iter()
                .filter(|a| !remove_set.contains(a))
                .collect();
            if admins != self.admins {
                self.admins = admins;
                changed = true;
            }
        }

        if !delta.add_member_types.is_empty() || !delta.remove_member_types.is_empty() {
            let mut types = self.allow_member_types.clone();
            for t in &delta.add_member_types {
                types.push(t.clone());
            }
            let remove = normalize_member_types(&delta.remove_member_types)?;
            let remove_set: BTreeSet<String> = remove.into_iter().collect();
            types = normalize_member_types(&types)?
                .into_iter()
                .filter(|t| !remove_set.contains(t))
                .collect();
            if types != self.allow_member_types {
                self.allow_member_types = types;
                changed = true;
            }
        }

        if let Some(v) = delta.allow_removal {
            if self.allow_removal != v {
                self.allow_removal = v;
                changed = true;
            }
        }

        if let Some(v) = &delta.inherited_admins {
            let norm = normalize_address_list(v, "inherited admin")?;
            if self.inherited_admins != norm {
                self.inherited_admins = norm;
                changed = true;
            }
        }

        if let Some(v) = &delta.inherited_allow_member_types {
            let norm = normalize_member_types(v)?;
            if self.inherited_allow_member_types != norm {
                self.inherited_allow_member_types = norm;
                changed = true;
            }
        }

        if let Some(v) = delta.inherited_allow_removal {
            if self.inherited_allow_removal != Some(v) {
                self.inherited_allow_removal = Some(v);
                changed = true;
            }
        }

        if changed {
            self.policy_version = self.policy_version.saturating_add(1);
        }
        Ok(changed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    AddMember,
    RemoveMember,
    UpdatePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRecord {
    pub ts_unix_ms: u128,
    pub governor: String,
    pub aura: String,
    pub policy_version: u64,
    pub actor: String,
    pub action: AuditAction,
    pub member: String,
    pub outcome: AuditOutcome,
    pub reason: String,
    pub trace: Option<DecisionTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionTrace {
    pub policy_version: u64,
    pub action: String,
    pub allowed: bool,
    pub steps: Vec<TraceStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceStep {
    pub check: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug)]
pub struct Governor {
    policy: GovernorPolicy,
    audit_path: Option<PathBuf>,
    audit_log: Vec<AuditRecord>,
}

impl Governor {
    pub fn new(policy: GovernorPolicy) -> Self {
        Self {
            policy,
            audit_path: None,
            audit_log: Vec::new(),
        }
    }

    pub fn with_audit_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.audit_path = Some(path.into());
        self
    }

    pub fn policy(&self) -> &GovernorPolicy {
        &self.policy
    }

    pub fn audit_log(&self) -> &[AuditRecord] {
        &self.audit_log
    }

    pub fn apply_policy_delta(
        &mut self,
        actor: &str,
        delta: &GovernorPolicyDelta,
    ) -> Result<DecisionTrace, GovernorError> {
        let mut trace = DecisionTrace {
            policy_version: self.policy.policy_version,
            action: "update_policy".to_string(),
            allowed: false,
            steps: Vec::new(),
        };

        let is_admin = self.policy.is_admin(actor)?;
        trace.steps.push(TraceStep {
            check: "actor_is_admin".to_string(),
            passed: is_admin,
            detail: if is_admin {
                "actor authorized for policy updates".to_string()
            } else {
                "actor not in effective admins".to_string()
            },
        });

        if !is_admin {
            self.record(
                actor,
                AuditAction::UpdatePolicy,
                "evt{governor,policy,update}",
                AuditOutcome::Denied,
                "actor not in admins",
                Some(&trace),
            )?;
            return Err(GovernorError::Denied("actor not in admins".into()));
        }

        let changed = self.policy.apply_delta(delta)?;
        trace.policy_version = self.policy.policy_version;
        trace.steps.push(TraceStep {
            check: "delta_applied".to_string(),
            passed: true,
            detail: if changed {
                "policy updated and version incremented".to_string()
            } else {
                "no effective change in delta".to_string()
            },
        });
        trace.allowed = true;

        self.record(
            actor,
            AuditAction::UpdatePolicy,
            "evt{governor,policy,update}",
            AuditOutcome::Allowed,
            if changed {
                "policy updated"
            } else {
                "policy unchanged"
            },
            Some(&trace),
        )?;

        Ok(trace)
    }

    pub fn add_member(&mut self, aura: &mut Aura, actor: &str, member: &str) -> Result<bool, GovernorError> {
        let mut trace = DecisionTrace {
            policy_version: self.policy.policy_version,
            action: "add_member".to_string(),
            allowed: false,
            steps: Vec::new(),
        };

        let managed_ok = self.ensure_managed_aura(aura).is_ok();
        trace.steps.push(TraceStep {
            check: "managed_aura_match".to_string(),
            passed: managed_ok,
            detail: if managed_ok {
                "aura matches managed policy".to_string()
            } else {
                "governor policy does not manage this aura".to_string()
            },
        });
        if !managed_ok {
            self.record(actor, AuditAction::AddMember, member, AuditOutcome::Denied, "aura not managed", Some(&trace))?;
            return Err(GovernorError::Denied("governor policy does not manage this aura".into()));
        }

        let is_admin = self.policy.is_admin(actor)?;
        trace.steps.push(TraceStep {
            check: "actor_is_admin".to_string(),
            passed: is_admin,
            detail: if is_admin {
                "actor in effective admins".to_string()
            } else {
                "actor not in effective admins".to_string()
            },
        });
        if !is_admin {
            self.record(actor, AuditAction::AddMember, member, AuditOutcome::Denied, "actor not in admins", Some(&trace))?;
            return Err(GovernorError::Denied("actor not in admins".into()));
        }

        let type_allowed = self.policy.member_type_allowed(member)?;
        trace.steps.push(TraceStep {
            check: "member_type_allowed".to_string(),
            passed: type_allowed,
            detail: if type_allowed {
                "member type allowed by effective policy".to_string()
            } else {
                "member type rejected by effective policy".to_string()
            },
        });
        if !type_allowed {
            self.record(actor, AuditAction::AddMember, member, AuditOutcome::Denied, "member type not allowed", Some(&trace))?;
            return Err(GovernorError::Denied("member type not allowed".into()));
        }

        let changed = aura.add_member(member).map_err(map_aura_error)?;
        trace.allowed = true;
        trace.steps.push(TraceStep {
            check: "membership_write".to_string(),
            passed: true,
            detail: if changed {
                "member inserted".to_string()
            } else {
                "member already present".to_string()
            },
        });
        let reason = if changed { "member added" } else { "already a member" };
        self.record(actor, AuditAction::AddMember, member, AuditOutcome::Allowed, reason, Some(&trace))?;
        Ok(changed)
    }

    pub fn remove_member(
        &mut self,
        aura: &mut Aura,
        actor: &str,
        member: &str,
    ) -> Result<bool, GovernorError> {
        let mut trace = DecisionTrace {
            policy_version: self.policy.policy_version,
            action: "remove_member".to_string(),
            allowed: false,
            steps: Vec::new(),
        };

        let managed_ok = self.ensure_managed_aura(aura).is_ok();
        trace.steps.push(TraceStep {
            check: "managed_aura_match".to_string(),
            passed: managed_ok,
            detail: if managed_ok {
                "aura matches managed policy".to_string()
            } else {
                "governor policy does not manage this aura".to_string()
            },
        });
        if !managed_ok {
            self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Denied, "aura not managed", Some(&trace))?;
            return Err(GovernorError::Denied("governor policy does not manage this aura".into()));
        }

        let removal_allowed = self.policy.effective_allow_removal();
        trace.steps.push(TraceStep {
            check: "removal_allowed".to_string(),
            passed: removal_allowed,
            detail: if removal_allowed {
                "removal enabled in effective policy".to_string()
            } else {
                "removal disabled by inherited or local policy".to_string()
            },
        });
        if !removal_allowed {
            self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Denied, "removal disabled by policy", Some(&trace))?;
            return Err(GovernorError::Denied("removal disabled by policy".into()));
        }

        let is_admin = self.policy.is_admin(actor)?;
        trace.steps.push(TraceStep {
            check: "actor_is_admin".to_string(),
            passed: is_admin,
            detail: if is_admin {
                "actor in effective admins".to_string()
            } else {
                "actor not in effective admins".to_string()
            },
        });
        if !is_admin {
            self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Denied, "actor not in admins", Some(&trace))?;
            return Err(GovernorError::Denied("actor not in admins".into()));
        }

        let changed = aura.remove_member(member).map_err(map_aura_error)?;
        trace.allowed = true;
        trace.steps.push(TraceStep {
            check: "membership_write".to_string(),
            passed: true,
            detail: if changed {
                "member removed".to_string()
            } else {
                "member not present".to_string()
            },
        });
        let reason = if changed { "member removed" } else { "member not present" };
        self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Allowed, reason, Some(&trace))?;
        Ok(changed)
    }

    fn ensure_managed_aura(&self, aura: &Aura) -> Result<(), GovernorError> {
        if aura.id.as_str() != self.policy.managed_aura {
            return Err(GovernorError::Denied(
                "governor policy does not manage this aura".into(),
            ));
        }
        Ok(())
    }

    fn record(
        &mut self,
        actor: &str,
        action: AuditAction,
        member: &str,
        outcome: AuditOutcome,
        reason: &str,
        trace: Option<&DecisionTrace>,
    ) -> Result<(), GovernorError> {
        let rec = AuditRecord {
            ts_unix_ms: now_ms(),
            governor: self.policy.governor.clone(),
            aura: self.policy.managed_aura.clone(),
            policy_version: self.policy.policy_version,
            actor: canonicalize(actor).map_err(|_| GovernorError::InvalidAddress)?,
            action,
            member: canonicalize(member).map_err(|_| GovernorError::InvalidAddress)?,
            outcome,
            reason: reason.to_string(),
            trace: trace.cloned(),
        };

        if let Some(path) = &self.audit_path {
            append_jsonl(path, &rec)?;
        }
        self.audit_log.push(rec);
        Ok(())
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn append_jsonl(path: &Path, rec: &AuditRecord) -> Result<(), GovernorError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(GovernorError::Io)?;
    }

    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(GovernorError::Io)?;
    let line = serde_json::to_string(rec).map_err(GovernorError::AuditSerialize)?;
    f.write_all(line.as_bytes()).map_err(GovernorError::Io)?;
    f.write_all(b"\n").map_err(GovernorError::Io)?;
    Ok(())
}

fn normalize_address_list(input: &[String], what: &str) -> Result<Vec<String>, GovernorError> {
    let mut out = BTreeSet::new();
    for item in input {
        let c = canonicalize(item)
            .map_err(|_| GovernorError::InvalidPolicy(format!("{what} must be valid VAS address")))?;
        out.insert(c);
    }
    Ok(out.into_iter().collect())
}

fn normalize_member_types(input: &[String]) -> Result<Vec<String>, GovernorError> {
    let mut allowed = BTreeSet::new();
    for t in input {
        let lower = t.trim().to_ascii_lowercase();
        if !is_allowed_member_type_code(&lower) {
            return Err(GovernorError::InvalidPolicy(format!(
                "invalid allow_member_types entry: {lower}"
            )));
        }
        allowed.insert(lower);
    }
    Ok(allowed.into_iter().collect())
}

fn map_aura_error(_: veer_aura::AuraError) -> GovernorError {
    GovernorError::InvalidAddress
}

fn address_type_code(kind: AddressType) -> &'static str {
    match kind {
        AddressType::User => "usr",
        AddressType::Device => "dev",
        AddressType::Fold => "fld",
        AddressType::Aura => "aur",
        AddressType::Service => "svc",
        AddressType::Vault => "vlt",
        AddressType::Agent => "agt",
        AddressType::Zone => "zon",
        AddressType::Node => "nod",
        AddressType::Event => "evt",
    }
}

fn is_allowed_member_type_code(code: &str) -> bool {
    matches!(code, "usr" | "dev" | "fld" | "agt" | "svc" | "nod")
}

#[derive(Debug)]
pub enum GovernorError {
    Io(std::io::Error),
    PolicyParse(toml::de::Error),
    AuditSerialize(serde_json::Error),
    InvalidPolicy(String),
    InvalidAddress,
    Denied(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use veer_aura::AuraId;

    fn sample_policy() -> GovernorPolicy {
        GovernorPolicy::from_toml_str(
            r#"
            governor = "svc{governor,company,live}"
            managed_aura = "aur{finance,private,open}"
            admins = ["usr{alice,corp,active}", "usr{bob,corp,active}"]
            allow_member_types = ["usr", "fld", "svc"]
            allow_removal = true
            "#,
        )
        .unwrap()
    }

    #[test]
    fn policy_is_canonicalized_and_validated() {
        let p = GovernorPolicy::from_toml_str(
            r#"
            governor = "SVC{Governor,Company,Live}"
            managed_aura = "AUR{Finance,Private,Open}"
            admins = ["USR{Alice,Corp,Active}", "usr{alice,corp,active}"]
            allow_member_types = ["USR", "fld"]
            "#,
        )
        .unwrap();

        assert_eq!(p.governor, "svc{governor,company,live}");
        assert_eq!(p.managed_aura, "aur{finance,private,open}");
        assert_eq!(p.admins, vec!["usr{alice,corp,active}".to_string()]);
        assert_eq!(p.allow_member_types, vec!["fld".to_string(), "usr".to_string()]);
    }

    #[test]
    fn add_member_enforces_admin_and_type_and_audits() {
        let mut g = Governor::new(sample_policy());
        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());

        let denied = g.add_member(&mut aura, "usr{mallory,corp,active}", "usr{jane,corp,active}");
        assert!(matches!(denied, Err(GovernorError::Denied(_))));

        let denied_type = g.add_member(
            &mut aura,
            "usr{alice,corp,active}",
            "vlt{payroll,corp,2026}",
        );
        assert!(matches!(denied_type, Err(GovernorError::Denied(_))));

        let ok = g
            .add_member(
                &mut aura,
                "usr{alice,corp,active}",
                "usr{jane,corp,active}",
            )
            .unwrap();
        assert!(ok);
        assert!(aura.contains_member("usr{jane,corp,active}"));

        assert_eq!(g.audit_log().len(), 3);
        assert_eq!(g.audit_log()[0].outcome, AuditOutcome::Denied);
        assert_eq!(g.audit_log()[2].outcome, AuditOutcome::Allowed);
        assert!(g.audit_log()[2].trace.is_some());
    }

    #[test]
    fn remove_member_obeys_policy_and_audits() {
        let mut g = Governor::new(sample_policy());
        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());
        aura.add_member("usr{jane,corp,active}").unwrap();

        let changed = g
            .remove_member(
                &mut aura,
                "usr{alice,corp,active}",
                "usr{jane,corp,active}",
            )
            .unwrap();
        assert!(changed);
        assert!(!aura.contains_member("usr{jane,corp,active}"));

        let denied = g.remove_member(
            &mut aura,
            "usr{mallory,corp,active}",
            "usr{jane,corp,active}",
        );
        assert!(matches!(denied, Err(GovernorError::Denied(_))));

        assert_eq!(g.audit_log().len(), 2);
    }

    #[test]
    fn appends_jsonl_audit_log() {
        let mut g = Governor::new(sample_policy()).with_audit_file(std::env::temp_dir().join(format!(
            "veer-governor-audit-{}-{}.jsonl",
            std::process::id(),
            now_ms()
        )));

        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());
        let _ = g.add_member(
            &mut aura,
            "usr{alice,corp,active}",
            "usr{jane,corp,active}",
        );

        let path = g.audit_path.clone().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"action\":\"add_member\""));
        assert!(text.contains("\"outcome\":\"allowed\""));
        assert!(text.contains("\"policy_version\":"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn inherited_admin_can_manage_membership() {
        let mut p = sample_policy();
        p.admins.clear();
        p.inherited_admins = vec!["usr{carol,corp,active}".into()];
        p.normalize().unwrap();

        let mut g = Governor::new(p);
        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());
        let ok = g
            .add_member(
                &mut aura,
                "usr{carol,corp,active}",
                "usr{jane,corp,active}",
            )
            .unwrap();
        assert!(ok);
    }

    #[test]
    fn policy_delta_updates_version_and_permissions() {
        let mut g = Governor::new(sample_policy());
        let before = g.policy().policy_version;

        let delta = GovernorPolicyDelta {
            add_admins: vec!["usr{dave,corp,active}".into()],
            remove_admins: vec![],
            add_member_types: vec![],
            remove_member_types: vec![],
            allow_removal: Some(false),
            inherited_admins: None,
            inherited_allow_member_types: None,
            inherited_allow_removal: None,
        };

        let trace = g
            .apply_policy_delta("usr{alice,corp,active}", &delta)
            .unwrap();
        assert!(trace.allowed);
        assert!(g.policy().policy_version > before);
        assert!(g.policy().is_admin("usr{dave,corp,active}").unwrap());
        assert!(!g.policy().effective_allow_removal());
    }

    #[test]
    fn denied_decision_contains_trace() {
        let mut g = Governor::new(sample_policy());
        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());

        let err = g.add_member(
            &mut aura,
            "usr{mallory,corp,active}",
            "usr{jane,corp,active}",
        );
        assert!(matches!(err, Err(GovernorError::Denied(_))));

        let rec = g.audit_log().last().unwrap();
        assert_eq!(rec.outcome, AuditOutcome::Denied);
        let trace = rec.trace.as_ref().unwrap();
        assert!(!trace.allowed);
        assert!(trace.steps.iter().any(|s| s.check == "actor_is_admin" && !s.passed));
    }
}
