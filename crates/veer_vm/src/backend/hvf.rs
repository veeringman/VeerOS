use anyhow::{bail, Context, Result};
use object::read::elf::{ElfFile32, ElfFile64, ProgramHeader};
use object::{Object, ObjectKind};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::ptr;

use crate::backend::ExitReason;
use crate::config::{BootSource, GuestArch, VmConfig};
struct HvfLapic {
    id: u32,
    tpr: u32,
    sivr: u32,
    lvt_timer: u32,
    lvt_lint0: u32,
    lvt_lint1: u32,
    lvt_error: u32,
    timer_init: u32,
    timer_current: u32,
    timer_div: u32,
}

impl Default for HvfLapic {
    fn default() -> Self {
        Self {
            id: 0,
            tpr: 0,
            sivr: 0x0000_0100,
            lvt_timer: 1 << 16,
            lvt_lint0: 1 << 16,
            lvt_lint1: 1 << 16,
            lvt_error: 1 << 16,
            timer_init: 0,
            timer_current: 0,
            timer_div: 0,
        }
    }
}

impl HvfLapic {
    fn apic_enabled(&self) -> bool {
        (self.sivr & LAPIC_SIVR_ENABLE) != 0
    }

    fn timer_masked(&self) -> bool {
        (self.lvt_timer & LAPIC_LVT_MASKED) != 0
    }

    fn timer_periodic(&self) -> bool {
        (self.lvt_timer & LAPIC_LVT_TIMER_PERIODIC) != 0
    }

    fn timer_vector(&self) -> u8 {
        (self.lvt_timer & 0xff) as u8
    }

    fn timer_divisor(&self) -> u32 {
        // xAPIC divide configuration encoding uses bits [3,1,0].
        // Common values: 0b0000->2, 0001->4, 0010->8, 0011->16,
        //                1000->32,1001->64,1010->128,1011->1.
        match self.timer_div & 0x0B {
            0x00 => 2,
            0x01 => 4,
            0x02 => 8,
            0x03 => 16,
            0x08 => 32,
            0x09 => 64,
            0x0A => 128,
            0x0B => 1,
            _ => 2,
        }
    }

    fn timer_quanta(&self, base: u32) -> u32 {
        let div = self.timer_divisor();
        let q = base / div;
        q.max(1)
    }

    fn tick(&mut self, base_quanta: u32) -> Option<u8> {
        if !self.apic_enabled() || self.timer_masked() || self.timer_init == 0 {
            return None;
        }

        if self.timer_current == 0 {
            self.timer_current = self.timer_init;
        }

        let q = self.timer_quanta(base_quanta);
        if self.timer_current > q {
            self.timer_current -= q;
            return None;
        }

        self.timer_current = if self.timer_periodic() {
            self.timer_init
        } else {
            0
        };

        let vec = self.timer_vector();
        if vec == 0 { None } else { Some(vec) }
    }

    fn read_u32(&mut self, addr: u64) -> u32 {
        match addr & 0xFFF {
            0x020 => self.id << 24,
            0x030 => 0x0005_0014,
            0x080 => self.tpr,
            0x0F0 => self.sivr,
            0x320 => self.lvt_timer,
            0x350 => self.lvt_lint0,
            0x360 => self.lvt_lint1,
            0x370 => self.lvt_error,
            0x380 => self.timer_init,
            0x390 => self.timer_current,
            0x3E0 => self.timer_div,
            _ => 0,
        }
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        match addr & 0xFFF {
            0x080 => self.tpr = value,
            0x0B0 => {
                // EOI write-only; no-op in stub.
            }
            0x0F0 => self.sivr = value,
            0x320 => self.lvt_timer = value,
            0x350 => self.lvt_lint0 = value,
            0x360 => self.lvt_lint1 = value,
            0x370 => self.lvt_error = value,
            0x380 => {
                self.timer_init = value;
                self.timer_current = value;
            }
            0x3E0 => self.timer_div = value,
            _ => {}
        }
    }
}

struct HvfIoapic {
    ioregsel: u32,
    redir: [u64; 24],
}

impl Default for HvfIoapic {
    fn default() -> Self {
        let mut s = Self {
            ioregsel: 0,
            redir: [1u64 << 16; 24],
        };
        s.init_isa_defaults(0);
        s
    }
}

impl HvfIoapic {
    fn init_isa_defaults(&mut self, bsp_apic_id: u8) {
        // QEMU/ACPI common defaults used by VeerOS x86_64 boot path.
        // PIT is on GSI2 in this configuration.
        self.route_irq(2, 32, bsp_apic_id);
        self.route_irq(1, 33, bsp_apic_id);
        self.route_irq(4, 36, bsp_apic_id);
    }

    fn route_irq(&mut self, irq: u8, vector: u8, dest_apic_id: u8) {
        let idx = irq as usize;
        if idx >= self.redir.len() {
            return;
        }
        // Fixed delivery, physical destination, edge/high polarity.
        // Keep mask bit clear (unmasked) for routed defaults.
        let lo = vector as u32;
        let hi = (dest_apic_id as u32) << 24;
        self.redir[idx] = (lo as u64) | ((hi as u64) << 32);
    }

    fn irq_vector(&self, irq: u8) -> Option<u8> {
        let idx = irq as usize;
        if idx >= self.redir.len() {
            return None;
        }
        let entry = self.redir[idx];
        // bit16 masked, bits10:8 delivery mode, vector bits7:0.
        let masked = ((entry >> 16) & 1) != 0;
        let delivery_mode = ((entry >> 8) & 0x7) as u8;
        if masked || delivery_mode != 0 {
            return None;
        }
        Some((entry & 0xff) as u8)
    }

    fn read_u32(&self, addr: u64) -> u32 {
        match (addr - IOAPIC_BASE) & 0xFFF {
            0x00 => self.ioregsel,
            0x10 => self.read_selected(),
            _ => 0,
        }
    }

    fn write_u32(&mut self, addr: u64, value: u32) {
        match (addr - IOAPIC_BASE) & 0xFFF {
            0x00 => self.ioregsel = value & 0xFF,
            0x10 => self.write_selected(value),
            _ => {}
        }
    }

