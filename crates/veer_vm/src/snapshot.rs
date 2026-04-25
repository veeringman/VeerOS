//! Snapshot / restore support for `veer-vm`.
//!
//! This initial implementation targets the common single-vCPU VeerOS case
//! without virtio devices. It snapshots guest RAM plus the KVM-managed CPU,
//! irqchip, PIT, and UART model state into a directory of binary files.

use anyhow::{bail, Context, Result};
use kvm_bindings::{
    kvm_debugregs, kvm_irqchip, kvm_lapic_state, kvm_mp_state, kvm_pit_state2, kvm_regs,
    kvm_sregs, kvm_vcpu_events, kvm_xcrs, kvm_xsave, KVM_IRQCHIP_IOAPIC,
    KVM_IRQCHIP_PIC_MASTER, KVM_IRQCHIP_PIC_SLAVE,
};
use kvm_ioctls::{VcpuFd, VmFd};
use std::fs;
use std::mem::{size_of, MaybeUninit};
use std::path::Path;

use crate::memory::GuestMem;
use crate::serial::{Serial16550, SerialSnapshot};
use crate::virtio::blk::{VirtioBlk, VirtioBlkSnapshot};
use crate::virtio::net::{VirtioNet, VirtioNetSnapshot};

const META_MAGIC: [u8; 8] = *b"VEERSNP1";
const META_VERSION: u32 = 1;

#[repr(C)]
struct MetaHeader {
    magic: [u8; 8],
    version: u32,
    pad: u32,
    memory_bytes: u64,
}

pub struct SnapshotMeta {
    pub memory_bytes: usize,
}

pub fn load_meta(dir: &Path) -> Result<SnapshotMeta> {
    let meta: MetaHeader = read_pod(&dir.join("meta.bin"))?;
    if meta.magic != META_MAGIC {
        bail!("snapshot {} has invalid magic", dir.display());
    }
    if meta.version != META_VERSION {
        bail!(
            "snapshot {} has unsupported version {}",
            dir.display(),
            meta.version
        );
    }
    Ok(SnapshotMeta {
        memory_bytes: meta.memory_bytes as usize,
    })
}

pub fn load_net_mac(dir: &Path) -> Result<Option<[u8; 6]>> {
    let present_path = dir.join("net.present");
    let net_present = if present_path.exists() {
        fs::read(&present_path)
            .with_context(|| format!("reading {}", present_path.display()))?
            .starts_with(b"1")
    } else {
        false
    };
    if !net_present {
        return Ok(None);
    }

    let bytes = fs::read(dir.join("net.bin"))
        .with_context(|| format!("reading {}/net.bin", dir.display()))?;
    anyhow::ensure!(bytes.len() >= 6, "short net snapshot");
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&bytes[..6]);
    Ok(Some(mac))
}

