//! Fixed-pool memory allocator for VeerOS.
//!
//! Provides deterministic, O(1) allocation and deallocation from a
//! statically-sized pool.  No heap, no fragmentation, no external
//! dependencies — suitable for hard-real-time embedded kernels.
//!
//! # Design
//!
//! The pool is carved from a caller-supplied `&'static mut [u8]` region
//! (placed in the linker script or a large `static mut` array).
//! It is divided into equal-sized blocks.  A free-list is threaded
//! through the first `usize` of each free block.
//!
//! Two pool sizes are provided:
//! - **Small blocks** (64 B) — for TCBs, messages, handles.
//! - **Large blocks** (1024 B) — for buffers, driver descriptors.
//!
//! The kernel instantiates one [`PoolAllocator`] per block size and
//! exposes them through a unified [`Heap`] façade.

use core::ptr;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Small-block size (bytes).  Must be ≥ size_of::<usize>().
pub const SMALL_BLOCK: usize = 64;

/// Large-block size (bytes).
pub const LARGE_BLOCK: usize = 1024;

// ---------------------------------------------------------------------------
// PoolAllocator — single block-size free-list allocator
// ---------------------------------------------------------------------------

/// A fixed-block-size pool allocator.
///
/// Free blocks form a singly-linked list where the first `usize` of each
/// free block holds the index of the next free block (`usize::MAX` = end).
pub struct PoolAllocator {
    /// Base address of the backing memory region.
    base: *mut u8,
    /// Size of each block in bytes.
    block_size: usize,
    /// Total number of blocks carved from the region.
    total_blocks: usize,
    /// Head of the free-list (block index, or `usize::MAX` if exhausted).
    free_head: usize,
    /// Number of blocks currently allocated.
    used: usize,
}

// Safety: the allocator is only accessed under a global critical section
// (interrupts disabled) in the kernel — no concurrent access.
unsafe impl Send for PoolAllocator {}
unsafe impl Sync for PoolAllocator {}

/// Returned on allocation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    /// The pool is completely exhausted.
    OutOfMemory,
    /// The requested size exceeds the block size.
    TooLarge,
}

impl PoolAllocator {
    /// Create an uninitialised (empty) allocator.
    ///
    /// Call [`init()`] with a memory region before using.
    pub const fn new() -> Self {
        Self {
            base: ptr::null_mut(),
            block_size: 0,
            total_blocks: 0,
            free_head: usize::MAX,
            used: 0,
        }
    }

    /// Initialise the pool over the given memory region.
    ///
    /// # Safety
    /// - `region` must be validly aligned for `usize` reads/writes.
    /// - `region` must not overlap with any other live allocation.
    /// - The caller must ensure single-threaded access during init.
    pub unsafe fn init(&mut self, region: &mut [u8], block_size: usize) {
        assert!(block_size >= core::mem::size_of::<usize>());
        let num = region.len() / block_size;
        assert!(num > 0);

        self.base = region.as_mut_ptr();
        self.block_size = block_size;
        self.total_blocks = num;
        self.used = 0;

        // Thread the free-list through the first usize of each block.
        for i in 0..num {
            let blk = self.base.add(i * block_size) as *mut usize;
            if i + 1 < num {
                blk.write(i + 1); // next free block index
            } else {
                blk.write(usize::MAX); // end of list
            }
        }
        self.free_head = 0;
    }

    /// Allocate one block. Returns a pointer to the block, or an error.
    ///
    /// O(1) — pops the head of the free-list.
    pub fn alloc(&mut self) -> Result<*mut u8, AllocError> {
        if self.free_head == usize::MAX {
            return Err(AllocError::OutOfMemory);
        }
        let idx = self.free_head;
        let blk = unsafe { self.base.add(idx * self.block_size) };
        // Advance free-list head.
        self.free_head = unsafe { (blk as *const usize).read() };
        self.used += 1;
        Ok(blk)
    }

    /// Free a previously allocated block.
    ///
    /// # Safety
    /// `ptr` must have been returned by a prior `alloc()` on **this** pool
    /// and must not have been freed already.
    pub unsafe fn free(&mut self, ptr: *mut u8) {
        let offset = ptr as usize - self.base as usize;
        let idx = offset / self.block_size;
        debug_assert!(idx < self.total_blocks);
        // Push onto the free-list head.
        (ptr as *mut usize).write(self.free_head);
        self.free_head = idx;
        self.used -= 1;
    }

    /// Block size this pool manages.
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Total blocks in the pool.
    pub fn total(&self) -> usize {
        self.total_blocks
    }

    /// Currently allocated blocks.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Free (available) blocks.
    pub fn free_count(&self) -> usize {
        self.total_blocks - self.used
    }
}

// ---------------------------------------------------------------------------
// Heap — unified façade over small + large pools
// ---------------------------------------------------------------------------

/// Kernel heap: two fixed-block-size pools (small + large).
///
/// Allocation picks the smallest pool that fits the requested size.
pub struct Heap {
    pub small: PoolAllocator,
    pub large: PoolAllocator,
}

impl Heap {
    pub const fn new() -> Self {
        Self {
            small: PoolAllocator::new(),
            large: PoolAllocator::new(),
        }
    }

    /// Initialise both pools from a single contiguous memory region.
    ///
    /// The region is split: the first `small_bytes` go to the small-block
    /// pool, the rest to the large-block pool.
    ///
    /// # Safety
    /// Same preconditions as [`PoolAllocator::init()`].
    pub unsafe fn init(&mut self, region: &mut [u8], small_bytes: usize) {
        let (small_region, large_region) = region.split_at_mut(small_bytes);
        self.small.init(small_region, SMALL_BLOCK);
        self.large.init(large_region, LARGE_BLOCK);
    }

    /// Allocate `size` bytes.  Returns a block pointer from the smallest
    /// fitting pool, or `AllocError`.
    pub fn alloc(&mut self, size: usize) -> Result<*mut u8, AllocError> {
        if size <= SMALL_BLOCK {
            self.small.alloc()
        } else if size <= LARGE_BLOCK {
            self.large.alloc()
        } else {
            Err(AllocError::TooLarge)
        }
    }

    /// Free a pointer that was returned by [`alloc()`].
    ///
    /// # Safety
    /// The pointer must have been returned by a prior `alloc()` call on
    /// this heap and must not yet have been freed.
    pub unsafe fn free(&mut self, ptr: *mut u8, size: usize) {
        if size <= SMALL_BLOCK {
            self.small.free(ptr);
        } else {
            self.large.free(ptr);
        }
    }

    /// Print a summary suitable for the shell `sysinfo` / `meminfo` command.
    pub fn write_stats(&self, w: &mut dyn core::fmt::Write) {
        let _ = core::fmt::write(
            w,
            format_args!(
                "  small pool : {}/{} blocks ({} B each)\n",
                self.small.used(),
                self.small.total(),
                SMALL_BLOCK,
            ),
        );
        let _ = core::fmt::write(
            w,
            format_args!(
                "  large pool : {}/{} blocks ({} B each)\n",
                self.large.used(),
                self.large.total(),
                LARGE_BLOCK,
            ),
        );
        let total_bytes = self.small.total() * SMALL_BLOCK + self.large.total() * LARGE_BLOCK;
        let used_bytes = self.small.used() * SMALL_BLOCK + self.large.used() * LARGE_BLOCK;
        let _ = core::fmt::write(
            w,
            format_args!(
                "  total      : {} / {} bytes used\n",
                used_bytes, total_bytes,
            ),
        );
    }
}
