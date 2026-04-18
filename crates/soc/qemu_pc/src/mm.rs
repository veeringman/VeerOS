//! x86-64 4-level page table management.
//!
//! Provides:
//! - Physical frame allocator (bitmap-based, 4 KiB granularity)
//! - Page table creation, mapping, and unmapping
//! - Identity-map and higher-half kernel mapping helpers
//! - CR3 load for address space switching
//!
//! Current design: flat identity map for kernel code/data/MMIO.
//! Per-process user page tables will be layered on top.

/// Page size (4 KiB).
pub const PAGE_SIZE: usize = 4096;
/// 2 MiB huge page size.
pub const HUGE_PAGE_SIZE: usize = 2 * 1024 * 1024;

/// Maximum physical memory we track (256 MiB for now — QEMU default).
const MAX_PHYS_MEM: usize = 256 * 1024 * 1024;
/// Number of 4 KiB frames in MAX_PHYS_MEM.
const MAX_FRAMES: usize = MAX_PHYS_MEM / PAGE_SIZE;
/// Bitmap words (each u64 covers 64 frames = 256 KiB).
const BITMAP_WORDS: usize = MAX_FRAMES / 64;

// ─── Page table entry flags ─────────────────────────────────────────────

pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_CACHE_DISABLE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_HUGE: u64 = 1 << 7; // PS bit: 2 MiB page (in PD) or 1 GiB page (in PDPT)
pub const PTE_GLOBAL: u64 = 1 << 8;
pub const PTE_NX: u64 = 1 << 63;

/// Mask to extract physical address from a PTE.
pub const PTE_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

// ═══════════════════════════════════════════════════════════════════════════
// Physical frame allocator (bitmap)
// ═══════════════════════════════════════════════════════════════════════════

/// Bitmap-based physical frame allocator.
pub struct FrameAllocator {
    /// Bitmap: bit=1 means frame is in use, bit=0 means free.
    bitmap: [u64; BITMAP_WORDS],
    /// Total number of frames available (may be < MAX_FRAMES).
    total_frames: usize,
    /// Number of frames currently allocated.
    allocated: usize,
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            bitmap: [0xFFFF_FFFF_FFFF_FFFF; BITMAP_WORDS], // all marked used initially
            total_frames: 0,
            allocated: 0,
        }
    }

    /// Initialize: mark all frames in the usable range as free.
    ///
    /// `usable_start` — first usable physical address (page-aligned).
    /// `usable_end` — last usable physical address (exclusive, page-aligned).
    ///
    /// Frames below `usable_start` (BIOS, kernel image, page tables, boot stack)
    /// remain marked as used.
    pub fn init(&mut self, usable_start: usize, usable_end: usize) {
        let first_frame = usable_start / PAGE_SIZE;
        let last_frame = core::cmp::min(usable_end / PAGE_SIZE, MAX_FRAMES);

        // Mark usable frames as free.
        for frame in first_frame..last_frame {
            let word = frame / 64;
            let bit = frame % 64;
            if word < BITMAP_WORDS {
                self.bitmap[word] &= !(1u64 << bit);
            }
        }
        self.total_frames = last_frame;
        self.allocated = first_frame; // everything below usable_start is "allocated"
    }

    /// Allocate a single 4 KiB frame. Returns the physical address, or None.
    pub fn alloc_frame(&mut self) -> Option<usize> {
        for word_idx in 0..BITMAP_WORDS {
            let word = self.bitmap[word_idx];
            if word != 0xFFFF_FFFF_FFFF_FFFF {
                // Find first free bit.
                let bit = word.trailing_ones() as usize;
                let frame = word_idx * 64 + bit;
                if frame >= self.total_frames {
                    return None;
                }
                self.bitmap[word_idx] |= 1u64 << bit;
                self.allocated += 1;
                return Some(frame * PAGE_SIZE);
            }
        }
        None
    }

    /// Free a single 4 KiB frame.
    pub fn free_frame(&mut self, phys_addr: usize) {
        let frame = phys_addr / PAGE_SIZE;
        let word = frame / 64;
        let bit = frame % 64;
        if word < BITMAP_WORDS {
            self.bitmap[word] &= !(1u64 << bit);
            if self.allocated > 0 {
                self.allocated -= 1;
            }
        }
    }

    /// Number of free frames.
    pub fn free_count(&self) -> usize {
        if self.total_frames > self.allocated {
            self.total_frames - self.allocated
        } else {
            0
        }
    }

    /// Total tracked frames.
    pub fn total_count(&self) -> usize {
        self.total_frames
    }

    /// Number of allocated frames.
    pub fn used_count(&self) -> usize {
        self.allocated
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Page table structures
// ═══════════════════════════════════════════════════════════════════════════

/// A single page table (used for PML4, PDPT, PD, and PT — all same layout).
#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [u64; 512],
}

