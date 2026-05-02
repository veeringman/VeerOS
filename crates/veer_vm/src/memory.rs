//! Guest physical memory region.
//!
//! Single contiguous `mmap`-backed region starting at guest physical
//! address 0. Registered with KVM via `set_user_memory_region` so guest
//! page tables can map it freely. Simpler than multi-region setups used by
//! Firecracker (no MMIO gap handling yet) — works for VeerOS because the
//! kernel is linked at 1 MiB and its 2 MiB identity-map page tables cover
//! the first 4 GiB of physical address space.

use anyhow::{bail, Context, Result};
use std::ptr::NonNull;

pub struct GuestMem {
    base: NonNull<u8>,
    size: usize,
    gpa_base: u64,
}

// SAFETY: the raw pointer is only used via explicit unsafe blocks that
// observe the lifetime of `GuestMem`.
unsafe impl Send for GuestMem {}
unsafe impl Sync for GuestMem {}

impl GuestMem {
    pub fn new(size: usize) -> Result<Self> {
        Self::new_with_base(0, size)
    }

    pub fn new_with_base(gpa_base: u64, size: usize) -> Result<Self> {
        if size == 0 || size % 4096 != 0 {
            bail!("guest memory size must be a non-zero multiple of 4 KiB (got {size})");
        }
        // PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS|MAP_NORESERVE.
        // NORESERVE lets us declare big regions without committing swap up front.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let e = std::io::Error::last_os_error();
            bail!("mmap guest memory ({size} bytes): {e}");
        }
        let base = NonNull::new(ptr as *mut u8).context("mmap returned null")?;
        Ok(Self {
            base,
            size,
            gpa_base,
        })
    }

    pub fn host_addr(&self) -> u64 {
        self.base.as_ptr() as u64
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn gpa_base(&self) -> u64 {
        self.gpa_base
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.base.as_ptr(), self.size) }
    }

    /// Translate a guest physical address into a host-visible slice.
    ///
    /// Returns an error if the requested range is not fully inside the
    /// guest memory region.
    pub fn slice_mut(&self, gpa: u64, len: usize) -> Result<&mut [u8]> {
        if gpa < self.gpa_base {
            bail!(
                "guest slice starts below mapped base: gpa={:#x} base={:#x}",
                gpa,
                self.gpa_base,
            );
        }
        let offset = gpa - self.gpa_base;
        let end = offset
            .checked_add(len as u64)
            .context("guest slice end overflow")?;
        if end > self.size as u64 {
            bail!(
                "guest slice {gpa:#x}+{len:#x} out of bounds (mapped {:#x}..{:#x})",
                self.gpa_base,
                self.gpa_base + self.size as u64,
            );
        }
        // SAFETY: bounds-checked above; slot lifetime tied to `&self`.
        unsafe {
            let p = self.base.as_ptr().add(offset as usize);
            Ok(std::slice::from_raw_parts_mut(p, len))
        }
    }

    /// Write `bytes` to guest physical address `gpa`.
    pub fn write(&self, gpa: u64, bytes: &[u8]) -> Result<()> {
        self.slice_mut(gpa, bytes.len())?.copy_from_slice(bytes);
        Ok(())
    }

    pub fn read_u8(&self, gpa: u64) -> Result<u8> {
        Ok(self.slice_mut(gpa, 1)?[0])
    }

    pub fn read_u16(&self, gpa: u64) -> Result<u16> {
        let s = self.slice_mut(gpa, 2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    pub fn read_u32(&self, gpa: u64) -> Result<u32> {
        let s = self.slice_mut(gpa, 4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub fn write_u8(&self, gpa: u64, v: u8) -> Result<()> {
        self.slice_mut(gpa, 1)?[0] = v;
        Ok(())
    }

    pub fn write_u16(&self, gpa: u64, v: u16) -> Result<()> {
        self.slice_mut(gpa, 2)?.copy_from_slice(&v.to_le_bytes());
        Ok(())
    }

    pub fn write_u32(&self, gpa: u64, v: u32) -> Result<()> {
        self.slice_mut(gpa, 4)?.copy_from_slice(&v.to_le_bytes());
        Ok(())
    }
}

impl Drop for GuestMem {
    fn drop(&mut self) {
        // SAFETY: `base` and `size` came from a successful `mmap`.
        unsafe {
            libc::munmap(self.base.as_ptr() as *mut _, self.size);
        }
    }
}
