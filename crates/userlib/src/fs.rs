//! Filesystem access — safe wrappers around the VFS syscalls.
//!
//! Provides `open`, `close`, `read`, `write`, `seek`, `stat`, `mkdir`,
//! `unlink`, `readdir`, etc. as safe functions that issue `ecall`
//! instructions to the kernel.

use crate::sys::{syscall1, syscall2, syscall3, syscall4};

// Syscall numbers (must match microkernel::syscall).
const SYS_OPEN: usize = 0xA0;
const SYS_CLOSE: usize = 0xA1;
const SYS_READ: usize = 0xA2;
const SYS_WRITE: usize = 0xA3;
const SYS_SEEK: usize = 0xA4;
const SYS_STAT: usize = 0xA5;
const SYS_FSTAT: usize = 0xA6;
const SYS_MKDIR: usize = 0xA7;
const SYS_UNLINK: usize = 0xA8;
const SYS_READDIR: usize = 0xA9;
const SYS_TRUNCATE: usize = 0xAA;
const SYS_RENAME: usize = 0xAB;
const SYS_GETCWD: usize = 0xAC;
const SYS_CHDIR: usize = 0xAD;

/// Open flags (mirrors kernel `OpenFlags`).
pub const O_RDONLY: usize = 0;
pub const O_WRONLY: usize = 1;
pub const O_RDWR: usize = 2;
pub const O_CREAT: usize = 4;
pub const O_TRUNC: usize = 8;
pub const O_APPEND: usize = 16;

/// Seek whence values.
pub const SEEK_SET: usize = 0;
pub const SEEK_CUR: usize = 1;
pub const SEEK_END: usize = 2;

/// A directory entry returned by `readdir`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub kind: u8,
    pub name_len: u8,
    pub size_lo: u16,
    pub size_hi: u16,
    pub _pad: u16,
    pub name: [u8; 28],
}

impl DirEntry {
    pub const fn zeroed() -> Self {
        Self {
            kind: 0,
            name_len: 0,
            size_lo: 0,
            size_hi: 0,
            _pad: 0,
            name: [0u8; 28],
        }
    }

    /// Get the entry name as `&str`.
    pub fn name_str(&self) -> &str {
        let len = self.name_len as usize;
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    /// Full file size.
    pub fn size(&self) -> u32 {
        (self.size_lo as u32) | ((self.size_hi as u32) << 16)
    }
}

/// File status information returned by `stat`/`fstat`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct StatBuf {
    pub kind: u8,
    pub dev_major: u8,
    pub dev_minor: u8,
    pub _pad: u8,
    pub size: u32,
    pub inode_id: u16,
    pub parent: u16,
}

impl StatBuf {
    pub const fn zeroed() -> Self {
        Self {
            kind: 0,
            dev_major: 0,
            dev_minor: 0,
            _pad: 0,
            size: 0,
            inode_id: 0,
            parent: 0,
        }
    }
}

/// Open a file or directory. Returns a file descriptor, or `usize::MAX` on error.
#[inline]
pub fn open(path: &str, flags: usize) -> Result<usize, ()> {
    let fd = syscall3(SYS_OPEN, path.as_ptr() as usize, path.len(), flags);
    if fd == usize::MAX {
        Err(())
    } else {
        Ok(fd)
    }
}

/// Close a file descriptor.
#[inline]
pub fn close(fd: usize) -> Result<(), ()> {
    let ret = syscall1(SYS_CLOSE, fd);
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Read from a file descriptor into `buf`. Returns bytes read.
#[inline]
pub fn read(fd: usize, buf: &mut [u8]) -> Result<usize, ()> {
    let ret = syscall3(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len());
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(ret)
    }
}

/// Write to a file descriptor from `buf`. Returns bytes written.
#[inline]
pub fn write(fd: usize, buf: &[u8]) -> Result<usize, ()> {
    let ret = syscall3(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len());
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(ret)
    }
}

/// Seek within a file. Returns the new position.
#[inline]
pub fn seek(fd: usize, offset: usize, whence: usize) -> Result<usize, ()> {
    let ret = syscall3(SYS_SEEK, fd, offset, whence);
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(ret)
    }
}

/// Get file metadata by path.
#[inline]
pub fn stat(path: &str, buf: &mut StatBuf) -> Result<(), ()> {
    let ret = syscall3(
        SYS_STAT,
        path.as_ptr() as usize,
        path.len(),
        buf as *mut StatBuf as usize,
    );
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Get file metadata by open file descriptor.
#[inline]
pub fn fstat(fd: usize, buf: &mut StatBuf) -> Result<(), ()> {
    let (ret, _) = syscall2(SYS_FSTAT, fd, buf as *mut StatBuf as usize);
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Create a directory.
#[inline]
pub fn mkdir(path: &str) -> Result<(), ()> {
    let (ret, _) = syscall2(SYS_MKDIR, path.as_ptr() as usize, path.len());
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Remove a file or empty directory.
#[inline]
pub fn unlink(path: &str) -> Result<(), ()> {
    let (ret, _) = syscall2(SYS_UNLINK, path.as_ptr() as usize, path.len());
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Read directory entries from an open directory fd.
/// Returns the number of entries read.
#[inline]
pub fn readdir(fd: usize, entries: &mut [DirEntry]) -> Result<usize, ()> {
    let ret = syscall3(
        SYS_READDIR,
        fd,
        entries.as_mut_ptr() as usize,
        entries.len(),
    );
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(ret)
    }
}

/// Truncate an open file to `new_size` bytes.
#[inline]
pub fn truncate(fd: usize, new_size: usize) -> Result<(), ()> {
    let (ret, _) = syscall2(SYS_TRUNCATE, fd, new_size);
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Rename / move a file or directory.
#[inline]
pub fn rename(old_path: &str, new_path: &str) -> Result<(), ()> {
    let ret = syscall4(
        SYS_RENAME,
        old_path.as_ptr() as usize,
        old_path.len(),
        new_path.as_ptr() as usize,
        new_path.len(),
    );
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

/// Get the current working directory.
#[inline]
pub fn getcwd(buf: &mut [u8]) -> Result<usize, ()> {
    let (ret, _) = syscall2(SYS_GETCWD, buf.as_mut_ptr() as usize, buf.len());
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(ret)
    }
}

/// Change the current working directory.
#[inline]
pub fn chdir(path: &str) -> Result<(), ()> {
    let (ret, _) = syscall2(SYS_CHDIR, path.as_ptr() as usize, path.len());
    if ret == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}
