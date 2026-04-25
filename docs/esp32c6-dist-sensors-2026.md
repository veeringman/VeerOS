# ESP32C6 Distribution Sensors — Integration Notes (2026)

## Overview
Recent VeerOS releases add a virtual sensor subsystem for ESP32C6 (QEMU and Xiao targets):

- **Virtual sensors**: Exposed as `/dev/sensor/<name>` device files (temperature, humidity, pressure, light, ...)
- **Shell commands**: `sensor list`, `sensor read <name>`, `sensor set <name> <value>`
- **Host/EdgeFabric injection**: Sensor values can be injected from the host for simulation/testing
- **Default sensors**: Pre-created at boot; extensible for custom IoT scenarios
- **QEMU and Xiao support**: Both emulated and real hardware targets

## Example
```
sensor list
sensor read temperature
sensor set humidity 6000
cat /dev/sensor/temperature
```

## References
- [kernel/qemu_esp32c6/src/main.rs](../crates/kernel/qemu_esp32c6/src/main.rs)
- [kernel/xiao_esp32c6/src/main.rs](../crates/kernel/xiao_esp32c6/src/main.rs)
- [scripts/veeros-vm](../scripts/veeros-vm)

---

_Last updated: 2026-04-25_