pub fn save(
    dir: &Path,
    guest: &GuestMem,
    vcpu: &VcpuFd,
    vm: &VmFd,
    uart: &Serial16550,
    blk: Option<&VirtioBlk>,
    disk_path: Option<&Path>,
    disk_read_only: bool,
    net: Option<&VirtioNet>,
    tap_name: Option<&str>,
    mac: [u8; 6],
) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("creating snapshot dir {}", dir.display()))?;

    let meta = MetaHeader {
        magic: META_MAGIC,
        version: META_VERSION,
        pad: 0,
        memory_bytes: guest.size() as u64,
    };
    write_pod(&dir.join("meta.bin"), &meta)?;
    fs::write(dir.join("memory.bin"), guest.as_slice())
        .with_context(|| format!("writing {}/memory.bin", dir.display()))?;

    write_pod(&dir.join("regs.bin"), &vcpu.get_regs().context("KVM_GET_REGS")?)?;
    write_pod(&dir.join("sregs.bin"), &vcpu.get_sregs().context("KVM_GET_SREGS")?)?;
    write_pod(&dir.join("lapic.bin"), &vcpu.get_lapic().context("KVM_GET_LAPIC")?)?;
    write_pod(
        &dir.join("mp_state.bin"),
        &vcpu.get_mp_state().context("KVM_GET_MP_STATE")?,
    )?;
    write_pod(
        &dir.join("vcpu_events.bin"),
        &vcpu.get_vcpu_events().context("KVM_GET_VCPU_EVENTS")?,
    )?;
    write_pod(&dir.join("xsave.bin"), &vcpu.get_xsave().context("KVM_GET_XSAVE")?)?;
    write_pod(&dir.join("xcrs.bin"), &vcpu.get_xcrs().context("KVM_GET_XCRS")?)?;
    write_pod(
        &dir.join("debugregs.bin"),
        &vcpu.get_debug_regs().context("KVM_GET_DEBUGREGS")?,
    )?;

    write_irqchip(vm, dir, KVM_IRQCHIP_PIC_MASTER, "pic_master.bin")?;
    write_irqchip(vm, dir, KVM_IRQCHIP_PIC_SLAVE, "pic_slave.bin")?;
    write_irqchip(vm, dir, KVM_IRQCHIP_IOAPIC, "ioapic.bin")?;
    write_pod(&dir.join("pit.bin"), &vm.get_pit2().context("KVM_GET_PIT2")?)?;

    write_serial(&dir.join("serial.bin"), &uart.snapshot())?;
    write_blk_state(dir, blk, disk_path, disk_read_only)?;
    write_net_state(dir, net, tap_name, mac)?;
    Ok(())
}

pub fn restore(
    dir: &Path,
    guest: &GuestMem,
    vcpu: &mut VcpuFd,
    vm: &VmFd,
    uart: &mut Serial16550,
    blk: Option<&mut VirtioBlk>,
    disk_path: Option<&Path>,
    disk_read_only: bool,
    net: Option<&mut VirtioNet>,
    tap_name: Option<&str>,
    mac: [u8; 6],
) -> Result<()> {
    let meta = load_meta(dir)?;
    anyhow::ensure!(
        meta.memory_bytes == guest.size(),
        "snapshot memory size {} does not match VM memory size {}",
        meta.memory_bytes,
        guest.size()
    );

    let memory = fs::read(dir.join("memory.bin"))
        .with_context(|| format!("reading {}/memory.bin", dir.display()))?;
    anyhow::ensure!(memory.len() == guest.size(), "snapshot memory dump size mismatch");
    guest.slice_mut(0, guest.size())?.copy_from_slice(&memory);

    let pic_master: kvm_irqchip = read_pod(&dir.join("pic_master.bin"))?;
    let pic_slave: kvm_irqchip = read_pod(&dir.join("pic_slave.bin"))?;
    let ioapic: kvm_irqchip = read_pod(&dir.join("ioapic.bin"))?;
    let pit: kvm_pit_state2 = read_pod(&dir.join("pit.bin"))?;
    vm.set_irqchip(&pic_master)
        .context("KVM_SET_IRQCHIP PIC_MASTER")?;
    vm.set_irqchip(&pic_slave)
        .context("KVM_SET_IRQCHIP PIC_SLAVE")?;
    vm.set_irqchip(&ioapic).context("KVM_SET_IRQCHIP IOAPIC")?;
    vm.set_pit2(&pit).context("KVM_SET_PIT2")?;

    let sregs: kvm_sregs = read_pod(&dir.join("sregs.bin"))?;
    let regs: kvm_regs = read_pod(&dir.join("regs.bin"))?;
    let lapic: kvm_lapic_state = read_pod(&dir.join("lapic.bin"))?;
    let mp_state: kvm_mp_state = read_pod(&dir.join("mp_state.bin"))?;
    let events: kvm_vcpu_events = read_pod(&dir.join("vcpu_events.bin"))?;
    let xsave: kvm_xsave = read_pod(&dir.join("xsave.bin"))?;
    let xcrs: kvm_xcrs = read_pod(&dir.join("xcrs.bin"))?;
    let debugregs: kvm_debugregs = read_pod(&dir.join("debugregs.bin"))?;

    vcpu.set_sregs(&sregs).context("KVM_SET_SREGS")?;
    vcpu.set_regs(&regs).context("KVM_SET_REGS")?;
    vcpu.set_lapic(&lapic).context("KVM_SET_LAPIC")?;
    vcpu.set_mp_state(mp_state).context("KVM_SET_MP_STATE")?;
    vcpu.set_vcpu_events(&events)
        .context("KVM_SET_VCPU_EVENTS")?;
    vcpu.set_xsave(&xsave).context("KVM_SET_XSAVE")?;
    vcpu.set_xcrs(&xcrs).context("KVM_SET_XCRS")?;
    vcpu.set_debug_regs(&debugregs)
        .context("KVM_SET_DEBUGREGS")?;

    let serial = read_serial(&dir.join("serial.bin"))?;
    uart.restore(&serial);
    restore_blk_state(dir, blk, disk_path, disk_read_only)?;
    restore_net_state(dir, net, tap_name, mac)?;
    Ok(())
}

