//! Simple linked-list heap allocator for the ESP32 WiFi/BLE blobs.
//!
//! The Espressif radio firmware blobs expect standard C `malloc`/`free`/
//! `calloc`/`realloc` to be available. This module provides them backed
//! by a statically-allocated 64 KiB buffer in DRAM.
//!
//! The allocator is a simple first-fit free-list. It is NOT thread-safe
//! and must only be called with interrupts disabled (which the blob does
//! via `wifi_int_disable`/`wifi_int_restore`).

use core::ffi::c_void;
use core::ptr;

/// Heap size — 64 KiB should be sufficient for WiFi blobs.
const HEAP_SIZE: usize = 128 * 1024;

/// Minimum allocation alignment. WiFi MAC DMA descriptors are 16-byte objects;
/// 4-byte alignment is enough for RISC-V but the blob/MAC want 16.
const MIN_ALIGN: usize = 16;

/// Block header size (stored before each allocation).
const HEADER_SIZE: usize = core::mem::size_of::<BlockHeader>();

/// The heap backing store — placed in BSS (DRAM) via `static mut`.
static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

/// Whether the heap has been initialised.
static mut HEAP_INIT: bool = false;

/// Head of the free list.
static mut FREE_LIST: *mut BlockHeader = ptr::null_mut();

/// Total bytes allocated (for `get_free_heap_size`).
static mut ALLOCATED: usize = 0;

/// Block header — each free block starts with this.
/// 16-byte aligned so the payload after the header is also 16-byte aligned
/// (WiFi MAC DMA descriptors require it).
#[repr(C, align(16))]
struct BlockHeader {
    /// Size of the usable region (excluding the header).
    size: usize,
    /// Next free block (null if end of list).
    next: *mut BlockHeader,
}

/// Align `val` up to `align` (must be power of 2).
#[inline]
fn align_up(val: usize, align: usize) -> usize {
    (val + align - 1) & !(align - 1)
}

unsafe fn heap_init() {
    let base = unsafe { HEAP.as_mut_ptr() };
    let aligned_base = align_up(base as usize, MIN_ALIGN);
    let usable = HEAP_SIZE - (aligned_base - base as usize);

    let first = aligned_base as *mut BlockHeader;
    unsafe {
        (*first).size = usable - HEADER_SIZE;
        (*first).next = ptr::null_mut();
        FREE_LIST = first;
        ALLOCATED = 0;
        HEAP_INIT = true;
    }
}

unsafe fn heap_alloc(size: usize) -> *mut u8 {
    if size == 0 {
        return ptr::null_mut();
    }

    if !unsafe { HEAP_INIT } {
        unsafe { heap_init() };
    }

    let aligned_size = align_up(size, MIN_ALIGN);
    // Minimum block size so free blocks can hold a header.
    let min_block = align_up(HEADER_SIZE, MIN_ALIGN);

    let mut prev: *mut BlockHeader = ptr::null_mut();
    let mut current = unsafe { FREE_LIST };

    while !current.is_null() {
        let cur = unsafe { &mut *current };
        if cur.size >= aligned_size {
            // Can we split?
            if cur.size >= aligned_size + HEADER_SIZE + min_block {
                // Split: create a new free block after the allocation.
                let new_free =
                    (current as *mut u8).add(HEADER_SIZE + aligned_size) as *mut BlockHeader;
                unsafe {
                    (*new_free).size = cur.size - aligned_size - HEADER_SIZE;
                    (*new_free).next = cur.next;
                }
                cur.size = aligned_size;
                cur.next = new_free;
            }

            // Unlink from free list.
            if prev.is_null() {
                unsafe { FREE_LIST = cur.next };
            } else {
                unsafe { (*prev).next = cur.next };
            }
            cur.next = ptr::null_mut();
            unsafe { ALLOCATED += cur.size };
            return (current as *mut u8).add(HEADER_SIZE);
        }
        prev = current;
        current = cur.next;
    }

    // Out of memory.
    ptr::null_mut()
}

unsafe fn heap_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }

    let header = ptr.sub(HEADER_SIZE) as *mut BlockHeader;
    unsafe { ALLOCATED -= (*header).size };

    // Insert back into the free list (sorted by address for coalescing).
    let mut prev: *mut BlockHeader = ptr::null_mut();
    let mut current = unsafe { FREE_LIST };

    while !current.is_null() && (current as usize) < (header as usize) {
        prev = current;
        current = unsafe { (*current).next };
    }

    unsafe {
        (*header).next = current;

        if prev.is_null() {
            FREE_LIST = header;
        } else {
            (*prev).next = header;
        }

        // Coalesce with next block.
        if !(*header).next.is_null() {
            let header_end = (header as *mut u8).add(HEADER_SIZE + (*header).size);
            if header_end == (*header).next as *mut u8 {
                (*header).size += HEADER_SIZE + (*(*header).next).size;
                (*header).next = (*(*header).next).next;
            }
        }

        // Coalesce with previous block.
        if !prev.is_null() {
            let prev_end = (prev as *mut u8).add(HEADER_SIZE + (*prev).size);
            if prev_end == header as *mut u8 {
                (*prev).size += HEADER_SIZE + (*header).size;
                (*prev).next = (*header).next;
            }
        }
    }
}

unsafe fn heap_realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
    if ptr.is_null() {
        return unsafe { heap_alloc(new_size) };
    }
    if new_size == 0 {
        unsafe { heap_free(ptr) };
        return ptr::null_mut();
    }

    let header = unsafe { &*(ptr.sub(HEADER_SIZE) as *const BlockHeader) };
    let old_size = header.size;

    if new_size <= old_size {
        return ptr;
    }

    let new_ptr = unsafe { heap_alloc(new_size) };
    if !new_ptr.is_null() {
        unsafe { ptr::copy_nonoverlapping(ptr, new_ptr, old_size.min(new_size)) };
        unsafe { heap_free(ptr) };
    }
    new_ptr
}

/// Get free heap size.
pub fn free_heap_size() -> usize {
    if !unsafe { HEAP_INIT } {
        return HEAP_SIZE;
    }
    HEAP_SIZE - unsafe { ALLOCATED } - HEADER_SIZE
}

// ═══════════════════════════════════════════════════════════════════════════
// C ABI exports required by the WiFi/BLE blobs
// ═══════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    unsafe { heap_alloc(size) as *mut c_void }
}

#[no_mangle]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    unsafe { heap_free(ptr as *mut u8) }
}

#[no_mangle]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total = nmemb.saturating_mul(size);
    let ptr = unsafe { heap_alloc(total) };
    if !ptr.is_null() {
        unsafe { ptr::write_bytes(ptr, 0, total) };
    }
    ptr as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { heap_realloc(ptr as *mut u8, size) as *mut c_void }
}

// Internal variants — blobs sometimes call these directly.
#[no_mangle]
pub unsafe extern "C" fn malloc_internal(size: usize) -> *mut c_void {
    unsafe { malloc(size) }
}

#[no_mangle]
pub unsafe extern "C" fn free_internal(ptr: *mut c_void) {
    unsafe { free(ptr) }
}

#[no_mangle]
pub unsafe extern "C" fn realloc_internal(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { realloc(ptr, size) }
}

#[no_mangle]
pub unsafe extern "C" fn calloc_internal(nmemb: usize, size: usize) -> *mut c_void {
    unsafe { calloc(nmemb, size) }
}

#[no_mangle]
pub unsafe extern "C" fn get_free_internal_heap_size() -> usize {
    free_heap_size()
}
