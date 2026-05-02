//! VAS (VeerOS Addressing Standard) parser and canonicalizer.
//!
//! Canonical form:
//!   type{atom1,atom2,...}
//!
//! Reserved types:
//!   usr, dev, fld, aur, svc, vlt, agt, zon, nod, evt

use core::fmt;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressType {
    User,
    Device,
    Fold,
    Aura,
    Service,
    Vault,
    Agent,
    Zone,
    Node,
    Event,
}

impl AddressType {
    pub fn as_str(self) -> &'static str {
        match self {
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

    fn parse(raw: &str) -> Result<Self, ParseError> {
        match raw {
            "usr" => Ok(AddressType::User),
            "dev" => Ok(AddressType::Device),
            "fld" => Ok(AddressType::Fold),
            "aur" => Ok(AddressType::Aura),
            "svc" => Ok(AddressType::Service),
            "vlt" => Ok(AddressType::Vault),
            "agt" => Ok(AddressType::Agent),
            "zon" => Ok(AddressType::Zone),
            "nod" => Ok(AddressType::Node),
            "evt" => Ok(AddressType::Event),
            _ => Err(ParseError::UnknownType),
        }
    }

    pub fn code(self) -> u8 {
        match self {
            AddressType::User => 1,
            AddressType::Device => 2,
            AddressType::Fold => 3,
            AddressType::Aura => 4,
            AddressType::Service => 5,
            AddressType::Vault => 6,
            AddressType::Agent => 7,
            AddressType::Zone => 8,
            AddressType::Node => 9,
            AddressType::Event => 10,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtomRegistry {
    ids: HashMap<String, u32>,
    atoms: Vec<String>,
}

impl AtomRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, atom: &str) -> u32 {
        if let Some(id) = self.ids.get(atom) {
            return *id;
        }
        let id = (self.atoms.len() as u32) + 1;
        self.ids.insert(atom.to_string(), id);
        self.atoms.push(atom.to_string());
        id
    }

    pub fn id_of(&self, atom: &str) -> Option<u32> {
        self.ids.get(atom).copied()
    }

    pub fn atom_of(&self, id: u32) -> Option<&str> {
        if id == 0 {
            return None;
        }
        self.atoms.get((id - 1) as usize).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryAddress {
    pub type_code: u8,
    pub atom_ids: Vec<u32>,
}

impl BinaryAddress {
    /// Stable byte format:
    /// [type_code:1][atom_count:1][atom_id_1:4][atom_id_2:4]...
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + (self.atom_ids.len() * 4));
        out.push(self.type_code);
        out.push(self.atom_ids.len() as u8);
        for id in &self.atom_ids {
            out.extend_from_slice(&id.to_be_bytes());
        }
        out
    }

    /// Compact deterministic ID derived from encoded bytes.
    pub fn compact_id(&self) -> u64 {
        fnv1a64(&self.to_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VasAddress {
    pub kind: AddressType,
    pub atoms: Vec<String>,
}

impl VasAddress {
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let s = input.trim();
        let open = s.find('{').ok_or(ParseError::MissingOpenBrace)?;
        let close = s.rfind('}').ok_or(ParseError::MissingCloseBrace)?;

        if close <= open {
            return Err(ParseError::InvalidBraceOrder);
        }

        if close != s.len() - 1 {
            return Err(ParseError::TrailingContent);
        }

        let raw_type = s[..open].trim().to_ascii_lowercase();
        let kind = AddressType::parse(&raw_type)?;

        let body = &s[open + 1..close];
        if body.trim().is_empty() {
            return Err(ParseError::EmptyAtoms);
        }

        let mut atoms = Vec::new();
        for raw_atom in body.split(',') {
            let atom = raw_atom.trim().to_ascii_lowercase();
            if atom.is_empty() {
                return Err(ParseError::EmptyAtom);
            }
            if !is_valid_atom(&atom) {
                return Err(ParseError::InvalidAtom);
            }
            atoms.push(atom);
        }

        Ok(Self { kind, atoms })
    }

    pub fn canonical(&self) -> String {
        format!("{}{{{}}}", self.kind.as_str(), self.atoms.join(","))
    }
}

impl fmt::Display for VasAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    MissingOpenBrace,
    MissingCloseBrace,
    InvalidBraceOrder,
    TrailingContent,
    UnknownType,
    EmptyAtoms,
    EmptyAtom,
    InvalidAtom,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            ParseError::MissingOpenBrace => "missing '{'",
            ParseError::MissingCloseBrace => "missing '}'",
            ParseError::InvalidBraceOrder => "invalid brace order",
            ParseError::TrailingContent => "trailing content after '}'",
            ParseError::UnknownType => "unknown address type",
            ParseError::EmptyAtoms => "missing atoms list",
            ParseError::EmptyAtom => "empty atom",
            ParseError::InvalidAtom => "invalid atom",
        };
        f.write_str(msg)
    }
}

fn is_valid_atom(atom: &str) -> bool {
    // Atoms are intentionally conservative in v1 for deterministic parsing and
    // fast binary encoding:
    //   - alphanumeric
    //   - '-', '_', '.'
    //   - wildcard '*'
    atom.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '*')
}

