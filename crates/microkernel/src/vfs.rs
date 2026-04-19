//! Virtual Filesystem (VFS) — unified file abstraction for VeerOS.
//!
//! Provides in-memory inodes, per-process file descriptors, path resolution,
//! and mount table. Everything is a file: regular files, directories, device
//! nodes, and pipes.
//!
//! # Design constraints
//! - `no_std`, no heap — all structures are fixed-size static arrays.
//! - ESP32-C6: 16 KB kernel heap → inodes fit in 64-byte small blocks.
//! - QEMU/RPi: 64 KB heap → more room but same data structures.

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum open file descriptors per process.
pub const MAX_FDS: usize = 16;

/// Maximum inodes in the system.
pub const MAX_INODES: usize = 128;

/// Maximum length of a file/directory name.
pub const MAX_NAME_LEN: usize = 27;

/// Maximum mount points.
pub const MAX_MOUNTS: usize = 4;

// ─── Mount table ─────────────────────────────────────────────────────

/// Filesystem type for a mount point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FsType {
    None = 0,
    RamFs = 1,
    Fat32 = 2,
}

/// A mount point entry.
///
/// For non-device (ramfs) inodes, `dev_major` on child inodes is set to
/// the 1-based mount slot index so the dispatcher can route I/O to the
/// correct filesystem backend.
#[derive(Debug, Clone, Copy)]
pub struct MountEntry {
    pub active: bool,
    pub fs_type: FsType,
    /// VFS inode ID of the directory where this filesystem is mounted.
    pub dir_inode: u16,
    /// Block device index (0-based) for FAT32 mounts.
    pub blk_index: u8,
    /// Human-readable label for `lsblk` / `mount` output (e.g. "sd0").
    pub label: [u8; 8],
}

impl MountEntry {
    pub const fn empty() -> Self {
        Self {
            active: false,
            fs_type: FsType::None,
            dir_inode: NO_INODE,
            blk_index: 0,
            label: [0u8; 8],
        }
    }

    pub fn label_str(&self) -> &str {
        let len = self.label.iter().position(|&b| b == 0).unwrap_or(8);
        core::str::from_utf8(&self.label[..len]).unwrap_or("")
    }

    pub fn set_label(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(8);
        self.label[..len].copy_from_slice(&bytes[..len]);
        if len < 8 {
            self.label[len] = 0;
        }
    }
}

/// Global mount table.
pub struct MountTable {
    pub mounts: [MountEntry; MAX_MOUNTS],
}

impl MountTable {
    pub const fn new() -> Self {
        Self {
            mounts: [MountEntry::empty(); MAX_MOUNTS],
        }
    }

    /// Find a free slot and register a mount. Returns the 1-based mount
    /// ID (used as `dev_major` on child inodes), or `None` if full.
    pub fn mount(&mut self, dir_inode: u16, fs_type: FsType, label: &str) -> Option<u8> {
        for (i, m) in self.mounts.iter_mut().enumerate() {
            if !m.active {
                m.active = true;
                m.fs_type = fs_type;
                m.dir_inode = dir_inode;
                m.set_label(label);
                return Some((i + 1) as u8); // 1-based
            }
        }
        None
    }

    /// Unmount the slot that owns `dir_inode`.
    pub fn unmount(&mut self, dir_inode: u16) -> bool {
        for m in self.mounts.iter_mut() {
            if m.active && m.dir_inode == dir_inode {
                *m = MountEntry::empty();
                return true;
            }
        }
        false
    }

    /// Look up the mount entry for a given mount_id (1-based index).
    pub fn get(&self, mount_id: u8) -> Option<&MountEntry> {
        if mount_id == 0 || mount_id as usize > MAX_MOUNTS {
            return None;
        }
        let m = &self.mounts[(mount_id - 1) as usize];
        if m.active {
            Some(m)
        } else {
            None
        }
    }
}

/// Root inode ID (always 0).
pub const ROOT_INODE: u16 = 0;

/// Sentinel value for "no inode".
pub const NO_INODE: u16 = 0xFFFF;

// ---------------------------------------------------------------------------
// Inode types
// ---------------------------------------------------------------------------

/// Kind of filesystem object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InodeKind {
    /// Slot is free / unused.
    Free = 0,
    /// Regular file.
    File = 1,
    /// Directory.
    Directory = 2,
    /// Device node (major, minor in `dev_major`/`dev_minor` fields).
    Device = 3,
}

/// Open flags for `SYS_OPEN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags(pub u8);

