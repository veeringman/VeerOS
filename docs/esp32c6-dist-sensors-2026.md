# ESP32-C6 Distributed Sensors — Integration Notes (2026)

## Overview

Recent VeerOS releases add a virtual sensor subsystem for ESP32-C6 (QEMU and Xiao targets):

- **Virtual sensors**: Exposed as `/dev/sensor/<name>` device files (temperature, humidity, pressure, light, ...)
- **Shell commands**: `sensor list`, `sensor read <name>`, `sensor set <name> <value>`
- **Host/EdgeFabric injection**: Sensor values can be injected from the host via the veer-connect management channel
- **Default sensors**: Pre-created at boot; extensible for custom IoT scenarios
- **QEMU and Xiao support**: Both emulated and real hardware targets

## Shell Commands

```
sensor list
sensor read temperature
sensor set humidity 6000
cat /dev/sensor/temperature
```

## EdgeFabric VirtualIoT Sensor Injection

EdgeFabric can read and write sensor values on any running VirtualIoT ESP32-C6 backend
through the veer-connect management channel. This enables simulation of sensor feeds,
fault injection, and integration test scenarios without physical hardware.

### Prerequisites

A VirtualIoT instance must be running with its veer-connect port mapped (user-net
or TAP). See [docs/edgefabric-esp32c6-runtime.md](edgefabric-esp32c6-runtime.md)
for how to start instances.

### List available sensors

```sh
# user-net: HOST_PORT is the --net-hostfwd-port assigned to the instance
target/debug/veer-connect shell 127.0.0.1 ${HOST_PORT} <<'CMDS'
root
toor
sensor list
exit
CMDS
```

Example output:

```
temperature  : 22.0
humidity     : 60
pressure     : 1013
light        : 512
```

### Read a sensor value

```sh
target/debug/veer-connect shell 127.0.0.1 ${HOST_PORT} <<'CMDS'
root
toor
sensor read temperature
exit
CMDS
```

### Inject a sensor value

```sh
target/debug/veer-connect shell 127.0.0.1 ${HOST_PORT} <<'CMDS'
root
toor
sensor set temperature 37.5
exit
CMDS
```

Sensor values persist in the running instance until the instance stops or a new value
is injected. Writes are immediately visible to subsequent `sensor read` calls.

### Batch injection from host (scripted)

To inject multiple sensors at once:

```sh
inject_sensor() {
  local host="$1" port="$2" name="$3" value="$4"
  printf 'root\ntoor\nsensor set %s %s\nexit\n' "$name" "$value" \
    | target/debug/veer-connect shell "$host" "$port"
}

inject_sensor 127.0.0.1 12001 temperature 38.1
inject_sensor 127.0.0.1 12001 humidity    72
inject_sensor 127.0.0.1 12001 pressure    1009
```

### Polling sensor state from EdgeFabric

EdgeFabric can poll sensor state on a schedule by reading the output of `sensor list`
and parsing the `<name> : <value>` lines. Use the same veer-connect one-shot pattern
above. A minimal poll interval of 1 second is recommended to avoid scheduler
contention in the guest.

## References

- [crates/kernel/qemu_esp32c6/src/main.rs](../crates/kernel/qemu_esp32c6/src/main.rs)
- [crates/kernel/fold_esp32c6/src/main.rs](../crates/kernel/fold_esp32c6/src/main.rs)
- [scripts/veeros-vm](../scripts/veeros-vm)
- [docs/edgefabric-esp32c6-runtime.md](edgefabric-esp32c6-runtime.md)

---

_Last updated: 2026-05-03_
