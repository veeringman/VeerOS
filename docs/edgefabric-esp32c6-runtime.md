# EdgeFabric Integration Guide: VeerOS ESP32-C6 VirtualIoT Backend

This document is the integration reference for creating and managing VeerOS ESP32-C6
**VirtualIoT** backends under EdgeFabric using `veer-vm` (directly or via `fold`).

A VirtualIoT is a software-only representation of an ESP32-C6 IoT node: it runs the
identical VeerOS kernel image as real hardware, exposes the same virtual sensors, shell,
and veer-connect encrypted management channel, and can be provisioned, introspected,
and torn down by EdgeFabric without any physical device.

## Scope

This guide defines:

- VirtualIoT concept and positioning
- Runtime topology (EdgeFabric → veer-vm / fold → VeerOS guest)
- Network backend options: user-net (macOS, no root) and TAP (Linux)
- Required host prerequisites per platform
- Lifecycle operations (create, start, observe, connect, stop, cleanup)
- Diagnostics: scheduler heartbeat, stack canaries, remote command matrix
- Scaling strategy and practical limits
- Error handling and recovery rules

## VirtualIoT Concept

EdgeFabric can treat every `veer-vm` process running `kernel-qemu-esp32c6` or
`kernel-fold-esp32c6` as a **VirtualIoT backend**: a fully addressable, remotely
manageable virtual ESP32-C6 node.

Key properties of a VirtualIoT backend:

| Property | Value |
|---|---|
| Architecture | RISC-V 32 (riscv32imc-unknown-none-elf) |
| Kernel images | `kernel-qemu-esp32c6`, `kernel-fold-esp32c6` |
| Network options | user-net (hostfwd, no root) or virtio-net TAP |
| Management port | TCP 2323 (veer-connect encrypted shell) |
| Sensors | `/dev/sensor/{temperature,humidity,pressure,light,...}` |
| Remote shell | veer-connect X25519 + ChaCha20-Poly1305 |
| Default credentials | `root / toor`, `user / veeros` |

## Runtime Topology

```
EdgeFabric control plane
  └─ veer-vm (or fold → veer-vm)
       ├─ kernel-qemu-esp32c6  (RISC-V 32 guest)
       │    ├─ scheduler + shell task
       │    ├─ net listener task (port 2323)
       │    └─ /dev/sensor/{temperature,humidity,...}
       └─ network backend
            ├─ user-net   → hostfwd :23xx → :2323  (macOS, no root)
            └─ virtio-net TAP                      (Linux, root)
```

Two kernel image choices:
- **`kernel-qemu-esp32c6`** — standalone `veer-vm` path, built via
  `cargo build -p kernel-qemu-esp32c6`
- **`kernel-fold-esp32c6`** — fold-managed path, built via
  `cargo build -p kernel-fold-esp32c6`

A single instance maps to one `veer-vm` process (and optionally one fold record).

## Host Prerequisites

### macOS (user-net — recommended, no root required)

No SIP changes or TAP setup needed. Build host tools and kernel:

```sh
cd /path/to/VeerOS
cargo build -p kernel-qemu-esp32c6 --target riscv32imc-unknown-none-elf
cargo build -p veer_vm
# Optional: fold-managed path
cargo build -p kernel-fold-esp32c6 --target riscv32imc-unknown-none-elf
cargo build -p fold_engine
```

Verify kernel exists:

```sh
test -f ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 && echo ok
```

### Linux (TAP backend — full virtio-net, supports multi-instance bridging)

Build as above, then create the shared TAP once:

```sh
sudo ip tuntap add dev tap0 mode tap user "$USER" 2>/dev/null || true
sudo ip link set tap0 up
```

## Network Backends

### user-net (macOS, entitlement-free)

`veer-vm` provides a built-in user-mode TCP/IP stack (smoltcp) with DHCP and host
port forwarding. No kernel extensions, no root, no vmnet entitlement required.

- Guest IP: `10.0.2.15`
- Host forward: `tcp::<HOST_PORT>-:2323`  → veer-connect management channel
- Launch flag: `--net user`

Each VirtualIoT instance must use a **distinct host port** for the hostfwd.

### TAP / virtio-net (Linux)

Full virtio-net device; guest appears on the LAN and acquires DHCP from the host
bridge or an external router. Multiple instances share the same `tap0` or use
per-instance TAP interfaces.