impl OpenFlags {
    pub const O_RDONLY: Self = Self(0);
    pub const O_WRONLY: Self = Self(1);
    pub const O_RDWR: Self = Self(2);
    pub const O_CREAT: Self = Self(4);
    pub const O_TRUNC: Self = Self(8);
    pub const O_APPEND: Self = Self(16);

    #[inline]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }

    #[inline]
    pub const fn from_raw(raw: u8) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn readable(self) -> bool {
        // Readable if O_RDONLY (0) or O_RDWR (2)
        let mode = self.0 & 0x03;
        mode == 0 || mode == 2
    }

    #[inline]
    pub const fn writable(self) -> bool {
        // Writable if O_WRONLY (1) or O_RDWR (2)
        let mode = self.0 & 0x03;
        mode == 1 || mode == 2
    }
}

/// Seek whence values.
pub const SEEK_SET: usize = 0;
pub const SEEK_CUR: usize = 1;
pub const SEEK_END: usize = 2;

// ---------------------------------------------------------------------------
// Inode
// ---------------------------------------------------------------------------

/// An inode — metadata for one filesystem object.
///
/// Designed to fit comfortably in a small structure. File data is stored
/// separately in the RamFS data pool, referenced by `data_offset` + `data_len`.
#[derive(Debug, Clone, Copy)]
pub struct Inode {
    /// Kind of inode (also indicates if the slot is in use).
    pub kind: InodeKind,
    /// File size in bytes (for regular files; 0 for directories).
    pub size: u32,
    /// Offset into the RamFS data pool where file data begins.
    pub data_offset: u32,
    /// Allocated capacity in the data pool.
    pub data_cap: u32,
    /// Parent inode ID (`NO_INODE` for root).
    pub parent: u16,
    /// First child inode ID (for directories; `NO_INODE` if empty).
    pub children_head: u16,
    /// Next sibling inode ID (`NO_INODE` if last).
    pub next_sibling: u16,
    /// Device major number (for `InodeKind::Device`).
    pub dev_major: u8,
    /// Device minor number (for `InodeKind::Device`).
    pub dev_minor: u8,
    /// Null-terminated file name.
    pub name: [u8; MAX_NAME_LEN + 1],
}

impl Inode {
    pub const fn empty() -> Self {
        Self {
            kind: InodeKind::Free,
            size: 0,
            data_offset: 0,
            data_cap: 0,
            parent: NO_INODE,
            children_head: NO_INODE,
            next_sibling: NO_INODE,
            dev_major: 0,
            dev_minor: 0,
            name: [0u8; MAX_NAME_LEN + 1],
        }
    }

    /// Get the name as a `&str`.
    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(MAX_NAME_LEN + 1);
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    /// Set the name from a string slice (truncates if too long).
    pub fn set_name(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let len = if bytes.len() > MAX_NAME_LEN { MAX_NAME_LEN } else { bytes.len() };
        self.name[..len].copy_from_slice(&bytes[..len]);
        self.name[len] = 0;
    }
}

// ---------------------------------------------------------------------------
// FileDescriptor
// ---------------------------------------------------------------------------

/// Per-process open file state.
#[derive(Debug, Clone, Copy)]
pub struct FileDescriptor {
    /// Inode this FD points to.
    pub inode_id: u16,
    /// Current read/write cursor position.
    pub cursor: u32,
    /// Open flags.
    pub flags: OpenFlags,
}

impl FileDescriptor {
    pub const fn empty() -> Self {
        Self {
            inode_id: NO_INODE,
            cursor: 0,
            flags: OpenFlags::O_RDONLY,
        }
    }
}

// ---------------------------------------------------------------------------
// DirEntry — returned by readdir
// ---------------------------------------------------------------------------

/// A directory entry returned to userspace by `SYS_READDIR`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct DirEntry {
    /// Inode kind (1=file, 2=dir, 3=device).
    pub kind: u8,
    /// Name length.
    pub name_len: u8,
    /// File size (low 16 bits for compactness in the entry).
    pub size_lo: u16,
    /// File size (high 16 bits).
    pub size_hi: u16,
    /// Reserved / padding.
    pub _pad: u16,
    /// File name (null-terminated).
    pub name: [u8; 28],
}

impl DirEntry {
    pub const SIZE: usize = core::mem::size_of::<Self>();
}

// ---------------------------------------------------------------------------
// StatBuf — returned by stat/fstat
// ---------------------------------------------------------------------------

/// File status information returned to userspace.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StatBuf {
    /// Inode kind (1=file, 2=dir, 3=device).
    pub kind: u8,
    /// Device major (for device nodes).
    pub dev_major: u8,
    /// Device minor (for device nodes).
    pub dev_minor: u8,
    pub _pad: u8,
    /// File size in bytes.
    pub size: u32,
    /// Inode ID.
    pub inode_id: u16,
    /// Parent inode ID.
    pub parent: u16,
}