    fn read_selected(&self) -> u32 {
        match self.ioregsel as u8 {
            0x00 => 0,
            0x01 => ((24u32 - 1) << 16) | 0x11,
            reg if reg >= 0x10 => {
                let idx = ((reg - 0x10) / 2) as usize;
                if idx < self.redir.len() {
                    if (reg & 1) == 0 {
                        self.redir[idx] as u32
                    } else {
                        (self.redir[idx] >> 32) as u32
                    }
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    fn write_selected(&mut self, value: u32) {
        let reg = self.ioregsel as u8;
        if reg < 0x10 {
            return;
        }
        let idx = ((reg - 0x10) / 2) as usize;
        if idx >= self.redir.len() {
            return;
        }
        if (reg & 1) == 0 {
            self.redir[idx] = (self.redir[idx] & !0xFFFF_FFFF) | (value as u64);
        } else {
            self.redir[idx] = (self.redir[idx] & 0xFFFF_FFFF) | ((value as u64) << 32);
        }
    }
}


const HV_SUCCESS: i32 = 0;
const HV_BUSY: i32 = -85377017;
const HV_MEMORY_READ: u64 = 1 << 0;
const HV_MEMORY_WRITE: u64 = 1 << 1;
const HV_MEMORY_EXEC: u64 = 1 << 2;
const MBINFO_GPA: u64 = 0x9_F000;
const MULTIBOOT1_BOOTLOADER_MAGIC: u64 = 0x2BAD_B002;

type HvVcpuId = u32;

// x86 general register ids from Hypervisor.framework (Intel path).
const HV_X86_RIP: u32 = 16;
const HV_X86_RFLAGS: u32 = 17;
const HV_X86_RSP: u32 = 4;
const HV_X86_RAX: u32 = 0;
const HV_X86_RCX: u32 = 1;
const HV_X86_RDX: u32 = 2;
const HV_X86_RBX: u32 = 3;

// Intel VMCS field encodings (guest state) used by Hypervisor.framework VMX APIs.
const VMCS_GUEST_CR0: u32 = 0x0000_6800;
const VMCS_GUEST_CR3: u32 = 0x0000_6802;
const VMCS_GUEST_CR4: u32 = 0x0000_6804;
const VMCS_GUEST_IA32_EFER: u32 = 0x0000_2806;

const VMCS_GUEST_ES_SELECTOR: u32 = 0x0000_0800;
const VMCS_GUEST_CS_SELECTOR: u32 = 0x0000_0802;
const VMCS_GUEST_SS_SELECTOR: u32 = 0x0000_0804;
const VMCS_GUEST_DS_SELECTOR: u32 = 0x0000_0806;
const VMCS_GUEST_FS_SELECTOR: u32 = 0x0000_0808;
const VMCS_GUEST_GS_SELECTOR: u32 = 0x0000_080A;

const VMCS_GUEST_ES_LIMIT: u32 = 0x0000_4800;
const VMCS_GUEST_CS_LIMIT: u32 = 0x0000_4802;
const VMCS_GUEST_SS_LIMIT: u32 = 0x0000_4804;
const VMCS_GUEST_DS_LIMIT: u32 = 0x0000_4806;
const VMCS_GUEST_FS_LIMIT: u32 = 0x0000_4808;
const VMCS_GUEST_GS_LIMIT: u32 = 0x0000_480A;

const VMCS_GUEST_ES_BASE: u32 = 0x0000_6806;
const VMCS_GUEST_CS_BASE: u32 = 0x0000_6808;
const VMCS_GUEST_SS_BASE: u32 = 0x0000_680A;
const VMCS_GUEST_DS_BASE: u32 = 0x0000_680C;
const VMCS_GUEST_FS_BASE: u32 = 0x0000_680E;
const VMCS_GUEST_GS_BASE: u32 = 0x0000_6810;

const VMCS_GUEST_ES_AR: u32 = 0x0000_4814;
const VMCS_GUEST_CS_AR: u32 = 0x0000_4816;
const VMCS_GUEST_SS_AR: u32 = 0x0000_4818;
const VMCS_GUEST_DS_AR: u32 = 0x0000_481A;
const VMCS_GUEST_FS_AR: u32 = 0x0000_481C;
const VMCS_GUEST_GS_AR: u32 = 0x0000_481E;

const VMCS_RO_EXIT_REASON: u32 = 0x0000_4402;
const VMCS_RO_EXIT_QUALIFICATION: u32 = 0x0000_6400;
const VMCS_RO_VMEXIT_INSTR_LEN: u32 = 0x0000_440C;
const VMCS_GUEST_PHYSICAL_ADDRESS: u32 = 0x0000_2400;
const VMCS_CTRL_VMENTRY_INTERRUPTION_INFO: u32 = 0x0000_4016;
const VMCS_GUEST_INTERRUPTIBILITY_STATE: u32 = 0x0000_4824;

const VMX_EXIT_REASON_CPUID: u32 = 10;
const VMX_EXIT_REASON_HLT: u32 = 12;
const VMX_EXIT_REASON_IO: u32 = 30;
const VMX_EXIT_REASON_EPT_VIOLATION: u32 = 48;

const LAPIC_BASE: u64 = 0xFEE0_0000;
const LAPIC_END: u64 = 0xFEE0_0FFF;
const IOAPIC_BASE: u64 = 0xFEC0_0000;
const IOAPIC_END: u64 = 0xFEC0_0FFF;

const LAPIC_SIVR_ENABLE: u32 = 1 << 8;
const LAPIC_LVT_MASKED: u32 = 1 << 16;
const LAPIC_LVT_TIMER_PERIODIC: u32 = 1 << 17;
const RFLAGS_IF: u64 = 1 << 9;

#[link(name = "Hypervisor", kind = "framework")]
unsafe extern "C" {
    fn hv_vm_create(flags: u64) -> i32;
    fn hv_vm_destroy() -> i32;
    fn hv_vm_map(uva: *mut c_void, gpa: u64, size: usize, flags: u64) -> i32;
    fn hv_vm_unmap(gpa: u64, size: usize) -> i32;
    fn hv_vcpu_create(vcpu: *mut HvVcpuId, exit: *mut *mut c_void, flags: u64) -> i32;
    fn hv_vcpu_destroy(vcpu: HvVcpuId) -> i32;
    fn hv_vcpu_write_register(vcpu: HvVcpuId, reg: u32, value: u64) -> i32;
    fn hv_vcpu_read_register(vcpu: HvVcpuId, reg: u32, value: *mut u64) -> i32;
    fn hv_vcpu_run(vcpu: HvVcpuId) -> i32;
    fn hv_vmx_vcpu_write_vmcs(vcpu: HvVcpuId, field: u32, value: u64) -> i32;
    fn hv_vmx_vcpu_read_vmcs(vcpu: HvVcpuId, field: u32, value: *mut u64) -> i32;
}

struct HvfVm {
    host_mem: *mut c_void,
    mem_size: usize,
    vcpu_id: Option<HvVcpuId>,
}

impl Drop for HvfVm {
    fn drop(&mut self) {
        // Best-effort cleanup on process exit path.
        if let Some(vcpu) = self.vcpu_id {
            let _ = unsafe { hv_vcpu_destroy(vcpu) };
        }
        let _ = unsafe { hv_vm_unmap(0, self.mem_size) };
        let _ = unsafe { hv_vm_destroy() };
        if !self.host_mem.is_null() {
            let _ = unsafe { libc::munmap(self.host_mem, self.mem_size) };
        }
    }
}

fn write_reg(vcpu: HvVcpuId, reg: u32, value: u64, name: &str) -> Result<()> {
    hv_check(
        unsafe { hv_vcpu_write_register(vcpu, reg, value) },
        name,
    )
}

fn setup_vcpu_initial_state(vcpu: HvVcpuId, entry: u64, mbinfo_gpa: u64) -> Result<()> {
    // Minimal skeleton register state; real boot wiring comes in later phases.
    setup_vmcs_protected_mode_state(vcpu)?;
    write_reg(vcpu, HV_X86_RIP, entry, "hv_vcpu_write_register(RIP)")?;
    write_reg(vcpu, HV_X86_RFLAGS, 0x2, "hv_vcpu_write_register(RFLAGS)")?;
    write_reg(vcpu, HV_X86_RSP, 0, "hv_vcpu_write_register(RSP)")?;
    write_reg(
        vcpu,
        HV_X86_RAX,
        MULTIBOOT1_BOOTLOADER_MAGIC,
        "hv_vcpu_write_register(RAX)",
    )?;
    write_reg(vcpu, HV_X86_RBX, mbinfo_gpa, "hv_vcpu_write_register(RBX)")?;
    Ok(())
}

#[repr(C)]
#[derive(Default)]
struct MultibootInfo {
    flags: u32,
    mem_lower: u32,
    mem_upper: u32,
    boot_device: u32,
    cmdline: u32,
    mods_count: u32,
    mods_addr: u32,
    syms: [u32; 4],
    mmap_length: u32,
    mmap_addr: u32,
}

struct LoadedKernel {
    entry: u64,
    end: u64,
}

fn is_iso_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("iso"))
        .unwrap_or(false)
}

fn write_guest(host_mem: *mut c_void, mem_size: usize, gpa: u64, bytes: &[u8]) -> Result<()> {
    let start = usize::try_from(gpa).context("guest write address does not fit host usize")?;
    let end = start
        .checked_add(bytes.len())
        .context("guest write range overflow")?;
    if end > mem_size {
        bail!(
            "guest write out of bounds: gpa={:#x} len={:#x} mapped={:#x}",
            gpa,
            bytes.len(),
            mem_size
        );
    }
    let host = unsafe { (host_mem as *mut u8).add(start) };
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), host, bytes.len()) };
    Ok(())
}

