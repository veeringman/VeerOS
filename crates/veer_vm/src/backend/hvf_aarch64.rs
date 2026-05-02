//! VeerHV-A64: Apple Silicon Hypervisor.framework backend.

use anyhow::{bail, Context, Result};
use object::{Object, ObjectSegment};
use std::ffi::{c_char, c_uchar, c_void, CString};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::ptr;
use std::time::Duration;

use crate::config::{BootSource, GuestArch, VmConfig, VmnetMode};

const HV_SUCCESS: i32 = 0;
const HV_MEMORY_READ: u64 = 1 << 0;
const HV_MEMORY_WRITE: u64 = 1 << 1;
const HV_MEMORY_EXEC: u64 = 1 << 2;
const AARCH64_GPA_BASE: u64 = 0x4000_0000; // RAM mapped at 1 GiB IPA
const AARCH64_KERNEL_LOAD: u64 = 0x4008_0000;
const PL011_UART_BASE: u64 = 0x0900_0000;
const PL011_UART_END: u64 = PL011_UART_BASE + 0x1000;
const PL011_DR: u64 = 0x00;
const PL011_FR: u64 = 0x18;
const PL011_FR_TXFE: u64 = 1 << 7;
const PL011_FR_RXFE: u64 = 1 << 4;
const VIRTIO_MMIO_NET_BASE: u64 = 0x0a00_0000;
const VIRTIO_MMIO_NET_END: u64 = VIRTIO_MMIO_NET_BASE + 0x1000;
const VIRTIO_MMIO_BLK_BASE: u64 = 0x0a00_1000;
const VIRTIO_MMIO_BLK_END: u64 = VIRTIO_MMIO_BLK_BASE + 0x1000;

const VM_MAGIC: u64 = 0x000;
const VM_VERSION: u64 = 0x004;
const VM_DEVICE_ID: u64 = 0x008;
const VM_VENDOR_ID: u64 = 0x00c;
const VM_DEVICE_FEATURES: u64 = 0x010;
const VM_DEVICE_FEATURES_SEL: u64 = 0x014;
const VM_DRIVER_FEATURES: u64 = 0x020;
const VM_DRIVER_FEATURES_SEL: u64 = 0x024;
const VM_QUEUE_SEL: u64 = 0x030;
const VM_QUEUE_NUM_MAX: u64 = 0x034;
const VM_QUEUE_NUM: u64 = 0x038;
const VM_QUEUE_READY: u64 = 0x044;
const VM_QUEUE_NOTIFY: u64 = 0x050;
const VM_INTERRUPT_STATUS: u64 = 0x060;
const VM_INTERRUPT_ACK: u64 = 0x064;
const VM_STATUS: u64 = 0x070;
const VM_QUEUE_DESC_LOW: u64 = 0x080;
const VM_QUEUE_DESC_HIGH: u64 = 0x084;
const VM_QUEUE_AVAIL_LOW: u64 = 0x090;
const VM_QUEUE_AVAIL_HIGH: u64 = 0x094;
const VM_QUEUE_USED_LOW: u64 = 0x0a0;
const VM_QUEUE_USED_HIGH: u64 = 0x0a4;
const VM_CONFIG_BASE: u64 = 0x100;

const VIRTIO_MAGIC: u32 = 0x7472_6976;
const VIRTIO_DEVICE_NET: u32 = 1;
const VIRTIO_DEVICE_BLK: u32 = 2;
const VIRTIO_VENDOR_VEER: u32 = 0x7665_6572;
const VIRTIO_NET_F_MAC: u32 = 1 << 5;
const VIRTIO_BLK_F_RO: u32 = 1 << 5;
const VIRTIO_F_VERSION_1: u32 = 1;
const VIRTIO_NET_HDR_SIZE: usize = 10;
const VIRTIO_NET_QUEUE_SIZE: u32 = 16;
const VIRTIO_BLK_QUEUE_SIZE: u32 = 8;
const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const SECTOR_SIZE: usize = 512;
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

const HV_EXIT_REASON_CANCELED: u32 = 0;
const HV_EXIT_REASON_EXCEPTION: u32 = 1;
const HV_EXIT_REASON_VTIMER_ACTIVATED: u32 = 2;
const HV_EXIT_REASON_UNKNOWN: u32 = 3;

const HV_REG_PC: u32 = 31;
const HV_REG_CPSR: u32 = 34;
const HV_REG_X0: u32 = 0;

const ESR_EC_DABORT_LOWER: u64 = 0x24;
const ESR_EC_DABORT_CURRENT: u64 = 0x25;

const VMNET_SUCCESS: i32 = 1000;
const VMNET_INVALID_ACCESS: i32 = 1005;
const VMNET_NOT_AUTHORIZED: i32 = 1010;

