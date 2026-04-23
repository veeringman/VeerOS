# EdgeFabric Integration Guide: VeerOS ESP32-C6 Virtual IoT Runtime

This document is the integration reference for running VeerOS ESP32-C6 virtual nodes under EdgeFabric using fold + veer-vm.

## Scope

This guide defines:

- Runtime topology (EdgeFabric -> fold -> veer-vm -> VeerOS guest)
- Required host prerequisites
- Lifecycle operations (create, start, observe, stop, cleanup)
- Scaling strategy and practical limits
- Error handling and recovery rules

## Runtime Topology

EdgeFabric controls virtual IoT instances by launching fold manifests that execute veer-vm in riscv32 software mode.

- Host process manager: `fold`
- VMM runtime: `veer-vm`
- Guest image: `kernel-qemu-esp32c6`
- Network backend: TAP (`tap0`) with virtio-net

A single instance maps to one fold record and one veer-vm process.

## Host Prerequisites

Run from repository root:

```sh
cd /home/vijay/rnd/VeerOS
cargo build -p fold_engine
cargo build -p veer_vm
```

Ensure the guest kernel exists:

```sh
test -f ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6
```

Ensure TAP exists (created once, reused):

```sh
sudo ip tuntap add dev tap0 mode tap user "$USER" 2>/dev/null || true
sudo ip link set tap0 up
```

## Instance Manifest Contract

EdgeFabric should materialize one TOML manifest per instance and call `fold spawn --manifest <file>`.

Minimal manifest template:

```toml
name = "edgefabric-node-001"
cmd  = "./target/debug/veer-vm"
args = [
  "--arch", "riscv32",
  "--cpu-throttle-ms", "20",
  "--kernel", "./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6",
  "--memory", "128",
  "--tap", "tap0",
]
hostname = "edgefabric-node-001"
workdir  = "/home/vijay/rnd/VeerOS"

[env]
PATH = "/usr/local/bin:/usr/bin:/bin"
TERM = "xterm-256color"

[namespaces]
pid   = true
mount = true
uts   = true
ipc   = true
net   = true
user  = true

[seccomp]
profile = "default"
```

## Lifecycle API (CLI-level)

EdgeFabric should use this lifecycle sequence.

### 1) Create/start

```sh
./target/debug/fold spawn --manifest /tmp/edgefabric-node-001.toml
```

Expected stdout:

- `fold <name> spawned (pid <pid>)`
- `log: <path>`

### 2) Read logs / readiness

```sh
./target/debug/fold logs edgefabric-node-001 | head -n 120
```

Treat instance as ready after these markers appear:

- `[veer-vm] rv32-soft: virtio-net tap=tap0 mac=`
- `[boot] starting scheduler`
- `[net] probing for VIRTIO-NET device...`
- `[net] found NIC`

### 3) List inventory

```sh
./target/debug/fold list
```

Use this as the source of truth for runtime state (`running` / `stopped`).

### 4) Stop

```sh
./target/debug/fold stop edgefabric-node-001
```

### 5) Remove state/log record

```sh
./target/debug/fold rm edgefabric-node-001 --force
```

## Batch Operations

### Start N instances

- Generate `edgefabric-node-001 .. edgefabric-node-NNN` manifests in `/tmp`
- Spawn with unique names
- Run readiness checks per instance in parallel with bounded concurrency

### Stop all EdgeFabric-managed instances

Naming convention recommended: prefix all names with `edgefabric-`.

```sh
./target/debug/fold list | awk 'NR>1 {print $1}' | grep '^edgefabric-' | while read -r n; do
  ./target/debug/fold stop "$n" >/dev/null 2>&1 || true
  ./target/debug/fold rm "$n" --force >/dev/null 2>&1 || true
done
```

## Resource and Scaling Guidance

Observed on this host profile:

- Practical sustained target: about 10 folds with `--cpu-throttle-ms 20`
- Hard-run target in stress sweep: about 15 folds before host CPU saturation
- Bottleneck: CPU, not memory

Recommendations:

- Default to `--cpu-throttle-ms 20`
- Keep steady-state pool <= 10 unless higher latency is acceptable
- For larger fleets, increase throttle (for example 30-40 ms) and re-benchmark

## Error Handling Rules

### `unexpected argument '--arch' found`

Cause: fold launched an outdated veer-vm binary.