fn zero_guest(host_mem: *mut c_void, mem_size: usize, gpa: u64, len: usize) -> Result<()> {
    let start = usize::try_from(gpa).context("guest zero address does not fit host usize")?;
    let end = start
        .checked_add(len)
        .context("guest zero range overflow")?;
    if end > mem_size {
        bail!(
            "guest zero out of bounds: gpa={:#x} len={:#x} mapped={:#x}",
            gpa,
            len,
            mem_size
        );
    }
    let host = unsafe { (host_mem as *mut u8).add(start) };
    unsafe { std::ptr::write_bytes(host, 0, len) };
    Ok(())
}

fn load_one_segment(
    image: &[u8],
    host_mem: *mut c_void,
    mem_size: usize,
    paddr: u64,
    filesz: usize,
    memsz: usize,
    offset: usize,
) -> Result<()> {
    if memsz == 0 {
        return Ok(());
    }
    if filesz > memsz {
        bail!("malformed PT_LOAD: filesz {filesz} > memsz {memsz}");
    }
    let file_end = offset
        .checked_add(filesz)
        .context("PT_LOAD offset+filesz overflow")?;
    if file_end > image.len() {
        bail!("PT_LOAD extends past end of ELF file");
    }

    if filesz > 0 {
        write_guest(host_mem, mem_size, paddr, &image[offset..file_end])?;
    }
    let bss = memsz - filesz;
    if bss > 0 {
        zero_guest(host_mem, mem_size, paddr + filesz as u64, bss)?;
    }
    Ok(())
}

fn load_elf64(image: &[u8], elf: &ElfFile64<'_>, host_mem: *mut c_void, mem_size: usize) -> Result<LoadedKernel> {
    match elf.kind() {
        ObjectKind::Executable => {}
        other => bail!("expected executable ELF, got {other:?}"),
    }

    let entry = elf.entry() as u64;
    if entry == 0 {
        bail!("ELF has no entry point");
    }

    let endian = elf.endian();
    let mut end = 0u64;
    let mut loaded_segments = 0usize;
    for seg in elf.elf_program_headers() {
        if seg.p_type(endian) != object::elf::PT_LOAD {
            continue;
        }
        let paddr = seg.p_paddr(endian) as u64;
        let filesz = seg.p_filesz(endian) as usize;
        let memsz = seg.p_memsz(endian) as usize;
        let offset = seg.p_offset(endian) as usize;
        load_one_segment(image, host_mem, mem_size, paddr, filesz, memsz, offset)?;
        end = end.max(paddr + memsz as u64);
        loaded_segments += 1;
    }
    if loaded_segments == 0 {
        bail!("no PT_LOAD segments found in ELF image");
    }
    Ok(LoadedKernel {
        entry,
        end: (end + 0xFFF) & !0xFFF,
    })
}

