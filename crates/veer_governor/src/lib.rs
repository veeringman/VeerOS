//! VeerOS Aura Governor v2.
//!
//! Capabilities:
//! - Static policy load from TOML
//! - Dynamic policy updates (delta patches)
//! - Inherited policy overlays (admins/member-types/removal gate)
//! - Explainable decision traces
//! - Append-only JSONL audit records

use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};
use veer_aura::Aura;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisConfig {
    pub min_events: usize,
    pub deny_ratio_threshold: f32,
    pub unknown_actor_denied_threshold: usize,
    pub member_type_denied_threshold: usize,
    pub policy_update_churn_threshold: usize,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            min_events: 8,
            deny_ratio_threshold: 0.35,
            unknown_actor_denied_threshold: 3,
            member_type_denied_threshold: 3,
            policy_update_churn_threshold: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditAnalysisReport {
    pub total_events: usize,
    pub allowed_events: usize,
    pub denied_events: usize,
    pub deny_ratio: f32,
    pub unique_actors: usize,
    pub findings: Vec<AnomalyFinding>,
    pub suggestions: Vec<PolicySuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnomalyFinding {
    pub code: String,
    pub severity: String,
    pub count: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicySuggestion {
    pub id: String,
    pub priority: String,
    pub summary: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedCapability {
    AddMember,
    RemoveMember,
    AddTemporaryMember,
    UpdatePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationGrant {
    pub aura: String,
    pub grantor: String,
    pub delegate: String,
    pub capabilities: BTreeSet<DelegatedCapability>,
    pub expires_unix_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FederationLink {
    pub parent_aura: String,
    pub child_aura: String,
    pub inherit_admins: bool,
    pub inherit_member_types: bool,
    pub inherit_allow_removal: bool,
}

#[derive(Debug, Default)]
pub struct FederatedGovernor {
    policies: HashMap<String, GovernorPolicy>,
    links: Vec<FederationLink>,
    delegations: Vec<DelegationGrant>,
}

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
    AddTemporaryMember,
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

    pub fn analyze_recent_audit(&self, limit: usize, cfg: &AnalysisConfig) -> AuditAnalysisReport {
        if limit == 0 || self.audit_log.is_empty() {
            return analyze_audit_records(&[], cfg);
        }

        let start = self.audit_log.len().saturating_sub(limit);
        analyze_audit_records(&self.audit_log[start..], cfg)
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

    pub fn add_temporary_agent_member(
        &mut self,
        aura: &mut Aura,
        actor: &str,
        member: &str,
        expires_unix_ms: u128,
        scopes: &[String],
        now_unix_ms: u128,
    ) -> Result<bool, GovernorError> {
        let mut trace = DecisionTrace {
            policy_version: self.policy.policy_version,
            action: "add_temporary_member".to_string(),
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
            self.record(
                actor,
                AuditAction::AddTemporaryMember,
                member,
                AuditOutcome::Denied,
                "aura not managed",
                Some(&trace),
            )?;
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
            self.record(
                actor,
                AuditAction::AddTemporaryMember,
                member,
                AuditOutcome::Denied,
                "actor not in admins",
                Some(&trace),
            )?;
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
            self.record(
                actor,
                AuditAction::AddTemporaryMember,
                member,
                AuditOutcome::Denied,
                "member type not allowed",
                Some(&trace),
            )?;
            return Err(GovernorError::Denied("member type not allowed".into()));
        }

        let changed = aura
            .add_temporary_member(member, actor, expires_unix_ms, scopes, now_unix_ms)
            .map_err(map_temporary_membership_error)?;

        trace.allowed = true;
        trace.steps.push(TraceStep {
            check: "temporary_membership_write".to_string(),
            passed: true,
            detail: if changed {
                "temporary membership inserted/updated".to_string()
            } else {
                "temporary membership unchanged".to_string()
            },
        });

        let reason = if changed {
            "temporary scoped member added"
        } else {
            "temporary scoped member unchanged"
        };
        self.record(
            actor,
            AuditAction::AddTemporaryMember,
            member,
            AuditOutcome::Allowed,
            reason,
            Some(&trace),
        )?;
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

impl FederatedGovernor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_policy(&mut self, mut policy: GovernorPolicy) -> Result<(), GovernorError> {
        policy.normalize()?;
        self.policies.insert(policy.managed_aura.clone(), policy);
        Ok(())
    }

    pub fn add_federation_link(&mut self, mut link: FederationLink) -> Result<(), GovernorError> {
        link.parent_aura = canonical_aura_addr(&link.parent_aura)?;
        link.child_aura = canonical_aura_addr(&link.child_aura)?;

        if link.parent_aura == link.child_aura {
            return Err(GovernorError::InvalidPolicy(
                "federation link cannot point an aura to itself".into(),
            ));
        }

        if !self.policies.contains_key(&link.parent_aura) || !self.policies.contains_key(&link.child_aura)
        {
            return Err(GovernorError::InvalidPolicy(
                "federation link requires both parent and child policies to be registered".into(),
            ));
        }

        if self
            .links
            .iter()
            .any(|l| l.parent_aura == link.parent_aura && l.child_aura == link.child_aura)
        {
            return Ok(());
        }

        self.links.push(link.clone());
        if self.has_cycle() {
            self.links.pop();
            return Err(GovernorError::InvalidPolicy(
                "federation link introduces an inheritance cycle".into(),
            ));
        }

        Ok(())
    }

    pub fn effective_policy(&self, aura: &str) -> Result<GovernorPolicy, GovernorError> {
        let aura = canonical_aura_addr(aura)?;
        let Some(base) = self.policies.get(&aura) else {
            return Err(GovernorError::Denied("aura policy not found".into()));
        };

        let mut effective = base.clone();
        let mut cur = aura.clone();
        let mut visited = BTreeSet::new();

        while let Some(link) = self.links.iter().find(|l| l.child_aura == cur) {
            if !visited.insert(link.child_aura.clone()) {
                break;
            }
            let Some(parent) = self.policies.get(&link.parent_aura) else {
                break;
            };

            if link.inherit_admins {
                for a in parent.effective_admins() {
                    if !effective.inherited_admins.contains(&a) {
                        effective.inherited_admins.push(a);
                    }
                }
            }

            if link.inherit_member_types {
                for t in parent.effective_member_types() {
                    if !effective.inherited_allow_member_types.contains(&t) {
                        effective.inherited_allow_member_types.push(t);
                    }
                }
            }

            if link.inherit_allow_removal {
                let inherited_gate = parent.effective_allow_removal();
                effective.inherited_allow_removal = Some(
                    effective
                        .inherited_allow_removal
                        .unwrap_or(true)
                        && inherited_gate,
                );
            }

            cur = link.parent_aura.clone();
        }

        effective.normalize()?;
        Ok(effective)
    }

    pub fn grant_delegation(
        &mut self,
        aura: &str,
        grantor: &str,
        delegate: &str,
        capabilities: BTreeSet<DelegatedCapability>,
        expires_unix_ms: u128,
        now_unix_ms: u128,
    ) -> Result<(), GovernorError> {
        if capabilities.is_empty() {
            return Err(GovernorError::Denied("delegation requires at least one capability".into()));
        }
        if expires_unix_ms <= now_unix_ms {
            return Err(GovernorError::Denied("delegation expiry must be in the future".into()));
        }

        let aura = canonical_aura_addr(aura)?;
        let grantor = canonicalize(grantor).map_err(|_| GovernorError::InvalidAddress)?;
        let delegate = canonicalize(delegate).map_err(|_| GovernorError::InvalidAddress)?;
        let effective = self.effective_policy(&aura)?;

        if !effective.is_admin(&grantor)? {
            return Err(GovernorError::Denied("grantor is not an effective admin for aura".into()));
        }

        self.delegations.retain(|d| {
            !(d.aura == aura && d.grantor == grantor && d.delegate == delegate)
        });
        self.delegations.push(DelegationGrant {
            aura,
            grantor,
            delegate,
            capabilities,
            expires_unix_ms,
        });

        Ok(())
    }

    pub fn is_authorized(
        &self,
        aura: &str,
        actor: &str,
        capability: DelegatedCapability,
        now_unix_ms: u128,
    ) -> Result<bool, GovernorError> {
        let aura = canonical_aura_addr(aura)?;
        let actor = canonicalize(actor).map_err(|_| GovernorError::InvalidAddress)?;
        let effective = self.effective_policy(&aura)?;

        if effective.is_admin(&actor)? {
            return Ok(true);
        }

        Ok(self.delegations.iter().any(|d| {
            d.aura == aura
                && d.delegate == actor
                && d.expires_unix_ms > now_unix_ms
                && d.capabilities.contains(&capability)
        }))
    }

    fn has_cycle(&self) -> bool {
        for p in self.policies.keys() {
            let mut seen = BTreeSet::new();
            let mut cur = p.clone();
            while let Some(next) = self.links.iter().find(|l| l.child_aura == cur).map(|l| l.parent_aura.clone()) {
                if !seen.insert(cur.clone()) {
                    return true;
                }
                cur = next;
            }
        }
        false
    }
}

fn canonical_aura_addr(input: &str) -> Result<String, GovernorError> {
    let canonical = canonicalize(input).map_err(|_| GovernorError::InvalidAddress)?;
    let addr = VasAddress::parse(&canonical).map_err(|_| GovernorError::InvalidAddress)?;
    if addr.kind != AddressType::Aura {
        return Err(GovernorError::InvalidPolicy(
            "federation operations require aur{...} addresses".into(),
        ));
    }
    Ok(canonical)
}

pub fn analyze_audit_records(records: &[AuditRecord], cfg: &AnalysisConfig) -> AuditAnalysisReport {
    let total_events = records.len();
    let allowed_events = records
        .iter()
        .filter(|r| r.outcome == AuditOutcome::Allowed)
        .count();
    let denied_events = total_events.saturating_sub(allowed_events);
    let deny_ratio = if total_events == 0 {
        0.0
    } else {
        denied_events as f32 / total_events as f32
    };

    let mut unique_actors = BTreeSet::new();
    let mut denied_unknown_actor = 0usize;
    let mut denied_member_type = 0usize;
    let mut update_policy_allowed = 0usize;

    for r in records {
        unique_actors.insert(r.actor.clone());

        if r.action == AuditAction::UpdatePolicy && r.outcome == AuditOutcome::Allowed {
            update_policy_allowed += 1;
        }

        if r.outcome == AuditOutcome::Denied {
            let reason = r.reason.to_ascii_lowercase();
            if reason.contains("actor not in admins") {
                denied_unknown_actor += 1;
            }
            if reason.contains("member type not allowed") {
                denied_member_type += 1;
            }
        }
    }

    let mut findings = Vec::new();
    let mut suggestions = Vec::new();

    if total_events >= cfg.min_events && deny_ratio >= cfg.deny_ratio_threshold {
        findings.push(AnomalyFinding {
            code: "deny_spike".to_string(),
            severity: "high".to_string(),
            count: denied_events,
            detail: format!(
                "deny ratio {:.1}% exceeded threshold {:.1}% over {} events",
                deny_ratio * 100.0,
                cfg.deny_ratio_threshold * 100.0,
                total_events
            ),
        });
        suggestions.push(PolicySuggestion {
            id: "review_recent_denials".to_string(),
            priority: "high".to_string(),
            summary: "Review recent denied decisions and tighten admission path".to_string(),
            rationale:
                "A sustained deny spike indicates policy/traffic mismatch; validate caller identity sources and member admission criteria".to_string(),
        });
    }

    if denied_unknown_actor >= cfg.unknown_actor_denied_threshold {
        findings.push(AnomalyFinding {
            code: "unknown_actor_denials".to_string(),
            severity: "medium".to_string(),
            count: denied_unknown_actor,
            detail: "Repeated denies where actor is not an effective admin".to_string(),
        });
        suggestions.push(PolicySuggestion {
            id: "delegate_scoped_admin".to_string(),
            priority: "medium".to_string(),
            summary: "Add scoped delegation for expected automation actors".to_string(),
            rationale:
                "Frequent admin-check failures suggest missing delegated principals; prefer time-bound delegated actors over broad permanent admin expansion".to_string(),
        });
    }

    if denied_member_type >= cfg.member_type_denied_threshold {
        findings.push(AnomalyFinding {
            code: "member_type_rejections".to_string(),
            severity: "medium".to_string(),
            count: denied_member_type,
            detail: "Repeated denies due to disallowed member type".to_string(),
        });
        suggestions.push(PolicySuggestion {
            id: "refine_member_type_policy".to_string(),
            priority: "medium".to_string(),
            summary: "Refine allow_member_types or onboarding flow".to_string(),
            rationale:
                "Rejected member-type requests likely indicate policy is stricter than real usage; tune type allowlist or enforce earlier validation in clients".to_string(),
        });
    }

    if update_policy_allowed >= cfg.policy_update_churn_threshold {
        findings.push(AnomalyFinding {
            code: "policy_update_churn".to_string(),
            severity: "medium".to_string(),
            count: update_policy_allowed,
            detail: "High count of successful policy updates in analysis window".to_string(),
        });
        suggestions.push(PolicySuggestion {
            id: "stabilize_policy_rollouts".to_string(),
            priority: "low".to_string(),
            summary: "Batch policy deltas and introduce staged rollouts".to_string(),
            rationale:
                "Frequent policy changes can create operational instability; use release windows and consolidated deltas for predictability".to_string(),
        });
    }

    findings.sort_by(|a, b| a.code.cmp(&b.code));
    suggestions.sort_by(|a, b| a.id.cmp(&b.id));

    AuditAnalysisReport {
        total_events,
        allowed_events,
        denied_events,
        deny_ratio,
        unique_actors: unique_actors.len(),
        findings,
        suggestions,
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

fn map_temporary_membership_error(err: veer_aura::AuraError) -> GovernorError {
    match err {
        veer_aura::AuraError::InvalidAddress => GovernorError::InvalidAddress,
        veer_aura::AuraError::NotAgentAddress => {
            GovernorError::Denied("temporary membership requires agt{...} member".into())
        }
        veer_aura::AuraError::InvalidExpiry => {
            GovernorError::Denied("temporary membership expiry must be in the future".into())
        }
        veer_aura::AuraError::InvalidScope => {
            GovernorError::Denied("temporary membership requires non-empty scope list".into())
        }
        _ => GovernorError::InvalidAddress,
    }
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

    #[test]
    fn audit_analysis_reports_findings_and_suggestions() {
        let records = vec![
            AuditRecord {
                ts_unix_ms: 1,
                governor: "svc{governor,company,live}".into(),
                aura: "aur{finance,private,open}".into(),
                policy_version: 3,
                actor: "usr{mallory,corp,active}".into(),
                action: AuditAction::AddMember,
                member: "usr{jane,corp,active}".into(),
                outcome: AuditOutcome::Denied,
                reason: "actor not in admins".into(),
                trace: None,
            },
            AuditRecord {
                ts_unix_ms: 2,
                governor: "svc{governor,company,live}".into(),
                aura: "aur{finance,private,open}".into(),
                policy_version: 3,
                actor: "usr{mallory,corp,active}".into(),
                action: AuditAction::AddMember,
                member: "vlt{payroll,corp,2026}".into(),
                outcome: AuditOutcome::Denied,
                reason: "member type not allowed".into(),
                trace: None,
            },
            AuditRecord {
                ts_unix_ms: 3,
                governor: "svc{governor,company,live}".into(),
                aura: "aur{finance,private,open}".into(),
                policy_version: 4,
                actor: "usr{alice,corp,active}".into(),
                action: AuditAction::UpdatePolicy,
                member: "evt{governor,policy,update}".into(),
                outcome: AuditOutcome::Allowed,
                reason: "policy updated".into(),
                trace: None,
            },
            AuditRecord {
                ts_unix_ms: 4,
                governor: "svc{governor,company,live}".into(),
                aura: "aur{finance,private,open}".into(),
                policy_version: 5,
                actor: "usr{alice,corp,active}".into(),
                action: AuditAction::UpdatePolicy,
                member: "evt{governor,policy,update}".into(),
                outcome: AuditOutcome::Allowed,
                reason: "policy updated".into(),
                trace: None,
            },
        ];

        let cfg = AnalysisConfig {
            min_events: 4,
            deny_ratio_threshold: 0.25,
            unknown_actor_denied_threshold: 1,
            member_type_denied_threshold: 1,
            policy_update_churn_threshold: 2,
        };

        let report = analyze_audit_records(&records, &cfg);
        assert_eq!(report.total_events, 4);
        assert_eq!(report.denied_events, 2);
        assert_eq!(report.unique_actors, 2);
        assert!(report.findings.iter().any(|f| f.code == "deny_spike"));
        assert!(report.findings.iter().any(|f| f.code == "member_type_rejections"));
        assert!(report.findings.iter().any(|f| f.code == "policy_update_churn"));
        assert!(report.findings.iter().any(|f| f.code == "unknown_actor_denials"));
        assert!(report.suggestions.iter().any(|s| s.id == "delegate_scoped_admin"));
    }

    #[test]
    fn audit_analysis_handles_empty_input() {
        let report = analyze_audit_records(&[], &AnalysisConfig::default());
        assert_eq!(report.total_events, 0);
        assert_eq!(report.denied_events, 0);
        assert!(report.findings.is_empty());
        assert!(report.suggestions.is_empty());
    }

    #[test]
    fn temporary_agent_membership_requires_admin_and_records_audit() {
        let mut g = Governor::new(sample_policy());
        g.policy.allow_member_types.push("agt".to_string());
        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());
        let now = 10_000u128;

        let denied = g.add_temporary_agent_member(
            &mut aura,
            "usr{mallory,corp,active}",
            "agt{opsbot,corp,live}",
            now + 500,
            &["observe".to_string()],
            now,
        );
        assert!(matches!(denied, Err(GovernorError::Denied(_))));

        let allowed = g
            .add_temporary_agent_member(
                &mut aura,
                "usr{alice,corp,active}",
                "agt{opsbot,corp,live}",
                now + 500,
                &["observe".to_string(), "deploy".to_string()],
                now,
            )
            .unwrap();
        assert!(allowed);
        assert!(aura.contains_member_at("agt{opsbot,corp,live}", now + 100));

        let rec = g.audit_log().last().unwrap();
        assert_eq!(rec.action, AuditAction::AddTemporaryMember);
        assert_eq!(rec.outcome, AuditOutcome::Allowed);
    }

    #[test]
    fn temporary_agent_membership_rejects_non_agent_member() {
        let mut g = Governor::new(sample_policy());
        let mut aura = Aura::new(AuraId::parse("aur{finance,private,open}").unwrap());
        let now = 15_000u128;

        let err = g.add_temporary_agent_member(
            &mut aura,
            "usr{alice,corp,active}",
            "usr{jane,corp,active}",
            now + 10,
            &["observe".to_string()],
            now,
        );
        assert!(matches!(err, Err(GovernorError::Denied(_))));
    }

    #[test]
    fn federation_inherits_parent_admins_and_member_types() {
        let mut root = GovernorPolicy::from_toml_str(
            r#"
            governor = "svc{gov,corp,live}"
            managed_aura = "aur{corp,private,open}"
            admins = ["usr{root,corp,active}"]
            allow_member_types = ["usr", "dev", "agt"]
            allow_removal = true
            "#,
        )
        .unwrap();
        root.normalize().unwrap();

        let mut team = GovernorPolicy::from_toml_str(
            r#"
            governor = "svc{gov,corp,live}"
            managed_aura = "aur{team,private,open}"
            admins = ["usr{alice,corp,active}"]
            allow_member_types = ["usr"]
            allow_removal = true
            "#,
        )
        .unwrap();
        team.normalize().unwrap();

        let mut fed = FederatedGovernor::new();
        fed.register_policy(root).unwrap();
        fed.register_policy(team).unwrap();
        fed.add_federation_link(FederationLink {
            parent_aura: "aur{corp,private,open}".into(),
            child_aura: "aur{team,private,open}".into(),
            inherit_admins: true,
            inherit_member_types: true,
            inherit_allow_removal: false,
        })
        .unwrap();

        let effective = fed.effective_policy("aur{team,private,open}").unwrap();
        assert!(effective
            .effective_admins()
            .contains(&"usr{root,corp,active}".to_string()));
        assert!(effective
            .effective_member_types()
            .contains(&"dev".to_string()));
        assert!(effective
            .effective_member_types()
            .contains(&"agt".to_string()));
    }

    #[test]
    fn federation_delegation_is_capability_and_expiry_scoped() {
        let mut fed = FederatedGovernor::new();
        fed.register_policy(sample_policy()).unwrap();

        let now = 100_000u128;
        let caps = BTreeSet::from([DelegatedCapability::AddMember]);
        fed.grant_delegation(
            "aur{finance,private,open}",
            "usr{alice,corp,active}",
            "agt{opsbot,corp,live}",
            caps,
            now + 50,
            now,
        )
        .unwrap();

        assert!(fed
            .is_authorized(
                "aur{finance,private,open}",
                "agt{opsbot,corp,live}",
                DelegatedCapability::AddMember,
                now + 10,
            )
            .unwrap());

        assert!(!fed
            .is_authorized(
                "aur{finance,private,open}",
                "agt{opsbot,corp,live}",
                DelegatedCapability::UpdatePolicy,
                now + 10,
            )
            .unwrap());

        assert!(!fed
            .is_authorized(
                "aur{finance,private,open}",
                "agt{opsbot,corp,live}",
                DelegatedCapability::AddMember,
                now + 60,
            )
            .unwrap());
    }

    #[test]
    fn federation_rejects_inheritance_cycles() {
        let mut fed = FederatedGovernor::new();
        fed.register_policy(
            GovernorPolicy::from_toml_str(
                r#"
                governor = "svc{gov,corp,live}"
                managed_aura = "aur{a,private,open}"
                admins = ["usr{alice,corp,active}"]
                "#,
            )
            .unwrap(),
        )
        .unwrap();
        fed.register_policy(
            GovernorPolicy::from_toml_str(
                r#"
                governor = "svc{gov,corp,live}"
                managed_aura = "aur{b,private,open}"
                admins = ["usr{alice,corp,active}"]
                "#,
            )
            .unwrap(),
        )
        .unwrap();

        fed.add_federation_link(FederationLink {
            parent_aura: "aur{a,private,open}".into(),
            child_aura: "aur{b,private,open}".into(),
            inherit_admins: true,
            inherit_member_types: true,
            inherit_allow_removal: true,
        })
        .unwrap();

        let err = fed.add_federation_link(FederationLink {
            parent_aura: "aur{b,private,open}".into(),
            child_aura: "aur{a,private,open}".into(),
            inherit_admins: true,
            inherit_member_types: false,
            inherit_allow_removal: false,
        });
        assert!(matches!(err, Err(GovernorError::InvalidPolicy(_))));
    }
}