- Launch flag: `--tap tap0`

## Instance Manifest Contract

### user-net manifest (macOS, recommended for VirtualIoT)

EdgeFabric should generate one manifest per instance with a unique name and host port:

```toml
name = "edgefabric-node-001"
cmd  = "./target/debug/veer-vm"
args = [
  "--arch",           "riscv32",
  "--cpu-throttle-ms","20",
  "--kernel",         "./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6",
  "--memory",         "32",
  "--net",            "user",
  "--net-hostfwd-port","12001",
]
workdir = "/path/to/VeerOS"

[env]
TERM = "xterm-256color"
```

`--net-hostfwd-port` maps `tcp::12001` on the host to guest port `2323`.
Allocate a unique port per instance (e.g. `12001`, `12002`, `12003`, …).

### TAP manifest (Linux)

```toml
name = "edgefabric-node-001"
cmd  = "./target/debug/veer-vm"
args = [
  "--arch",           "riscv32",
  "--cpu-throttle-ms","20",
  "--kernel",         "./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6",
  "--memory",         "128",
  "--tap",            "tap0",
]
workdir = "/path/to/VeerOS"

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

### Direct veer-vm (user-net, macOS)

```sh
# 1) Start (background)
target/debug/veer-vm \
  --arch riscv32 \
  --kernel target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
  --memory 32 \
  --net user \
  --net-hostfwd-port 12001 \
  >logs/edgefabric-node-001.log 2>&1 &
echo $! > state/edgefabric-node-001.pid

# 2) Readiness check (poll log)
grep -q 'DHCP complete' logs/edgefabric-node-001.log

# 3) Connect (management channel)
target/debug/veer-connect shell 127.0.0.1 12001

# 4) Stop
kill "$(cat state/edgefabric-node-001.pid)"
```

### Via fold (all platforms)

#### 1) Create/start

```sh
./target/debug/fold spawn --manifest /tmp/edgefabric-node-001.toml
```

Expected stdout:

- `fold <name> spawned (pid <pid>)`
- `log: <path>`

#### 2) Read logs / readiness

```sh
./target/debug/fold logs edgefabric-node-001 | head -n 120
```

Treat instance as ready after these markers appear:

- `[veer-vm] rv32-soft: entering guest`  (or `[veer-vm] rv32-soft: user-net dhcp=10.0.2.15`)
- `[boot] starting scheduler`
- `[net] DHCP complete — listening on port 2323`

#### 3) Connect via veer-connect

```sh
# user-net: connect to the mapped host port
target/debug/veer-connect shell 127.0.0.1 12001
# TAP: connect to guest IP
target/debug/veer-connect shell 10.0.2.15 2323
```

Default credentials: `root / toor` or `user / veeros`.

#### 4) List inventory

```sh
./target/debug/fold list
```

Use this as the source of truth for runtime state (`running` / `stopped`).

#### 5) Stop

```sh
./target/debug/fold stop edgefabric-node-001
```

#### 6) Remove state/log record

```sh
./target/debug/fold rm edgefabric-node-001 --force
```

## Diagnostics

### Scheduler and memory health (via veer-connect)

Connect and run `meminfo`:

```
$ target/debug/veer-connect shell 127.0.0.1 12001
login: root
password: toor
root@veeros-esp32c6-vm> meminfo
  small pool : 0/128 blocks (64 B each)
  large pool : 0/8 blocks (1024 B each)
  total      : 0 / 16384 bytes used
  scheduler ticks : 1203
  scheduler slot  : 1
  shell stack canary : ok
  remote stack canary: 0/4 bad
```

Key fields:
- `scheduler ticks`: monotonically increasing — confirms scheduler is alive
- `scheduler slot`: currently running task slot index
- `shell stack canary : ok`: shell stack has not overflowed
- `remote stack canary: 0/4 bad`: no remote session stacks corrupted

### Automated remote command matrix

Run the scripted smoke test against a running VirtualIoT:

```sh
# Boot kernel, then:
./scripts/macos-esp32c6-connect-matrix.sh --qemu-only
```

Asserts: encrypted session, meminfo heartbeat, canary ok, ps fallback, drivers table.

### Driver inventory

```
root@veeros-esp32c6-vm> drivers
  ID  NAME              MMIO  IRQ  DMA  NET
   0  uart0                1   10   no   no
   1  clint                1    7   no   no
   2  virtio-net           1    1  yes  yes
   3  wifi-sim             0   -1   no  yes
   4  ble-sim              0   -1   no   no
   5  802154-sim           0   -1   no  yes
