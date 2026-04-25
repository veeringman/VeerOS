//! VeerOS host-runnable demo.
//!
//! This binary boots VeerOS on the host machine by emulating the serial
//! console over stdin / stdout.  It runs the full boot banner sequence
//! followed by the interactive VeerOS shell with **live AI-native
//! subsystems** (agents, intents, memory, fabric).
//!
//! ```
//! cargo run -p veeros-demo
//! ```

use std::io::{self, Read, Write as IoWrite};
use std::os::unix::io::AsRawFd;

use arch::{Console, Serial};
use core::fmt::Write;
use shell::{Shell, ShellEnv};

use microkernel::agent::{AgentState, AgentTable, Goal, GoalPriority};
use microkernel::fabric::{
    ExecutionFabric, LocalityZone, NodeArch, NodeCapability, NodeResources,
};
use microkernel::intent::{IntentClass, IntentEngine, IntentStatus};
use microkernel::intent_sched::IntentScheduler;
use microkernel::memory_engine::{MemoryEngine, MemoryScope, MemoryTag};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ═══════════════════════════════════════════════════════════════════════════
// Terminal raw mode (POSIX)
// ═══════════════════════════════════════════════════════════════════════════

/// Saved original terminal settings so we can restore on exit.
static mut ORIG_TERMIOS: Option<libc::termios> = None;

/// Put stdin into raw mode and save the original settings.
fn enable_raw_mode() {
    unsafe {
        let mut orig: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut orig);
        ORIG_TERMIOS = Some(orig);

        let mut raw = orig;
        libc::cfmakeraw(&mut raw);
        // Keep ISIG so Ctrl-C can still generate SIGINT if needed,
        // but we handle Ctrl-C in the shell itself.
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;  // read returns after 1 byte
        raw.c_cc[libc::VTIME] = 0; // no timeout
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
    }
}

/// Restore the original terminal settings.
fn disable_raw_mode() {
    unsafe {
        if let Some(ref orig) = ORIG_TERMIOS {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, orig);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AI-native kernel subsystem statics
// ═══════════════════════════════════════════════════════════════════════════

static mut AGENTS: AgentTable = AgentTable::new();
static mut INTENTS: IntentEngine = IntentEngine::new();
static mut MEMORY: MemoryEngine = MemoryEngine::new();
static mut FABRIC: ExecutionFabric = ExecutionFabric::new();
static mut SCHED: IntentScheduler = IntentScheduler::new();
/// Monotonic tick counter (incremented on each intent/agent operation).
static mut TICK: u64 = 1;

fn next_tick() -> u64 {
    unsafe {
        TICK += 1;
        TICK
    }
}

// ─── Agent callbacks ────────────────────────────────────────────────────

fn do_get_agent_list(w: &mut dyn Write) {
    unsafe {
        let count = AGENTS.active_count();
        let _ = writeln!(w, "  Active agents: {count}/32");
        let _ = writeln!(w, "  {:>3}  {:>10}  {:>8}  Goal", "ID", "State", "Ticks");
        let _ = writeln!(w, "  ---  ----------  --------  ----");
        for (i, a) in AGENTS.agents.iter().enumerate() {
            if a.state == AgentState::Free {
                continue;
            }
            let state = match a.state {
                AgentState::Free => "free",
                AgentState::Spawned => "spawned",
                AgentState::Planning => "planning",
                AgentState::Executing => "executing",
                AgentState::Blocked => "blocked",
                AgentState::Completed => "completed",
                AgentState::Failed => "failed",
            };
            let desc = a.goal.desc_bytes();
            let goal_str = core::str::from_utf8(desc).unwrap_or("?");
            let _ = writeln!(w, "  {:>3}  {:>10}  {:>8}  {}", i, state, a.ticks_used, goal_str);
        }
    }
}

fn do_agent_cmd(sub: &str, rest: &str, w: &mut dyn Write) {
    unsafe {
        match sub {
            "spawn" => {
                if rest.is_empty() {
                    let _ = writeln!(w, "  usage: agents spawn <goal description>");
                    return;
                }
                let goal = Goal::from_bytes(rest.as_bytes(), GoalPriority::Normal);
                let tick = next_tick();
                match AGENTS.spawn(goal, 0, usize::MAX, tick) {
                    Some(id) => {
                        AGENTS.transition(id, AgentState::Planning);
                        let _ = writeln!(w, "  agent #{id} spawned (planning)");
                    }
                    None => {
                        let _ = writeln!(w, "  error: agent table full");
                    }
                }
            }
            "status" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 || AGENTS.agents[id].state == AgentState::Free {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                let a = &AGENTS.agents[id];
                let desc = core::str::from_utf8(a.goal.desc_bytes()).unwrap_or("?");
                let _ = writeln!(w, "  Agent #{id}");
                let _ = writeln!(w, "    State     : {:?}", a.state);
                let _ = writeln!(w, "    Goal      : {desc}");
                let _ = writeln!(w, "    Ticks used: {}", a.ticks_used);
                let _ = writeln!(w, "    Children  : {}", a.child_count);
                let _ = writeln!(w, "    Replans   : {}/{}", a.replan_count, a.max_replans);
            }
            "execute" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 || AGENTS.agents[id].state == AgentState::Free {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                AGENTS.transition(id, AgentState::Executing);
                let _ = writeln!(w, "  agent #{id} → executing");
            }
            "complete" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                if AGENTS.complete(id) {
                    let _ = writeln!(w, "  agent #{id} → completed");
                } else {
                    let _ = writeln!(w, "  error: cannot complete agent #{id}");
                }
            }
            "kill" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                AGENTS.destroy(id);
                let _ = writeln!(w, "  agent #{id} destroyed");
            }
            "count" => {
                let _ = writeln!(w, "  active agents: {}", AGENTS.active_count());
            }
            _ => {
                let _ = writeln!(w, "  usage: agents [list|spawn|status|execute|complete|kill|count] <args>");
            }
        }
    }
}

