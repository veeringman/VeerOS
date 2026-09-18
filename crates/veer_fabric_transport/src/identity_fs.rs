//! Host file backend for persistent VeerOS node identity.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crypto::Hash;
use microkernel::node_identity::{IdentityStore, IDENTITY_RECORD_LEN};

/// Default location: `$XDG_STATE_HOME/veeros/node-identity.bin` or `~/.local/state/veeros/...`.
pub fn default_identity_path() -> PathBuf {
    if let Ok(path) = std::env::var("VEER_NODE_IDENTITY") {
        return PathBuf::from(path);
    }
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("veeros").join("node-identity.bin")
}

/// Mix OS entropy into a 32-byte seed. Prefers `/dev/urandom` on Unix.
pub fn host_entropy() -> [u8; 32] {
    let mut raw = [0u8; 32];
    #[cfg(unix)]
    {
        if let Ok(mut file) = fs::File::open("/dev/urandom") {
            if file.read_exact(&mut raw).is_ok() {
                return raw;
            }
        }
    }

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    raw[..8].copy_from_slice(&nanos.to_le_bytes()[..8]);
    raw[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    let digest = crypto::sha256::Sha256::digest(&raw);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest.bytes[..32]);
    out
}

/// Filesystem-backed [`IdentityStore`].
pub struct FileIdentityStore {
    path: PathBuf,
}

impl FileIdentityStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl IdentityStore for FileIdentityStore {
    fn load(&self, buf: &mut [u8]) -> Option<usize> {
        let data = fs::read(&self.path).ok()?;
        if data.len() > buf.len() || data.len() > IDENTITY_RECORD_LEN {
            return None;
        }
        buf[..data.len()].copy_from_slice(&data);
        Some(data.len())
    }

    fn save(&mut self, data: &[u8]) -> bool {
        if let Some(parent) = self.path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        let tmp = self.path.with_extension("bin.tmp");
        if fs::write(&tmp, data).is_err() {
            return false;
        }
        let _ = fs::remove_file(&self.path);
        fs::rename(&tmp, &self.path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use microkernel::node_identity::{NodeIdentityManager, NodeKeypair};

    #[test]
    fn file_store_survives_reload() {
        let dir = std::env::temp_dir().join(format!(
            "veeros-id-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("node-identity.bin");

        let mut store = FileIdentityStore::new(&path);
        let mut first = NodeIdentityManager::new();
        assert!(first.init_persistent(&mut store, host_entropy(), 2, 1, 1, 1));

        let mut store2 = FileIdentityStore::new(&path);
        let mut second = NodeIdentityManager::new();
        assert!(second.init_persistent(&mut store2, [0xFFu8; 32], 2, 1, 1, 2));
        assert_eq!(first.local_id, second.local_id);
        assert_eq!(
            NodeKeypair::decode_record(&first.local_keypair.encode_record())
                .unwrap()
                .ed25519_pk,
            second.local_keypair.ed25519_pk
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