impl PageTable {
    pub const fn new() -> Self {
        Self {
            entries: [0; 512],
        }
    }

    /// Zero all entries.
    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            *e = 0;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Page table index extraction
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the PML4 index (bits 47:39) from a virtual address.
pub fn pml4_index(virt: usize) -> usize {
    (virt >> 39) & 0x1FF
}

/// Extract the PDPT index (bits 38:30) from a virtual address.
pub fn pdpt_index(virt: usize) -> usize {
    (virt >> 30) & 0x1FF
}

/// Extract the PD index (bits 29:21) from a virtual address.
pub fn pd_index(virt: usize) -> usize {
    (virt >> 21) & 0x1FF
}

/// Extract the PT index (bits 20:12) from a virtual address.
pub fn pt_index(virt: usize) -> usize {
    (virt >> 12) & 0x1FF
}

// ═══════════════════════════════════════════════════════════════════════════
// CR3 / TLB operations
// ═══════════════════════════════════════════════════════════════════════════

/// Load a new PML4 physical address into CR3 (switch address space).
#[cfg(target_arch = "x86_64")]
pub fn load_cr3(pml4_phys: usize) {
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) pml4_phys, options(nostack));
    }
}

/// Read the current CR3 value.
#[cfg(target_arch = "x86_64")]
pub fn read_cr3() -> usize {
    let val: usize;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) val, options(nostack, nomem));
    }
    val
}

/// Flush a single TLB entry for a given virtual address.
#[cfg(target_arch = "x86_64")]
pub fn invlpg(virt: usize) {
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack));
    }
}

/// Flush the entire TLB by reloading CR3.
#[cfg(target_arch = "x86_64")]
pub fn flush_tlb() {
    let cr3 = read_cr3();
    load_cr3(cr3);
}

// Non-x86 stubs for cargo check.
#[cfg(not(target_arch = "x86_64"))]
pub fn load_cr3(_pml4_phys: usize) {}
#[cfg(not(target_arch = "x86_64"))]
pub fn read_cr3() -> usize { 0 }
#[cfg(not(target_arch = "x86_64"))]
pub fn invlpg(_virt: usize) {}
#[cfg(not(target_arch = "x86_64"))]
pub fn flush_tlb() {}

// ═══════════════════════════════════════════════════════════════════════════
// Mapping helpers (identity-mapped kernel, using the frame allocator)
// ═══════════════════════════════════════════════════════════════════════════

