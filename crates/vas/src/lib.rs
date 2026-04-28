//! VAS (VeerOS Addressing Standard) parser and canonicalizer.
//!
//! Canonical form:
//!   type{atom1,atom2,...}
//!
//! Reserved types:
//!   usr, dev, fld, aur, svc, vlt, agt, zon, nod, evt

use core::fmt;

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
}