```

### Process snapshot

```
root@veeros-esp32c6-vm> ps
  task table: temporarily unavailable
  reason: scheduler metadata inspection is disabled to avoid hangs
```

`ps` is intentionally non-blocking while scheduler metadata inspection is hardened.
Use `meminfo` scheduler fields for health monitoring instead.



## Batch Operations

### Start N VirtualIoT instances (user-net, macOS)

Allocate a unique host port per instance starting from a base (e.g. 12001):

```sh
BASE_PORT=12001
for i in $(seq -w 001 010); do
  PORT=$((BASE_PORT + 10#$i - 1))
  NAME="edgefabric-node-$i"
  target/debug/veer-vm \
    --arch riscv32 \
    --kernel target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
    --memory 32 --net user --net-hostfwd-port "$PORT" \
    >logs/${NAME}.log 2>&1 &
  echo $! > state/${NAME}.pid
  echo "spawned $NAME on port $PORT"
done
```

Poll readiness:

```sh
for i in $(seq -w 001 010); do
  until grep -q 'DHCP complete' "logs/edgefabric-node-$i.log" 2>/dev/null; do sleep 1; done
  echo "edgefabric-node-$i ready"
done
```

### Start N instances (fold, TAP, Linux)

- Generate `edgefabric-node-001 .. edgefabric-node-NNN` manifests in `/tmp`
- Spawn with unique names
- Run readiness checks per instance in parallel with bounded concurrency

### Stop all EdgeFabric-managed instances (fold)

Naming convention: prefix all names with `edgefabric-`.

```sh
./target/debug/fold list | awk 'NR>1 {print $1}' | grep '^edgefabric-' | while read -r n; do
  ./target/debug/fold stop "$n" >/dev/null 2>&1 || true
  ./target/debug/fold rm "$n" --force >/dev/null 2>&1 || true
done
```

### Stop all EdgeFabric-managed instances (direct veer-vm)

```sh
for pid_file in state/edgefabric-node-*.pid; do
  kill "$(cat "$pid_file")" 2>/dev/null || true
done
rm -f state/edgefabric-node-*.pid
```

## Resource and Scaling Guidance

Observed on macOS host (Apple Silicon, user-net):

- Practical sustained target: about 10 VirtualIoT instances with `--cpu-throttle-ms 20`
- Each instance: ~32 MB RAM (user-net) — 10 instances ≈ 320 MB
- Hard-run target in stress sweep: about 15 instances before host CPU saturation
- Bottleneck: CPU, not memory; no TAP/root overhead in user-net mode

On Linux with TAP:

- Each instance shares tap0; virtio-net DMA adds latency per packet
- Default to `--cpu-throttle-ms 20` and keep pool ≤ 10 for steady state

Recommendations:

- Prefer user-net on macOS for development and CI; TAP for production on Linux
- Increase throttle (e.g. 30–40 ms) and re-benchmark when scaling beyond 10

## Error Handling Rules

### `unexpected argument '--arch' found`

Cause: fold or launch script used an outdated veer-vm binary.

Action:

- Use workspace binary path (`./target/debug/veer-vm`) in manifest `cmd`
- Rebuild: `cargo build -p veer_vm`

### `TUNSETIFF ... Device or resource busy` (TAP, Linux)

Cause: TAP held open by a crashed instance or conflicting setup.

Action:

- Stop all veer-vm/fold instances
- Reuse existing `tap0` if already present
- Avoid deleting/recreating TAP while instances are running

### `hostfwd port <PORT> already in use` (user-net, macOS)

Cause: Two instances allocated the same hostfwd port, or a previous instance did not
fully exit.

Action:

- Allocate a unique port per instance (recommended: `BASE_PORT + instance_index`)
- Kill the stale process: `lsof -i TCP:<PORT>` then `kill <pid>`

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
components and the veer-vm (or fold) runtime adapter.

Two `net_backend` modes are supported:

| Mode | Field | Platform |
|---|---|---|
| user-net | `"net_backend": "user"` | macOS, any (no root) |
| TAP | `"net_backend": "tap"` | Linux (root) |

### JSON (request — user-net)

```json
{
  "name": "edgefabric-node-001",
  "kernel": "./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6",
  "memory_mib": 32,
  "net_backend": "user",
  "hostfwd_port": 12001,
  "cpu_throttle_ms": 20,
  "env": {
    "TERM": "xterm-256color"
  },
  "labels": {
    "tenant": "acme",
    "site": "rack-a"
  }
}
```

### JSON (request — TAP)

```json
{
  "name": "edgefabric-node-001",
  "kernel": "./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6",
  "memory_mib": 128,
  "net_backend": "tap",
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
  "pid": 31245,
  "log_path": "/var/log/veeros/edgefabric-node-001.log",
  "net_backend": "user",
  "hostfwd_port": 12001,
  "connect_host": "127.0.0.1",
  "connect_port": 12001,
  "state": "running",
  "last_error": "",
  "created_at": "2026-04-24T10:58:12Z",
  "updated_at": "2026-04-24T10:58:21Z"
}
```

### YAML (request — user-net)

```yaml
name: edgefabric-node-001
kernel: ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6
memory_mib: 32
net_backend: user
hostfwd_port: 12001
cpu_throttle_ms: 20
env:
  TERM: xterm-256color
labels:
  tenant: acme
  site: rack-a
```

### Field semantics

- `name`: globally unique instance name.
- `kernel`: riscv32 guest image path on host.
- `memory_mib`: guest memory passed to `veer-vm --memory`.
- `net_backend`: `"user"` (macOS, no root) or `"tap"` (Linux, root).
- `hostfwd_port`: host-side TCP port forwarded to guest 2323 (user-net only); must be unique per instance.
- `tap`: pre-created TAP interface (tap-backend only).
- `connect_host` / `connect_port`: where to reach the veer-connect shell for this instance.
- `cpu_throttle_ms`: veer-vm busy-loop throttle; larger value lowers host CPU.
- `state`: one of `starting`, `running`, `stopped`, `error`.
- `last_error`: last terminal error string, empty when healthy.

## VirtualIoT Sensor Management

EdgeFabric can read and write virtual sensor values on any running VirtualIoT instance
through the veer-connect management channel.

### List sensors

```sh
target/debug/veer-connect shell 127.0.0.1 12001 <<'EOF'
root
toor
sensor list
exit
EOF
```

### Inject sensor value

```sh
target/debug/veer-connect shell 127.0.0.1 12001 <<'EOF'
root
toor
sensor set temperature 23.5
sensor set humidity 55
exit
EOF
```

Sensor values persist in the running VirtualIoT instance until the instance stops.
See [docs/esp32c6-dist-sensors-2026.md](esp32c6-dist-sensors-2026.md) for the full
sensor reference.

## Security Notes

- user-net: no root, no kernel extension, no TAP privileges needed (macOS)
- TAP mode: `user = true` keeps runtime rootless-friendly in user namespaces; `seccomp.profile = "default"` enforced in fold manifest
- veer-connect channel: X25519 key exchange, ChaCha20-Poly1305 encryption (unauthenticated by default in dev builds; add credential validation before production)
- Hostfwd ports should be bound to loopback only (`127.0.0.1`) in production

## Quick Smoke Test (macOS, user-net)

```sh
# Build
cargo build -p kernel-qemu-esp32c6 --target riscv32imc-unknown-none-elf
cargo build -p veer_vm

# Launch
target/debug/veer-vm \
  --arch riscv32 \
  --kernel target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
  --memory 32 --net user --net-hostfwd-port 12001 \
  >logs/smoke.log 2>&1 &
VMM_PID=$!

# Readiness
until grep -q 'DHCP complete' logs/smoke.log 2>/dev/null; do sleep 1; done

# Connect and check
target/debug/veer-connect shell 127.0.0.1 12001

# Cleanup
kill $VMM_PID
```

## Quick Smoke Test (Linux, TAP / fold)

```sh
./target/debug/fold spawn \
  --manifest /tmp/edgefabric-smoke.toml   # TAP manifest
./target/debug/fold logs edgefabric-smoke | head -n 120
target/debug/veer-connect shell 10.0.2.15 2323
./target/debug/fold stop edgefabric-smoke
./target/debug/fold rm edgefabric-smoke --force
```