/// Map a single 4 KiB page: virt → phys with given flags.
///
/// Allocates intermediate page tables (PDPT, PD, PT) from `alloc` as needed.
/// Assumes identity mapping for kernel page table access (physical == virtual).
///
/// Returns `true` on success, `false` if allocation fails.
pub fn map_page(
    pml4: &mut PageTable,
    virt: usize,
    phys: usize,
    flags: u64,
    alloc: &mut FrameAllocator,
) -> bool {
    let pml4i = pml4_index(virt);
    let pdpti = pdpt_index(virt);
    let pdi = pd_index(virt);
    let pti = pt_index(virt);

    // User-accessible pages need PTE_USER at every level of the hierarchy.
    let user_bit = flags & PTE_USER;

    // Ensure PDPT exists.
    if pml4.entries[pml4i] & PTE_PRESENT == 0 {
        let frame = match alloc.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        zero_frame(frame);
        pml4.entries[pml4i] = (frame as u64) | PTE_PRESENT | PTE_WRITABLE | user_bit;
    } else if user_bit != 0 {
        pml4.entries[pml4i] |= PTE_USER;
    }
    let pdpt = unsafe { &mut *((pml4.entries[pml4i] & PTE_ADDR_MASK) as *mut PageTable) };

    // Ensure PD exists.
    if pdpt.entries[pdpti] & PTE_PRESENT == 0 {
        let frame = match alloc.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        zero_frame(frame);
        pdpt.entries[pdpti] = (frame as u64) | PTE_PRESENT | PTE_WRITABLE | user_bit;
    } else if user_bit != 0 {
        pdpt.entries[pdpti] |= PTE_USER;
    }
    let pd = unsafe { &mut *((pdpt.entries[pdpti] & PTE_ADDR_MASK) as *mut PageTable) };

    // Ensure PT exists.
    if pd.entries[pdi] & PTE_PRESENT == 0 {
        let frame = match alloc.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        zero_frame(frame);
        pd.entries[pdi] = (frame as u64) | PTE_PRESENT | PTE_WRITABLE | user_bit;
    } else if user_bit != 0 {
        pd.entries[pdi] |= PTE_USER;
    }
    let pt = unsafe { &mut *((pd.entries[pdi] & PTE_ADDR_MASK) as *mut PageTable) };

    // Set the PT entry.
    pt.entries[pti] = (phys as u64 & PTE_ADDR_MASK) | flags | PTE_PRESENT;

    true
}

/// Unmap a single 4 KiB page. Does NOT free intermediate tables.
pub fn unmap_page(pml4: &mut PageTable, virt: usize) {
    let pml4i = pml4_index(virt);
    if pml4.entries[pml4i] & PTE_PRESENT == 0 { return; }
    let pdpt = unsafe { &mut *((pml4.entries[pml4i] & PTE_ADDR_MASK) as *mut PageTable) };

    let pdpti = pdpt_index(virt);
    if pdpt.entries[pdpti] & PTE_PRESENT == 0 { return; }
    let pd = unsafe { &mut *((pdpt.entries[pdpti] & PTE_ADDR_MASK) as *mut PageTable) };

    let pdi = pd_index(virt);
    if pd.entries[pdi] & PTE_PRESENT == 0 { return; }
    let pt = unsafe { &mut *((pd.entries[pdi] & PTE_ADDR_MASK) as *mut PageTable) };

    let pti = pt_index(virt);
    pt.entries[pti] = 0;
    invlpg(virt);
}

/// Identity-map a range of physical pages (phys == virt).
pub fn identity_map_range(
    pml4: &mut PageTable,
    start: usize,
    end: usize,
    flags: u64,
    alloc: &mut FrameAllocator,
) -> bool {
    let mut addr = start & !(PAGE_SIZE - 1);
    while addr < end {
        if !map_page(pml4, addr, addr, flags, alloc) {
            return false;
        }
        addr += PAGE_SIZE;
    }
    true
}

