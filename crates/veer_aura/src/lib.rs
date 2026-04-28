//! VeerOS Aura object model.
//!
//! Provides:
//! - Aura identity (`aur{...}`) validation
//! - Aura member set management
//! - Parent/child Aura hierarchy with cycle protection
//! - Governor reference for policy/trust control plane binding

use std::collections::{BTreeSet, HashMap};

use vas::{canonicalize, VasAddress, AddressType};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuraId(String);

impl AuraId {
    pub fn parse(input: &str) -> Result<Self, AuraError> {
        let canonical = canonicalize(input).map_err(|_| AuraError::InvalidAddress)?;
        let addr = VasAddress::parse(&canonical).map_err(|_| AuraError::InvalidAddress)?;
        if addr.kind != AddressType::Aura {
            return Err(AuraError::NotAuraAddress);
        }
        Ok(Self(canonical))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aura {
    pub id: AuraId,
    pub display_name: Option<String>,
    pub governor_ref: Option<String>,
    pub parent: Option<AuraId>,
    pub children: BTreeSet<AuraId>,
    pub members: BTreeSet<String>,
    pub temporary_members: HashMap<String, TemporaryMembership>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporaryMembership {
    pub granted_by: String,
    pub expires_unix_ms: u128,
    pub scopes: BTreeSet<String>,
}

impl Aura {
    pub fn new(id: AuraId) -> Self {
        Self {
            id,
            display_name: None,
            governor_ref: None,
            parent: None,
            children: BTreeSet::new(),
            members: BTreeSet::new(),
            temporary_members: HashMap::new(),
        }
    }

    pub fn set_display_name(&mut self, name: impl Into<String>) {
        self.display_name = Some(name.into());
    }

    pub fn clear_display_name(&mut self) {
        self.display_name = None;
    }

    pub fn set_governor_ref(&mut self, governor_ref: impl Into<String>) -> Result<(), AuraError> {
        let canonical = canonicalize(&governor_ref.into()).map_err(|_| AuraError::InvalidAddress)?;
        self.governor_ref = Some(canonical);
        Ok(())
    }

    pub fn clear_governor_ref(&mut self) {
        self.governor_ref = None;
    }

    pub fn add_member(&mut self, member_address: &str) -> Result<bool, AuraError> {
        let canonical = canonicalize(member_address).map_err(|_| AuraError::InvalidAddress)?;
        let addr = VasAddress::parse(&canonical).map_err(|_| AuraError::InvalidAddress)?;
        match addr.kind {
            AddressType::User
            | AddressType::Device
            | AddressType::Fold
            | AddressType::Agent
            | AddressType::Service
            | AddressType::Node => Ok(self.members.insert(canonical)),
            _ => Err(AuraError::InvalidMemberType),
        }
    }

    pub fn remove_member(&mut self, member_address: &str) -> Result<bool, AuraError> {
        let canonical = canonicalize(member_address).map_err(|_| AuraError::InvalidAddress)?;
        let removed_static = self.members.remove(&canonical);
        let removed_temp = self.temporary_members.remove(&canonical).is_some();
        Ok(removed_static || removed_temp)
    }

    pub fn contains_member(&self, member_address: &str) -> bool {
        canonicalize(member_address)
            .ok()
            .map(|s| {
                self.members.contains(&s)
                    || self
                        .temporary_members
                        .get(&s)
                        .map(|m| m.expires_unix_ms > now_unix_ms())
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub fn contains_member_at(&self, member_address: &str, now_unix_ms: u128) -> bool {
        canonicalize(member_address)
            .ok()
            .map(|s| {
                self.members.contains(&s)
                    || self
                        .temporary_members
                        .get(&s)
                        .map(|m| m.expires_unix_ms > now_unix_ms)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub fn temporary_membership(&self, member_address: &str) -> Option<&TemporaryMembership> {
        let canonical = canonicalize(member_address).ok()?;
        self.temporary_members.get(&canonical)
    }

    pub fn add_temporary_member(
        &mut self,
        member_address: &str,
        granted_by: &str,
        expires_unix_ms: u128,
        scopes: &[String],
        now_unix_ms: u128,
    ) -> Result<bool, AuraError> {
        let canonical_member = canonicalize(member_address).map_err(|_| AuraError::InvalidAddress)?;
        let member = VasAddress::parse(&canonical_member).map_err(|_| AuraError::InvalidAddress)?;
        if member.kind != AddressType::Agent {
            return Err(AuraError::NotAgentAddress);
        }

        let canonical_granted_by = canonicalize(granted_by).map_err(|_| AuraError::InvalidAddress)?;
        if expires_unix_ms <= now_unix_ms {
            return Err(AuraError::InvalidExpiry);
        }

        let mut norm_scopes = BTreeSet::new();
        for scope in scopes {
            let s = scope.trim().to_ascii_lowercase();
            if s.is_empty() {
                return Err(AuraError::InvalidScope);
            }
            norm_scopes.insert(s);
        }
        if norm_scopes.is_empty() {
            return Err(AuraError::InvalidScope);
        }

        let next = TemporaryMembership {
            granted_by: canonical_granted_by,
            expires_unix_ms,
            scopes: norm_scopes,
        };

        let changed = self.temporary_members.get(&canonical_member) != Some(&next);
        self.temporary_members.insert(canonical_member, next);
        Ok(changed)
    }

    pub fn prune_expired_temporary_members(&mut self, now_unix_ms: u128) -> usize {
        let before = self.temporary_members.len();
        self.temporary_members
            .retain(|_, grant| grant.expires_unix_ms > now_unix_ms);
        before.saturating_sub(self.temporary_members.len())
    }
}

fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct AuraGraph {
    auras: HashMap<AuraId, Aura>,
}

impl AuraGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_aura(&mut self, id: AuraId) -> Result<(), AuraError> {
        if self.auras.contains_key(&id) {
            return Err(AuraError::AlreadyExists);
        }
        self.auras.insert(id.clone(), Aura::new(id));
        Ok(())
    }

    pub fn get(&self, id: &AuraId) -> Option<&Aura> {
        self.auras.get(id)
    }

    pub fn get_mut(&mut self, id: &AuraId) -> Option<&mut Aura> {
        self.auras.get_mut(id)
    }

    pub fn set_parent(&mut self, child: &AuraId, parent: Option<&AuraId>) -> Result<(), AuraError> {
        if !self.auras.contains_key(child) {
            return Err(AuraError::NotFound);
        }
        if let Some(p) = parent {
            if !self.auras.contains_key(p) {
                return Err(AuraError::NotFound);
            }
            if child == p {
                return Err(AuraError::CycleDetected);
            }
            if self.is_descendant_of(p, child) {
                return Err(AuraError::CycleDetected);
            }
        }

        // Detach existing parent link.
        let current_parent = self.auras.get(child).and_then(|a| a.parent.clone());
        if let Some(old_parent) = current_parent {
            if let Some(p) = self.auras.get_mut(&old_parent) {
                p.children.remove(child);
            }
        }

        // Attach new parent link.
        if let Some(p) = parent {
            if let Some(parent_aura) = self.auras.get_mut(p) {
                parent_aura.children.insert(child.clone());
            }
        }

        if let Some(child_aura) = self.auras.get_mut(child) {
            child_aura.parent = parent.cloned();
        }

        Ok(())
    }

    pub fn auras(&self) -> impl Iterator<Item = &Aura> {
        self.auras.values()
    }

    fn is_descendant_of(&self, candidate_child: &AuraId, candidate_ancestor: &AuraId) -> bool {
        let mut cur = Some(candidate_child.clone());
        while let Some(id) = cur {
            if &id == candidate_ancestor {
                return true;
            }
            cur = self.auras.get(&id).and_then(|a| a.parent.clone());
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuraError {
    InvalidAddress,
    NotAuraAddress,
    InvalidMemberType,
    NotAgentAddress,
    InvalidExpiry,
    InvalidScope,
    NotFound,
    AlreadyExists,
    CycleDetected,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aura_id_requires_aur_type() {
        let ok = AuraId::parse("AUR{design,private,open}").unwrap();
        assert_eq!(ok.as_str(), "aur{design,private,open}");

        let bad = AuraId::parse("svc{render,company,live}").unwrap_err();
        assert_eq!(bad, AuraError::NotAuraAddress);
    }

    #[test]
    fn aura_member_types_are_filtered() {
        let id = AuraId::parse("aur{team,private,open}").unwrap();
        let mut aura = Aura::new(id);

        assert!(aura.add_member("usr{vijay,home,active}").unwrap());
        assert!(aura.add_member("fld{worker,gpu,warm}").unwrap());
        assert_eq!(aura.members.len(), 2);
        assert!(aura.contains_member("USR{vijay,home,active}"));

        let err = aura.add_member("vlt{finance,payroll,2026}").unwrap_err();
        assert_eq!(err, AuraError::InvalidMemberType);
    }

    #[test]
    fn graph_parent_child_links_update_both_sides() {
        let root = AuraId::parse("aur{company,private,open}").unwrap();
        let team = AuraId::parse("aur{team,private,open}").unwrap();

        let mut g = AuraGraph::new();
        g.create_aura(root.clone()).unwrap();
        g.create_aura(team.clone()).unwrap();

        g.set_parent(&team, Some(&root)).unwrap();

        let root_a = g.get(&root).unwrap();
        let team_a = g.get(&team).unwrap();
        assert!(root_a.children.contains(&team));
        assert_eq!(team_a.parent.as_ref(), Some(&root));
    }

    #[test]
    fn graph_cycle_is_rejected() {
        let a = AuraId::parse("aur{a,private,open}").unwrap();
        let b = AuraId::parse("aur{b,private,open}").unwrap();
        let c = AuraId::parse("aur{c,private,open}").unwrap();

        let mut g = AuraGraph::new();
        g.create_aura(a.clone()).unwrap();
        g.create_aura(b.clone()).unwrap();
        g.create_aura(c.clone()).unwrap();

        g.set_parent(&b, Some(&a)).unwrap();
        g.set_parent(&c, Some(&b)).unwrap();

        let err = g.set_parent(&a, Some(&c)).unwrap_err();
        assert_eq!(err, AuraError::CycleDetected);
    }

    #[test]
    fn governor_ref_is_canonicalized() {
        let id = AuraId::parse("aur{finance,private,open}").unwrap();
        let mut aura = Aura::new(id);
        aura.set_governor_ref("SVC{governor,company,live}").unwrap();
        assert_eq!(aura.governor_ref.as_deref(), Some("svc{governor,company,live}"));
    }

    #[test]
    fn temporary_agent_membership_requires_agent_scope_and_future_expiry() {
        let id = AuraId::parse("aur{ops,private,open}").unwrap();
        let mut aura = Aura::new(id);
        let now = 1_000u128;

        let err = aura
            .add_temporary_member(
                "usr{alice,corp,active}",
                "usr{owner,corp,active}",
                now + 10,
                &["observe".to_string()],
                now,
            )
            .unwrap_err();
        assert_eq!(err, AuraError::NotAgentAddress);

        let err = aura
            .add_temporary_member(
                "agt{bot,ops,live}",
                "usr{owner,corp,active}",
                now,
                &["observe".to_string()],
                now,
            )
            .unwrap_err();
        assert_eq!(err, AuraError::InvalidExpiry);

        let err = aura
            .add_temporary_member(
                "agt{bot,ops,live}",
                "usr{owner,corp,active}",
                now + 10,
                &[],
                now,
            )
            .unwrap_err();
        assert_eq!(err, AuraError::InvalidScope);
    }

    #[test]
    fn temporary_agent_membership_expires_and_prunes() {
        let id = AuraId::parse("aur{ops,private,open}").unwrap();
        let mut aura = Aura::new(id);
        let now = 5_000u128;

        let changed = aura
            .add_temporary_member(
                "agt{bot,ops,live}",
                "usr{owner,corp,active}",
                now + 20,
                &["observe".to_string(), "deploy".to_string()],
                now,
            )
            .unwrap();
        assert!(changed);
        assert!(aura.contains_member_at("agt{bot,ops,live}", now + 10));
        assert!(!aura.contains_member_at("agt{bot,ops,live}", now + 25));

        let pruned = aura.prune_expired_temporary_members(now + 25);
        assert_eq!(pruned, 1);
        assert!(aura.temporary_membership("agt{bot,ops,live}").is_none());
    }
}