// ─── Intent callbacks ───────────────────────────────────────────────────

fn do_get_intent_list(w: &mut dyn Write) {
    unsafe {
        let count = INTENTS.active_count();
        let _ = writeln!(w, "  Active intents: {count}/16");
        let _ = writeln!(w, "  {:>3}  {:>10}  {:>10}  Description", "ID", "Class", "Status");
        let _ = writeln!(w, "  ---  ----------  ----------  -----------");
        for slot in INTENTS.intents.iter() {
            if slot.status == IntentStatus::Free {
                continue;
            }
            let class = match slot.class {
                IntentClass::Compute => "compute",
                IntentClass::Deploy => "deploy",
                IntentClass::Monitor => "monitor",
                IntentClass::Communicate => "comm",
                IntentClass::Data => "data",
                IntentClass::Admin => "admin",
                IntentClass::Pipeline => "pipeline",
                IntentClass::Custom => "custom",
            };
            let status = match slot.status {
                IntentStatus::Free => "free",
                IntentStatus::Pending => "pending",
                IntentStatus::Planning => "planning",
                IntentStatus::Active => "active",
                IntentStatus::Fulfilled => "fulfilled",
                IntentStatus::Failed => "failed",
                IntentStatus::Cancelled => "cancelled",
            };
            let desc = core::str::from_utf8(&slot.description[..slot.desc_len]).unwrap_or("?");
            let _ = writeln!(w, "  {:>3}  {:>10}  {:>10}  {}", slot.id, class, status, desc);
        }
    }
}

