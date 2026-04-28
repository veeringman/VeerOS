//! VeerOS Aura Governor MVP.
//!
//! Capabilities:
//! - Load static policy from TOML
//! - Authorize and apply Aura membership add/remove operations
//! - Emit append-only JSONL audit records for every decision

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
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default = "default_member_types")]
    pub allow_member_types: Vec<String>,
    #[serde(default = "default_allow_removal")]
    pub allow_removal: bool,
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

        let mut admins = BTreeSet::new();
        for admin in &self.admins {
            let c = canonicalize(admin)
                .map_err(|_| GovernorError::InvalidPolicy("admin must be valid VAS address".into()))?;
            admins.insert(c);
        }
        self.admins = admins.into_iter().collect();

        let mut allowed = BTreeSet::new();
        for t in &self.allow_member_types {
            let lower = t.trim().to_ascii_lowercase();
            if !is_allowed_member_type_code(&lower) {
                return Err(GovernorError::InvalidPolicy(format!(
                    "invalid allow_member_types entry: {lower}"
                )));
            }
            allowed.insert(lower);
        }
        self.allow_member_types = allowed.into_iter().collect();

        Ok(())
    }

    fn is_admin(&self, actor: &str) -> Result<bool, GovernorError> {
        let canonical_actor = canonicalize(actor).map_err(|_| GovernorError::InvalidAddress)?;
        Ok(self.admins.binary_search(&canonical_actor).is_ok())
    }

    fn member_type_allowed(&self, member: &str) -> Result<bool, GovernorError> {
        let canonical_member = canonicalize(member).map_err(|_| GovernorError::InvalidAddress)?;
        let addr = VasAddress::parse(&canonical_member).map_err(|_| GovernorError::InvalidAddress)?;
        Ok(self
            .allow_member_types
            .iter()
            .any(|t| address_type_code(addr.kind) == t))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    AddMember,
    RemoveMember,
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
    pub actor: String,
    pub action: AuditAction,
    pub member: String,
    pub outcome: AuditOutcome,
    pub reason: String,
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

    pub fn add_member(&mut self, aura: &mut Aura, actor: &str, member: &str) -> Result<bool, GovernorError> {
        self.ensure_managed_aura(aura)?;

        if !self.policy.is_admin(actor)? {
            self.record(actor, AuditAction::AddMember, member, AuditOutcome::Denied, "actor not in admins")?;
            return Err(GovernorError::Denied("actor not in admins".into()));
        }
        if !self.policy.member_type_allowed(member)? {
            self.record(actor, AuditAction::AddMember, member, AuditOutcome::Denied, "member type not allowed")?;
            return Err(GovernorError::Denied("member type not allowed".into()));
        }

        let changed = aura.add_member(member).map_err(map_aura_error)?;
        let reason = if changed { "member added" } else { "already a member" };
        self.record(actor, AuditAction::AddMember, member, AuditOutcome::Allowed, reason)?;
        Ok(changed)
    }

    pub fn remove_member(
        &mut self,
        aura: &mut Aura,
        actor: &str,
        member: &str,
    ) -> Result<bool, GovernorError> {
        self.ensure_managed_aura(aura)?;

        if !self.policy.allow_removal {
            self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Denied, "removal disabled by policy")?;
            return Err(GovernorError::Denied("removal disabled by policy".into()));
        }
        if !self.policy.is_admin(actor)? {
            self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Denied, "actor not in admins")?;
            return Err(GovernorError::Denied("actor not in admins".into()));
        }

        let changed = aura.remove_member(member).map_err(map_aura_error)?;
        let reason = if changed { "member removed" } else { "member not present" };
        self.record(actor, AuditAction::RemoveMember, member, AuditOutcome::Allowed, reason)?;
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
    ) -> Result<(), GovernorError> {
        let rec = AuditRecord {
            ts_unix_ms: now_ms(),
            governor: self.policy.governor.clone(),
            aura: self.policy.managed_aura.clone(),
            actor: canonicalize(actor).map_err(|_| GovernorError::InvalidAddress)?,
            action,
            member: canonicalize(member).map_err(|_| GovernorError::InvalidAddress)?,
            outcome,
            reason: reason.to_string(),
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

        let _ = std::fs::remove_file(path);
    }
}