#[link(name = "veer_vmnet_shim", kind = "static")]
unsafe extern "C" {
    fn veer_vmnet_start(mode: u32, mac: *const c_char, out_interface: *mut *mut c_void) -> i32;
    fn veer_vmnet_stop(interface: *mut c_void) -> i32;
    fn veer_vmnet_write_frame(interface: *mut c_void, frame: *const c_uchar, len: usize) -> i32;
    fn veer_vmnet_read_frame(
        interface: *mut c_void,
        frame: *mut c_uchar,
        cap: usize,
        out_len: *mut usize,
    ) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct HvVcpuExitException {
    syndrome: u64,
    virtual_address: u64,
    physical_address: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct HvVcpuExit {
    reason: u32,
    exception: HvVcpuExitException,
}

type HvVcpu = u64;

#[link(name = "Hypervisor", kind = "framework")]
unsafe extern "C" {
    fn hv_vm_create(config: *mut c_void) -> i32;
    fn hv_vm_destroy() -> i32;
    fn hv_vm_map(addr: *mut c_void, ipa: u64, size: usize, flags: u64) -> i32;
    fn hv_vm_unmap(ipa: u64, size: usize) -> i32;
    fn hv_vcpu_create(vcpu: *mut HvVcpu, exit: *mut *const HvVcpuExit, config: *mut c_void) -> i32;
    fn hv_vcpu_destroy(vcpu: HvVcpu) -> i32;
    fn hv_vcpu_get_reg(vcpu: HvVcpu, reg: u32, value: *mut u64) -> i32;
    fn hv_vcpu_set_reg(vcpu: HvVcpu, reg: u32, value: u64) -> i32;
    fn hv_vcpu_run(vcpu: HvVcpu) -> i32;
    fn hv_vcpus_exit(vcpus: *mut HvVcpu, vcpu_count: u32) -> i32;
    fn hv_vcpu_set_vtimer_mask(vcpu: HvVcpu, masked: bool) -> i32;
}

struct HvVm;

impl HvVm {
    fn create() -> Result<Self> {
        hv_check(unsafe { hv_vm_create(ptr::null_mut()) }, "hv_vm_create")?;
        Ok(Self)
    }
}

impl Drop for HvVm {
    fn drop(&mut self) {
        let _ = unsafe { hv_vm_destroy() };
    }
}

struct GuestMem {
    ptr: *mut c_void,
    size: usize,
}

impl GuestMem {
    fn new(size: usize) -> Result<Self> {
        let page_size = page_size();
        let aligned_size = align_up(size, page_size);
        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                aligned_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            bail!("mmap failed while allocating AArch64 guest memory");
        }
        Ok(Self {
            ptr,
            size: aligned_size,
        })
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr.cast::<u8>(), self.size) }
    }

    fn map(&self) -> Result<()> {
        // All MMIO devices (PL011, GIC, virtio-mmio) are below AARCH64_GPA_BASE,
        // so the entire RAM window maps as one contiguous region — no holes.
        eprintln!(
            "[hvf] hv_vm_map IPA {:#010x}..{:#010x} size={:#x} host={:p}",
            AARCH64_GPA_BASE,
            AARCH64_GPA_BASE as usize + self.size,
            self.size,
            self.ptr
        );
        hv_check(
            unsafe {
                hv_vm_map(
                    self.ptr,
                    AARCH64_GPA_BASE,
                    self.size,
                    HV_MEMORY_READ | HV_MEMORY_WRITE | HV_MEMORY_EXEC,
                )
            },
            "hv_vm_map",
        )
    }

    fn read_u16(&self, gpa: u64) -> Result<u16> {
        let bytes = self.read_array::<2>(gpa)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&self, gpa: u64) -> Result<u32> {
        let bytes = self.read_array::<4>(gpa)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64(&self, gpa: u64) -> Result<u64> {
        let bytes = self.read_array::<8>(gpa)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn write_u16(&self, gpa: u64, value: u16) -> Result<()> {
        self.write(gpa, &value.to_le_bytes())
    }

    fn write_u32(&self, gpa: u64, value: u32) -> Result<()> {
        self.write(gpa, &value.to_le_bytes())
    }

    fn read(&self, gpa: u64, len: usize) -> Result<Vec<u8>> {
        if gpa < AARCH64_GPA_BASE {
            bail!("guest read at {:#x} is below GPA base", gpa);
        }
        let start = usize::try_from(gpa - AARCH64_GPA_BASE)
            .context("guest read address does not fit usize")?;
        let end = start
            .checked_add(len)
            .context("guest read range overflow")?;
        if end > self.size {
            bail!("guest read {:#x}..{:#x} outside guest memory", start, end);
        }
        let src = unsafe { std::slice::from_raw_parts(self.ptr.cast::<u8>().add(start), len) };
        Ok(src.to_vec())
    }

    fn write(&self, gpa: u64, data: &[u8]) -> Result<()> {
        if gpa < AARCH64_GPA_BASE {
            bail!("guest write at {:#x} is below GPA base", gpa);
        }
        let start = usize::try_from(gpa - AARCH64_GPA_BASE)
            .context("guest write address does not fit usize")?;
        let end = start
            .checked_add(data.len())
            .context("guest write range overflow")?;
        if end > self.size {
            bail!("guest write {:#x}..{:#x} outside guest memory", start, end);
        }
        let dst =
            unsafe { std::slice::from_raw_parts_mut(self.ptr.cast::<u8>().add(start), data.len()) };
        dst.copy_from_slice(data);
        Ok(())
    }

    fn read_array<const N: usize>(&self, gpa: u64) -> Result<[u8; N]> {
        let bytes = self.read(gpa, N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

impl Drop for GuestMem {
    fn drop(&mut self) {
        let _ = unsafe { hv_vm_unmap(AARCH64_GPA_BASE, self.size) };
        unsafe { libc::munmap(self.ptr, self.size) };
    }
}

#[derive(Clone, Copy)]
struct VirtQueueMmio {
    num: u32,
    ready: bool,
    desc_addr: u64,
    avail_addr: u64,
    used_addr: u64,
    last_avail_idx: u16,
}

impl VirtQueueMmio {
    const fn new() -> Self {
        Self {
            num: VIRTIO_NET_QUEUE_SIZE,
            ready: false,
            desc_addr: 0,
            avail_addr: 0,
            used_addr: 0,
            last_avail_idx: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlkReqHeader {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

struct VmnetInterface {
    interface: *mut c_void,
}

struct VirtioMmioBlk {
    status: u32,
    device_features_sel: u32,
    driver_features_sel: u32,
    driver_features: [u32; 2],
    queue_sel: u32,
    interrupt_status: u32,
    queue: VirtQueueMmio,
    file: File,
    capacity: u64,
    read_only: bool,
}

impl VirtioMmioBlk {
    fn open(path: &Path, read_only: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(!read_only)
            .open(path)
            .with_context(|| format!("open disk image {}", path.display()))?;
        let size = file.metadata()?.len();
        if size == 0 || size % SECTOR_SIZE as u64 != 0 {
            bail!(
                "disk image {} must be non-empty and 512-byte aligned",
                path.display()
            );
        }
        Ok(Self {
            status: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
            driver_features: [0; 2],
            queue_sel: 0,
            interrupt_status: 0,
            queue: VirtQueueMmio::new(),
            file,
            capacity: size / SECTOR_SIZE as u64,
            read_only,
        })
    }

    fn read(&self, offset: u64, width: usize) -> u64 {
        if (VM_CONFIG_BASE..VM_CONFIG_BASE + 8).contains(&offset) {
            let raw = self.capacity.to_le_bytes();
            let start = (offset - VM_CONFIG_BASE) as usize;
            let mut value = 0u64;
            for i in 0..width.min(8 - start) {
                value |= (raw[start + i] as u64) << (i * 8);
            }
            return value;
        }

        let value = match offset & !3 {
            VM_MAGIC => VIRTIO_MAGIC,
            VM_VERSION => 2,
            VM_DEVICE_ID => VIRTIO_DEVICE_BLK,
            VM_VENDOR_ID => VIRTIO_VENDOR_VEER,
            VM_DEVICE_FEATURES => match self.device_features_sel {
                0 => {
                    if self.read_only {
                        VIRTIO_BLK_F_RO
                    } else {
                        0
                    }
                }
                1 => VIRTIO_F_VERSION_1,
                _ => 0,
            },
            VM_QUEUE_NUM_MAX => VIRTIO_BLK_QUEUE_SIZE,
            VM_QUEUE_READY => self.queue.ready as u32,
            VM_INTERRUPT_STATUS => self.interrupt_status,
            VM_STATUS => self.status,
            _ => 0,
        };
        lane_value(value, offset, width)
    }

    fn write(&mut self, guest: &GuestMem, offset: u64, width: usize, value: u64) -> Result<()> {
        let value = lane_write_value(value, width);
        match offset & !3 {
            VM_DEVICE_FEATURES_SEL => self.device_features_sel = value,
            VM_DRIVER_FEATURES_SEL => self.driver_features_sel = value,
            VM_DRIVER_FEATURES => {
                let sel = (self.driver_features_sel & 1) as usize;
                self.driver_features[sel] = value;
            }
            VM_QUEUE_SEL => self.queue_sel = value,
            VM_QUEUE_NUM => self.queue.num = value.min(VIRTIO_BLK_QUEUE_SIZE),
            VM_QUEUE_READY => self.queue.ready = value != 0,
            VM_QUEUE_DESC_LOW => {
                self.queue.desc_addr = (self.queue.desc_addr & !0xffff_ffff) | value as u64
            }
            VM_QUEUE_DESC_HIGH => {
                self.queue.desc_addr = (self.queue.desc_addr & 0xffff_ffff) | ((value as u64) << 32)
            }
            VM_QUEUE_AVAIL_LOW => {
                self.queue.avail_addr = (self.queue.avail_addr & !0xffff_ffff) | value as u64
            }
            VM_QUEUE_AVAIL_HIGH => {
                self.queue.avail_addr =
                    (self.queue.avail_addr & 0xffff_ffff) | ((value as u64) << 32)
            }
            VM_QUEUE_USED_LOW => {
                self.queue.used_addr = (self.queue.used_addr & !0xffff_ffff) | value as u64
            }
            VM_QUEUE_USED_HIGH => {
                self.queue.used_addr = (self.queue.used_addr & 0xffff_ffff) | ((value as u64) << 32)
            }
            VM_QUEUE_NOTIFY => self.process_queue(guest)?,
            VM_INTERRUPT_ACK => self.interrupt_status &= !value,
            VM_STATUS => {
                if value == 0 {
                    self.status = 0;
                    self.queue_sel = 0;
                    self.interrupt_status = 0;
                    self.driver_features = [0; 2];
                    self.queue = VirtQueueMmio::new();
                } else {
                    self.status = value;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn process_queue(&mut self, guest: &GuestMem) -> Result<()> {
        loop {
            let Some(head) = pop_avail(guest, &mut self.queue)? else {
                break;
            };
            let used_len = match self.process_one(guest, head) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("[veer-vm] virtio-mmio-blk: request failed: {e:#}");
                    0
                }
            };
            push_used(guest, &self.queue, head, used_len)?;
            self.interrupt_status |= 1;
        }
        Ok(())
    }

    fn process_one(&mut self, guest: &GuestMem, head: u16) -> Result<u32> {
        let chain = read_chain(guest, &self.queue, head)?;
        if chain.len() < 3 {
            bail!("blk request chain too short");
        }
        let hdr_desc = chain[0];
        if hdr_desc.len < 16 {
            bail!("blk request header too small");
        }
        let hdr = guest.read(hdr_desc.addr, 16)?;
        let req_type = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
        let sector = u64::from_le_bytes(hdr[8..16].try_into().unwrap());
        let status_desc = chain[chain.len() - 1];
        if (status_desc.flags & VIRTQ_DESC_F_WRITE) == 0 || status_desc.len == 0 {
            bail!("blk request missing writable status descriptor");
        }

        let mut write_status = |byte: u8| -> Result<()> { guest.write(status_desc.addr, &[byte]) };

        match req_type {
            VIRTIO_BLK_T_IN => {
                let mut copied = 0u32;
                let mut disk_off = sector * SECTOR_SIZE as u64;
                for desc in &chain[1..chain.len() - 1] {
                    if (desc.flags & VIRTQ_DESC_F_WRITE) == 0 {
                        write_status(VIRTIO_BLK_S_IOERR)?;
                        bail!("blk read data descriptor is not writable");
                    }
                    if disk_off + desc.len as u64 > self.capacity * SECTOR_SIZE as u64 {
                        write_status(VIRTIO_BLK_S_IOERR)?;
                        bail!("blk read past end of disk");
                    }
                    let mut buf = vec![0u8; desc.len as usize];
                    self.file.read_exact_at(&mut buf, disk_off)?;
                    guest.write(desc.addr, &buf)?;
                    disk_off += desc.len as u64;
                    copied = copied.saturating_add(desc.len);
                }
                write_status(VIRTIO_BLK_S_OK)?;
                Ok(copied + 1)
            }
            VIRTIO_BLK_T_OUT => {
                if self.read_only {
                    write_status(VIRTIO_BLK_S_IOERR)?;
                    return Ok(1);
                }
                let mut disk_off = sector * SECTOR_SIZE as u64;
                for desc in &chain[1..chain.len() - 1] {
                    if (desc.flags & VIRTQ_DESC_F_WRITE) != 0 {
                        write_status(VIRTIO_BLK_S_IOERR)?;
                        bail!("blk write data descriptor is writable");
                    }
                    if disk_off + desc.len as u64 > self.capacity * SECTOR_SIZE as u64 {
                        write_status(VIRTIO_BLK_S_IOERR)?;
                        bail!("blk write past end of disk");
                    }
                    let buf = guest.read(desc.addr, desc.len as usize)?;
                    self.file.write_all_at(&buf, disk_off)?;
                    disk_off += desc.len as u64;
                }
                write_status(VIRTIO_BLK_S_OK)?;
                Ok(1)
            }
            other => {
                eprintln!("[veer-vm] virtio-mmio-blk: unsupported request type {other}");
                write_status(VIRTIO_BLK_S_UNSUPP)?;
                Ok(1)
            }
        }
    }
}

impl VmnetInterface {
    fn start(mode: VmnetMode, mac: [u8; 6]) -> Result<Self> {
        let mode_raw = match mode {
            VmnetMode::Host => 1,
            VmnetMode::Shared => 2,
        };
        let mac = CString::new(format_mac(mac)).context("formatting vmnet MAC")?;
        let mut interface = ptr::null_mut();
        let rc = unsafe { veer_vmnet_start(mode_raw, mac.as_ptr(), &mut interface) };
        if rc != VMNET_SUCCESS {
            bail!("{}", vmnet_error(rc));
        }
        if interface.is_null() {
            bail!("vmnet_start_interface returned success without an interface");
        }
        Ok(Self { interface })
    }

    fn write_frame(&self, frame: &[u8]) -> Result<()> {
        let rc = unsafe { veer_vmnet_write_frame(self.interface, frame.as_ptr(), frame.len()) };
        if rc == VMNET_SUCCESS {
            Ok(())
        } else {
            bail!("vmnet_write failed: {}", vmnet_return_name(rc))
        }
    }

    fn read_frame(&self, frame: &mut [u8]) -> Result<Option<usize>> {
        let mut len = 0usize;
        let rc = unsafe {
            veer_vmnet_read_frame(self.interface, frame.as_mut_ptr(), frame.len(), &mut len)
        };
        if rc == VMNET_SUCCESS {
            Ok((len > 0).then_some(len))
        } else {
            bail!("vmnet_read failed: {}", vmnet_return_name(rc))
        }
    }
}

impl Drop for VmnetInterface {
    fn drop(&mut self) {
        let _ = unsafe { veer_vmnet_stop(self.interface) };
    }
}

struct VirtioMmioNet {
    status: u32,
    device_features_sel: u32,
    driver_features_sel: u32,
    driver_features: [u32; 2],
    queue_sel: u32,
    interrupt_status: u32,
    mac: [u8; 6],
    queues: [VirtQueueMmio; 2],
    vmnet: VmnetInterface,
    rx_buf: [u8; 2048],
}

impl VirtioMmioNet {
    fn start(mode: VmnetMode, mac: [u8; 6]) -> Result<Self> {
        let vmnet = VmnetInterface::start(mode, mac)?;
        Ok(Self {
            status: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
            driver_features: [0; 2],
            queue_sel: 0,
            interrupt_status: 0,
            mac,
            queues: [VirtQueueMmio::new(), VirtQueueMmio::new()],
            vmnet,
            rx_buf: [0; 2048],
        })
    }

    fn read(&self, offset: u64, width: usize) -> u64 {
        if offset >= VM_CONFIG_BASE && offset < VM_CONFIG_BASE + 6 {
            return self.mac[(offset - VM_CONFIG_BASE) as usize] as u64;
        }

        let value = match offset & !3 {
            VM_MAGIC => VIRTIO_MAGIC,
            VM_VERSION => 2,
            VM_DEVICE_ID => VIRTIO_DEVICE_NET,
            VM_VENDOR_ID => VIRTIO_VENDOR_VEER,
            VM_DEVICE_FEATURES => match self.device_features_sel {
                0 => VIRTIO_NET_F_MAC,
                1 => VIRTIO_F_VERSION_1,
                _ => 0,
            },
            VM_QUEUE_NUM_MAX => VIRTIO_NET_QUEUE_SIZE,
            VM_QUEUE_READY => self.queues[(self.queue_sel & 1) as usize].ready as u32,
            VM_INTERRUPT_STATUS => self.interrupt_status,
            VM_STATUS => self.status,
            _ => 0,
        };
        lane_value(value, offset, width)
    }

    fn write(&mut self, guest: &GuestMem, offset: u64, width: usize, value: u64) -> Result<()> {
        let value = lane_write_value(value, width);
        let queue_idx = (self.queue_sel & 1) as usize;
        match offset & !3 {
            VM_DEVICE_FEATURES_SEL => self.device_features_sel = value,
            VM_DRIVER_FEATURES_SEL => self.driver_features_sel = value,
            VM_DRIVER_FEATURES => {
                let sel = (self.driver_features_sel & 1) as usize;
                self.driver_features[sel] = value;
            }
            VM_QUEUE_SEL => self.queue_sel = value,
            VM_QUEUE_NUM => self.queues[queue_idx].num = value.min(VIRTIO_NET_QUEUE_SIZE),
            VM_QUEUE_READY => self.queues[queue_idx].ready = value != 0,
            VM_QUEUE_DESC_LOW => {
                let queue = &mut self.queues[queue_idx];
                queue.desc_addr = (queue.desc_addr & !0xffff_ffff) | value as u64;
            }
            VM_QUEUE_DESC_HIGH => {
                let queue = &mut self.queues[queue_idx];
                queue.desc_addr = (queue.desc_addr & 0xffff_ffff) | ((value as u64) << 32);
            }
            VM_QUEUE_AVAIL_LOW => {
                let queue = &mut self.queues[queue_idx];
                queue.avail_addr = (queue.avail_addr & !0xffff_ffff) | value as u64;
            }
            VM_QUEUE_AVAIL_HIGH => {
                let queue = &mut self.queues[queue_idx];
                queue.avail_addr = (queue.avail_addr & 0xffff_ffff) | ((value as u64) << 32);
            }
            VM_QUEUE_USED_LOW => {
                let queue = &mut self.queues[queue_idx];
                queue.used_addr = (queue.used_addr & !0xffff_ffff) | value as u64;
            }
            VM_QUEUE_USED_HIGH => {
                let queue = &mut self.queues[queue_idx];
                queue.used_addr = (queue.used_addr & 0xffff_ffff) | ((value as u64) << 32);
            }
            VM_QUEUE_NOTIFY => {
                if value == 1 {
                    self.process_tx(guest)?;
                } else if value == 0 {
                    self.pump_rx(guest)?;
                }
            }
            VM_INTERRUPT_ACK => self.interrupt_status &= !value,
            VM_STATUS => {
                if value == 0 {
                    self.status = 0;
                    self.queue_sel = 0;
                    self.interrupt_status = 0;
                    self.driver_features = [0; 2];
                    self.queues = [VirtQueueMmio::new(), VirtQueueMmio::new()];
                } else {
                    self.status = value;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn process_tx(&mut self, guest: &GuestMem) -> Result<()> {
        loop {
            let Some(head) = pop_avail(guest, &mut self.queues[1])? else {
                break;
            };
            let desc = read_desc(guest, self.queues[1].desc_addr, head)?;
            let packet = guest.read(desc.addr, desc.len as usize)?;
            if packet.len() > VIRTIO_NET_HDR_SIZE {
                self.vmnet.write_frame(&packet[VIRTIO_NET_HDR_SIZE..])?;
            }
            push_used(guest, &self.queues[1], head, desc.len)?;
            self.interrupt_status |= 1;
        }
        Ok(())
    }

    fn pump_rx(&mut self, guest: &GuestMem) -> Result<()> {
        while let Some(len) = self.vmnet.read_frame(&mut self.rx_buf)? {
            let frame = self.rx_buf[..len].to_vec();
            if !self.deliver_rx(guest, &frame)? {
                break;
            }
        }
        Ok(())
    }

    fn deliver_rx(&mut self, guest: &GuestMem, frame: &[u8]) -> Result<bool> {
        let Some(head) = pop_avail(guest, &mut self.queues[0])? else {
            return Ok(false);
        };
        let desc = read_desc(guest, self.queues[0].desc_addr, head)?;
        if (desc.flags & VIRTQ_DESC_F_WRITE) == 0 || desc.len as usize <= VIRTIO_NET_HDR_SIZE {
            push_used(guest, &self.queues[0], head, 0)?;
            return Ok(false);
        }

        let frame_len = frame.len().min(desc.len as usize - VIRTIO_NET_HDR_SIZE);
        guest.write(desc.addr, &[0u8; VIRTIO_NET_HDR_SIZE])?;
        guest.write(desc.addr + VIRTIO_NET_HDR_SIZE as u64, &frame[..frame_len])?;
        push_used(
            guest,
            &self.queues[0],
            head,
            (VIRTIO_NET_HDR_SIZE + frame_len) as u32,
        )?;
        self.interrupt_status |= 1;
        Ok(true)
    }
}

fn pop_avail(guest: &GuestMem, queue: &mut VirtQueueMmio) -> Result<Option<u16>> {
    if !queue.ready || queue.num == 0 || queue.avail_addr == 0 {
        return Ok(None);
    }
    let avail_idx = guest.read_u16(queue.avail_addr + 2)?;
    if queue.last_avail_idx == avail_idx {
        return Ok(None);
    }
    let slot = (queue.last_avail_idx as u32 % queue.num) as u64;
    let head = guest.read_u16(queue.avail_addr + 4 + slot * 2)?;
    queue.last_avail_idx = queue.last_avail_idx.wrapping_add(1);
    Ok(Some(head))
}

fn push_used(guest: &GuestMem, queue: &VirtQueueMmio, head: u16, len: u32) -> Result<()> {
    if queue.used_addr == 0 || queue.num == 0 {
        return Ok(());
    }
    let used_idx = guest.read_u16(queue.used_addr + 2)?;
    let slot = (used_idx as u32 % queue.num) as u64;
    guest.write_u32(queue.used_addr + 4 + slot * 8, head as u32)?;
    guest.write_u32(queue.used_addr + 4 + slot * 8 + 4, len)?;
    guest.write_u16(queue.used_addr + 2, used_idx.wrapping_add(1))?;
    Ok(())
}

fn read_desc(guest: &GuestMem, desc_base: u64, head: u16) -> Result<VirtqDesc> {
    let addr = desc_base + head as u64 * 16;
    Ok(VirtqDesc {
        addr: guest.read_u64(addr)?,
        len: guest.read_u32(addr + 8)?,
        flags: guest.read_u16(addr + 12)?,
        next: guest.read_u16(addr + 14)?,
    })
}

fn read_chain(guest: &GuestMem, queue: &VirtQueueMmio, head: u16) -> Result<Vec<VirtqDesc>> {
    let mut out = Vec::new();
    let mut current = head;
    for _ in 0..queue.num.max(1) {
        let desc = read_desc(guest, queue.desc_addr, current)?;
        out.push(desc);
        if (desc.flags & VIRTQ_DESC_F_NEXT) == 0 {
            return Ok(out);
        }
        current = desc.next;
    }
    bail!("virtqueue descriptor chain loop or too long")
}

fn lane_value(value: u32, offset: u64, width: usize) -> u64 {
    let shifted = value >> ((offset & 3) * 8);
    match width {
        1 => (shifted & 0xff) as u64,
        2 => (shifted & 0xffff) as u64,
        _ => value as u64,
    }
}

fn lane_write_value(value: u64, width: usize) -> u32 {
    match width {
        1 => (value & 0xff) as u32,
        2 => (value & 0xffff) as u32,
        _ => value as u32,
    }
}

fn format_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

fn vmnet_return_name(rc: i32) -> &'static str {
    match rc {
        VMNET_SUCCESS => "VMNET_SUCCESS",
        1001 => "VMNET_FAILURE",
        1002 => "VMNET_MEM_FAILURE",
        1003 => "VMNET_INVALID_ARGUMENT",
        1004 => "VMNET_SETUP_INCOMPLETE",
        VMNET_INVALID_ACCESS => "VMNET_INVALID_ACCESS",
        1006 => "VMNET_PACKET_TOO_BIG",
        1007 => "VMNET_BUFFER_EXHAUSTED",
        1008 => "VMNET_TOO_MANY_PACKETS",
        1009 => "VMNET_SHARING_SERVICE_BUSY",
        VMNET_NOT_AUTHORIZED => "VMNET_NOT_AUTHORIZED",
        _ => "VMNET_UNKNOWN",
    }
}

fn vmnet_error(rc: i32) -> String {
    let name = vmnet_return_name(rc);
    if rc == VMNET_INVALID_ACCESS || rc == VMNET_NOT_AUTHORIZED {
        format!(
            "vmnet_start_interface failed: {name} ({rc}). This host may require the com.apple.vm.networking entitlement or administrator approval; current user is non-admin, so use admin user vijaysharma if macOS prompts or policy requires approval."
        )
    } else {
        format!("vmnet_start_interface failed: {name} ({rc})")
    }
}

fn page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

fn hv_check(rc: i32, what: &str) -> Result<()> {
    if rc == HV_SUCCESS {
        Ok(())
    } else {
        bail!("{} failed with hv_return_t={}", what, rc)
    }
}

fn load_kernel(path: &Path, guest: &mut GuestMem) -> Result<u64> {
    let bytes = fs::read(path).with_context(|| format!("reading kernel {}", path.display()))?;
    if let Ok(elf) = object::File::parse(&*bytes) {
        let entry = elf.entry();
        for segment in elf.segments() {
            let address = segment.address();
            let size = segment.size();
            if size == 0 {
                continue;
            }
            // ELF VAs are in IPA space (>= AARCH64_GPA_BASE); subtract the
            // base to get the byte offset within the host allocation.
            if address < AARCH64_GPA_BASE {
                bail!(
                    "ELF segment at {:#x} is below GPA base {:#x}",
                    address,
                    AARCH64_GPA_BASE
                );
            }
            let start = usize::try_from(address - AARCH64_GPA_BASE)
                .context("ELF segment address does not fit host usize")?;
            let size = usize::try_from(size).context("ELF segment size does not fit host usize")?;
            let end = start
                .checked_add(size)
                .context("ELF segment overflows guest memory range")?;
            if end > guest.size {
                bail!("ELF segment {:#x}..{:#x} exceeds guest memory", start, end);
            }
            let data = segment.data().context("reading ELF segment data")?;
            let dst = &mut guest.as_mut_slice()[start..end];
            dst.fill(0);
            let copy_len = data.len().min(dst.len());
            dst[..copy_len].copy_from_slice(&data[..copy_len]);
        }
        return Ok(entry);
    }

    // Raw binary: load at kernel load address.
    let start = usize::try_from(AARCH64_KERNEL_LOAD - AARCH64_GPA_BASE).unwrap();
    let end = start
        .checked_add(bytes.len())
        .context("raw kernel image overflows guest memory range")?;
    if end > guest.size {
        bail!("raw kernel image exceeds guest memory");
    }
    guest.as_mut_slice()[start..end].copy_from_slice(&bytes);
    Ok(AARCH64_KERNEL_LOAD)
}

fn read_reg(vcpu: HvVcpu, reg: u32) -> Result<u64> {
    let mut value = 0u64;
    hv_check(
        unsafe { hv_vcpu_get_reg(vcpu, reg, &mut value) },
        "hv_vcpu_get_reg",
    )?;
    Ok(value)
}

fn write_reg(vcpu: HvVcpu, reg: u32, value: u64) -> Result<()> {
    hv_check(
        unsafe { hv_vcpu_set_reg(vcpu, reg, value) },
        "hv_vcpu_set_reg",
    )
}

fn advance_pc(vcpu: HvVcpu) -> Result<()> {
    let pc = read_reg(vcpu, HV_REG_PC)?;
    write_reg(vcpu, HV_REG_PC, pc + 4)
}

fn handle_data_abort(
    vcpu: HvVcpu,
    exit: &HvVcpuExit,
    guest: &GuestMem,
    net: Option<&mut VirtioMmioNet>,
    blk: Option<&mut VirtioMmioBlk>,
) -> Result<bool> {
    let syndrome = exit.exception.syndrome;
    let ec = (syndrome >> 26) & 0x3f;
    if ec != ESR_EC_DABORT_LOWER && ec != ESR_EC_DABORT_CURRENT {
        return Ok(false);
    }

    let ipa = exit.exception.physical_address;
    let iss = syndrome & 0x01ff_ffff;
    let is_write = ((iss >> 6) & 1) != 0;
    let srt = ((iss >> 16) & 0x1f) as u32;
    let width = 1usize << ((iss >> 22) & 0x3);

    if (PL011_UART_BASE..PL011_UART_END).contains(&ipa) {
        let offset = ipa - PL011_UART_BASE;

        if is_write && offset == PL011_DR {
            let value = read_reg(vcpu, HV_REG_X0 + srt)?;
            print!("{}", (value as u8) as char);
            let _ = std::io::stdout().flush();
            advance_pc(vcpu)?;
            return Ok(true);
        }

        if !is_write {
            let value = match offset {
                PL011_FR => PL011_FR_TXFE | PL011_FR_RXFE,
                _ => 0,
            };
            write_reg(vcpu, HV_REG_X0 + srt, value)?;
            advance_pc(vcpu)?;
            return Ok(true);
        }

        return Ok(false);
    }

    if (VIRTIO_MMIO_NET_BASE..VIRTIO_MMIO_NET_END).contains(&ipa) {
        let offset = ipa - VIRTIO_MMIO_NET_BASE;
        if let Some(net) = net {
            if is_write {
                let value = read_reg(vcpu, HV_REG_X0 + srt)?;
                net.write(guest, offset, width, value)?;
            } else {
                let value = net.read(offset, width);
                write_reg(vcpu, HV_REG_X0 + srt, value)?;
            }
        } else {
            // No vmnet session: return 0 for reads so the guest probe fails
            // gracefully, silently ignore writes.
            if !is_write {
                write_reg(vcpu, HV_REG_X0 + srt, 0)?;
            }
        }
        advance_pc(vcpu)?;
        return Ok(true);
    }

    if (VIRTIO_MMIO_BLK_BASE..VIRTIO_MMIO_BLK_END).contains(&ipa) {
        let offset = ipa - VIRTIO_MMIO_BLK_BASE;
        if let Some(blk) = blk {
            if is_write {
                let value = read_reg(vcpu, HV_REG_X0 + srt)?;
                blk.write(guest, offset, width, value)?;
            } else {
                let value = blk.read(offset, width);
                write_reg(vcpu, HV_REG_X0 + srt, value)?;
            }
        } else if !is_write {
            write_reg(vcpu, HV_REG_X0 + srt, 0)?;
        }
        advance_pc(vcpu)?;
        return Ok(true);
    }

    Ok(false)
}

pub fn preflight() -> Result<()> {
    eprintln!("[veer-vm] Apple Silicon HVF preflight");
    probe()
}

pub fn probe() -> Result<()> {
    let _vm = HvVm::create()?;
    eprintln!("[veer-vm] hv_vm_create/hv_vm_destroy: ok");
    Ok(())
}

pub fn vcpu_probe() -> Result<()> {
    let _vm = HvVm::create()?;
    let mut vcpu = 0;
    let mut exit: *const HvVcpuExit = ptr::null();
    hv_check(
        unsafe { hv_vcpu_create(&mut vcpu, &mut exit, ptr::null_mut()) },
        "hv_vcpu_create",
    )?;
    hv_check(unsafe { hv_vcpu_destroy(vcpu) }, "hv_vcpu_destroy")?;
    eprintln!("[veer-vm] hv_vcpu_create/hv_vcpu_destroy: ok");
    Ok(())
}

pub fn run(cfg: VmConfig) -> Result<()> {
    if cfg.guest_arch != GuestArch::Aarch64 {
        bail!("Apple Silicon HVF backend requires --arch aarch64");
    }
    if cfg.cpus != 1 {
        bail!("Apple Silicon HVF backend currently supports one vCPU");
    }
    if cfg.tap_name.is_some() {
        bail!("--tap is not supported on Apple Silicon HVF; use --vmnet shared|host");
    }

    let kernel_path = match cfg.boot {
        BootSource::Kernel(path) => path,
        BootSource::Snapshot(_) => {
            bail!("snapshot restore is not implemented for Apple Silicon HVF")
        }
    };

    let _vm = HvVm::create()?;
    let mut guest = GuestMem::new(cfg.memory_bytes)?;
    let entry = load_kernel(&kernel_path, &mut guest)?;
    guest.map()?;
    let mut net = if let Some(mode) = cfg.vmnet_mode {
        eprintln!(
            "[veer-vm] virtio-mmio-net: base={:#x} vmnet={} mac={}",
            VIRTIO_MMIO_NET_BASE,
            mode.as_str(),
            format_mac(cfg.mac)
        );
        Some(VirtioMmioNet::start(mode, cfg.mac)?)
    } else {
        None
    };
    let mut blk = if let Some(path) = cfg.disk_path.as_deref() {
        eprintln!(
            "[veer-vm] virtio-mmio-blk: base={:#x} disk={} ro={}",
            VIRTIO_MMIO_BLK_BASE,
            path.display(),
            cfg.disk_read_only
        );
        Some(VirtioMmioBlk::open(path, cfg.disk_read_only)?)
    } else {
        None
    };

    let mut vcpu = 0;
    let mut exit: *const HvVcpuExit = ptr::null();
    hv_check(
        unsafe { hv_vcpu_create(&mut vcpu, &mut exit, ptr::null_mut()) },
        "hv_vcpu_create",
    )?;

    write_reg(vcpu, HV_REG_PC, entry)?;
    write_reg(vcpu, HV_REG_X0, 0)?;
    write_reg(vcpu, HV_REG_CPSR, 0x3c5)?;
    let _ = unsafe { hv_vcpu_set_vtimer_mask(vcpu, true) };

    eprintln!(
        "[veer-vm] entering AArch64 guest: kernel={} entry={:#x} memory={} MiB",
        kernel_path.display(),
        entry,
        cfg.memory_bytes / 1024 / 1024
    );

    let run_once = std::env::var_os("VEER_VM_HVF_RUN_ONCE").is_some();
    let max_exits = if run_once { 1024 } else { usize::MAX };
    if run_once {
        let exit_vcpu = vcpu;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            let mut vcpus = [exit_vcpu];
            let _ = unsafe { hv_vcpus_exit(vcpus.as_mut_ptr(), vcpus.len() as u32) };
        });
    }

    // When vmnet is active, kick the vCPU out every 2 ms so the host loop
    // can call pump_rx() and deliver incoming frames (e.g. ARP requests).
    // Without this the guest spins in its smoltcp poll loop without ever
    // writing QUEUE_NOTIFY, so hv_vcpu_run() never returns.
    if net.is_some() {
        let kick_vcpu = vcpu;
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(2));
            let mut vcpus = [kick_vcpu];
            let _ = unsafe { hv_vcpus_exit(vcpus.as_mut_ptr(), vcpus.len() as u32) };
        });
    }

    for step in 0..max_exits {
        hv_check(unsafe { hv_vcpu_run(vcpu) }, "hv_vcpu_run")?;
        if exit.is_null() {
            bail!("hv_vcpu_run returned without exit information");
        }
        let exit_ref = unsafe { &*exit };
        match exit_ref.reason {
            HV_EXIT_REASON_EXCEPTION => {
                if !handle_data_abort(vcpu, exit_ref, &guest, net.as_mut(), blk.as_mut())? {
                    bail!(
                        "unhandled AArch64 exception exit at step {}: syndrome={:#x} ipa={:#x} va={:#x}",
                        step,
                        exit_ref.exception.syndrome,
                        exit_ref.exception.physical_address,
                        exit_ref.exception.virtual_address
                    );
                }
            }
            HV_EXIT_REASON_VTIMER_ACTIVATED => {
                let _ = unsafe { hv_vcpu_set_vtimer_mask(vcpu, true) };
            }
            HV_EXIT_REASON_CANCELED => {
                // Triggered by the periodic kick thread (when vmnet is active)
                // or by the run-once timeout thread.  In both cases we just
                // want to let the host loop do its work (pump_rx etc.) and
                // then re-enter the guest — unless run_once is set.
                if run_once {
                    break;
                }
            }
            HV_EXIT_REASON_UNKNOWN => bail!("unknown AArch64 HVF exit"),
            other => bail!("unhandled AArch64 HVF exit reason {}", other),
        }
        if let Some(net) = net.as_mut() {
            net.pump_rx(&guest)?;
        }
    }

    hv_check(unsafe { hv_vcpu_destroy(vcpu) }, "hv_vcpu_destroy")?;
    if run_once {
        eprintln!("[veer-vm] AArch64 run-once loop complete");
    }
    Ok(())
}