fn do_intent_cmd(sub: &str, rest: &str, w: &mut dyn Write) {
    unsafe {
        match sub {
            "submit" => {
                // intent submit <class> <description>
                let (class_str, desc) = match rest.find(' ') {
                    Some(i) => (rest[..i].trim(), rest[i + 1..].trim()),
                    None => (rest.trim(), ""),
                };
                let class = match class_str {
                    "compute" => IntentClass::Compute,
                    "deploy" => IntentClass::Deploy,
                    "monitor" => IntentClass::Monitor,
                    "comm" | "communicate" => IntentClass::Communicate,
                    "data" => IntentClass::Data,
                    "admin" => IntentClass::Admin,
                    "pipeline" => IntentClass::Pipeline,
                    "custom" => IntentClass::Custom,
                    _ => {
                        let _ = writeln!(w, "  unknown class: {class_str}");
                        let _ = writeln!(w, "  classes: compute deploy monitor comm data admin pipeline custom");
                        return;
                    }
                };
                if desc.is_empty() {
                    let _ = writeln!(w, "  usage: intent submit <class> <description>");
                    return;
                }
                let tick = next_tick();
                match INTENTS.submit(class, desc.as_bytes(), GoalPriority::Normal, 0, tick) {
                    Some(id) => {
                        // auto-decompose
                        INTENTS.decompose(id);
                        let _ = writeln!(w, "  intent #{id} submitted (class={class_str})");
                        // show the plan steps
                        let idx = INTENTS.intents.iter().position(|s| s.id == id);
                        if let Some(idx) = idx {
                            let plan = &INTENTS.plans[idx];
                            let _ = writeln!(w, "  plan decomposed into {} step(s):", plan.step_count);
                            for s in 0..plan.step_count {
                                let step = &plan.steps[s];
                                let g = core::str::from_utf8(step.goal.desc_bytes()).unwrap_or("?");
                                let rel = match step.relation {
                                    microkernel::intent::StepRelation::Independent => "independent",
                                    microkernel::intent::StepRelation::DependsOn => "depends-on",
                                    microkernel::intent::StepRelation::Parallel => "parallel",
                                };
                                let _ = writeln!(w, "    step {s}: {g} ({rel})");
                            }
                        }
                    }
                    None => {
                        let _ = writeln!(w, "  error: intent table full");
                    }
                }
            }
            "status" => {
                let id = rest.parse::<u16>().unwrap_or(0);
                match INTENTS.status(id) {
                    Some(s) => {
                        let _ = writeln!(w, "  intent #{id}: {:?}", s);
                    }
                    None => {
                        let _ = writeln!(w, "  error: no intent with id {id}");
                    }
                }
            }
            "cancel" => {
                let id = rest.parse::<u16>().unwrap_or(0);
                if INTENTS.cancel(id) {
                    let _ = writeln!(w, "  intent #{id} cancelled");
                } else {
                    let _ = writeln!(w, "  error: cannot cancel intent #{id}");
                }
            }
            "stats" => {
                let s = &SCHED.stats;
                let _ = writeln!(w, "  Intent Scheduler Statistics");
                let _ = writeln!(w, "    intents submitted : {}", s.intents_submitted);
                let _ = writeln!(w, "    intents fulfilled : {}", s.intents_fulfilled);
                let _ = writeln!(w, "    intents failed    : {}", s.intents_failed);
                let _ = writeln!(w, "    agents spawned    : {}", s.agents_spawned);
                let _ = writeln!(w, "    agents completed  : {}", s.agents_completed);
                let _ = writeln!(w, "    agents failed     : {}", s.agents_failed);
                let _ = writeln!(w, "    replans           : {}", s.replans);
                let _ = writeln!(w, "    last tick          : {}", s.last_tick);
            }
            _ => {
                let _ = writeln!(w, "  usage: intent [list|submit|status|cancel|stats] <args>");
            }
        }
    }
}

// ─── Memory callbacks ───────────────────────────────────────────────────

