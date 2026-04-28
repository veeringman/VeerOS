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
        Ok(self.members.remove(&canonical))
    }

    pub fn contains_member(&self, member_address: &str) -> bool {
        canonicalize(member_address)
            .ok()
            .map(|s| self.members.contains(&s))
            .unwrap_or(false)
    }
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
}