fn write_blk_state(
    dir: &Path,
    blk: Option<&VirtioBlk>,
    disk_path: Option<&Path>,
    disk_read_only: bool,
) -> Result<()> {
    let has_blk = blk.is_some();
    fs::write(dir.join("blk.present"), if has_blk { b"1" } else { b"0" })
        .with_context(|| format!("writing {}/blk.present", dir.display()))?;

    if let Some(blk) = blk {
        let snap = blk.snapshot_state();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&snap.capacity.to_le_bytes());
        bytes.push(if snap.read_only { 1 } else { 0 });
        bytes.push(if disk_read_only { 1 } else { 0 });
        bytes.extend_from_slice(&(snap.transport.device_features).to_le_bytes());
        bytes.extend_from_slice(&(snap.transport.guest_features).to_le_bytes());
        bytes.push(snap.transport.device_status);
        bytes.push(snap.transport.isr_status);
        bytes.extend_from_slice(&snap.transport.queue_select.to_le_bytes());
        let queue_count = snap.transport.queues.len() as u32;
        bytes.extend_from_slice(&queue_count.to_le_bytes());
        for q in &snap.transport.queues {
            bytes.extend_from_slice(&q.pfn.to_le_bytes());
            bytes.extend_from_slice(&q.size.to_le_bytes());
            bytes.extend_from_slice(&q.last_avail_idx.to_le_bytes());
            bytes.extend_from_slice(&q.next_used_idx.to_le_bytes());
        }
        fs::write(dir.join("blk.bin"), bytes)
            .with_context(|| format!("writing {}/blk.bin", dir.display()))?;

        if let Some(path) = disk_path {
            let canonical = canonical_path_string(path);
            fs::write(dir.join("blk.disk_path"), canonical)
                .with_context(|| format!("writing {}/blk.disk_path", dir.display()))?;
        }
    }
    Ok(())
}