/// Zero a 4 KiB frame at the given physical address.
/// Assumes identity-mapped kernel.
fn zero_frame(phys: usize) {
    let ptr = phys as *mut u8;
    unsafe {
        core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Higher-half kernel mapping
// ═══════════════════════════════════════════════════════════════════════════

/// Higher-half kernel virtual base address.
/// Canonical 64-bit: 0xFFFF_8000_0000_0000 (PML4 index 256).
pub const KERNEL_VIRT_BASE: usize = 0xFFFF_8000_0000_0000;

/// PML4 index for the higher-half kernel mapping.
const KERNEL_PML4_INDEX: usize = 256;

/// Set up higher-half kernel mapping by copying PML4[0] (identity map)
/// into PML4[256] (higher-half). The identity map is kept for transition.
///
/// After this call, kernel code/data at physical address P is also
/// accessible at virtual address KERNEL_VIRT_BASE + P.
///
/// `pml4` — the current PML4 table (identity-mapped).
pub fn setup_higher_half(pml4: &mut PageTable) {
    // Copy the PDPT pointer from PML4[0] to PML4[256].
    // This makes the same 1 GiB identity map available at the higher-half.
    pml4.entries[KERNEL_PML4_INDEX] = pml4.entries[0];
}

/// Remove the low identity mapping (PML4[0]).
///
/// Call this only after all kernel code uses higher-half addresses
/// (including the instruction pointer). Not safe to call during
/// early boot — deferred to after SMP bringup.
pub fn remove_identity_map(pml4: &mut PageTable) {
    pml4.entries[0] = 0;
    flush_tlb();
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-process address space
// ═══════════════════════════════════════════════════════════════════════════

/// User-space virtual address range (below canonical hole).
pub const USER_VIRT_BASE: usize = 0x0000_0040_0000_0000;   // 256 GiB mark
/// Default user stack top.
pub const USER_STACK_TOP: usize = 0x0000_0080_0000_0000;    // 512 GiB mark
/// User stack size (64 KiB).
pub const USER_STACK_SIZE: usize = 64 * 1024;

/// Maximum per-process page tables we track.
pub const MAX_ADDRESS_SPACES: usize = 16;

/// A per-process address space: owns a PML4 physical address.
#[derive(Clone, Copy)]
pub struct AddressSpace {
    /// Physical address of this process's PML4 table.
    pub pml4_phys: usize,
    /// Process ID that owns this address space (0 = free slot).
    pub pid: u16,
    /// Number of user pages mapped.
    pub user_page_count: usize,
}

impl AddressSpace {
    pub const EMPTY: Self = Self {
        pml4_phys: 0,
        pid: 0,
        user_page_count: 0,
    };
}

/// Table of per-process address spaces.
pub struct AddressSpaceTable {
    pub spaces: [AddressSpace; MAX_ADDRESS_SPACES],
    pub count: usize,
}

impl AddressSpaceTable {
    pub const fn new() -> Self {
        Self {
            spaces: [AddressSpace::EMPTY; MAX_ADDRESS_SPACES],
            count: 0,
        }
    }

    /// Create a new address space for a process.
    ///
    /// Allocates a fresh PML4 and copies the kernel's higher-half mapping
    /// (PML4 entries 256–511) so kernel code is accessible in every process.
    ///
    /// `kernel_pml4` — pointer to the kernel's PML4 (identity-mapped physical address).
    /// `alloc` — the physical frame allocator.
    ///
    /// Returns the slot index, or None on failure.
    pub fn create(
        &mut self,
        pid: u16,
        kernel_pml4: &PageTable,
        alloc: &mut FrameAllocator,
    ) -> Option<usize> {
        if self.count >= MAX_ADDRESS_SPACES {
            return None;
        }

        // Allocate a frame for the new PML4.
        let pml4_phys = alloc.alloc_frame()?;
        zero_frame(pml4_phys);

        // Copy kernel higher-half entries (PML4[256..512]).
        let new_pml4 = unsafe { &mut *(pml4_phys as *mut PageTable) };
        for i in KERNEL_PML4_INDEX..512 {
            new_pml4.entries[i] = kernel_pml4.entries[i];
        }
        // Also copy identity map (PML4[0]) while we're still in identity-map mode.
        new_pml4.entries[0] = kernel_pml4.entries[0];

        let idx = self.count;
        self.spaces[idx] = AddressSpace {
            pml4_phys,
            pid,
            user_page_count: 0,
        };
        self.count += 1;
        Some(idx)
    }

    /// Find the address space for a given process ID.
    pub fn find(&self, pid: u16) -> Option<usize> {
        for i in 0..self.count {
            if self.spaces[i].pid == pid {
                return Some(i);
            }
        }
        None
    }

    /// Map a user page in a process's address space.
    pub fn map_user_page(
        &mut self,
        idx: usize,
        virt: usize,
        phys: usize,
        alloc: &mut FrameAllocator,
    ) -> bool {
        if idx >= self.count {
            return false;
        }
        let pml4_phys = self.spaces[idx].pml4_phys;
        let pml4 = unsafe { &mut *(pml4_phys as *mut PageTable) };
        let flags = PTE_WRITABLE | PTE_USER;
        if map_page(pml4, virt, phys, flags, alloc) {
            self.spaces[idx].user_page_count += 1;
            true
        } else {
            false
        }
    }

    /// Switch to a process's address space (load its CR3).
    pub fn switch_to(&self, idx: usize) {
        if idx < self.count {
            load_cr3(self.spaces[idx].pml4_phys);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CR4 control bits for security
// ═══════════════════════════════════════════════════════════════════════════

/// Enable SMEP (Supervisor Mode Execution Prevention) — prevents kernel
/// from executing code in user-accessible pages (CR4 bit 20).
#[cfg(target_arch = "x86_64")]
pub fn enable_smep() {
    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
        cr4 |= 1 << 20; // SMEP
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack));
    }
}

/// Enable SMAP (Supervisor Mode Access Prevention) — prevents kernel
/// from reading/writing user-accessible pages (CR4 bit 21).
/// Kernel must use STAC/CLAC around legitimate user-memory accesses.
#[cfg(target_arch = "x86_64")]
pub fn enable_smap() {
    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
        cr4 |= 1 << 21; // SMAP
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack));
    }
}

