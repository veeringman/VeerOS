# IoT Shell vs VeerOS Shell Terminal Implementation Analysis

## Executive Summary

Both implementations use xterm.js for terminal rendering, but they differ significantly in:
1. **WebSocket message protocol** (raw bytes vs JSON)
2. **Backend routing** (complete for IoT, partial for VeerOS)
3. **Terminal resize handling** (missing in VeerOS)

**Status**: VeerOS shell frontend is prepared for structured JSON messaging but the backend only partially implements the protocol.

---

## 1. FILE LOCATIONS & LINE NUMBERS

### Frontend UI Components (Leptos/React)

| Component | File | Lines | Purpose |
|-----------|------|-------|---------|
| **IoT Shell** | `EdgeFabric/crates/ef-dashboard/src/pages/virtual_iot_shell.rs` | 1-260 | Embedded terminal card for virtual IoT devices |
| **VeerOS Shell** | `EdgeFabric/crates/ef-dashboard/src/pages/veeros_shell.rs` | 1-240 | Standalone page for VeerOS hosts via SSH |

### Backend API Handlers

| Handler | File | Function | Lines | Status |
|---------|------|----------|-------|--------|
| **IoT Shell WS** | `EdgeFabric/crates/ef-api/src/routes/virtual_iot.rs` | `virtual_iot_shell_ws()` | 653-664 | ✅ Complete |
| **IoT Shell Handler** | `` | `handle_virtual_iot_shell()` | 664-676 | ✅ Complete |
| **IoT Agent Proxy** | `` | `proxy_shell_to_agent()` | 678-747 | ✅ Complete |
| **IoT Local Shell** | `` | `local_shell_session()` | 749-845 | ✅ Complete |
| **VeerOS Shell WS** | `EdgeFabric/crates/ef-api/src/routes/streaming.rs` | `ws_veeros_shell_handler()` | ~200-230 | ✅ Complete |
| **VeerOS Shell Handler** | `` | `handle_veeros_shell()` | ~235-280 | ✅ Complete |
| **SSH Session** | `` | `run_ssh_session()` | 1137-1315 | ✅ Complete |
| **Parse SSH Target** | `` | `parse_ssh_target()` | 1107-1135 | ✅ Complete |

---

## 2. API ROUTES

### IoT Shell Route
```
GET /api/v1/devices/{device_id}/virtual-iot/shell/ws
```
- **Query Parameters**: None
- **Upgrade**: WebSocket
- **Handler**: `virtual_iot::virtual_iot_shell_ws()`

### VeerOS Shell Route
```
GET /api/v1/veeros/shell/ws?host={host}&port={port}&agent_id={agent_id}
```
- **Query Parameters**:
  - `host` (required): Target host IP/hostname
  - `port` (optional, default: 22): SSH port
  - `agent_id` (optional): Remote agent UUID for proxying
- **Upgrade**: WebSocket
- **Handler**: `streaming::ws_veeros_shell_handler()`

---

## 3. WEBSOCKET MESSAGE PROTOCOL DIFFERENCES

### IoT Shell Protocol (Raw Bytes)

**Frontend Client Code** (virtual_iot_shell.rs:~110-115):
```javascript
term.onData(function(data) {
    var ws = window.__ef_shell_ws;
    if (ws && ws.readyState === 1) { ws.send(data); }  // ← Raw bytes!
});
```

**Backend Reception** (virtual_iot.rs:~764-815):
```rust
// In local_shell_session():
while let Some(Ok(msg)) = ws_rx.next().await {
    match msg {
        Message::Text(text) => {
            if child_stdin.write_all(text.as_bytes()).await.is_err() { break; }
        }
        Message::Binary(data) => {
            if child_stdin.write_all(&data).await.is_err() { break; }
        }
        // ← Accepts both Text and Binary, writes directly to veer-connect stdin
    }
}
```

**Message Flow**:
```
Client (xterm.js keystroke)
  ↓ raw byte
WebSocket Message::Text("a") or Message::Binary([97])
  ↓
veer-connect stdin
  ↓
shell
```

---

