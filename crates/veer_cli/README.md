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
veer aura grant-agent aur{design,private,open} --agent agt{opsbot,corp,live} --expires-unix-ms 1790000000000 --scope observe --scope deploy
veer aura overlap
veer aura overlap --member usr{alice,corp,active}
veer aura leave aur{design,private,open}
```

Local Aura CLI state is stored at:

- `$XDG_STATE_HOME/veeros/aura-state.json`
- fallback: `$HOME/.local/state/veeros/aura-state.json`

`grant-agent` stores a temporary, scoped membership grant for an agent with an explicit expiry timestamp.

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

Resolver-backed selection is enabled by default via:

`$XDG_CONFIG_HOME/veeros/resolve-map.toml` (fallback: `$HOME/.config/veeros/resolve-map.toml`)

```toml
[[services."svc{render,company,live}"]]
node = "render-a.internal"
transport = "tcp:2232"
latency_ms = 8
healthy = true
required_aura = "aur{design,private,open}"

[[services."svc{render,company,live}"]]
node = "render-b.internal"
transport = "tcp:2232"
latency_ms = 14
healthy = true
```

Connect target selection order:

1. `--host/--port` explicit overrides
2. Resolver map (`resolve-map.toml`) via VeerResolve + Aura filtering
3. Legacy route map (`legacy-routes.toml`) DNS/IP fallback

You can override resolver map path with `--resolve-map <path>`.

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

### Quantum cloud execution (IBM)

Run a real quantum program template against IBM Quantum:

```bash
export IBM_QUANTUM_API_TOKEN="<your-token>"
export IBM_QUANTUM_INSTANCE="<your-instance>"

veer quantum ibm-run --program bell --backend ibm_brisbane --shots 1024
```

JSON output mode:

```bash
veer quantum ibm-run --program ghz3 --backend ibm_brisbane --shots 2048 --json
```

Supported built-in templates:

- `bell`
- `ghz3`
- `qft3`

### Legacy socket bridge

Classic TCP bridge for legacy clients:

```bash
veer gateway bridge --listen 127.0.0.1:19000 --target-host 10.10.10.21 --target-port 2232
```

This keeps older IP/socket-based clients working while moving service identity
to VAS-based `veer connect` flows.

### Decision trace debugger

Inspect solver + policy decisions from a graph snapshot:

```bash
veer trace decision \
	--graph ./graph-spec.toml \
	--source svc{render,company,live} \
	--edge-kind reachability \
	--target-kind node \
	--policy ./policy.toml
```

JSON output mode:

```bash
veer trace decision \
	--graph ./graph-spec.toml \
	--source svc{render,company,live} \
	--edge-kind reachability \
	--json
```

`graph-spec.toml` format:

```toml
[[vertices]]
id = "svc{render,company,live}"

[[vertices]]
id = "nod{edge-a,zone-1,ready}"
[vertices.attrs]
zone = "z1"

[[edges]]
from = "svc{render,company,live}"
to = "nod{edge-a,zone-1,ready}"
kind = "reachability"

[edges.weights]
latency = 8.0
trust = 0.9
cost = 0.2
affinity = 0.6
load = 0.4
```

Analyze governor audit logs for anomalies and policy suggestions:

```bash
veer trace governor \
	--audit ./governor-audit.jsonl \
	--limit 300
```

JSON output mode:

```bash
veer trace governor \
	--audit ./governor-audit.jsonl \
	--json
```
