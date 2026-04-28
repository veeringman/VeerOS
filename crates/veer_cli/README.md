# veer CLI

Unified VeerOS CLI for Aura workflows, fold launch overlays, and service connect flows.

## Build

```bash
cargo build -p veer_cli
```

Binary path:

```bash
./target/debug/veer
```

## Commands

### Aura management

```bash
veer aura create aur{design,private,open} --name "Design Aura"
veer aura join aur{design,private,open}
veer aura share aur{design,private,open} --with usr{alice,corp,active} --with usr{bob,corp,active}
veer aura leave aur{design,private,open}
```

Local Aura CLI state is stored at:

- `$XDG_STATE_HOME/veeros/aura-state.json`
- fallback: `$HOME/.local/state/veeros/aura-state.json`

### Fold launch with Aura overlays

```bash
veer fold launch --manifest crates/fold_engine/examples/rootless.toml --aura aur{design,private,open}
```

This command canonicalizes Aura addresses, merges them with manifest `auras`, writes a temporary manifest, and delegates launch to:

```bash
fold spawn --manifest <temp-file>
```

### Service connect flow

```bash
veer connect svc{render,company,live} --host 127.0.0.1 --port 2232 --aura aur{design,private,open}
```

The command validates service and Aura addresses, exports:

- `VEER_SERVICE`
- `VEER_CALLER_AURAS`

then delegates transport to:

```bash
veer-connect shell <host> <port>
```