fn load_elf32(image: &[u8], elf: &ElfFile32<'_>, host_mem: *mut c_void, mem_size: usize) -> Result<LoadedKernel> {
    match elf.kind() {
        ObjectKind::Executable => {}
        other => bail!("expected executable ELF, got {other:?}"),
    }

    let entry = elf.entry() as u64;
    if entry == 0 {
        bail!("ELF has no entry point");
    }

    let endian = elf.endian();
    let mut end = 0u64;
    let mut loaded_segments = 0usize;
    for seg in elf.elf_program_headers() {
        if seg.p_type(endian) != object::elf::PT_LOAD {
            continue;
        }
        let paddr = seg.p_paddr(endian) as u64;
        let filesz = seg.p_filesz(endian) as usize;
        let memsz = seg.p_memsz(endian) as usize;
        let offset = seg.p_offset(endian) as usize;
        load_one_segment(image, host_mem, mem_size, paddr, filesz, memsz, offset)?;
        end = end.max(paddr + memsz as u64);
        loaded_segments += 1;
    }
    if loaded_segments == 0 {
        bail!("no PT_LOAD segments found in ELF image");
    }
    Ok(LoadedKernel {
        entry,
        end: (end + 0xFFF) & !0xFFF,
    })
}

fn load_kernel(path: &Path, host_mem: *mut c_void, mem_size: usize) -> Result<LoadedKernel> {
    if is_iso_path(path) {
        bail!("ISO kernel boot is not implemented for the macOS HVF backend yet");
    }
    let image = fs::read(path)
        .with_context(|| format!("reading kernel ELF {}", path.display()))?;
    if let Ok(elf) = ElfFile64::parse(image.as_slice()) {
        return load_elf64(&image, &elf, host_mem, mem_size)
            .with_context(|| format!("parsing ELF64 {}", path.display()));
    }
    if let Ok(elf) = ElfFile32::parse(image.as_slice()) {
        return load_elf32(&image, &elf, host_mem, mem_size)
            .with_context(|| format!("parsing ELF32 {}", path.display()));
    }
    bail!("unsupported or malformed ELF image: {}", path.display())
}

fn write_multiboot_info(host_mem: *mut c_void, mem_size: usize) -> Result<u64> {
    let info = MultibootInfo {
        flags: 0x0000_0001,
        mem_lower: 640,
        mem_upper: ((mem_size.saturating_sub(1024 * 1024)) / 1024) as u32,
        ..Default::default()
    };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            &info as *const _ as *const u8,
            std::mem::size_of::<MultibootInfo>(),
        )
    };
    write_guest(host_mem, mem_size, MBINFO_GPA, bytes)?;
    Ok(MBINFO_GPA)
}

fn write_vmcs(vcpu: HvVcpuId, field: u32, value: u64, name: &str) -> Result<()> {
    hv_check(
        unsafe { hv_vmx_vcpu_write_vmcs(vcpu, field, value) },
        name,
    )
}

fn read_vmcs(vcpu: HvVcpuId, field: u32, name: &str) -> Result<u64> {
    let mut value = 0u64;
    hv_check(
        unsafe { hv_vmx_vcpu_read_vmcs(vcpu, field, &mut value as *mut u64) },
        name,
    )?;
    Ok(value)
}

fn inject_external_interrupt(vcpu: HvVcpuId, vector: u8) -> Result<()> {
    // Valid bit(31)=1, type(10:8)=0 for external interrupt, vector(7:0).
    let info = (1u64 << 31) | (vector as u64);
    write_vmcs(
        vcpu,
        VMCS_CTRL_VMENTRY_INTERRUPTION_INFO,
        info,
        "hv_vmx_vcpu_write_vmcs(VMENTRY_INTERRUPTION_INFO)",
    )
}

fn guest_can_accept_ext_interrupt(vcpu: HvVcpuId) -> Result<bool> {
    let rflags = read_reg(vcpu, HV_X86_RFLAGS, "hv_vcpu_read_register(RFLAGS)")?;
    if (rflags & RFLAGS_IF) == 0 {
        return Ok(false);
    }
    let interruptibility = read_vmcs(
        vcpu,
        VMCS_GUEST_INTERRUPTIBILITY_STATE,
        "hv_vmx_vcpu_read_vmcs(GUEST_INTERRUPTIBILITY_STATE)",
    )?;
    // For external interrupts, STI/MOV-SS blocking states should defer delivery.
    let blocked_by_sti_or_mov_ss = (interruptibility & 0x3) != 0;
    Ok(!blocked_by_sti_or_mov_ss)
}

struct ExitState {
    reason_raw: u64,
    reason: u32,
    qualification: u64,
    instr_len: u64,
    guest_phys_addr: Option<u64>,
}

fn read_exit_state(vcpu: HvVcpuId) -> Result<ExitState> {
    let reason_raw = read_vmcs(vcpu, VMCS_RO_EXIT_REASON, "hv_vmx_vcpu_read_vmcs(EXIT_REASON)")?;
    let qualification = read_vmcs(
        vcpu,
        VMCS_RO_EXIT_QUALIFICATION,
        "hv_vmx_vcpu_read_vmcs(EXIT_QUALIFICATION)",
    )?;
    let instr_len = read_vmcs(
        vcpu,
        VMCS_RO_VMEXIT_INSTR_LEN,
        "hv_vmx_vcpu_read_vmcs(VMEXIT_INSTR_LEN)",
    )?;
    let reason = (reason_raw & 0xffff) as u32;
    let guest_phys_addr = if reason == VMX_EXIT_REASON_EPT_VIOLATION {
        Some(read_vmcs(
            vcpu,
            VMCS_GUEST_PHYSICAL_ADDRESS,
            "hv_vmx_vcpu_read_vmcs(GUEST_PHYSICAL_ADDRESS)",
        )?)
    } else {
        None
    };
    Ok(ExitState {
        reason_raw,
        reason,
        qualification,
        instr_len,
        guest_phys_addr,
    })
}

