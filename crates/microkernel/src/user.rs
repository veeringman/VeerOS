//! User identity and authentication subsystem.
//!
//! Feature-gated: `multi-user` enables the full identity system with login,
//! per-user process ownership, and access control.  When `multi-user` is
//! **not** enabled (the default), all processes run as UID 0 (root) with
//! no authentication overhead.

/// User identifier (16-bit).  UID 0 = root/system.
pub type UserId = u16;

/// Group identifier (16-bit).  GID 0 = root group.
pub type GroupId = u16;

/// UID for the root/system user.
pub const ROOT_UID: UserId = 0;
/// GID for the root group.
pub const ROOT_GID: GroupId = 0;
/// UID when no user is logged in (single-user mode implicit root).
pub const NOBODY_UID: UserId = 0xFFFF;

/// Maximum number of user accounts.
pub const MAX_USERS: usize = 8;
/// Maximum number of groups.
pub const MAX_GROUPS: usize = 8;
/// Maximum number of concurrent sessions.
pub const MAX_SESSIONS: usize = 4;
/// Failed login lockout threshold.
const MAX_LOGIN_ATTEMPTS: u8 = 3;

// ─── User entry ─────────────────────────────────────────────────────────

/// User account flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserFlags(pub u8);

impl UserFlags {
    pub const ENABLED: Self = Self(1 << 0);
    pub const LOCKED: Self = Self(1 << 1);

    pub fn is_enabled(self) -> bool {
        self.0 & Self::ENABLED.0 != 0
    }
    pub fn is_locked(self) -> bool {
        self.0 & Self::LOCKED.0 != 0
    }
}

/// A user account entry.
#[derive(Debug, Clone, Copy)]
pub struct UserEntry {
    pub uid: UserId,
    pub gid: GroupId,
    pub name: &'static str,
    /// Simple password hash (SipHash-style u64 for no_std; not cryptographically
    /// strong — will be upgraded when crypto crate lands).
    pub password_hash: u64,
    pub flags: UserFlags,
}

impl UserEntry {
    pub const fn empty() -> Self {
        Self {
            uid: NOBODY_UID,
            gid: 0,
            name: "",
            password_hash: 0,
            flags: UserFlags(0),
        }
    }
}

/// A group entry.
#[derive(Debug, Clone, Copy)]
pub struct GroupEntry {
    pub gid: GroupId,
    pub name: &'static str,
    /// Bitmask of member UIDs (bit N = user with table index N).
    pub members: u8,
}

impl GroupEntry {
    pub const fn empty() -> Self {
        Self {
            gid: 0,
            name: "",
            members: 0,
        }
    }
}

// ─── Session ────────────────────────────────────────────────────────────

/// Active login session — ties a UID to a process tree.
#[derive(Debug, Clone, Copy)]
pub struct Session {
    pub active: bool,
    pub uid: UserId,
    pub gid: GroupId,
    /// Opaque session token returned to userspace.
    pub token: u32,
}

impl Session {
    pub const fn empty() -> Self {
        Self {
            active: false,
            uid: NOBODY_UID,
            gid: 0,
            token: 0,
        }
    }
}

// ─── User table ─────────────────────────────────────────────────────────

/// Fixed-size user/group database and session manager.
pub struct UserTable {
    pub users: [UserEntry; MAX_USERS],
    pub user_count: usize,
    pub groups: [GroupEntry; MAX_GROUPS],
    pub group_count: usize,
    pub sessions: [Session; MAX_SESSIONS],
    next_token: u32,
    /// Per-user failed login attempt counter (indexed by user table slot).
    failed_attempts: [u8; MAX_USERS],
}

impl UserTable {
    pub const fn new() -> Self {
        Self {
            users: [UserEntry::empty(); MAX_USERS],
            user_count: 0,
            groups: [GroupEntry::empty(); MAX_GROUPS],
            group_count: 0,
            sessions: [Session::empty(); MAX_SESSIONS],
            next_token: 1,
            failed_attempts: [0; MAX_USERS],
        }
    }

    /// Initialise with a default root user.  Call once at boot.
    pub fn init_defaults(&mut self) {
        // Root user: uid=0, gid=0, password="toor"
        self.users[0] = UserEntry {
            uid: 0,
            gid: 0,
            name: "root",
            password_hash: simple_hash(b"toor"),
            flags: UserFlags::ENABLED,
        };
        // Default normal user: uid=1, gid=1, password="veeros"
        self.users[1] = UserEntry {
            uid: 1,
            gid: 1,
            name: "user",
            password_hash: simple_hash(b"veeros"),
            flags: UserFlags::ENABLED,
        };
        self.user_count = 2;

        // Root group
        self.groups[0] = GroupEntry {
            gid: 0,
            name: "root",
            members: 0x01, // bit 0 = user index 0 (root)
        };
        // Users group
        self.groups[1] = GroupEntry {
            gid: 1,
            name: "users",
            members: 0x03, // bit 0 = root, bit 1 = user
        };
        self.group_count = 2;
    }

    /// Look up a user by name.  Returns the table index if found.
    pub fn find_user(&self, name: &str) -> Option<usize> {
        for i in 0..self.user_count {
            if self.users[i].name == name {
                return Some(i);
            }
        }
        None
    }

