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

    // Ensure PDPT exists.
    if pml4.entries[pml4i] & PTE_PRESENT == 0 {
        let frame = match alloc.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        zero_frame(frame);
        pml4.entries[pml4i] = (frame as u64) | PTE_PRESENT | PTE_WRITABLE;
    }
    let pdpt = unsafe { &mut *((pml4.entries[pml4i] & PTE_ADDR_MASK) as *mut PageTable) };

    // Ensure PD exists.
    if pdpt.entries[pdpti] & PTE_PRESENT == 0 {
        let frame = match alloc.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        zero_frame(frame);
        pdpt.entries[pdpti] = (frame as u64) | PTE_PRESENT | PTE_WRITABLE;
    }
    let pd = unsafe { &mut *((pdpt.entries[pdpti] & PTE_ADDR_MASK) as *mut PageTable) };

    // Ensure PT exists.
    if pd.entries[pdi] & PTE_PRESENT == 0 {
        let frame = match alloc.alloc_frame() {
            Some(f) => f,
            None => return false,
        };
        zero_frame(frame);
        pd.entries[pdi] = (frame as u64) | PTE_PRESENT | PTE_WRITABLE;
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
