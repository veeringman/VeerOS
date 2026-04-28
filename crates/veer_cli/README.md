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

Legacy DNS/IP fallback can be driven by a route map:

`$XDG_CONFIG_HOME/veeros/legacy-routes.toml` (fallback: `$HOME/.config/veeros/legacy-routes.toml`)

```toml
[services."svc{render,company,live}"]
dns = ["render.internal.example", "render.backup.example"]
ips = ["10.10.10.21", "10.10.10.22"]
port = 2232
```

With this file present, you can omit `--host` and/or `--port`:

```bash
veer connect svc{render,company,live} --aura aur{design,private,open}
```

### Legacy socket bridge

Classic TCP bridge for legacy clients:

```bash
veer gateway bridge --listen 127.0.0.1:19000 --target-host 10.10.10.21 --target-port 2232
```

This keeps older IP/socket-based clients working while moving service identity
to VAS-based `veer connect` flows.