    /// Look up a user by UID.  Returns the table index.
    pub fn find_uid(&self, uid: UserId) -> Option<usize> {
        for i in 0..self.user_count {
            if self.users[i].uid == uid {
                return Some(i);
            }
        }
        None
    }

    /// Attempt to authenticate a user.  Returns a session token on success.
    pub fn login(&mut self, name: &str, password: &[u8]) -> Result<u32, LoginError> {
        let idx = self.find_user(name).ok_or(LoginError::BadCredentials)?;
        let user = &self.users[idx];

        if !user.flags.is_enabled() {
            return Err(LoginError::AccountDisabled);
        }
        if user.flags.is_locked() {
            return Err(LoginError::AccountLocked);
        }
        if self.failed_attempts[idx] >= MAX_LOGIN_ATTEMPTS {
            return Err(LoginError::TooManyAttempts);
        }

        let hash = simple_hash(password);
        if hash != user.password_hash {
            self.failed_attempts[idx] += 1;
            if self.failed_attempts[idx] >= MAX_LOGIN_ATTEMPTS {
                // Lock the account after too many failures.
                self.users[idx].flags = UserFlags(
                    self.users[idx].flags.0 | UserFlags::LOCKED.0,
                );
            }
            return Err(LoginError::BadCredentials);
        }

        // Successful — reset failed attempts.
        self.failed_attempts[idx] = 0;

        // Create session.
        for s in self.sessions.iter_mut() {
            if !s.active {
                let token = self.next_token;
                self.next_token = self.next_token.wrapping_add(1);
                if self.next_token == 0 {
                    self.next_token = 1; // token 0 is invalid
                }
                *s = Session {
                    active: true,
                    uid: user.uid,
                    gid: user.gid,
                    token,
                };
                return Ok(token);
            }
        }
        Err(LoginError::TooManySessions)
    }

    /// Invalidate a session by token.  Returns `true` if found.
    pub fn logout(&mut self, token: u32) -> bool {
        for s in self.sessions.iter_mut() {
            if s.active && s.token == token {
                *s = Session::empty();
                return true;
            }
        }
        false
    }

    /// Look up a session by token.  Returns (uid, gid) if valid.
    pub fn session_info(&self, token: u32) -> Option<(UserId, GroupId)> {
        for s in &self.sessions {
            if s.active && s.token == token {
                return Some((s.uid, s.gid));
            }
        }
        None
    }

    /// Get the user name for a given UID.
    pub fn name_for_uid(&self, uid: UserId) -> &'static str {
        for i in 0..self.user_count {
            if self.users[i].uid == uid {
                return self.users[i].name;
            }
        }
        "unknown"
    }

    /// Get the count of active sessions.
    pub fn active_session_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.active).count()
    }

    /// Change the password for a user (by UID).  Returns `true` on success.
    pub fn change_password(&mut self, uid: UserId, new_password: &[u8]) -> bool {
        if let Some(idx) = self.find_uid(uid) {
            self.users[idx].password_hash = simple_hash(new_password);
            // Unlock if previously locked.
            self.users[idx].flags = UserFlags(
                self.users[idx].flags.0 & !UserFlags::LOCKED.0,
            );
            self.failed_attempts[idx] = 0;
            true
        } else {
            false
        }
    }

    /// Add a new user.  Returns the UID on success.
    pub fn add_user(
        &mut self,
        name: &'static str,
        gid: GroupId,
        password: &[u8],
    ) -> Option<UserId> {
        if self.user_count >= MAX_USERS {
            return None;
        }
        // Next UID = max existing + 1
        let uid = self.users[..self.user_count]
            .iter()
            .map(|u| u.uid)
            .max()
            .unwrap_or(0)
            + 1;
        let idx = self.user_count;
        self.users[idx] = UserEntry {
            uid,
            gid,
            name,
            password_hash: simple_hash(password),
            flags: UserFlags::ENABLED,
        };
        self.user_count += 1;
        Some(uid)
    }

    /// Remove a user by UID.  Returns `true` if removed.
    /// Cannot remove UID 0 (root).
    pub fn remove_user(&mut self, uid: UserId) -> bool {
        if uid == ROOT_UID {
            return false;
        }
        if let Some(idx) = self.find_uid(uid) {
            // Invalidate any sessions for this user.
            for s in self.sessions.iter_mut() {
                if s.active && s.uid == uid {
                    *s = Session::empty();
                }
            }
            // Shift entries down.
            for i in idx..self.user_count.saturating_sub(1) {
                self.users[i] = self.users[i + 1];
                self.failed_attempts[i] = self.failed_attempts[i + 1];
            }
            self.user_count -= 1;
            self.users[self.user_count] = UserEntry::empty();
            self.failed_attempts[self.user_count] = 0;
            true
        } else {
            false
        }
    }
}

/// Login failure reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginError {
    BadCredentials,
    AccountDisabled,
    AccountLocked,
    TooManyAttempts,
    TooManySessions,
}

/// Simple non-cryptographic hash for password comparison.
/// Uses FNV-1a (64-bit).  Will be replaced by SHA-256 when crypto lands.
pub const fn simple_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < data.len() {
        hash ^= data[i] as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}