### VeerOS Shell Protocol (Structured JSON)

**Frontend Client Code** (veeros_shell.rs:~100-125):
```javascript
term.onData(function(data) {
    var ws = window.__ef_veeros_shell_ws;
    if (ws && ws.readyState === 1) {
        ws.send(JSON.stringify({type:'terminal_input', data:data}));  // ← JSON!
    }
});

term.onResize(function(size) {
    var ws = window.__ef_veeros_shell_ws;
    if (ws && ws.readyState === 1) {
        ws.send(JSON.stringify({type:'terminal_resize', cols:size.cols, rows:size.rows}));
    }
});
```

**Backend Reception** (streaming.rs:~1260-1285):
```rust
// In run_ssh_session():
while let Some(Ok(msg)) = ws_rx.next().await {
    match msg {
        Message::Text(t) => {
            let text = t.to_string();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("terminal_input") => {
                        if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
                            let _ = input_tx2.send(data.as_bytes().to_vec()).await;
                            // ← Extracts data field and sends to SSH
                        }
                    }
                    Some("terminal_resize") => {
                        let cols = v.get("cols").and_then(|c| c.as_u64()).unwrap_or(80);
                        let rows = v.get("rows").and_then(|r| r.as_u64()).unwrap_or(24);
                        // ← Handles terminal resize! Sends stty command
                        let cmd = format!("stty cols {} rows {}\r\n", cols, rows);
                        let _ = input_tx2.send(cmd.into_bytes()).await;
                    }
                    _ => {}
                }
            }
        }
    }
}
```

**Message Flow**:
```
Client (xterm.js keystroke + resize event)
  ↓
JSON {type:'terminal_input', data:'a'} or {type:'terminal_resize', cols:120, rows:30}
  ↓
WebSocket Message::Text
  ↓
Parsed, extracted to 'a' or 'stty cols 120 rows 30\r\n'
  ↓
SSH stdin via script PTY
  ↓
shell with proper terminal size!
```

---

## 4. CONNECTION & INITIALIZATION

### IoT Shell Initialization

**Frontend** (virtual_iot_shell.rs:~135-160):
```rust
let ws_url = format!(
    "ws://{host}:6060/api/v1/devices/{}/virtual-iot/shell/ws",
    device_id_for_effect
);
let ws = web_sys::WebSocket::new(&ws_url)?;
```

**Backend** (virtual_iot.rs:~653-676):
1. `virtual_iot_shell_ws()` receives WebSocket upgrade
2. Calls `handle_virtual_iot_shell()`
3. Looks up device → finds attached backend
4. Routes to either:
   - **`proxy_shell_to_agent()`** if `backend.agent_id` is set
     - Connects to agent's `/ws/veeros/{vm_name}/shell` endpoint
     - Relays all messages bidirectionally
   - **`local_shell_session()`** if no agent
     - Spawns veer-connect binary locally
     - Pipes stdin/stdout/stderr through channels

**Server Sends** (virtual_iot.rs:~797):
```json
{
  "type": "shell_ready",
  "device_id": "uuid",
  "host": "192.168.1.100",
  "port": 2323,
  "via": "agent",  // or omitted for local
  "agent_id": "uuid"
}
```

---

### VeerOS Shell Initialization

**Frontend** (veeros_shell.rs:~30-40):
```rust
let ws_path = if agent_q.is_empty() {
    format!("/api/v1/veeros/shell/ws?host={host_q}&port={port_q}")
} else {
    format!("/api/v1/veeros/shell/ws?host={host_q}&port={port_q}&agent_id={agent_q}")
};

let ws_url = format!("ws://{host}:6060{}", ws_path);
let ws = web_sys::WebSocket::new(&ws_url)?;
```

**Backend** (streaming.rs:~200-280):
1. `ws_veeros_shell_handler()` receives WebSocket upgrade + query params
2. Calls `handle_veeros_shell()`
3. If `agent_id` provided:
   - Resolves agent URL
   - Spawns SSH to `host:port` via agent's `/ws/ssh` endpoint
   - **Note**: Uses agent for relay, NOT full veer-connect proxy
