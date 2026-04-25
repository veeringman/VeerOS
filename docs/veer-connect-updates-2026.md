# veer-connect — Secure Connect Updates (2026)

## Overview
`veer-connect` provides secure, encrypted shell and file transfer for VeerOS nodes (including ESP32C6):

- **Modes**: `shell` (interactive terminal), `push` (upload), `pull` (download)
- **Security**: X25519 key exchange, VSC protocol, encrypted session
- **CLI**: Rust binary (`crates/veer-connect`), Python client (`scripts/veeros-connect`)
- **Integration**: Used by `veeros-vm` for ESP32C6 remote shell and file access
- **Progress feedback**: Shows upload/download progress and status

## Example Usage
```
veer-connect shell <host> <port>
veer-connect push <host> <port> <local-path> <remote-path>
veer-connect pull <host> <port> <remote-path> <local-path>
```

## References
- [crates/veer-connect/](../crates/veer-connect/)
- [scripts/veeros-connect](../scripts/veeros-connect)
- [scripts/veeros-vm](../scripts/veeros-vm)

---

_Last updated: 2026-04-25_