fn classify_exit(vcpu: HvVcpuId, exit_state: &ExitState) -> Result<ExitReason> {
    let reason = exit_state.reason;
    let qualification = exit_state.qualification;

    let mapped = match reason {
        VMX_EXIT_REASON_HLT => ExitReason::Hlt,
        VMX_EXIT_REASON_CPUID => ExitReason::Other("cpuid".to_string()),
        VMX_EXIT_REASON_IO => {
            // Intel VMX IO qualification layout:
            // bit3=direction (0=out,1=in), bits2:0=size-1, bits31:16=port.
            let direction_in = ((qualification >> 3) & 0x1) != 0;
            let len = ((qualification & 0x7) as usize) + 1;
            let port = ((qualification >> 16) & 0xffff) as u16;
            if direction_in {
                ExitReason::IoIn { port, len }
            } else {
                let rax = read_reg(vcpu, HV_X86_RAX, "hv_vcpu_read_register(RAX)")?;
                let mut data = vec![0u8; len];
                for (i, b) in data.iter_mut().enumerate() {
                    *b = ((rax >> (i * 8)) & 0xff) as u8;
                }
                ExitReason::IoOut {
                    port,
                    data,
                }
            }
        }
        VMX_EXIT_REASON_EPT_VIOLATION => {
            // EPT qualification bits 0/1/2 indicate read/write/execute access.
            let addr = exit_state.guest_phys_addr.unwrap_or(0);
            if (qualification & (1 << 1)) != 0 {
                let rax = read_reg(vcpu, HV_X86_RAX, "hv_vcpu_read_register(RAX)")?;
                let mut data = vec![0u8; 4];
                for (i, b) in data.iter_mut().enumerate() {
                    *b = ((rax >> (i * 8)) & 0xff) as u8;
                }
                ExitReason::MmioWrite {
                    addr,
                    data,
                }
            } else {
                ExitReason::MmioRead { addr, len: 4 }
            }
        }
        other => ExitReason::Other(format!(
            "vmexit reason={} raw={:#x} qualification={:#x}",
            other, exit_state.reason_raw, qualification
        )),
    };

    Ok(mapped)
}

fn advance_guest_rip(vcpu: HvVcpuId, instr_len: u64) -> Result<()> {
    let rip = read_reg(vcpu, HV_X86_RIP, "hv_vcpu_read_register(RIP)")?;
    write_reg(
        vcpu,
        HV_X86_RIP,
        rip.wrapping_add(instr_len),
        "hv_vcpu_write_register(RIP)",
    )
}

fn apply_cpuid_policy(_leaf: u32, _subleaf: u32, _eax: &mut u32, _ebx: &mut u32, _ecx: &mut u32, _edx: &mut u32) {
    // Placeholder for future VeerOS-specific CPUID filtering.
}

fn emulate_cpuid_passthrough(vcpu: HvVcpuId, instr_len: u64) -> Result<(u32, u32)> {
    let leaf = read_reg(vcpu, HV_X86_RAX, "hv_vcpu_read_register(RAX)")? as u32;
    let subleaf = read_reg(vcpu, HV_X86_RCX, "hv_vcpu_read_register(RCX)")? as u32;

    #[cfg(target_arch = "x86_64")]
    {
        let r = std::arch::x86_64::__cpuid_count(leaf, subleaf);
        let mut eax = r.eax;
        let mut ebx = r.ebx;
        let mut ecx = r.ecx;
        let mut edx = r.edx;
        apply_cpuid_policy(leaf, subleaf, &mut eax, &mut ebx, &mut ecx, &mut edx);
        write_reg(vcpu, HV_X86_RAX, eax as u64, "hv_vcpu_write_register(RAX)")?;
        write_reg(vcpu, HV_X86_RBX, ebx as u64, "hv_vcpu_write_register(RBX)")?;
        write_reg(vcpu, HV_X86_RCX, ecx as u64, "hv_vcpu_write_register(RCX)")?;
        write_reg(vcpu, HV_X86_RDX, edx as u64, "hv_vcpu_write_register(RDX)")?;
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        bail!("CPUID emulation is only available on x86_64 hosts");
    }

    advance_guest_rip(vcpu, instr_len)?;
    Ok((leaf, subleaf))
}

fn write_rax_low(vcpu: HvVcpuId, len: usize, value: u64) -> Result<()> {
    let old = read_reg(vcpu, HV_X86_RAX, "hv_vcpu_read_register(RAX)")?;
    let bits = (len.min(8) * 8) as u32;
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let merged = (old & !mask) | (value & mask);
    write_reg(vcpu, HV_X86_RAX, merged, "hv_vcpu_write_register(RAX)")
}