4. If no agent:
   - **Spawns SSH locally** directly
   - Uses `script` PTY wrapper for interactive prompts
   - Handles terminal_input/terminal_resize JSON messages

**Server Sends** (streaming.rs:~156):
```json
{
  "type": "shell_ready",
  "host": "192.168.1.100",
  "port": 22,
  "backend": "veeros_host"
}
```

---

## 5. STDIN/STDOUT/STDERR HANDLING

### IoT Shell - Local Path

**Architecture** (virtual_iot.rs:~765-845):
```
┌─────────────────┐
│   xterm.js      │
│   (browser)     │
└────────┬────────┘
         │ WebSocket
         ↓
    ┌────────────────────┐
    │ Leptos WS handler  │
    └────────┬───────────┘
             │ (split)
             ↓
    ┌──────────────────────────────────────────┐
    │  local_shell_session()                   │
    │  ├─ spawn veer-connect                   │
    │  │  ├─ [stdin] ← WS text/binary messages │
    │  │  ├─ [stdout] → WS Text messages       │
    │  │  └─ [stderr] → WS Text (dim ANSI)     │
    │  └─ Use channels to avoid deadlock       │
    └──────────────────────────────────────────┘
         ↓ (3 channels: out_tx_stdout, out_tx_stderr, writer_task)
    ┌────────────────────┐
    │ veer-connect shell │
    │ (encrypts to host) │
    └────────────────────┘
```

**Key Pattern** (virtual_iot.rs:~820-835):
```rust
// Use unbounded_channel to avoid stalling
let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

// Spawn separate tasks for stdout, stderr (avoids holding Mutex across .send().await)
tokio::spawn(async move {
    // reads veer-connect's stdout
    loop {
        match child_stdout.read(&mut buf).await {
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = out_tx_stdout.send(Message::Text(chunk.into()))?;
            }
        }
    }
});

// Single writer task to avoid contention on ws_tx
tokio::spawn(async move {
    while let Some(msg) = out_rx.recv().await {
        if ws_tx.send(msg).await.is_err() { break; }
    }
});
```

---

### IoT Shell - Agent Proxy Path

**Architecture** (virtual_iot.rs:~678-747):
```
┌──────────────────┐
│   xterm.js       │
└────────┬─────────┘
         │ WebSocket
         ↓
    ┌────────────────────────────────────┐
    │ proxy_shell_to_agent()             │
    │ ├─ Connect to agent's             │
    │ │  /ws/veeros/{vm_name}/shell     │
    │ └─ Bidirectional relay            │
    │    agent ↔ dashboard              │
    └────────────────────────────────────┘
         ↓ (tungstenite)
    ┌────────────────────────────────────┐
    │ ef-agent (remote machine)          │
    │ └─ runs veer-connect locally       │
    └────────────────────────────────────┘
```

**Code** (virtual_iot.rs:~708-740):
```rust
// Bidirectional relay
let a2d = tokio::spawn(async move {
    while let Some(msg) = agent_rx.next().await {
        let relay = match msg {
            Ok(TsMsg::Text(t)) => Some(Message::Text(t.to_string().into())),
            Ok(TsMsg::Binary(b)) => Some(Message::Binary(b.into())),
            Ok(TsMsg::Close(_)) | Err(_) => break,
            _ => None,
        };
        if let Some(m) = relay {
            if client_tx.send(m).await.is_err() { break; }
        }
    }
});

let d2a = tokio::spawn(async move {
    while let Some(Ok(msg)) = client_rx.next().await {
        let relay = match msg {
            Message::Text(t) => Some(TsMsg::Text(t.to_string().into())),
            Message::Binary(b) => Some(TsMsg::Binary(b.into())),
            Message::Close(_) => break,
            _ => None,
        };
        if let Some(m) = relay {
            if agent_tx.send(m).await.is_err() { break; }
        }
    }
});
```

---

### VeerOS Shell - SSH Local Path