fn do_memory_cmd(sub: &str, rest: &str, w: &mut dyn Write) {
    unsafe {
        match sub {
            "" | "stats" => {
                let pcount = MEMORY.persistent.count();
                let _ = writeln!(w, "  Memory Engine Status");
                let _ = writeln!(w, "    persistent entries: {pcount}");
            }
            "set" | "store" => {
                // memory set <key> <value>
                let (key, val) = match rest.find(' ') {
                    Some(i) => (rest[..i].trim(), rest[i + 1..].trim()),
                    None => {
                        let _ = writeln!(w, "  usage: memory set <key> <value>");
                        return;
                    }
                };
                let tick = next_tick();
                if MEMORY.persistent.store(
                    key.as_bytes(),
                    val.as_bytes(),
                    MemoryTag::UserKnow,
                    MemoryScope::Global,
                    0,
                    tick,
                    200,
                ) {
                    let _ = writeln!(w, "  stored: {key} = {val}");
                } else {
                    let _ = writeln!(w, "  error: persistent store full");
                }
            }
            "get" | "query" => {
                if rest.is_empty() {
                    let _ = writeln!(w, "  usage: memory get <key>");
                    return;
                }
                match MEMORY.persistent.query(rest.as_bytes(), MemoryScope::Global, 0) {
                    Some(entry) => {
                        let val = core::str::from_utf8(&entry.value[..entry.value_len]).unwrap_or("?");
                        let _ = writeln!(w, "  {rest} = {val}  (reads={}, confidence={})", entry.read_count, entry.confidence);
                    }
                    None => {
                        let _ = writeln!(w, "  key not found: {rest}");
                    }
                }
            }
            "delete" | "del" => {
                if rest.is_empty() {
                    let _ = writeln!(w, "  usage: memory delete <key>");
                    return;
                }
                if MEMORY.persistent.delete(rest.as_bytes(), MemoryScope::Global, 0) {
                    let _ = writeln!(w, "  deleted: {rest}");
                } else {
                    let _ = writeln!(w, "  key not found: {rest}");
                }
            }
            _ => {
                let _ = writeln!(w, "  usage: memory [stats|set|get|delete] <args>");
            }
        }
    }
}

// ─── Fabric callback ────────────────────────────────────────────────────