pub fn canonicalize(input: &str) -> Result<String, ParseError> {
    VasAddress::parse(input).map(|a| a.canonical())
}

pub fn encode_with_registry(address: &VasAddress, registry: &mut AtomRegistry) -> BinaryAddress {
    let atom_ids = address
        .atoms
        .iter()
        .map(|atom| registry.intern(atom))
        .collect::<Vec<_>>();

    BinaryAddress {
        type_code: address.kind.code(),
        atom_ids,
    }
}

pub fn parse_and_encode(
    input: &str,
    registry: &mut AtomRegistry,
) -> Result<BinaryAddress, ParseError> {
    let addr = VasAddress::parse(input)?;
    Ok(encode_with_registry(&addr, registry))
}

fn fnv1a64(data: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_canonicalizes_with_normalization() {
        let addr = VasAddress::parse(" SVC { Render , Company , LIVE } ").unwrap();
        assert_eq!(addr.kind, AddressType::Service);
        assert_eq!(addr.atoms, vec!["render", "company", "live"]);
        assert_eq!(addr.canonical(), "svc{render,company,live}");
    }

    #[test]
    fn accepts_wildcard_atom() {
        let addr = VasAddress::parse("svc{*,company,live}").unwrap();
        assert_eq!(addr.canonical(), "svc{*,company,live}");
    }

    #[test]
    fn rejects_unknown_type() {
        let err = VasAddress::parse("abc{foo}").unwrap_err();
        assert_eq!(err, ParseError::UnknownType);
    }

    #[test]
    fn rejects_empty_atom() {
        let err = VasAddress::parse("svc{foo,,bar}").unwrap_err();
        assert_eq!(err, ParseError::EmptyAtom);
    }

    #[test]
    fn rejects_invalid_atom_chars() {
        let err = VasAddress::parse("svc{foo/bar}").unwrap_err();
        assert_eq!(err, ParseError::InvalidAtom);
    }

    #[test]
    fn rejects_trailing_content() {
        let err = VasAddress::parse("svc{foo}x").unwrap_err();
        assert_eq!(err, ParseError::TrailingContent);
    }

    #[test]
    fn helper_canonicalize() {
        let s = canonicalize("aur{Design,PRIVATE,open}").unwrap();
        assert_eq!(s, "aur{design,private,open}");
    }

    #[test]
    fn registry_intern_is_stable() {
        let mut reg = AtomRegistry::new();
        let a1 = reg.intern("render");
        let a2 = reg.intern("render");
        let b = reg.intern("company");
        assert_eq!(a1, a2);
        assert_ne!(a1, b);
        assert_eq!(reg.id_of("render"), Some(a1));
        assert_eq!(reg.atom_of(b), Some("company"));
    }

    #[test]
    fn encode_uses_registry_ids() {
        let mut reg = AtomRegistry::new();
        let addr = VasAddress::parse("svc{render,company,live}").unwrap();
        let enc = encode_with_registry(&addr, &mut reg);
        assert_eq!(enc.type_code, AddressType::Service.code());
        assert_eq!(enc.atom_ids.len(), 3);
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn compact_id_is_deterministic() {
        let mut r1 = AtomRegistry::new();
        let mut r2 = AtomRegistry::new();
        let e1 = parse_and_encode("svc{render,company,live}", &mut r1).unwrap();
        let e2 = parse_and_encode("svc{render,company,live}", &mut r2).unwrap();
        assert_eq!(e1.to_bytes(), e2.to_bytes());
        assert_eq!(e1.compact_id(), e2.compact_id());
    }
}