fn restore_blk_state(
    dir: &Path,
    blk: Option<&mut VirtioBlk>,
    disk_path: Option<&Path>,
    disk_read_only: bool,
) -> Result<()> {
    let present_path = dir.join("blk.present");
    let blk_present = if present_path.exists() {
        fs::read(&present_path)
            .with_context(|| format!("reading {}", present_path.display()))?
            .starts_with(b"1")
    } else {
        false
    };

    if !blk_present {
        if blk.is_some() {
            bail!("snapshot has no virtio-blk state, but --disk was provided");
        }
        return Ok(());
    }

    let Some(blk) = blk else {
        bail!("snapshot includes virtio-blk state; pass matching --disk on restore");
    };

    let expected_path_file = dir.join("blk.disk_path");
    if expected_path_file.exists() {
        let expected = String::from_utf8(
            fs::read(&expected_path_file)
                .with_context(|| format!("reading {}", expected_path_file.display()))?,
        )
        .context("parsing blk.disk_path as utf-8")?;
        let Some(actual_path) = disk_path else {
            bail!("snapshot requires --disk {}", expected);
        };
        let actual = canonical_path_string(actual_path);
        if expected.trim() != actual {
            bail!("snapshot disk mismatch: expected {}, got {}", expected.trim(), actual);
        }
    }

    let bytes = fs::read(dir.join("blk.bin"))
        .with_context(|| format!("reading {}/blk.bin", dir.display()))?;
    let mut idx = 0usize;

    let take_u64 = |bytes: &[u8], idx: &mut usize| -> Result<u64> {
        let end = *idx + 8;
        anyhow::ensure!(end <= bytes.len(), "truncated blk snapshot");
        let v = u64::from_le_bytes(bytes[*idx..end].try_into().unwrap());
        *idx = end;
        Ok(v)
    };
    let take_u32 = |bytes: &[u8], idx: &mut usize| -> Result<u32> {
        let end = *idx + 4;
        anyhow::ensure!(end <= bytes.len(), "truncated blk snapshot");
        let v = u32::from_le_bytes(bytes[*idx..end].try_into().unwrap());
        *idx = end;
        Ok(v)
    };
    let take_u16 = |bytes: &[u8], idx: &mut usize| -> Result<u16> {
        let end = *idx + 2;
        anyhow::ensure!(end <= bytes.len(), "truncated blk snapshot");
        let v = u16::from_le_bytes(bytes[*idx..end].try_into().unwrap());
        *idx = end;
        Ok(v)
    };
    let take_u8 = |bytes: &[u8], idx: &mut usize| -> Result<u8> {
        anyhow::ensure!(*idx < bytes.len(), "truncated blk snapshot");
        let v = bytes[*idx];
        *idx += 1;
        Ok(v)
    };

    let capacity = take_u64(&bytes, &mut idx)?;
    let read_only = take_u8(&bytes, &mut idx)? != 0;
    let snapshot_disk_ro = take_u8(&bytes, &mut idx)? != 0;
    if snapshot_disk_ro != disk_read_only {
        bail!(
            "snapshot disk read-only mismatch: snapshot={} restore={} (set --disk-ro accordingly)",
            snapshot_disk_ro,
            disk_read_only
        );
    }
    let device_features = take_u32(&bytes, &mut idx)?;
    let guest_features = take_u32(&bytes, &mut idx)?;
    let device_status = take_u8(&bytes, &mut idx)?;
    let isr_status = take_u8(&bytes, &mut idx)?;
    let queue_select = take_u16(&bytes, &mut idx)?;
    let queue_count = take_u32(&bytes, &mut idx)? as usize;

    let mut queues = Vec::with_capacity(queue_count);
    for _ in 0..queue_count {
        queues.push(crate::virtio::QueueState {
            pfn: take_u32(&bytes, &mut idx)?,
            size: take_u16(&bytes, &mut idx)?,
            last_avail_idx: take_u16(&bytes, &mut idx)?,
            next_used_idx: take_u16(&bytes, &mut idx)?,
        });
    }
    anyhow::ensure!(idx == bytes.len(), "unexpected trailing bytes in blk snapshot");

    let snap = VirtioBlkSnapshot {
        transport: crate::virtio::VirtioTransportSnapshot {
            device_features,
            guest_features,
            device_status,
            isr_status,
            queue_select,
            queues,
        },
        capacity,
        read_only,
    };
    blk.restore_state(&snap)
}