**Architecture** (streaming.rs:~1137-1315):
```
┌─────────────────┐
│   xterm.js      │
│   (browser)     │
└────────┬────────┘
         │ WebSocket JSON
         ↓
    ┌────────────────────────────────────────┐
    │ run_ssh_session()                      │
    │ ├─ Parse terminal_input JSON           │
    │ ├─ Parse terminal_resize JSON          │
    │ └─ Spawn: script -q /dev/null sh ...   │
    │    └─ Inside: ssh -tt -o ...           │
    └────────────────────────────────────────┘
         ↓ PTY (proper terminal, SIGWINCH support)
    ┌────────────────────────────────────────┐
    │ SSH process with real terminal         │
    │ ├─ Prompts work (password, hostkey)    │
    │ ├─ Terminal size respects stty cols/rows
    │ └─ Full interactive shell              │
    └────────────────────────────────────────┘
```

**Key Differences from IoT**:
1. **Uses `script` wrapper** for PTY emulation (supports interactive prompts)
2. **Spawns SSH process** instead of veer-connect
3. **Parses JSON messages** to extract terminal_input and resize commands
4. **Handles terminal resize** by sending `stty cols X rows Y\r\n`

**JSON Message Parsing** (streaming.rs:~1260-1285):
```rust
match v.get("type").and_then(|t| t.as_str()) {
    Some("terminal_input") => {
        if let Some(data) = v.get("data").and_then(|d| d.as_str()) {
            let _ = input_tx2.send(data.as_bytes().to_vec()).await;
        }
    }
    Some("terminal_resize") => {
        let cols = v.get("cols").and_then(|c| c.as_u64()).unwrap_or(80);
        let rows = v.get("rows").and_then(|r| r.as_u64()).unwrap_or(24);
        let cmd = format!("stty cols {} rows {}\r\n", cols, rows);  // ← Key difference!
        let _ = input_tx2.send(cmd.into_bytes()).await;
    }
    _ => {}
}
```

---

### VeerOS Shell - Agent Proxy Path

**Issue**: Unlike IoT shell, VeerOS shell does NOT proxy to agent when `agent_id` is provided.

**Current Code** (streaming.rs:~235-265):
```rust
async fn handle_veeros_shell(socket: WebSocket, query: VeerosShellQuery, state: AppState) {
    // ...
    if let Some(agent_id) = query.agent_id {
        if let Ok(Some(agent)) = ef_db::repo::agent::AgentRepo::find_by_id(...).await {
            if let Some(ref streaming_url) = agent.streaming_url {
                let agent_ws_url = format!(
                    "{}/ws/ssh?host={}&port={}",
                    streaming_url.trim_end_matches('/'),
                    host,
                    port
                ).replace("http://", "ws://").replace("https://", "wss://");

                tracing::info!(..., "Proxying VeerOS shell via remote agent");
                proxy_ws_to_agent(ws_tx, ws_rx, &agent_ws_url, req_id).await;
                return;
            }
        }
    }

    // Falls through to local SSH if no agent
    run_ssh_session(ws_tx, ws_rx, None, &host, port, req_id).await;
}
```

**Analysis**: 
- The code attempts to proxy to `/ws/ssh` endpoint
- But that endpoint is designed for IoT stream proxying, not JSON terminal_input/terminal_resize
- **Mismatch**: Frontend sends JSON, but agent's `/ws/ssh` expects raw bytes
- **Missing**: Need agent-side SSH handler that understands JSON protocol

---

## 6. KEY DIFFERENCES SUMMARY

| Aspect | IoT Shell | VeerOS Shell |
|--------|-----------|--------------|
| **Frontend Message Format** | Raw bytes | JSON `{type:'terminal_input', data:...}` |
| **Terminal Resize Support** | ❌ Not supported | ✅ JSON `{type:'terminal_resize', ...}` |
| **Shell Backend** | `veer-connect` (custom) | SSH standard |
| **Stdin Handling** | Direct pass-through | Parses JSON, extracts data field |
| **Agent Proxy** | ✅ Full relay to agent's shell WS | ⚠️ Attempts proxy but protocol mismatch |
| **PTY Emulation** | `veer-connect` handles it | `script` wrapper + SSH |
| **Interactive Prompts** | ✅ Works (veer-connect TTY) | ✅ Works (script PTY) |
| **Status** | ✅ Fully implemented | ⚠️ Partially implemented |