fn emulate_io_exit_default(vcpu: HvVcpuId, exit: &ExitReason, instr_len: u64, uart: &mut HvfUart16550) -> Result<bool> {
    match exit {
        ExitReason::IoIn { port, len } => {
            let val = if *len == 1 {
                uart.io_in(*port) as u64
            } else {
                0
            };
            write_rax_low(vcpu, *len, val)?;
            advance_guest_rip(vcpu, instr_len)?;
            Ok(true)
        }
        ExitReason::IoOut { port, data } => {
            let handled = if data.len() == 1 {
                uart.io_out(*port, data[0])?
            } else {
                false
            };
            if !handled {
                // Consume unknown writes for now; full device routing lands later.
            }
            advance_guest_rip(vcpu, instr_len)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

enum ExitDispatch {
    Handled(String),
    Stop(ExitReason),
}

#[derive(Clone, Copy, Debug)]
enum MmioRegion {
    Lapic,
    Ioapic,
}

fn classify_mmio_region(addr: u64) -> Option<MmioRegion> {
    if (LAPIC_BASE..=LAPIC_END).contains(&addr) {
        return Some(MmioRegion::Lapic);
    }
    if (IOAPIC_BASE..=IOAPIC_END).contains(&addr) {
        return Some(MmioRegion::Ioapic);
    }
    None
}

fn mmio_write_value_le(data: &[u8]) -> u32 {
    let mut v = 0u32;
    for (i, b) in data.iter().take(4).enumerate() {
        v |= (*b as u32) << (i * 8);
    }
    v
}

fn emulate_mmio_exit_default(
    vcpu: HvVcpuId,
    exit: &ExitReason,
    instr_len: u64,
    lapic: &mut HvfLapic,
    ioapic: &mut HvfIoapic,
) -> Result<Option<String>> {
    match exit {
        ExitReason::MmioRead { addr, len } => {
            let Some(region) = classify_mmio_region(*addr) else {
                return Ok(None);
            };
            let value = match region {
                MmioRegion::Lapic => lapic.read_u32(*addr) as u64,
                MmioRegion::Ioapic => ioapic.read_u32(*addr) as u64,
            };
            write_rax_low(vcpu, *len, value)?;
            advance_guest_rip(vcpu, instr_len)?;
            Ok(Some(format!(
                "handled MMIO read region={:?} addr={:#x} len={} value={:#x}",
                region, addr, len, value
            )))
        }
        ExitReason::MmioWrite { addr, data } => {
            let Some(region) = classify_mmio_region(*addr) else {
                return Ok(None);
            };
            let value = mmio_write_value_le(data);
            match region {
                MmioRegion::Lapic => lapic.write_u32(*addr, value),
                MmioRegion::Ioapic => ioapic.write_u32(*addr, value),
            }
            // Consume writes while LAPIC/IOAPIC emulation is still skeletal.
            advance_guest_rip(vcpu, instr_len)?;
            Ok(Some(format!(
                "handled MMIO write region={:?} addr={:#x} value={:#x}",
                region, addr, value
            )))
        }
        _ => Ok(None),
    }
}

#[derive(Default)]
struct HvfUart16550 {
    ier: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    dll: u8,
    dlm: u8,
    rx_fifo: VecDeque<u8>,
    rx_irq_latched: bool,
}

impl HvfUart16550 {
    fn preload_rx_bytes(&mut self, bytes: &[u8]) {
        self.rx_fifo.extend(bytes.iter().copied());
    }

    fn rx_ready(&self) -> bool {
        !self.rx_fifo.is_empty()
    }

    fn poll_irq_vector(&mut self, ioapic: &HvfIoapic) -> Option<u8> {
        // IER bit0 enables RX-data-available interrupts.
        if (self.ier & 0x01) == 0 || !self.rx_ready() || self.rx_irq_latched {
            return None;
        }
        let vec = ioapic.irq_vector(4)?;
        self.rx_irq_latched = true;
        Some(vec)
    }

    fn dlab(&self) -> bool {
        (self.lcr & 0x80) != 0
    }

    fn io_in(&mut self, port: u16) -> u8 {
        match port {
            0x3F8 => {
                if self.dlab() {
                    self.dll
                } else {
                    let b = self.rx_fifo.pop_front().unwrap_or(0);
                    if self.rx_fifo.is_empty() {
                        self.rx_irq_latched = false;
                    }
                    b
                }
            }
            0x3F9 => {
                if self.dlab() {
                    self.dlm
                } else {
                    self.ier
                }
            }
            0x3FA => {
                if self.rx_ready() && (self.ier & 0x01) != 0 {
                    0x04 // IIR: RX data available interrupt pending.
                } else {
                    0x01 // IIR: no interrupt pending.
                }
            }
            0x3FB => self.lcr,
            0x3FC => self.mcr,
            0x3FD => {
                let mut lsr = 0x60; // THR empty + transmitter empty.
                if self.rx_ready() {
                    lsr |= 0x01; // Data ready.
                }
                lsr
            }
            0x3FE => 0x00, // MSR default.
            0x3FF => self.scr,
            _ => 0,
        }
    }

    fn io_out(&mut self, port: u16, value: u8) -> Result<bool> {
        match port {
            0x3F8 => {
                if self.dlab() {
                    self.dll = value;
                    return Ok(true);
                }
                // THR write -> host stdout.
                let mut out = std::io::stdout().lock();
                out.write_all(&[value])
                    .context("writing UART byte to host stdout")?;
                out.flush().context("flushing host stdout for UART")?;
                Ok(true)
            }
            0x3F9 => {
                if self.dlab() {
                    self.dlm = value;
                } else {
                    self.ier = value;
                }
                Ok(true)
            }
            0x3FB => {
                self.lcr = value;
                Ok(true)
            }
            0x3FC => {
                self.mcr = value;
                Ok(true)
            }
            0x3FF => {
                self.scr = value;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn dispatch_exit(
    vcpu: HvVcpuId,
    exit_state: &ExitState,
    uart: &mut HvfUart16550,
    lapic: &mut HvfLapic,
    ioapic: &mut HvfIoapic,
) -> Result<ExitDispatch> {
    if exit_state.reason == VMX_EXIT_REASON_CPUID {
        let (leaf, subleaf) = emulate_cpuid_passthrough(vcpu, exit_state.instr_len)?;
        return Ok(ExitDispatch::Handled(format!(
            "handled CPUID exit leaf={:#x} subleaf={:#x}",
            leaf, subleaf
        )));
    }

    let exit = classify_exit(vcpu, exit_state)?;
    if emulate_io_exit_default(vcpu, &exit, exit_state.instr_len, uart)? {
        return Ok(ExitDispatch::Handled(format!("handled IO exit: {:?}", exit)));
    }
    if let Some(msg) = emulate_mmio_exit_default(vcpu, &exit, exit_state.instr_len, lapic, ioapic)? {
        return Ok(ExitDispatch::Handled(msg));
    }
    Ok(ExitDispatch::Stop(exit))
}

fn write_flat_seg(vcpu: HvVcpuId, selector: u16, sel_field: u32, limit_field: u32, base_field: u32, ar_field: u32, ar: u32, name: &str) -> Result<()> {
    write_vmcs(vcpu, sel_field, selector as u64, name)?;
    write_vmcs(vcpu, limit_field, 0xFFFF_FFFF, name)?;
    write_vmcs(vcpu, base_field, 0, name)?;
    write_vmcs(vcpu, ar_field, ar as u64, name)?;
    Ok(())
}

fn setup_vmcs_protected_mode_state(vcpu: HvVcpuId) -> Result<()> {
    // Mirror the KVM PM-entry contract: CR0=PE|ET, CR4=0, EFER=0, flat 32-bit segments.
    write_vmcs(vcpu, VMCS_GUEST_CR0, 0x11, "hv_vmx_vcpu_write_vmcs(GUEST_CR0)")?;
    write_vmcs(vcpu, VMCS_GUEST_CR3, 0, "hv_vmx_vcpu_write_vmcs(GUEST_CR3)")?;
    write_vmcs(vcpu, VMCS_GUEST_CR4, 0, "hv_vmx_vcpu_write_vmcs(GUEST_CR4)")?;
    write_vmcs(vcpu, VMCS_GUEST_IA32_EFER, 0, "hv_vmx_vcpu_write_vmcs(GUEST_IA32_EFER)")?;

    // Access-rights bits align with Intel VMCS segment AR format.
    // Code: type=0xB (exec/read/accessed), S=1, P=1, D/B=1, G=1.
    let cs_ar = 0xC09B_u32;
    // Data: type=0x3 (read/write/accessed), S=1, P=1, D/B=1, G=1.
    let data_ar = 0xC093_u32;

    write_flat_seg(
        vcpu,
        0x08,
        VMCS_GUEST_CS_SELECTOR,
        VMCS_GUEST_CS_LIMIT,
        VMCS_GUEST_CS_BASE,
        VMCS_GUEST_CS_AR,
        cs_ar,
        "hv_vmx_vcpu_write_vmcs(GUEST_CS_*)",
    )?;

    write_flat_seg(
        vcpu,
        0x10,
        VMCS_GUEST_DS_SELECTOR,
        VMCS_GUEST_DS_LIMIT,
        VMCS_GUEST_DS_BASE,
        VMCS_GUEST_DS_AR,
        data_ar,
        "hv_vmx_vcpu_write_vmcs(GUEST_DS_*)",
    )?;
    write_flat_seg(
        vcpu,
        0x10,
        VMCS_GUEST_ES_SELECTOR,
        VMCS_GUEST_ES_LIMIT,
        VMCS_GUEST_ES_BASE,
        VMCS_GUEST_ES_AR,
        data_ar,
        "hv_vmx_vcpu_write_vmcs(GUEST_ES_*)",
    )?;
    write_flat_seg(
        vcpu,
        0x10,
        VMCS_GUEST_SS_SELECTOR,
        VMCS_GUEST_SS_LIMIT,
        VMCS_GUEST_SS_BASE,
        VMCS_GUEST_SS_AR,
        data_ar,
        "hv_vmx_vcpu_write_vmcs(GUEST_SS_*)",
    )?;
    write_flat_seg(
        vcpu,
        0x10,
        VMCS_GUEST_FS_SELECTOR,
        VMCS_GUEST_FS_LIMIT,
        VMCS_GUEST_FS_BASE,
        VMCS_GUEST_FS_AR,
        data_ar,
        "hv_vmx_vcpu_write_vmcs(GUEST_FS_*)",
    )?;
    write_flat_seg(
        vcpu,
        0x10,
        VMCS_GUEST_GS_SELECTOR,
        VMCS_GUEST_GS_LIMIT,
        VMCS_GUEST_GS_BASE,
        VMCS_GUEST_GS_AR,
        data_ar,
        "hv_vmx_vcpu_write_vmcs(GUEST_GS_*)",
    )?;

    Ok(())
}

fn hv_check(rc: i32, what: &str) -> Result<()> {
    if rc == HV_SUCCESS {
        Ok(())
    } else {
        let hint = if what == "hv_vm_create" {
            " (verify Intel VT-x is enabled and no other hypervisor has exclusive ownership)"
        } else {
            ""
        };
        bail!("{} failed with hv_return_t={}{}", what, rc, hint)
    }
}

fn hv_return_name(rc: i32) -> &'static str {
    match rc {
        HV_SUCCESS => "HV_SUCCESS",
        HV_BUSY => "HV_BUSY",
        _ => "UNKNOWN",
    }
}

fn sysctl_u32(name: &str) -> Option<u32> {
    let c_name = std::ffi::CString::new(name).ok()?;
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>();
    let rc = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            (&mut value as *mut u32).cast::<c_void>(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if rc == 0 && size == std::mem::size_of::<u32>() {
        Some(value)
    } else {
        None
    }
}

fn hvf_preflight_hints() -> String {
    let hv_support = sysctl_u32("kern.hv_support");
    let hv_vmm_present = sysctl_u32("kern.hv_vmm_present");
    let mut hints = String::new();
    if let Some(v) = hv_support {
        hints.push_str(&format!(" kern.hv_support={};", v));
    }
    if let Some(v) = hv_vmm_present {
        hints.push_str(&format!(" kern.hv_vmm_present={};", v));
    }
    if hints.is_empty() {
        " unable to read HVF sysctls;".to_string()
    } else {
        hints
    }
}

pub fn preflight() -> Result<()> {
    let hv_support = sysctl_u32("kern.hv_support");
    let hv_vmm_present = sysctl_u32("kern.hv_vmm_present");
    let vmx = std::process::Command::new("/usr/sbin/sysctl")
        .arg("-n")
        .arg("machdep.cpu.features")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("VMX"));

    eprintln!("[veer-vm] macOS HVF preflight");
    match hv_support {
        Some(v) => eprintln!("  kern.hv_support={}", v),
        None => eprintln!("  kern.hv_support=<unavailable>"),
    }
    match hv_vmm_present {
        Some(v) => eprintln!("  kern.hv_vmm_present={}", v),
        None => eprintln!("  kern.hv_vmm_present=<unavailable>"),
    }
    match vmx {
        Some(v) => eprintln!("  machdep.cpu.features contains VMX={}", v),
        None => eprintln!("  machdep.cpu.features VMX=<unavailable>"),
    }

    if hv_support != Some(1) {
        bail!("Hypervisor.framework not supported on this host (kern.hv_support != 1)");
    }
    if hv_vmm_present == Some(1) {
        bail!("host appears to be running inside a VM (kern.hv_vmm_present=1); nested HVF is typically unavailable");
    }

    eprintln!("[veer-vm] preflight checks passed; if hv_vm_create still fails, check for hypervisor contention");
    Ok(())
}

pub fn probe() -> Result<()> {
    preflight()?;
    let create_rc = unsafe { hv_vm_create(0) };
    if create_rc != HV_SUCCESS {
        bail!(
            "hv_vm_create failed with hv_return_t={} ({}){}",
            create_rc,
            hv_return_name(create_rc),
            hvf_preflight_hints()
        );
    }

    hv_check(unsafe { hv_vm_destroy() }, "hv_vm_destroy")
        .context("destroying temporary probe VM")?;
    eprintln!("[veer-vm] hvf probe passed: vm create/destroy succeeded");
    Ok(())
}

fn read_reg(vcpu: HvVcpuId, reg: u32, name: &str) -> Result<u64> {
    let mut value = 0u64;
    hv_check(
        unsafe { hv_vcpu_read_register(vcpu, reg, &mut value as *mut u64) },
        name,
    )?;
    Ok(value)
}

fn run_vcpu_placeholder(vcpu: HvVcpuId) -> Result<()> {
    // Optional bring-up hook for staged development.
    if std::env::var_os("VEER_VM_HVF_RUN_ONCE").is_none() {
        return Ok(());
    }

    let mut uart = HvfUart16550::default();
    if let Some(seed) = std::env::var_os("VEER_VM_HVF_UART_RX") {
        let s = seed.to_string_lossy();
        uart.preload_rx_bytes(s.as_bytes());
        eprintln!("[veer-vm] preloaded {} UART RX bytes from VEER_VM_HVF_UART_RX", s.len());
    }
    if let Some(path) = std::env::var_os("VEER_VM_HVF_UART_RX_FILE") {
        let p = std::path::PathBuf::from(path);
        if let Ok(meta) = std::fs::metadata(&p) {
            if meta.is_file() {
                if let Ok(bytes) = std::fs::read(&p) {
                    uart.preload_rx_bytes(&bytes);
                    eprintln!(
                        "[veer-vm] preloaded {} UART RX bytes from {}",
                        bytes.len(),
                        p.display()
                    );
                }
            }
        }
    }
    let mut lapic = HvfLapic::default();
    let mut ioapic = HvfIoapic::default();
    let inject_timer = std::env::var_os("VEER_VM_HVF_INJECT_TIMER").is_some();
    let mut pending_timer_irq: Option<u8> = None;
    let mut pending_uart_irq: Option<u8> = None;
    const MAX_STEPS: usize = 32;
    for _ in 0..MAX_STEPS {
        if inject_timer {
            if pending_timer_irq.is_none() {
                pending_timer_irq = lapic.tick(1000);
            }
            if pending_uart_irq.is_none() {
                pending_uart_irq = uart.poll_irq_vector(&ioapic);
            }
            let inject_vec = pending_timer_irq.or(pending_uart_irq);
            if let Some(vector) = inject_vec {
                if guest_can_accept_ext_interrupt(vcpu)? {
                    if let Err(err) = inject_external_interrupt(vcpu, vector) {
                        eprintln!(
                            "[veer-vm] warning: timer interrupt injection failed for vector {}: {:#}",
                            vector, err
                        );
                    } else {
                        eprintln!("[veer-vm] injected external interrupt vector {}", vector);
                        if pending_timer_irq == Some(vector) {
                            pending_timer_irq = None;
                        }
                        if pending_uart_irq == Some(vector) {
                            pending_uart_irq = None;
                        }
                    }
                }
            }
        }
        hv_check(unsafe { hv_vcpu_run(vcpu) }, "hv_vcpu_run")?;
        let exit_state = read_exit_state(vcpu)?;

        let rip = read_reg(vcpu, HV_X86_RIP, "hv_vcpu_read_register(RIP)")?;
        match dispatch_exit(vcpu, &exit_state, &mut uart, &mut lapic, &mut ioapic)? {
            ExitDispatch::Handled(msg) => {
                eprintln!("[veer-vm] {}", msg);
                continue;
            }
            ExitDispatch::Stop(exit) => {
                eprintln!("[veer-vm] HVF run-once returned, RIP={:#x}, exit={:?}", rip, exit);
                return Ok(());
            }
        }
    }

    eprintln!(
        "[veer-vm] HVF run-once reached handling limit ({} steps)",
        MAX_STEPS
    );
    Ok(())
}

pub fn run(cfg: VmConfig) -> Result<()> {
    if cfg.guest_arch != GuestArch::X86_64 {
        bail!("macOS HVF skeleton currently supports only --arch x86_64");
    }
    let kernel_path = match &cfg.boot {
        BootSource::Kernel(path) => path,
        BootSource::Snapshot(_) => {
            bail!("--restore is not implemented for the macOS HVF skeleton yet")
        }
    };
    if cfg.snapshot_save.is_some() {
        bail!("--snapshot-save is not implemented for the macOS HVF skeleton yet");
    }

    let create_rc = unsafe { hv_vm_create(0) };
    if create_rc != HV_SUCCESS {
        bail!(
            "hv_vm_create failed with hv_return_t={} ({}) (verify Intel VT-x is enabled and no other hypervisor has exclusive ownership).{}",
            create_rc,
            hv_return_name(create_rc),
            hvf_preflight_hints()
        );
    }

    let mem_ptr = unsafe {
        libc::mmap(
            ptr::null_mut(),
            cfg.memory_bytes,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if mem_ptr == libc::MAP_FAILED {
        bail!("mmap failed while allocating guest memory");
    }

    hv_check(
        unsafe {
            hv_vm_map(
                mem_ptr,
                0,
                cfg.memory_bytes,
                HV_MEMORY_READ | HV_MEMORY_WRITE | HV_MEMORY_EXEC,
            )
        },
        "hv_vm_map",
    )
    .context("mapping guest memory into HVF VM")?;

    let loaded = load_kernel(kernel_path, mem_ptr, cfg.memory_bytes)
        .with_context(|| format!("loading kernel {}", kernel_path.display()))?;
    let mbinfo_gpa = write_multiboot_info(mem_ptr, cfg.memory_bytes)?;
    eprintln!(
        "[veer-vm] loaded kernel {}: entry={:#x} end={:#x} memory={} MiB",
        kernel_path.display(),
        loaded.entry,
        loaded.end,
        cfg.memory_bytes / (1024 * 1024),
    );

    let mut vcpu: HvVcpuId = 0;
    let mut exit_ptr: *mut c_void = ptr::null_mut();
    hv_check(
        unsafe { hv_vcpu_create(&mut vcpu as *mut HvVcpuId, &mut exit_ptr as *mut *mut c_void, 0) },
        "hv_vcpu_create",
    )
    .context("creating vCPU 0")?;
    setup_vcpu_initial_state(vcpu, loaded.entry, mbinfo_gpa)
        .context("programming initial vCPU register state")?;
    run_vcpu_placeholder(vcpu)?;

    let _vm_guard = HvfVm {
        host_mem: mem_ptr,
        mem_size: cfg.memory_bytes,
        vcpu_id: Some(vcpu),
    };

    eprintln!(
        "[veer-vm] HVF backend initialized (macOS skeleton, guest={} memory={} MiB, vcpu={})",
        cfg.guest_arch.as_str(),
        cfg.memory_bytes / (1024 * 1024),
        vcpu,
    );
    Ok(())
}