fn canonical_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn write_net_state(
    dir: &Path,
    net: Option<&VirtioNet>,
    tap_name: Option<&str>,
    mac: [u8; 6],
) -> Result<()> {
    let has_net = net.is_some();
    fs::write(dir.join("net.present"), if has_net { b"1" } else { b"0" })
        .with_context(|| format!("writing {}/net.present", dir.display()))?;

    if let Some(net) = net {
        let snap = net.snapshot_state();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&snap.mac);
        bytes.extend_from_slice(&(snap.transport.device_features).to_le_bytes());
        bytes.extend_from_slice(&(snap.transport.guest_features).to_le_bytes());
        bytes.push(snap.transport.device_status);
        bytes.push(snap.transport.isr_status);
        bytes.extend_from_slice(&snap.transport.queue_select.to_le_bytes());
        let queue_count = snap.transport.queues.len() as u32;
        bytes.extend_from_slice(&queue_count.to_le_bytes());
        for q in &snap.transport.queues {
            bytes.extend_from_slice(&q.pfn.to_le_bytes());
            bytes.extend_from_slice(&q.size.to_le_bytes());
            bytes.extend_from_slice(&q.last_avail_idx.to_le_bytes());
            bytes.extend_from_slice(&q.next_used_idx.to_le_bytes());
        }
        fs::write(dir.join("net.bin"), bytes)
            .with_context(|| format!("writing {}/net.bin", dir.display()))?;

        fs::write(dir.join("net.tap"), snap.tap_name.as_bytes())
            .with_context(|| format!("writing {}/net.tap", dir.display()))?;
    }

    if let Some(tap) = tap_name {
        fs::write(dir.join("net.tap.requested"), tap.as_bytes())
            .with_context(|| format!("writing {}/net.tap.requested", dir.display()))?;
    }
    fs::write(dir.join("net.mac.requested"), mac)
        .with_context(|| format!("writing {}/net.mac.requested", dir.display()))?;
    Ok(())
}

fn restore_net_state(
    dir: &Path,
    net: Option<&mut VirtioNet>,
    tap_name: Option<&str>,
    mac: [u8; 6],
) -> Result<()> {
    let present_path = dir.join("net.present");
    let net_present = if present_path.exists() {
        fs::read(&present_path)
            .with_context(|| format!("reading {}", present_path.display()))?
            .starts_with(b"1")
    } else {
        false
    };

    if !net_present {
        if net.is_some() {
            bail!("snapshot has no virtio-net state, but --tap was provided");
        }
        return Ok(());
    }

    let Some(net) = net else {
        bail!("snapshot includes virtio-net state; pass matching --tap on restore");
    };

    let expected_tap = String::from_utf8(
        fs::read(dir.join("net.tap"))
            .with_context(|| format!("reading {}/net.tap", dir.display()))?,
    )
    .context("parsing net.tap as utf-8")?;
    let Some(actual_tap) = tap_name else {
        bail!("snapshot requires --tap {}", expected_tap);
    };
    if expected_tap.trim() != actual_tap {
        bail!("snapshot tap mismatch: expected {}, got {}", expected_tap.trim(), actual_tap);
    }

    let bytes = fs::read(dir.join("net.bin"))
        .with_context(|| format!("reading {}/net.bin", dir.display()))?;
    let mut idx = 0usize;

    let take_u32 = |bytes: &[u8], idx: &mut usize| -> Result<u32> {
        let end = *idx + 4;
        anyhow::ensure!(end <= bytes.len(), "truncated net snapshot");
        let v = u32::from_le_bytes(bytes[*idx..end].try_into().unwrap());
        *idx = end;
        Ok(v)
    };
    let take_u16 = |bytes: &[u8], idx: &mut usize| -> Result<u16> {
        let end = *idx + 2;
        anyhow::ensure!(end <= bytes.len(), "truncated net snapshot");
        let v = u16::from_le_bytes(bytes[*idx..end].try_into().unwrap());
        *idx = end;
        Ok(v)
    };
    let take_u8 = |bytes: &[u8], idx: &mut usize| -> Result<u8> {
        anyhow::ensure!(*idx < bytes.len(), "truncated net snapshot");
        let v = bytes[*idx];
        *idx += 1;
        Ok(v)
    };

    anyhow::ensure!(bytes.len() >= 6, "short net snapshot");
    let mut snap_mac = [0u8; 6];
    snap_mac.copy_from_slice(&bytes[idx..idx + 6]);
    idx += 6;
    if snap_mac != mac {
        bail!(
            "snapshot MAC mismatch: snapshot={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} requested={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            snap_mac[0], snap_mac[1], snap_mac[2], snap_mac[3], snap_mac[4], snap_mac[5],
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        );
    }

    let device_features = take_u32(&bytes, &mut idx)?;
    let guest_features = take_u32(&bytes, &mut idx)?;
    let device_status = take_u8(&bytes, &mut idx)?;
    let isr_status = take_u8(&bytes, &mut idx)?;
    let queue_select = take_u16(&bytes, &mut idx)?;
    let queue_count = take_u32(&bytes, &mut idx)? as usize;

    let mut queues = Vec::with_capacity(queue_count);
    for _ in 0..queue_count {
        queues.push(crate::virtio::QueueState {
            pfn: take_u32(&bytes, &mut idx)?,
            size: take_u16(&bytes, &mut idx)?,
            last_avail_idx: take_u16(&bytes, &mut idx)?,
            next_used_idx: take_u16(&bytes, &mut idx)?,
        });
    }
    anyhow::ensure!(idx == bytes.len(), "unexpected trailing bytes in net snapshot");

    let snap = VirtioNetSnapshot {
        transport: crate::virtio::VirtioTransportSnapshot {
            device_features,
            guest_features,
            device_status,
            isr_status,
            queue_select,
            queues,
        },
        mac: snap_mac,
        tap_name: expected_tap.trim().to_string(),
    };
    net.restore_state(&snap)
}

