# veer_sdk

High-level Rust SDK for VeerOS orchestration workflows.

## Scope

- Aura orchestration: create Aura, join members, inspect overlap graph
- Service orchestration: register bindings and resolve connect plans with Aura policy scoping
- Fold orchestration: build canonical launch plans with deduplicated Aura memberships

## Usage

```rust
use veer_resolve::Endpoint;
use veer_sdk::VeerDeveloperSdk;

let mut sdk = VeerDeveloperSdk::new();
sdk.create_aura("aur{design,private,open}")?;
sdk.join_aura_member("aur{design,private,open}", "usr{alice,corp,active}")?;

sdk.register_service_binding(
    "svc{render,company,live}",
    Endpoint {
        node: "render-a.internal".into(),
        transport: "quic:4433".into(),
        latency_ms: 8,
        healthy: true,
    },
    Some("aur{design,private,open}"),
)?;

let connect = sdk.plan_service_connect(
    "svc{render,company,live}",
    &["aur{design,private,open}".to_string()],
)?;

assert_eq!(connect.service, "svc{render,company,live}");
# Ok::<(), anyhow::Error>(())
```
