# ESP32C6 Programming Model — Shell Scripts (2026)

## Overview
VeerOS provides a shell-based programming and deployment model for ESP32C6 targets, supporting rapid build, flash, and monitoring:

- **Scripts**: `build-esp32c6.sh`, `build-qemu-esp32c6.sh`
- **Distribution profiles**: Selectable via `--dist` (minimal, app, rt, full)
- **WiFi/BLE/802.15.4**: Feature flags for radio stacks
- **Auto-detect**: Serial port, flash baud, monitor
- **Remote shell**: Use `veer-connect` for encrypted shell access
- **Integration**: Used in CI, local dev, and for hardware bring-up

## Example Usage
```
./scripts/build-esp32c6.sh --dist full --ssid MyWiFi --password Secret123
veer-connect shell <host> <port>
```

## References
- [scripts/build-esp32c6.sh](../scripts/build-esp32c6.sh)
- [scripts/build-qemu-esp32c6.sh](../scripts/build-qemu-esp32c6.sh)
- [docs/wifi-bringup-esp32c6.md](wifi-bringup-esp32c6.md)

---

_Last updated: 2026-04-25_