// ---------------------------------------------------------------------------
// InodeTable
// ---------------------------------------------------------------------------

/// The global inode table — all filesystem metadata lives here.
pub struct InodeTable {
    pub inodes: [Inode; MAX_INODES],
}

impl InodeTable {
    pub const fn new() -> Self {
        Self {
            inodes: [Inode::empty(); MAX_INODES],
        }
    }

    /// Allocate a free inode slot. Returns the inode ID, or `None`.
    pub fn alloc(&mut self) -> Option<u16> {
        for (i, inode) in self.inodes.iter().enumerate() {
            if inode.kind == InodeKind::Free {
                return Some(i as u16);
            }
        }
        None
    }

    /// Get a reference to an inode by ID.
    pub fn get(&self, id: u16) -> Option<&Inode> {
        let idx = id as usize;
        if idx < MAX_INODES && self.inodes[idx].kind != InodeKind::Free {
            Some(&self.inodes[idx])
        } else {
            None
        }
    }

    /// Get a mutable reference to an inode by ID.
    pub fn get_mut(&mut self, id: u16) -> Option<&mut Inode> {
        let idx = id as usize;
        if idx < MAX_INODES && self.inodes[idx].kind != InodeKind::Free {
            Some(&mut self.inodes[idx])
        } else {
            None
        }
    }

    /// Create the root directory inode (id=0) and standard directories.
    pub fn init_root(&mut self) {
        // Root directory (inode 0)
        let root = &mut self.inodes[0];
        root.kind = InodeKind::Directory;
        root.parent = NO_INODE;
        root.children_head = NO_INODE;
        root.next_sibling = NO_INODE;
        root.name[0] = b'/';
        root.name[1] = 0;

        // Create /dev, /tmp, /etc
        self.mkdir_in(ROOT_INODE, "dev");
        self.mkdir_in(ROOT_INODE, "tmp");
        self.mkdir_in(ROOT_INODE, "etc");
    }

    /// Create a directory as a child of `parent_id`.
    /// Returns the new inode ID, or `None` if full.
    pub fn mkdir_in(&mut self, parent_id: u16, name: &str) -> Option<u16> {
        let id = self.alloc()?;
        let inode = &mut self.inodes[id as usize];
        inode.kind = InodeKind::Directory;
        inode.size = 0;
        inode.data_offset = 0;
        inode.data_cap = 0;
        inode.parent = parent_id;
        inode.children_head = NO_INODE;
        inode.next_sibling = NO_INODE;
        inode.dev_major = 0;
        inode.dev_minor = 0;
        inode.set_name(name);

        // Link into parent's children list.
        self.add_child(parent_id, id);
        Some(id)
    }

    /// Create a regular file as a child of `parent_id`.
    /// Returns the new inode ID, or `None` if full.
    pub fn create_file_in(&mut self, parent_id: u16, name: &str) -> Option<u16> {
        let id = self.alloc()?;
        let inode = &mut self.inodes[id as usize];
        inode.kind = InodeKind::File;
        inode.size = 0;
        inode.data_offset = 0;
        inode.data_cap = 0;
        inode.parent = parent_id;
        inode.children_head = NO_INODE;
        inode.next_sibling = NO_INODE;
        inode.dev_major = 0;
        inode.dev_minor = 0;
        inode.set_name(name);

        self.add_child(parent_id, id);
        Some(id)
    }

    /// Create a device node as a child of `parent_id`.
    pub fn create_device_in(
        &mut self,
        parent_id: u16,
        name: &str,
        major: u8,
        minor: u8,
    ) -> Option<u16> {
        let id = self.alloc()?;
        let inode = &mut self.inodes[id as usize];
        inode.kind = InodeKind::Device;
        inode.size = 0;
        inode.data_offset = 0;
        inode.data_cap = 0;
        inode.parent = parent_id;
        inode.children_head = NO_INODE;
        inode.next_sibling = NO_INODE;
        inode.dev_major = major;
        inode.dev_minor = minor;
        inode.set_name(name);

        self.add_child(parent_id, id);
        Some(id)
    }

    /// Add `child_id` to the front of `parent_id`'s children list.
    fn add_child(&mut self, parent_id: u16, child_id: u16) {
        let pi = parent_id as usize;
        if pi < MAX_INODES {
            let old_head = self.inodes[pi].children_head;
            self.inodes[child_id as usize].next_sibling = old_head;
            self.inodes[pi].children_head = child_id;
        }
    }

