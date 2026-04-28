//! VeerResolve MVP.
//!
//! Local resolver for VAS addresses with:
//! - canonical address lookup
//! - compact deterministic object IDs
//! - simple in-memory resolve cache
//! - Aura-scoped policy filtering

use std::collections::HashMap;

use vas::{canonicalize, parse_and_encode, AtomRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub node: String,
    pub transport: String,
    pub latency_ms: u32,
    pub healthy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceBinding {
    pub endpoint: Endpoint,
    /// Optional required Aura. If set, caller must include this Aura.
    pub required_aura: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveResult {
    pub canonical_address: String,
    pub object_id: u64,
    pub selected: Endpoint,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolveStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub denied_by_aura: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    InvalidAddress,
    NotFound,
    NoHealthyEndpoint,
    DeniedByAura,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    value: ResolveResult,
}

#[derive(Default)]
pub struct Resolver {
    /// canonical address -> candidate bindings
    registry: HashMap<String, Vec<ServiceBinding>>,
    /// cache key = canonical address + sorted caller auras
    cache: HashMap<String, CacheEntry>,
    atom_registry: AtomRegistry,
    stats: ResolveStats,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, address: &str, binding: ServiceBinding) -> Result<(), ResolveError> {
        let canonical = canonicalize(address).map_err(|_| ResolveError::InvalidAddress)?;
        self.registry.entry(canonical).or_default().push(binding);
        Ok(())
    }

    pub fn resolve(
        &mut self,
        address: &str,
        caller_auras: &[&str],
    ) -> Result<ResolveResult, ResolveError> {
        let canonical = canonicalize(address).map_err(|_| ResolveError::InvalidAddress)?;
        let cache_key = Self::cache_key(&canonical, caller_auras);

        if let Some(entry) = self.cache.get(&cache_key) {
            self.stats.cache_hits += 1;
            return Ok(entry.value.clone());
        }
        self.stats.cache_misses += 1;

        let candidates = self.registry.get(&canonical).ok_or(ResolveError::NotFound)?;
        let mut aura_denied = false;

        let allowed_healthy: Vec<&ServiceBinding> = candidates
            .iter()
            .filter(|b| {
                if let Some(required) = &b.required_aura {
                    let allowed = caller_auras.iter().any(|a| *a == required);
                    if !allowed {
                        aura_denied = true;
                    }
                    allowed
                } else {
                    true
                }
            })
            .filter(|b| b.endpoint.healthy)
            .collect();

        if allowed_healthy.is_empty() {
            if aura_denied {
                self.stats.denied_by_aura += 1;
                return Err(ResolveError::DeniedByAura);
            }
            return Err(ResolveError::NoHealthyEndpoint);
        }

        let selected = allowed_healthy
            .into_iter()
            .min_by_key(|b| b.endpoint.latency_ms)
            .expect("non-empty checked")
            .endpoint
            .clone();

        let resolved = ResolveResult {
            canonical_address: canonical.clone(),
            object_id: object_id_for(&canonical, &mut self.atom_registry)
                .expect("canonical address must be re-parseable"),
            selected,
        };

        self.cache.insert(cache_key, CacheEntry { value: resolved.clone() });
        Ok(resolved)
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn stats(&self) -> &ResolveStats {
        &self.stats
    }

    fn cache_key(canonical: &str, caller_auras: &[&str]) -> String {
        let mut auras: Vec<&str> = caller_auras.to_vec();
        auras.sort_unstable();
        auras.dedup();
        format!("{}|{}", canonical, auras.join(","))
    }
}

/// Deterministic compact object ID for MVP.
///
/// Encodes canonical VAS address into compact binary form (type code + atom
/// IDs via registry), then derives a stable 64-bit compact ID from bytes.
pub fn object_id_for(canonical_address: &str, registry: &mut AtomRegistry) -> Result<u64, ResolveError> {
    let enc = parse_and_encode(canonical_address, registry).map_err(|_| ResolveError::InvalidAddress)?;
    Ok(enc.compact_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(node: &str, lat: u32, healthy: bool) -> Endpoint {
        Endpoint {
            node: node.into(),
            transport: "quic".into(),
            latency_ms: lat,
            healthy,
        }
    }

    #[test]
    fn resolves_with_canonicalization() {
        let mut r = Resolver::new();
        r.register(
            "svc{render,company,live}",
            ServiceBinding { endpoint: ep("nod-a", 12, true), required_aura: None },
        )
        .unwrap();

        let out = r.resolve(" SVC{Render,Company,Live}", &[]).unwrap();
        assert_eq!(out.canonical_address, "svc{render,company,live}");
        assert_eq!(out.selected.node, "nod-a");
    }

    #[test]
    fn picks_lowest_latency_healthy_candidate() {
        let mut r = Resolver::new();
        r.register(
            "svc{render,company,live}",
            ServiceBinding { endpoint: ep("nod-slow", 40, true), required_aura: None },
        )
        .unwrap();
        r.register(
            "svc{render,company,live}",
            ServiceBinding { endpoint: ep("nod-fast", 8, true), required_aura: None },
        )
        .unwrap();
        r.register(
            "svc{render,company,live}",
            ServiceBinding { endpoint: ep("nod-down", 1, false), required_aura: None },
        )
        .unwrap();

        let out = r.resolve("svc{render,company,live}", &[]).unwrap();
        assert_eq!(out.selected.node, "nod-fast");
    }

    #[test]
    fn enforces_aura_scoped_lookup() {
        let mut r = Resolver::new();
        r.register(
            "svc{payroll,corp,live}",
            ServiceBinding {
                endpoint: ep("nod-secure", 3, true),
                required_aura: Some("aur{finance,private,open}".into()),
            },
        )
        .unwrap();

        let denied = r.resolve("svc{payroll,corp,live}", &["aur{design,private,open}"]);
        assert_eq!(denied.unwrap_err(), ResolveError::DeniedByAura);

        let allowed = r
            .resolve(
                "svc{payroll,corp,live}",
                &["aur{finance,private,open}", "aur{company,private,open}"],
            )
            .unwrap();
        assert_eq!(allowed.selected.node, "nod-secure");
    }

    #[test]
    fn caches_by_address_and_aura_set() {
        let mut r = Resolver::new();
        r.register(
            "svc{render,company,live}",
            ServiceBinding { endpoint: ep("nod-a", 12, true), required_aura: None },
        )
        .unwrap();

        let _ = r.resolve("svc{render,company,live}", &["aur{a,b,c}"]).unwrap();
        let _ = r.resolve("svc{render,company,live}", &["aur{a,b,c}"]).unwrap();

        assert_eq!(r.stats().cache_misses, 1);
        assert_eq!(r.stats().cache_hits, 1);
    }

    #[test]
    fn object_id_is_deterministic() {
        let mut reg = AtomRegistry::new();
        let a = object_id_for("svc{render,company,live}", &mut reg).unwrap();
        let b = object_id_for("svc{render,company,live}", &mut reg).unwrap();
        assert_eq!(a, b);
    }
}
