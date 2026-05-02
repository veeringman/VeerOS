//! Configuration file parser — `no_std`, zero-alloc, `key=value` format.
//!
//! Parses files like `/etc/net/wifi`:
//!
//! ```text
//! ssid=MARS
//! password=Naitla123
//! ```
//!
//! Lines starting with `#` are comments. Empty lines are ignored.
//! Keys and values are trimmed of leading/trailing whitespace.
//! Maximum key length: 31 bytes. Maximum value length: 127 bytes.

/// Maximum key length.
pub const MAX_KEY: usize = 32;
/// Maximum value length.
pub const MAX_VAL: usize = 128;
/// Maximum entries per config.
pub const MAX_ENTRIES: usize = 16;

/// A single key=value entry.
#[derive(Clone)]
pub struct ConfigEntry {
    pub key: [u8; MAX_KEY],
    pub key_len: u8,
    pub val: [u8; MAX_VAL],
    pub val_len: u8,
}

impl ConfigEntry {
    pub const fn empty() -> Self {
        Self {
            key: [0u8; MAX_KEY],
            key_len: 0,
            val: [0u8; MAX_VAL],
            val_len: 0,
        }
    }

    pub fn key_str(&self) -> &str {
        core::str::from_utf8(&self.key[..self.key_len as usize]).unwrap_or("")
    }

    pub fn val_str(&self) -> &str {
        core::str::from_utf8(&self.val[..self.val_len as usize]).unwrap_or("")
    }

    pub fn val_bytes(&self) -> &[u8] {
        &self.val[..self.val_len as usize]
    }
}

/// Parsed configuration — up to [`MAX_ENTRIES`] key=value pairs.
pub struct Config {
    pub entries: [ConfigEntry; MAX_ENTRIES],
    pub count: usize,
}

impl Config {
    pub const fn new() -> Self {
        Self {
            entries: [const { ConfigEntry::empty() }; MAX_ENTRIES],
            count: 0,
        }
    }

    /// Parse a `key=value` config from raw bytes.
    pub fn parse(data: &[u8]) -> Self {
        let mut cfg = Self::new();
        let mut start = 0;
        while start < data.len() && cfg.count < MAX_ENTRIES {
            // Find end of line.
            let end = data[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| start + p)
                .unwrap_or(data.len());
            let line = &data[start..end];
            start = end + 1;

            // Trim leading whitespace.
            let line = trim(line);
            // Skip empty lines and comments.
            if line.is_empty() || line[0] == b'#' {
                continue;
            }

            // Split on first '='.
            if let Some(eq) = line.iter().position(|&b| b == b'=') {
                let key = trim(&line[..eq]);
                let val = trim(&line[eq + 1..]);
                if !key.is_empty() {
                    let e = &mut cfg.entries[cfg.count];
                    let kl = key.len().min(MAX_KEY);
                    e.key[..kl].copy_from_slice(&key[..kl]);
                    e.key_len = kl as u8;
                    let vl = val.len().min(MAX_VAL);
                    e.val[..vl].copy_from_slice(&val[..vl]);
                    e.val_len = vl as u8;
                    cfg.count += 1;
                }
            }
        }
        cfg
    }

    /// Look up a value by key. Returns `None` if not found.
    pub fn get(&self, key: &str) -> Option<&str> {
        let kb = key.as_bytes();
        for i in 0..self.count {
            let e = &self.entries[i];
            if e.key_len as usize == kb.len() && &e.key[..kb.len()] == kb {
                return Some(e.val_str());
            }
        }
        None
    }

    /// Look up a value by key as raw bytes.
    pub fn get_bytes(&self, key: &str) -> Option<&[u8]> {
        let kb = key.as_bytes();
        for i in 0..self.count {
            let e = &self.entries[i];
            if e.key_len as usize == kb.len() && &e.key[..kb.len()] == kb {
                return Some(e.val_bytes());
            }
        }
        None
    }
}

/// Trim leading and trailing ASCII whitespace from a byte slice.
fn trim(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|&c| c != b' ' && c != b'\t' && c != b'\r')
        .unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|&c| c != b' ' && c != b'\t' && c != b'\r')
        .map(|p| p + 1)
        .unwrap_or(start);
    &b[start..end]
}