/// Enable NXE (No-Execute Enable) in IA32_EFER — required for PTE_NX.
#[cfg(target_arch = "x86_64")]
pub fn enable_nxe() {
    unsafe {
        let efer: u64;
        core::arch::asm!(
            "mov ecx, 0xC0000080",
            "rdmsr",
            "shl rdx, 32",
            "or rax, rdx",
            out("rax") efer,
            out("rcx") _,
            out("rdx") _,
            options(nomem, nostack),
        );
        let new_efer = efer | (1 << 11); // NXE bit
        let lo = new_efer as u32;
        let hi = (new_efer >> 32) as u32;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xC000_0080u32,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// User address space construction helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Create a user address space for the given process.
///
/// 1. Allocates a new PML4 and copies the kernel half (PML4[0] and [256..512]).
/// 2. Identity-maps the user code region (`code_start..code_end`) as
///    read-only + executable + user-accessible.
/// 3. Allocates `stack_pages` physical frames and maps them below
///    `USER_STACK_TOP` as read-write + no-execute + user-accessible.
///
/// Returns `(pml4_phys, stack_bottom_virt, stack_top_virt)` on success.
pub fn create_user_address_space(
    kernel_pml4: &PageTable,
    alloc: &mut FrameAllocator,
    code_start: usize,
    code_end: usize,
    stack_pages: usize,
) -> Option<(usize, usize, usize)> {
    // Allocate PML4 frame.
    let pml4_phys = alloc.alloc_frame()?;
    zero_frame(pml4_phys);

    let pml4 = unsafe { &mut *(pml4_phys as *mut PageTable) };

    // Copy kernel mappings: identity map (PML4[0]) + higher-half (PML4[256..512]).
    // The identity map is needed so that ISR handlers / MMIO remain reachable
    // when CR3 is set to this address space during ring-3 execution.
    pml4.entries[0] = kernel_pml4.entries[0];
    for i in KERNEL_PML4_INDEX..512 {
        pml4.entries[i] = kernel_pml4.entries[i];
    }

    // Identity-map user code pages as User + Read + Execute (no write).
    let code_flags = PTE_USER; // PTE_PRESENT is added by map_page; read-only (no PTE_WRITABLE)
    let mut addr = code_start & !(PAGE_SIZE - 1);
    while addr < code_end {
        if !map_page(pml4, addr, addr, code_flags, alloc) {
            return None;
        }
        addr += PAGE_SIZE;
    }

    // Allocate and map user stack pages.
    let stack_size = stack_pages * PAGE_SIZE;
    let stack_top = USER_STACK_TOP;
    let stack_bottom = stack_top - stack_size;
    let stack_flags = PTE_USER | PTE_WRITABLE | PTE_NX;

    for i in 0..stack_pages {
        let frame = alloc.alloc_frame()?;
        zero_frame(frame);
        let virt = stack_bottom + i * PAGE_SIZE;
        if !map_page(pml4, virt, frame, stack_flags, alloc) {
            return None;
        }
    }

    Some((pml4_phys, stack_bottom, stack_top))
}

// Non-x86 stubs.
#[cfg(not(target_arch = "x86_64"))]
pub fn enable_smep() {}
#[cfg(not(target_arch = "x86_64"))]
pub fn enable_smap() {}
#[cfg(not(target_arch = "x86_64"))]
pub fn enable_nxe() {}
