use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};
use veer_aura::{AuraGraph, AuraId};
use veer_resolve::{Endpoint, ResolveError, Resolver, ServiceBinding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldLaunchPlan {
    pub manifest: PathBuf,
    pub auras: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConnectPlan {
    pub service: String,
    pub caller_auras: Vec<String>,
    pub endpoint: EndpointPlan,
    pub object_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointPlan {
    pub node: String,
    pub transport: String,
    pub latency_ms: u32,
    pub healthy: bool,
}

#[derive(Default)]
pub struct VeerDeveloperSdk {
    aura_graph: AuraGraph,
    resolver: Resolver,
}

impl VeerDeveloperSdk {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_aura(&mut self, aura: &str) -> Result<String> {
        let aura = canonical_typed(aura, AddressType::Aura, "aura")?;
        let id = AuraId::parse(&aura)
            .map_err(|e| anyhow::anyhow!("parsing canonical aura id failed: {:?}", e))?;
        self.aura_graph
            .create_aura(id)
            .map_err(|e| anyhow::anyhow!("create aura failed: {:?}", e))?;
        Ok(aura)
    }

    pub fn join_aura_member(&mut self, aura: &str, member: &str) -> Result<bool> {
        let aura = canonical_typed(aura, AddressType::Aura, "aura")?;
        let member = canonical_member_target(member)?;
        let id = AuraId::parse(&aura)
            .map_err(|e| anyhow::anyhow!("parsing canonical aura id failed: {:?}", e))?;
        self.aura_graph
            .add_member_to_aura(&id, &member)
            .map_err(|e| anyhow::anyhow!("add member failed: {:?}", e))
    }

    pub fn member_auras(&self, member: &str) -> Result<Vec<String>> {
        let member = canonical_member_target(member)?;
        let out = self
            .aura_graph
            .auras_for_member(&member, true, now_unix_ms())
            .map_err(|e| anyhow::anyhow!("query member auras failed: {:?}", e))?;
        Ok(out.into_iter().map(|a| a.as_str().to_string()).collect())
    }

    pub fn aura_overlap_edges(&self) -> Vec<(String, String, Vec<String>)> {
        self.aura_graph
            .overlap_graph(true, now_unix_ms())
            .into_iter()
            .map(|e| (e.left.as_str().to_string(), e.right.as_str().to_string(), e.shared_members))
            .collect()
    }

    pub fn register_service_binding(
        &mut self,
        service: &str,
        endpoint: Endpoint,
        required_aura: Option<&str>,
    ) -> Result<()> {
        let service = canonical_typed(service, AddressType::Service, "service")?;
        let required_aura = required_aura
            .map(|a| canonical_typed(a, AddressType::Aura, "aura"))
            .transpose()?;

        self.resolver
            .register(
                &service,
                ServiceBinding {
                    endpoint,
                    required_aura,
                },
            )
            .map_err(|e| anyhow::anyhow!("register binding failed: {:?}", e))
    }

    pub fn plan_service_connect(&mut self, service: &str, caller_auras: &[String]) -> Result<ServiceConnectPlan> {
        let service = canonical_typed(service, AddressType::Service, "service")?;
        let caller_auras = canonicalize_auras(caller_auras)?;
        let aura_refs: Vec<&str> = caller_auras.iter().map(|s| s.as_str()).collect();

        let resolved = self
            .resolver
            .resolve(&service, &aura_refs)
            .map_err(map_resolve_error)?;

        Ok(ServiceConnectPlan {
            service,
            caller_auras,
            endpoint: EndpointPlan {
                node: resolved.selected.node,
                transport: resolved.selected.transport,
                latency_ms: resolved.selected.latency_ms,
                healthy: resolved.selected.healthy,
            },
            object_id: resolved.object_id,
        })
    }

    pub fn plan_fold_launch(&self, manifest: &Path, auras: &[String]) -> Result<FoldLaunchPlan> {
        if !manifest.exists() {
            bail!("manifest not found: {}", manifest.display());
        }
        let auras = canonicalize_auras(auras)?;
        Ok(FoldLaunchPlan {
            manifest: manifest.to_path_buf(),
            auras,
        })
    }

    pub fn merge_fold_auras(&self, existing: &[String], extra: &[String]) -> Result<Vec<String>> {
        let mut set = BTreeSet::new();
        let mut out = Vec::new();
        for aura in existing.iter().chain(extra.iter()) {
            let c = canonical_typed(aura, AddressType::Aura, "aura")?;
            if set.insert(c.clone()) {
                out.push(c);
            }
        }
        Ok(out)
    }
}

fn canonical_typed(input: &str, expected: AddressType, what: &str) -> Result<String> {
    let canonical = canonicalize(input)
        .map_err(|e| anyhow::anyhow!("invalid {} address {}: {}", what, input, e))?;
    let parsed = VasAddress::parse(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid {} address {}: {}", what, input, e))?;
    if parsed.kind != expected {
        bail!("{} address must use {}{{...}} type", what, expected.as_str());
    }
    Ok(canonical)
}

fn canonical_member_target(input: &str) -> Result<String> {
    let canonical = canonicalize(input)
        .map_err(|e| anyhow::anyhow!("invalid member target {}: {}", input, e))?;
    let parsed = VasAddress::parse(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid member target {}: {}", input, e))?;
    match parsed.kind {
        AddressType::User
        | AddressType::Device
        | AddressType::Fold
        | AddressType::Agent
        | AddressType::Service
        | AddressType::Node => Ok(canonical),
        _ => bail!("member target must be one of usr/dev/fld/agt/svc/nod"),
    }
}

fn canonicalize_auras(input: &[String]) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for aura in input {
        let c = canonical_typed(aura, AddressType::Aura, "aura")?;
        if seen.insert(c.clone()) {
            out.push(c);
        }
    }
    Ok(out)
}

fn map_resolve_error(err: ResolveError) -> anyhow::Error {
    match err {
        ResolveError::InvalidAddress => anyhow::anyhow!("invalid service address"),
        ResolveError::NotFound => anyhow::anyhow!("service not found"),
        ResolveError::NoHealthyEndpoint => anyhow::anyhow!("no healthy endpoint"),
        ResolveError::DeniedByAura => anyhow::anyhow!("denied by aura policy"),
    }
}

fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_endpoint(node: &str, latency_ms: u32, healthy: bool) -> Endpoint {
        Endpoint {
            node: node.to_string(),
            transport: "quic:4433".to_string(),
            latency_ms,
            healthy,
        }
    }

    #[test]
    fn sdk_handles_multi_aura_participation_and_overlap() {
        let mut sdk = VeerDeveloperSdk::new();
        sdk.create_aura("aur{design,private,open}").unwrap();
        sdk.create_aura("aur{ops,private,open}").unwrap();
        sdk.join_aura_member("aur{design,private,open}", "usr{alice,corp,active}")
            .unwrap();
        sdk.join_aura_member("aur{ops,private,open}", "usr{alice,corp,active}")
            .unwrap();

        let in_auras = sdk.member_auras("usr{alice,corp,active}").unwrap();
        assert_eq!(
            in_auras,
            vec![
                "aur{design,private,open}".to_string(),
                "aur{ops,private,open}".to_string(),
            ]
        );

        let edges = sdk.aura_overlap_edges();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "aur{design,private,open}");
        assert_eq!(edges[0].1, "aur{ops,private,open}");
        assert_eq!(edges[0].2, vec!["usr{alice,corp,active}".to_string()]);
    }

    #[test]
    fn sdk_plans_service_connect_with_aura_scoping() {
        let mut sdk = VeerDeveloperSdk::new();
        sdk.register_service_binding(
            "svc{render,company,live}",
            mk_endpoint("render-a.internal", 15, true),
            Some("aur{design,private,open}"),
        )
        .unwrap();
        sdk.register_service_binding(
            "svc{render,company,live}",
            mk_endpoint("render-b.internal", 7, true),
            Some("aur{design,private,open}"),
        )
        .unwrap();

        let plan = sdk
            .plan_service_connect(
                "svc{render,company,live}",
                &["aur{design,private,open}".to_string()],
            )
            .unwrap();

        assert_eq!(plan.endpoint.node, "render-b.internal");
        assert_eq!(plan.endpoint.latency_ms, 7);
        assert_eq!(plan.service, "svc{render,company,live}");
        assert!(!plan.caller_auras.is_empty());
    }

    #[test]
    fn sdk_fold_plan_dedups_and_validates_auras() {
        let sdk = VeerDeveloperSdk::new();
        let out = sdk
            .merge_fold_auras(
                &["aur{team,private,open}".to_string()],
                &[
                    "AUR{Team,Private,Open}".to_string(),
                    "aur{ops,private,open}".to_string(),
                ],
            )
            .unwrap();
        assert_eq!(
            out,
            vec![
                "aur{team,private,open}".to_string(),
                "aur{ops,private,open}".to_string(),
            ]
        );
    }
}