fn do_get_fabric_status(w: &mut dyn Write) {
    unsafe {
        let (total, healthy) = FABRIC.node_counts();
        let _ = writeln!(w, "  Execution Fabric: {healthy}/{total} nodes healthy");
        let _ = writeln!(w, "  {:>3}  {:>8}  {:>8}  {:>6}  {:>5}  {:>5}  Name",
            "IDX", "Arch", "Zone", "Cores", "RAM", "Load%");
        let _ = writeln!(w, "  ---  --------  --------  ------  -----  -----  ----");
        for (i, node) in FABRIC.nodes.iter().enumerate() {
            if node.health == microkernel::fabric::NodeHealth::Unused {
                continue;
            }
            let arch = match node.arch {
                NodeArch::Riscv32 => "rv32",
                NodeArch::Riscv64 => "rv64",
                NodeArch::Aarch64 => "arm64",
                NodeArch::X86_64 => "x86_64",
                NodeArch::Xtensa => "xtensa",
                NodeArch::Unknown => "??",
            };
            let zone = match node.zone {
                LocalityZone::Local => "local",
                LocalityZone::Rack => "rack",
                LocalityZone::DataCenter => "dc",
                LocalityZone::Region => "region",
                LocalityZone::Global => "global",
            };
            let name = core::str::from_utf8(&node.name[..node.name_len]).unwrap_or("?");
            let load_pct = (node.resources.cpu_load as u32 * 100) / 255;
            let ram_mb = node.resources.ram_kib / 1024;
            let _ = writeln!(w, "  {:>3}  {:>8}  {:>8}  {:>6}  {:>4}M  {:>4}%  {}",
                i, arch, zone, node.resources.cpu_cores, ram_mb, load_pct, name);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Serial backend over host stdin / stdout
// ═══════════════════════════════════════════════════════════════════════════

/// Host serial implementation — stdin for RX, stdout for TX.
struct StdSerial;

impl Serial for StdSerial {
    fn write_byte(&self, byte: u8) {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        if byte == b'\n' {
            // In raw mode bare LF doesn't return the cursor.
            let _ = lock.write_all(b"\r\n");
        } else {
            let _ = lock.write_all(&[byte]);
        }
        let _ = lock.flush();
    }

    fn read_byte(&self) -> u8 {
        let mut buf = [0u8; 1];
        let _ = io::stdin().lock().read_exact(&mut buf);
        buf[0]
    }

    fn has_data(&self) -> bool {
        let fd = io::stdin().as_raw_fd();
        let mut count: libc::c_int = 0;
        unsafe {
            libc::ioctl(fd, libc::FIONREAD, &mut count);
        }
        count > 0
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// main — simulated boot sequence → shell
// ═══════════════════════════════════════════════════════════════════════════

fn main() {
    // Raw terminal so the shell gets byte-at-a-time input.
    enable_raw_mode();

    // Make sure we restore the terminal no matter how we exit.
    // (Ctrl-D / `exit` / panic)
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            disable_raw_mode();
        }
    }
    let _guard = Guard;

    // ── Pre-populate the execution fabric with demo nodes ────────────
    unsafe {
        // Local node — the host machine
        let local = FABRIC.register_local(
            b"host-demo",
            NodeArch::X86_64,
            NodeResources {
                cpu_cores: 4,
                cpu_mhz: 3600,
                ram_kib: 16 * 1024 * 1024, // 16 GiB
                ram_free_kib: 12 * 1024 * 1024,
                accel_count: 0,
                cpu_load: 25,
                active_agents: 0,
            },
        );
        FABRIC.nodes[local].add_capability(NodeCapability::Compute);
        FABRIC.nodes[local].add_capability(NodeCapability::Network);
        FABRIC.nodes[local].add_capability(NodeCapability::CryptoAccel);

        // Simulated remote: a Raspberry Pi 5 edge node
        if let Some(rpi) = FABRIC.register_remote(
            b"rpi5-edge-01",
            NodeArch::Aarch64,
            LocalityZone::Rack,
            (1 << NodeCapability::Compute as u8)
                | (1 << NodeCapability::Sensors as u8)
                | (1 << NodeCapability::LowPower as u8),
            NodeResources {
                cpu_cores: 4,
                cpu_mhz: 2400,
                ram_kib: 8 * 1024 * 1024,
                ram_free_kib: 6 * 1024 * 1024,
                accel_count: 0,
                cpu_load: 10,
                active_agents: 0,
            },
            800, // 0.8ms RTT
            1,
        ) {
            let _ = rpi; // registered
        }

        // Simulated remote: ESP32-C6 sensor node
        if let Some(esp) = FABRIC.register_remote(
            b"esp32c6-sensor",
            NodeArch::Riscv32,
            LocalityZone::Rack,
            (1 << NodeCapability::Sensors as u8)
                | (1 << NodeCapability::Radio as u8)
                | (1 << NodeCapability::LowPower as u8),
            NodeResources {
                cpu_cores: 1,
                cpu_mhz: 160,
                ram_kib: 512,
                ram_free_kib: 384,
                accel_count: 0,
                cpu_load: 5,
                active_agents: 0,
            },
            1200, // 1.2ms RTT
            1,
        ) {
            let _ = esp;
        }

        // Simulated remote: cloud GPU inference node
        if let Some(gpu) = FABRIC.register_remote(
            b"cloud-gpu-a100",
            NodeArch::X86_64,
            LocalityZone::DataCenter,
            (1 << NodeCapability::Compute as u8)
                | (1 << NodeCapability::GpuCompute as u8)
                | (1 << NodeCapability::ModelInference as u8)
                | (1 << NodeCapability::CloudApi as u8),
            NodeResources {
                cpu_cores: 16,
                cpu_mhz: 3200,
                ram_kib: 64 * 1024 * 1024,
                ram_free_kib: 48 * 1024 * 1024,
                accel_count: 8,
                cpu_load: 40,
                active_agents: 0,
            },
            5000, // 5ms RTT
            1,
        ) {
            let _ = gpu;
        }

        // Seed some persistent memory entries
        MEMORY.persistent.store(b"os.name", b"VeerOS", MemoryTag::System, MemoryScope::Global, 0, 1, 255);
        MEMORY.persistent.store(b"os.arch", b"multi-arch", MemoryTag::System, MemoryScope::Global, 0, 1, 255);
        MEMORY.persistent.store(b"demo.mode", b"interactive", MemoryTag::Config, MemoryScope::Global, 0, 1, 200);
    }

    // Build the console over our host serial backend.
    let serial = StdSerial;
    let mut con = Console::new(serial);

    // ── Boot banner ──────────────────────────────────────────────────
    let _ = writeln!(con, "");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "  VeerOS v{VERSION}  \u{2014}  AI-Native OS");
    let _ = writeln!(con, "  Platform : Host (demo)");
    let _ = writeln!(con, "  Scheduler: minimal");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "");
    let _ = writeln!(con, "[boot] trap vector skipped (host build)");
    let _ = writeln!(con, "[boot] interrupt controller  \u{2014} n/a (host)");
    let _ = writeln!(con, "[boot] systimer tick          \u{2014} n/a (host)");
    let _ = writeln!(con, "[boot] idle task registered");
    let _ = writeln!(con, "[ai]   agent table            \u{2014} 32 slots");
    let _ = writeln!(con, "[ai]   intent engine          \u{2014} 16 slots");
    let _ = writeln!(con, "[ai]   memory engine          \u{2014} persistent + episodic");
    unsafe {
        let (total, healthy) = FABRIC.node_counts();
        let _ = writeln!(con, "[ai]   execution fabric      \u{2014} {total} nodes ({healthy} healthy)");
    }
    let _ = writeln!(con, "[ai]   intent scheduler       \u{2014} online");
    let _ = writeln!(con, "[boot] shell starting...");
    let _ = writeln!(con, "");
    let _ = writeln!(con, "Type 'help' for commands.  Try: agents, intent, memory, fabric");
    let _ = writeln!(con, "");

    // ── Launch the shell ─────────────────────────────────────────────
    let env = ShellEnv {
        version: VERSION,
        platform: "Host (demo)",
        scheduler: "minimal",
        get_uptime_ticks: None,
        get_task_list: None,
        get_mem_info: None,
        get_driver_list: None,
        wifi_cmd: None,
        bt_cmd: None,
        zigbee_cmd: None,
        sensor_cmd: None,
        sensor_cmd: None,
        get_current_user: None,
        get_user_list: None,
        vfs_list_dir: None,
        vfs_read_file: None,
        vfs_write_file: None,
        vfs_mkdir: None,
        vfs_stat: None,
        vfs_unlink: None,
        vfs_rename: None,
        vfs_getcwd: None,
        vfs_chdir: None,
        vfs_tree: None,
        vfs_touch: None,
        mount_list: None,
        mount_fs: None,
        umount_fs: None,
        lsblk: None,
        input_status: None,
        usb_list: None,
        ble_hid_list: None,
        gpio_cmd: None,
        i2c_cmd: None,
        spi_cmd: None,
        hw_info: None,
        get_temp_millic: None,
        dmesg: None,
        reboot: None,
        shutdown: None,
        caps_cmd: None,
        auditlog_cmd: None,
        ifconfig_cmd: None,
        ping_cmd: None,
        netstat_cmd: None,
        login: None,
        logout: None,
        change_password: None,
        add_user: None,
        remove_user: None,
        // AI-native — fully wired
        get_agent_list: Some(do_get_agent_list),
        agent_cmd: Some(do_agent_cmd),
        get_intent_list: Some(do_get_intent_list),
        intent_cmd: Some(do_intent_cmd),
        memory_cmd: Some(do_memory_cmd),
        get_fabric_status: Some(do_get_fabric_status),
        peers_cmd: None,
        mesh_cmd: None,
        zkp_cmd: None,
        hostname_cmd: None,
        df_cmd: None,
    };
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    let _ = writeln!(con, "VeerOS halted.");
}