    /// Remove `child_id` from `parent_id`'s children list.
    fn remove_child(&mut self, parent_id: u16, child_id: u16) {
        let pi = parent_id as usize;
        if pi >= MAX_INODES {
            return;
        }
        if self.inodes[pi].children_head == child_id {
            self.inodes[pi].children_head = self.inodes[child_id as usize].next_sibling;
            return;
        }
        // Walk the sibling list.
        let mut cur = self.inodes[pi].children_head;
        while cur != NO_INODE {
            let ci = cur as usize;
            if self.inodes[ci].next_sibling == child_id {
                self.inodes[ci].next_sibling = self.inodes[child_id as usize].next_sibling;
                return;
            }
            cur = self.inodes[ci].next_sibling;
        }
    }

    /// Resolve a path to an inode ID, starting from `start`.
    /// Supports `.` and `..`. Returns `None` if not found.
    pub fn resolve(&self, start: u16, path: &str) -> Option<u16> {
        let mut current = start;

        // Absolute path starts from root.
        let path = path.trim();
        if path.is_empty() || path == "/" {
            return Some(ROOT_INODE);
        }

        if path.starts_with('/') {
            current = ROOT_INODE;
        }

        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if component == ".." {
                let inode = self.get(current)?;
                if inode.parent != NO_INODE {
                    current = inode.parent;
                }
                // At root, `..` stays at root.
                continue;
            }
            // Look up child by name.
            current = self.find_child(current, component)?;
        }
        Some(current)
    }

    /// Find a child of `parent_id` by name.
    pub fn find_child(&self, parent_id: u16, name: &str) -> Option<u16> {
        let parent = self.get(parent_id)?;
        if parent.kind != InodeKind::Directory {
            return None;
        }
        let mut child = parent.children_head;
        while child != NO_INODE {
            let ci = child as usize;
            if ci >= MAX_INODES {
                break;
            }
            let inode = &self.inodes[ci];
            if inode.name_str() == name {
                return Some(child);
            }
            child = inode.next_sibling;
        }
        None
    }

    /// Unlink (remove) an inode. For directories, must be empty.
    /// Returns `true` on success.
    pub fn unlink(&mut self, id: u16) -> bool {
        let idx = id as usize;
        if idx >= MAX_INODES || self.inodes[idx].kind == InodeKind::Free {
            return false;
        }
        // Don't unlink root.
        if id == ROOT_INODE {
            return false;
        }
        // For directories, must be empty.
        if self.inodes[idx].kind == InodeKind::Directory
            && self.inodes[idx].children_head != NO_INODE
        {
            return false;
        }
        let parent = self.inodes[idx].parent;
        if parent != NO_INODE {
            self.remove_child(parent, id);
        }
        self.inodes[idx] = Inode::empty();
        true
    }

    /// Rename: move an inode to a new parent with a new name.
    pub fn rename(&mut self, id: u16, new_parent: u16, new_name: &str) -> bool {
        let idx = id as usize;
        if idx >= MAX_INODES || self.inodes[idx].kind == InodeKind::Free {
            return false;
        }
        if id == ROOT_INODE {
            return false;
        }
        let old_parent = self.inodes[idx].parent;
        if old_parent != NO_INODE {
            self.remove_child(old_parent, id);
        }
        self.inodes[idx].parent = new_parent;
        self.inodes[idx].set_name(new_name);
        self.add_child(new_parent, id);
        true
    }

    /// Build the absolute path of an inode into the provided buffer.
    /// Returns the number of bytes written.
    pub fn build_path(&self, id: u16, buf: &mut [u8]) -> usize {
        if id == ROOT_INODE {
            if !buf.is_empty() {
                buf[0] = b'/';
                return 1;
            }
            return 0;
        }

        // Collect ancestors.
        let mut stack = [0u16; 32];
        let mut depth = 0usize;
        let mut cur = id;
        while cur != ROOT_INODE && cur != NO_INODE && depth < stack.len() {
            stack[depth] = cur;
            depth += 1;
            let idx = cur as usize;
            if idx >= MAX_INODES {
                break;
            }
            cur = self.inodes[idx].parent;
        }

        let mut pos = 0usize;
        // Write from root down.
        let mut i = depth;
        while i > 0 {
            i -= 1;
            if pos < buf.len() {
                buf[pos] = b'/';
                pos += 1;
            }
            let name = self.inodes[stack[i] as usize].name_str();
            for &b in name.as_bytes() {
                if pos < buf.len() {
                    buf[pos] = b;
                    pos += 1;
                }
            }
        }
        pos
    }
}