---

## 7. MISSING PIECES IN VEEROS SHELL

### 1. Agent SSH Proxy Handler

**Problem**: `handle_veeros_shell()` calls `proxy_ws_to_agent()` which does a simple byte relay, but the frontend sends JSON.

**Missing**: Agent-side handler that:
- Receives JSON `{type:'terminal_input', ...}` from dashboard
- Runs SSH locally
- Sends SSH output back as raw bytes
- Handles terminal resize signals

**Location Needed**: In ef-agent's WebSocket handler (not in VeerOS codebase)

### 2. Terminal Resize Sync Issue

**Current**: Backend correctly parses `terminal_resize` JSON and sends `stty cols X rows Y\r\n`

**Issue**: xterm.js window resize → `term.onResize()` is called → sends JSON message

**Works**: For direct SSH connection (no agent)

**Broken**: When agent proxy is used → agent doesn't understand JSON terminal_resize

### 3. Explicit Integration Test Missing

**Status**: No test in VeerOS repository that validates VeerOS shell → EdgeFabric API → SSH → host connection

---

## 8. COMPARISON TABLE: API ENDPOINT BEHAVIORS

| Aspect | `/api/v1/devices/{id}/virtual-iot/shell/ws` | `/api/v1/veeros/shell/ws` |
|--------|------|------|
| **Query Params** | None (device_id in path) | host, port, agent_id |
| **Message Protocol** | Raw bytes | JSON |
| **Resize Handling** | Not supported | Supported via stty |
| **Local Spawn** | `veer-connect` | `script` + `ssh` |
| **Agent Proxy** | Yes, to `/ws/veeros/{vm}/shell` | Yes, to `/ws/ssh` (mismatch) |
| **Server Banner** | Filters `shell_ready` JSON | Filters `shell_ready` or `stream_ready` JSON |
| **Stderr Handling** | ANSI dimmed via format | Raw via parallel channel |
| **Deadlock Prevention** | Unbounded channel pattern | Select with stderr channel |

---

## 9. LINE NUMBER REFERENCE

### critical Mismatch Points

| Location | Issue |
|----------|-------|
| [virtual_iot_shell.rs:115](EdgeFabric/crates/ef-dashboard/src/pages/virtual_iot_shell.rs#L115) | Raw bytes: `ws.send(data)` |
| [veeros_shell.rs:110](EdgeFabric/crates/ef-dashboard/src/pages/veeros_shell.rs#L110) | JSON: `ws.send(JSON.stringify({type:'terminal_input', data:data}))` |
| [virtual_iot_shell.rs:125](EdgeFabric/crates/ef-dashboard/src/pages/virtual_iot_shell.rs#L125) | No resize handler |
| [veeros_shell.rs:115](EdgeFabric/crates/ef-dashboard/src/pages/veeros_shell.rs#L115) | Resize handler: `term.onResize(...)` |
| [virtual_iot.rs:764](EdgeFabric/crates/ef-api/src/routes/virtual_iot.rs#L764) | Write text/binary directly |
| [streaming.rs:1265](EdgeFabric/crates/ef-api/src/routes/streaming.rs#L1265) | Parse JSON before writing |
| [virtual_iot.rs:678](EdgeFabric/crates/ef-api/src/routes/virtual_iot.rs#L678) | Agent proxy implemented ✅ |
| [streaming.rs:235](EdgeFabric/crates/ef-api/src/routes/streaming.rs#L235) | Agent proxy attempts but protocol mismatch ⚠️ |

---

## Conclusion

The **VeerOS shell implementation is architecturally complete** on the backend but suffers from a **protocol mismatch when using agent proxying**. 

**Recommended Fix Priority**:
1. **High**: Either change VeerOS frontend back to raw bytes (like IoT) OR implement proper JSON handler in agent's `/ws/ssh` endpoint
2. **Medium**: Add integration tests for both local and agent-proxied paths
3. **Low**: Document the protocol differences clearly for future maintainers
