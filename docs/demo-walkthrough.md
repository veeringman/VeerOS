# VeerOS AI-Native Demo Walkthrough

Run the demo interactively:

```bash
cargo run -p veeros-demo          # or: ./scripts/demo.sh
```

Run with QUIC bridge smoke enabled:

```bash
cargo run -p veeros-demo --features quic-demo
```

## Quick Demo (automated)

```bash
./scripts/demo.sh --scenario deploy
./scripts/demo.sh --scenario pipeline
./scripts/demo.sh --scenario monitor
./scripts/demo.sh --scenario full
```

## Interactive Session Examples

### 1. Inspect the Execution Fabric

```
veeros> fabric
  Execution Fabric: 4/4 nodes healthy
  IDX      Arch      Zone   Cores    RAM  Load%  Name
  ---  --------  --------  ------  -----  -----  ----
    0    x86_64     local       4  16384M     9%  host-demo
    1     arm64      rack       4  8192M     3%  rpi5-edge-01
    2      rv32      rack       1     0M     1%  esp32c6-sensor
    3    x86_64        dc      16  65536M    15%  cloud-gpu-a100
```

The fabric pre-populates 4 heterogeneous nodes: a local x86 host, an ARM64 Raspberry Pi edge node, a RISC-V ESP32-C6 sensor, and a cloud GPU inference server.

### 1b. QUIC Mesh Bridge Smoke (feature-gated)

When started with `--features quic-demo`, the boot log includes a QUIC smoke status line and the shell exposes mesh smoke commands.

```
veeros> mesh status
  mesh transport: host QUIC bridge smoke available
  usage: mesh quic-smoke

veeros> mesh quic-smoke
  running QUIC mesh bridge smoke...
  quic smoke: ok

veeros> fabric
  ...execution fabric status...
```

If built without `quic-demo`, `mesh quic-smoke` explains that the feature must be enabled at build time.

### 1c. Controlled Fault Injection (Replan Trigger)

```
veeros> fabric fault inject rpi5-edge-01 degraded
  fault update: node #1 (rpi5-edge-01) Healthy -> Degraded
  replan signal: triggered (scheduler replans = ...)

veeros> fabric fault status
  fault table (fabric health):
    #0  host-demo           Healthy
    #1  rpi5-edge-01        Degraded
    ...

veeros> fabric fault clear rpi5-edge-01
  fault update: node #1 (rpi5-edge-01) Degraded -> Healthy

veeros> fabric snapshot
  world snapshot: nodes=4 hash=0x...............

veeros> fabric trace
  decision trace (oldest -> newest):
    ...plan/replan/snapshot decisions with reasons...

veeros> fabric simulate latency-first pipeline rebalance realtime telemetry
  simulate policy=latency-first class=Pipeline
  required capability: compute
  ranked candidates:
    1. #0 (host-demo) score=...
    2. #1 (rpi5-edge-01) score=...
  winner: #0 (host-demo)
```

### 2. Spawn and Manage Agents

```
veeros> agents spawn analyze sensor data
  agent #0 spawned (planning)

veeros> agents spawn run ML inference
  agent #1 spawned (planning)

veeros> agents list
  Active agents: 2/32
   ID       State     Ticks  Goal
  ---  ----------  --------  ----
    0    planning         0  analyze sensor data
    1    planning         0  run ML inference

veeros> agents execute 0
  agent #0 → executing

veeros> agents complete 0
  agent #0 → completed

veeros> agents status 1
  Agent #1
    State     : Planning
    Goal      : run ML inference
    Ticks used: 0
    Children  : 0
    Replans   : 0/3
```

### 3. Submit Intents with Auto-Decomposition

```
veeros> intent submit deploy upgrade edge firmware
  intent #1 submitted (class=deploy)
  plan decomposed into 3 step(s):
    step 0: validate deployment config (independent)
    step 1: provision resources (depends-on)
    step 2: verify deployment health (depends-on)

veeros> intent submit pipeline ingest transform export logs
  intent #2 submitted (class=pipeline)
  plan decomposed into 4 step(s):
    step 0: ingest raw data (independent)
    step 1: transform and normalize (depends-on)
    step 2: validate output quality (depends-on)
    step 3: output to destination (depends-on)

veeros> intent list
  Active intents: 2/16
   ID       Class      Status  Description
  ---  ----------  ----------  -----------
    1      deploy    planning  upgrade edge firmware
    2    pipeline    planning  ingest transform export logs

veeros> intent cancel 1
  intent #1 cancelled
```

Intent classes and their auto-decomposition:
- `compute` → 1 step (execute)
- `deploy` → 3 steps (validate → provision → verify)
- `data` → 2 steps (acquire → transform)
- `pipeline` → 4 steps (ingest → transform → validate → output)
- `monitor`, `communicate`, `admin`, `custom` → 1 step each

### 4. Persistent Memory Store

```
veeros> memory set app.version 3.2.1
  stored: app.version = 3.2.1

veeros> memory set sensor.threshold 42
  stored: sensor.threshold = 42

veeros> memory get app.version
  app.version = 3.2.1  (reads=1, confidence=200)

veeros> memory get os.name
  os.name = VeerOS  (reads=1, confidence=255)

veeros> memory stats
  Memory Engine Status
    persistent entries: 5

veeros> memory delete sensor.threshold
  deleted: sensor.threshold
```

Pre-seeded keys: `os.name`, `os.arch`, `demo.mode`.

### 5. Run Scripted Demos

```
veeros> demo
  VeerOS AI-Native Demo Scenarios
  ─────────────────────────────────
  demo deploy    Deploy a service (intent → agents → memory)
  demo pipeline  Data pipeline (multi-step intent decomposition)
  demo monitor   Spawn a monitoring agent swarm
  demo living    Run fabric cognition loop (sense->plan->act->reflect)
  demo incident  Run autonomous incident drill (fault->replan->audit)
  demo full      Run all scenarios end-to-end

veeros> demo deploy
  ...full scripted deployment walkthrough...

veeros> demo full
  ...runs all three scenarios...
```

### 5b. Living Fabric Cognition Loop (No LLM Backend)

```
veeros> demo living
  ...senses fabric + mesh state...
  ...injects a controlled node fault and triggers replan...
  ...writes world model memory...
  ...submits planning intent...
  ...spawns execution agents...
  ...emits deterministic world snapshot hash for audit/replay...
  ...records explainable decision outcome...

veeros> demo incident
  ...baseline topology...
  ...mission intent loaded...
  ...fault injected, autonomous replan emitted...
  ...decision trace + snapshot audit shown...
  ...recovery completed...
```

This scenario demonstrates the intended VeerOS direction: LLM-like capability from distributed fabric cognition primitives (`sense -> plan -> act -> reflect`) without requiring a direct LLM runtime in the loop.

### 6. Man Pages

```
veeros> man agents
veeros> man intent
veeros> man memory
veeros> man fabric
veeros> man demo
```

## Architecture Exercised

```
┌─────────────────────────────────────────────┐
│                   Shell                      │
│  agents / intent / memory / fabric / demo   │
├─────────┬──────────┬──────────┬─────────────┤
│  Agent  │  Intent  │ Memory   │  Execution  │
│  Table  │  Engine  │ Engine   │  Fabric     │
│ (32 slots)│(16 slots)│ KV+Ring │ (8 nodes)  │
├─────────┴──────────┴──────────┴─────────────┤
│              Intent Scheduler                │
│   decompose → place → spawn → monitor → GC  │
└─────────────────────────────────────────────┘
```

All subsystems are real kernel code (`microkernel` crate) running live — not mocks.