Action:

- Use workspace binary path (`./target/debug/veer-vm`) in manifest `cmd`
- Rebuild: `cargo build -p veer_vm`

### `TUNSETIFF ... Device or resource busy`

Cause: TAP currently held/open elsewhere or conflicting setup.

Action:

- Stop all veer-vm/fold instances
- Reuse existing `tap0` if already present
- Avoid deleting/recreating TAP while instances are running

### Instance listed as `stopped` immediately after spawn

Action:

- Read logs: `fold logs <name>`
- Typical root causes: wrong kernel path, wrong `workdir`, missing TAP

## Monitoring Signals for EdgeFabric

Per-instance signals:

- `fold list` state
- log markers (`found NIC`, scheduler started)
- process liveness by fold PID

Host-level signals:

- total `veer-vm` CPU from `ps -C veer-vm -o %cpu=`
- `MemAvailable` from `/proc/meminfo`
- load average from `uptime`

Suggested autoscaling guardrails:

- Soft ceiling: host CPU <= 85%
- Memory floor: `MemAvailable >= 1024 MB`

## Recommended EdgeFabric Adapter Contract

For each node record, persist:

- `name`
- `manifest_path`
- `fold_pid`
- `log_path`
- `mac` (parsed from logs)
- `state` (`starting`, `running`, `stopped`, `error`)
- `last_error`

State transitions:

- `starting` -> `running` on readiness markers
- `starting` -> `error` on early stop or fatal logs
- `running` -> `stopped` on stop/rm

## Adapter Schema (JSON/YAML)

Use the following as a reference contract between EdgeFabric control-plane
components and the fold + veer-vm runtime adapter.

### JSON (request)

```json
{
  "name": "edgefabric-node-001",
  "kernel": "./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6",
  "memory_mib": 128,
  "tap": "tap0",
  "cpu_throttle_ms": 20,
  "user_ns": true,
  "env": {
    "TERM": "xterm-256color"
  },
  "labels": {
    "tenant": "acme",
    "site": "rack-a"
  }
}
```

### JSON (state record)

```json
{
  "name": "edgefabric-node-001",
  "manifest_path": "/tmp/edgefabric-node-001.toml",
  "fold_pid": 31245,
  "log_path": "/home/vijay/.local/state/veeros/fold/edgefabric-node-001.log",
  "mac": "02:00:4a:b3:1f:90",
  "state": "running",
  "last_error": "",
  "created_at": "2026-04-24T10:58:12Z",
  "updated_at": "2026-04-24T10:58:21Z"
}
```

### YAML (request)

```yaml
name: edgefabric-node-001
kernel: ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6
memory_mib: 128
tap: tap0
cpu_throttle_ms: 20
user_ns: true
env:
  TERM: xterm-256color
labels:
  tenant: acme
  site: rack-a
```

### YAML (state record)

```yaml
name: edgefabric-node-001
manifest_path: /tmp/edgefabric-node-001.toml
fold_pid: 31245
log_path: /home/vijay/.local/state/veeros/fold/edgefabric-node-001.log
mac: 02:00:4a:b3:1f:90
state: running
last_error: ""
created_at: 2026-04-24T10:58:12Z
updated_at: 2026-04-24T10:58:21Z
```

### Field semantics

- `name`: globally unique instance name in fold state dir.
- `kernel`: riscv32 guest image path on host.
- `memory_mib`: guest memory passed to `veer-vm --memory`.
- `tap`: pre-created TAP interface (for example `tap0`).
- `cpu_throttle_ms`: veer-vm busy-loop throttle; larger value lowers host CPU.
- `state`: one of `starting`, `running`, `stopped`, `error`.
- `last_error`: last terminal error string, empty when healthy.

## Security Notes

- `user = true` keeps runtime rootless-friendly in user namespaces
- `seccomp.profile = "default"` is enabled in fold manifest
- TAP must be pre-provisioned by host operator

## Quick Smoke Test

Run one instance end-to-end:

```sh
./target/debug/fold vm spawn \
  --arch riscv32 \
  --kernel ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
  --memory 128 \
  --tap tap0 \
  --user-ns \
  --name edgefabric-smoke

./target/debug/fold logs edgefabric-smoke | head -n 120
./target/debug/fold stop edgefabric-smoke
./target/debug/fold rm edgefabric-smoke --force
```
