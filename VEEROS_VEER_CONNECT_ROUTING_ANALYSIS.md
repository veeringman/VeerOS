# VeerOS veer-connect vs SSH Routing Analysis

**Date:** May 4, 2026  
**Workspace:** EdgeFabric  
**Purpose:** Identify where veer-connect is invoked, how devices indicate their terminal protocol, and why 192.168.29.5:2323 uses SSH instead of veer-connect.

---

## 1. Where veer-connect is Spawned/Used

### ✅ veer-connect IS USED for Virtual IoT Backends

**File:** [ef-api/src/routes/virtual_iot.rs](crates/ef-api/src/routes/virtual_iot.rs#L810-L828)  
**Function:** `local_shell_session()`  
**Lines:** 810-828

```rust
async fn local_shell_session(
    socket: WebSocket,
    device_id: Uuid,
    backend: VirtualIotBackendRow,
    state: AppState,
) {
    let host = backend.shell_host.clone().unwrap_or_else(|| "localhost".into());
    let port = backend.shell_port.unwrap_or_default().to_string();
    let veer_connect_path = resolve_veer_connect_path(&state);
    let mut child = match Command::new(&veer_connect_path)
        .current_dir(&state.veeros.root_dir)
        .env("TERM", "xterm-256color")
        .args(["shell", &host, &port])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(device_id = %device_id, "Failed to spawn veer-connect: {e}");
            return;
        }
    };
```

**Key Points:**
- Invocation: `veer-connect shell {host} {port}`
- Only used for **Virtual IoT Backends** attached to devices
- NOT used for generic SSH/VeerOS devices
- Path resolved via `resolve_veer_connect_path()` function (line 991)

---

### ❌ veer-connect NOT USED for Generic VeerOS Devices

**File:** [ef-api/src/routes/streaming.rs](crates/ef-api/src/routes/streaming.rs#L145-L220)  
**Function:** `handle_stream()` and related routing  
**Lines:** 145-220

```rust
// SSH-like devices: run SSH session locally (agent on this machine)
if device.os == "ssh" || device.os == "veeros" {
    let (username, host, port) = match parse_ssh_target(&serial) {
        Some(hp) => hp,
        None => {
            let _ = ws_tx.send(Message::Text(
                serde_json::json!({"type":"error","message":format!("Invalid SSH serial: {serial}")}).to_string().into()
            )).await;
            return;
        }
    };
    let device_type = if device.os == "veeros" { "veeros" } else { "ssh" };
    tracing::info!(%id, serial = %serial, %host, port, "Starting local SSH terminal session");
    run_ssh_session(ws_tx, ws_rx, username.as_deref(), &host, port, id).await;
    return;
}
```

**Key Point:**  
Both `device.os == "veeros"` and `device.os == "ssh"` route to the same `run_ssh_session()` function — **NOT to veer-connect**.

---

## 2. How Devices Indicate veer-connect vs SSH

### Device Type Routing Decision

The routing is determined by the `device.os` field in the Device record:

| os value | Routing | Protocol | Source Code |
|----------|---------|----------|-------------|
| `"veeros"` | SSH terminal via `run_ssh_session()` | SSH (port 22 default) | [streaming.rs:145-220](crates/ef-api/src/routes/streaming.rs#L145-L220) |
| `"ssh"` | SSH terminal via `run_ssh_session()` | SSH (port 22 default) | [streaming.rs:145-220](crates/ef-api/src/routes/streaming.rs#L145-220) |
| `"iot"` | Serial console stream | Serial/UART | [streaming.rs:221-245](crates/ef-api/src/routes/streaming.rs#L221-245) |
| `"android"` | ADB screenshot stream | ADB (port 5555) | [streaming.rs:246+](crates/ef-api/src/routes/streaming.rs#L246) |

### Device Capability Properties (iOS/Android only)

**File:** [ef-control/src/discovery.rs](crates/ef-control/src/discovery.rs)

For ADB devices (lines ~50-65):
```rust
devices.push(Device {
    ...
    capabilities: vec!["adb".into()],
    ...
});
```

For SSH devices discovered via mDNS/ARP (lines ~210-260):
```rust
pub async fn scan_ssh() -> Result<Vec<Device>> {
    info!("Scanning for SSH servers on the network...");
    let mut all = Vec::new();
    all.extend(scan_ssh_mdns().await);
    let arp_hosts = scan_ssh_arp_probe().await;
    // Each device becomes Os::Ssh with serial = "user@host:port"
}
```

---

## 3. Device Discovery for VeerOS

### VeerOS Hosts Discovery (Centralized in Agents)

**File:** [ef-api/src/routes/discovery.rs](crates/ef-api/src/routes/discovery.rs#L1-80)

VeerOS hosts are NOT discovered locally. Instead:

1. **ef-api fans out to all agents**
   ```rust
   let agents = AgentRepo::list(&state.pool, None, None, false).await?;
   
   for agent in &agents {
       let base = veeros_proxy::resolve_agent_base_url(&state.pool, agent.id, None).await?;
       // Each agent reports its discovered VeerOS hosts via GET /veeros/discovery/hosts
       veeros_proxy::discovery_hosts(&base, quantum_only).await?
   }
   ```

2. **Results are upserted into `discovered_hosts` table**
   ```rust
   for host in hosts {
       upsert_host_from_value(&pool, agent_id, host).await;
   }
   ```

3. **Virtual IoT backends create devices with explicit `os="veeros-vm"` or `os="veeros"`**
   - Not auto-discovered
   - Created via REST API POST `/api/v1/devices/virtual-iot/backends`
   - See [virtual_iot.rs lines 195-240](crates/ef-api/src/routes/virtual_iot.rs#L195-240)

### VeerOS Shell Info Definition

**File:** [ef-control/src/command/veeros.rs](crates/ef-control/src/command/veeros.rs#L208-218)  
**Function:** `VeerOsDriver::shell_info()`

```rust
pub async fn shell_info(cfg: &VeerOsConfig, vm_name: &str) -> Result<VeerOsShellInfo> {
    let config = Self::vm_config(cfg, vm_name).await?;
    let port = config
        .get("ssh_port")
        .and_then(|v| v.parse::<u16>().ok())
        .ok_or_else(|| EdgeFabricError::Internal("VeerOS VM is missing ssh_port".into()))?;
    let host = "localhost".to_string();
    Ok(VeerOsShellInfo {
        vm_name: vm_name.to_string(),
        host: host.clone(),
        port,
        connect_command: format!("{} shell {} {}", cfg.veer_connect_path, host, port),
    })
}
```

**Key Points:**
- `connect_command` field contains the veer-connect invocation: `{veer_connect_path} shell {host} {port}`
- But this field is **stored as metadata** and **NOT USED** in the generic streaming routes
- It IS used only in virtual_iot backend local shell sessions

---

## 4. Routes That Decide Between SSH and veer-connect

### Route 1: Generic Device Streaming (Virtual Devices + SSH/VeerOS)

**Route:** `GET /api/v1/devices/{id}/stream` or `GET /api/v1/stream/{session_id}`  
**Handler:** [streaming.rs#145-245](crates/ef-api/src/routes/streaming.rs#L145-245)  
**Decision Logic:**
```rust
if device.os == "ssh" || device.os == "veeros" {
    // Always routes to SSH terminal (run_ssh_session)
    run_ssh_session(ws_tx, ws_rx, username.as_deref(), &host, port, id).await;
} else if device.os == "iot" {
    // Routes to serial stream
    run_serial_stream(ws_tx, ws_rx, &serial, id).await;
} else {
    // Routes to ADB screenshot/logcat
    run_screenshot_loop(ws_tx, ws_rx, &effective_serial, id).await;
}
```

### Route 2: Virtual IoT Backend Shell (ONLY place veer-connect is used)

**Route:** `GET /api/v1/devices/{id}/virtual-iot/shell/ws`  
**Handler:** [virtual_iot.rs#700-800](crates/ef-api/src/routes/virtual_iot.rs#L700-800)  
**Decision Logic:**
```rust
pub async fn virtual_iot_shell_ws(
    ws: WebSocketUpgrade,
    Path(device_id): Path<Uuid>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_shell_upgrade(socket, device_id, state))
}

async fn handle_shell_upgrade(socket: WebSocket, device_id: Uuid, state: AppState) {
    let backend = attached_backend(&state, device_id).await?;
    
    if let Some(agent_id) = backend.agent_id {
        // Remote agent: proxy to agent's shell endpoint
        remote_shell_session(socket, backend, state).await;
    } else {
        // Local backend: SPAWN veer-connect directly
        local_shell_session(socket, device_id, backend, state).await;
    }
}
```

### Route 3: VeerOS Direct Shell (for raw host:port)

**Route:** `GET /api/v1/veeros/shell/ws` (with query params `?host=...&port=...&agent_id=...`)  
**Handler:** [streaming.rs#116-143](crates/ef-api/src/routes/streaming.rs#L116-143)  
**Decision Logic:**
```rust
pub async fn ws_veeros_shell_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<VeerosShellQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_veeros_shell(socket, query, state))
}

async fn handle_veeros_shell(socket: WebSocket, query: VeerosShellQuery, state: AppState) {
    if let Some(agent_id) = query.agent_id {
        // If agent has streaming URL, proxy via agent's /ws/ssh endpoint
        let agent_ws_url = format!(
            "{}/ws/ssh?host={}&port={}",
            streaming_url,
            host,
            port
        );
        proxy_ws_to_agent(ws_tx, ws_rx, &agent_ws_url, req_id).await;
    } else {
        // Local: run SSH session locally
        run_ssh_session(ws_tx, ws_rx, None, &host, port, req_id).await;
    }
}
```

---

## 5. Current Assumption: Why 192.168.29.5:2323 Uses SSH, Not veer-connect

### The Problem

When a device is registered with:
- `os = "veeros"` (or `os = "ssh"`)
- `serial = "192.168.29.5:2323"`

The system **always routes to SSH**, not veer-connect, because:

### Root Cause #1: Generic Devices Always Use SSH

In [streaming.rs#650-670](crates/ef-api/src/routes/streaming.rs#L650-670):

```rust
if device.os == "ssh" || device.os == "veeros" {
    let (username, host, port) = match parse_ssh_target(&serial) {
        Some(hp) => hp,
        None => {...}
    };
    // Device with os="veeros" and serial="192.168.29.5:2323" parses as:
    // username = None
    // host = "192.168.29.5"
    // port = 2323 (from parse_ssh_target function)
    run_ssh_session(ws_tx, ws_rx, username.as_deref(), &host, 2323, id).await;
}
```

### Root Cause #2: veer-connect ONLY Used for Virtual IoT Backends

veer-connect is **only spawned** in [virtual_iot.rs#810-828](crates/ef-api/src/routes/virtual_iot.rs#L810-828), which requires:
1. Device type: `virtual_iot` (with `os = "virtual_iot"` in device table)
2. Backend type: `veeros-vm` provider
3. Route: **exclusively** `/api/v1/devices/{id}/virtual-iot/shell/ws`

Generic SSH/VeerOS devices **cannot** use this route because:
- They don't have a `virtual_iot` config
- They don't have an attached virtual IoT backend
- The `attached_backend()` function will fail with 404

### Serial Parsing Function

**File:** [streaming.rs#1090-1113](crates/ef-api/src/routes/streaming.rs#L1090-1113)

```rust
fn parse_ssh_target(serial: &str) -> Option<(Option<String>, String, u16)> {
    let (username, host_port) = if let Some(at) = serial.rfind('@') {
        let user = serial[..at].trim();
        let host_port = &serial[at + 1..];
        let username = if user.is_empty() { None } else { Some(user.to_string()) };
        (username, host_port)
    } else {
        (None, serial)
    };

    if let Some(colon) = host_port.rfind(':') {
        let host = host_port[..colon].to_string();
        let port = host_port[colon + 1..].parse::<u16>().ok()?;
        Some((username, host, port))
    } else {
        let host = host_port.to_string();
        Some((username, host, 22))  // Default port 22
    }
}
```

**Examples:**
- `"192.168.29.5:2323"` → `(None, "192.168.29.5", 2323)`
- `"root@192.168.29.5"` → `(Some("root"), "192.168.29.5", 22)`
- `"user@host:2222"` → `(Some("user"), "host", 2222)`

---

## 6. Configuration Paths

### veer-connect Path Configuration

**File:** [ef-core/src/config.rs](crates/ef-core/src/config.rs#L88-107)

```rust
pub struct VeerOsConfig {
    pub root_dir: String,
    pub vm_script_path: String,
    pub veer_connect_path: String,  // Configured path to veer-connect binary
    pub veer_cli_path: String,
    pub default_target: String,
    pub default_memory_mb: u32,
    pub default_network_mode: String,
    pub default_region: String,
}

impl Default for VeerOsConfig {
    fn default() -> Self {
        Self {
            root_dir: "/home/vijay/rnd/VeerOS".into(),
            vm_script_path: "/home/vijay/rnd/VeerOS/scripts/veeros-vm".into(),
            veer_connect_path: "/home/vijay/rnd/VeerOS/target/release/veer-connect".into(),
            veer_cli_path: "/home/vijay/rnd/VeerOS/target/release/veer".into(),
            default_target: "esp32c6".into(),
            default_memory_mb: 128,
            default_network_mode: "nat".into(),
            default_region: "local".into(),
        }
    }
}
```

### Path Resolution for Virtual IoT

**File:** [virtual_iot.rs#991-999](crates/ef-api/src/routes/virtual_iot.rs#L991-999)

```rust
fn resolve_veer_connect_path(state: &AppState) -> String {
    let configured = std::path::Path::new(&state.veeros.veer_connect_path);
    if configured.exists() {
        state.veeros.veer_connect_path.clone()
    } else {
        format!("{}/target/debug/veer-connect", state.veeros.root_dir)
    }
}
```

---

## 7. Summary Table

| Component | File | Lines | Details |
|-----------|------|-------|---------|
| **veer-connect invoked** | `virtual_iot.rs` | 810-828 | `Command::new(&veer_connect_path).args(["shell", &host, &port])` |
| **Device routing decision** | `streaming.rs` | 145-220 | Checks `device.os` field |
| **SSH session handler** | `streaming.rs` | 1115-1350 | Uses SSH via `script` PTY wrapper |
| **Parse SSH serial** | `streaming.rs` | 1090-1113 | Parses `user@host:port` format |
| **VeerOS shell info** | `veeros.rs` | 208-218 | Generates connect_command (stored, not used) |
| **Virtual IoT shell** | `virtual_iot.rs` | 700-850 | Routes to `local_shell_session()` with veer-connect |
| **Config definition** | `config.rs` | 88-107 | VeerOsConfig struct with veer_connect_path |
| **Device discovery** | `discovery.rs` | 1-160 | VeerOS hosts discovered via agents, SSH via mDNS/ARP |

---

## 8. Key Findings & Implications

### What Works (veer-connect Integration)
✅ Virtual IoT backends with `provider="veeros-vm"` correctly spawn veer-connect  
✅ veer-connect path is configurable via VeerOsConfig  
✅ Shell commands are properly proxied through agents when remote  

### What Doesn't Work (Generic VeerOS Device Routing)
❌ Generic devices with `os="veeros"` cannot use veer-connect  
❌ Port 2323 (veer-connect default) is interpreted as SSH port, not veer-connect indicator  
❌ No capability/property flag distinguishes veer-connect-capable hosts from SSH-only hosts  
❌ veer-connect is defined in `VeerOsShellInfo` but ignored by generic streaming routes  

### Required Changes to Support veer-connect for Generic Devices
1. Add device capability: `"shell:veer_connect"` vs `"shell:ssh"`
2. Modify `handle_stream()` to check capabilities before routing
3. Create dedicated `run_veer_connect_session()` function (parallel to `run_ssh_session()`)
4. Update device discovery to flag capabilities based on `/veeros/discovery/hosts` endpoint
5. Route based on priority: virtual_iot > veer_connect capability > SSH fallback