fn write_irqchip(vm: &VmFd, dir: &Path, chip_id: u32, name: &str) -> Result<()> {
    let mut chip = kvm_irqchip {
        chip_id,
        ..unsafe { MaybeUninit::<kvm_irqchip>::zeroed().assume_init() }
    };
    vm.get_irqchip(&mut chip)
        .with_context(|| format!("KVM_GET_IRQCHIP id={chip_id}"))?;
    write_pod(&dir.join(name), &chip)
}

fn write_serial(path: &Path, state: &SerialSnapshot) -> Result<()> {
    let mut bytes = Vec::with_capacity(12 + state.rx.len());
    bytes.extend_from_slice(&[
        state.ier,
        state.lcr,
        state.mcr,
        state.scr,
        state.dll,
        state.dlm,
        0,
        0,
    ]);
    let rx_len = state.rx.len() as u32;
    bytes.extend_from_slice(&rx_len.to_le_bytes());
    bytes.extend_from_slice(&state.rx);
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn read_serial(path: &Path) -> Result<SerialSnapshot> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    anyhow::ensure!(bytes.len() >= 12, "short serial snapshot {}", path.display());
    let rx_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    anyhow::ensure!(
        bytes.len() == 12 + rx_len,
        "serial snapshot length mismatch {}",
        path.display()
    );
    Ok(SerialSnapshot {
        ier: bytes[0],
        lcr: bytes[1],
        mcr: bytes[2],
        scr: bytes[3],
        dll: bytes[4],
        dlm: bytes[5],
        rx: bytes[12..].to_vec(),
    })
}

fn write_pod<T>(path: &Path, value: &T) -> Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn read_pod<T>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    anyhow::ensure!(
        bytes.len() == size_of::<T>(),
        "{} has wrong size: expected {}, got {}",
        path.display(),
        size_of::<T>(),
        bytes.len()
    );
    let mut value = MaybeUninit::<T>::uninit();
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            value.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
        Ok(value.assume_init())
    }
}
