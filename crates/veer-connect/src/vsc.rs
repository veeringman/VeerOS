//! VSC v1 client-side protocol: handshake, framing, encryption.

use crypto::chacha20::ChaCha20Poly1305;
use crypto::rng::ChaChaRng;
use crypto::sha256::Sha256;
use crypto::x25519::{x25519_diffie_hellman, x25519_keypair};
use crypto::{Aead, Hash, zeroize};

use std::io::{Read, Write};
use std::net::TcpStream;

// ── Protocol constants (must match crates/net/src/secure.rs) ────────────

/// VSC magic bytes + version.
pub const VSC_MAGIC: [u8; 4] = [b'V', b'S', b'C', 0x01];

/// Maximum plaintext per frame.
pub const MAX_FRAME_PT: usize = 256;

/// Poly1305 tag size.
pub const TAG_LEN: usize = 16;

/// Maximum ciphertext per frame.
const MAX_FRAME_CT: usize = MAX_FRAME_PT + TAG_LEN;

// ── Mode bytes (sent by client after handshake) ─────────────────────────

pub const MODE_SHELL: u8 = 0x00;
pub const MODE_PUSH: u8 = 0x01;
pub const MODE_PULL: u8 = 0x02;

// ── SecureChannel ───────────────────────────────────────────────────────

/// Client-side encrypted channel.
pub struct SecureChannel {
    cipher: ChaCha20Poly1305,
    /// Client sends on odd nonces (1, 3, 5, ...).
    tx_counter: u64,
    /// Client receives on even nonces (0, 2, 4, ...).
    rx_counter: u64,
}

impl SecureChannel {
    fn make_nonce(counter: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[4..12].copy_from_slice(&counter.to_le_bytes());
        nonce
    }

    /// Encrypt plaintext → frame bytes: [2-byte LE len][ciphertext+tag].
    pub fn encrypt(&mut self, plaintext: &[u8], out: &mut [u8]) -> Option<usize> {
        let ct_len = plaintext.len() + TAG_LEN;
        if ct_len > MAX_FRAME_CT || out.len() < ct_len + 2 {
            return None;
        }
        let len_bytes = (ct_len as u16).to_le_bytes();
        out[0] = len_bytes[0];
        out[1] = len_bytes[1];
        out[2..2 + plaintext.len()].copy_from_slice(plaintext);

        let nonce = Self::make_nonce(self.tx_counter);
        self.tx_counter += 2;

        let total = self.cipher
            .seal_in_place(&nonce, &[], &mut out[2..], plaintext.len())
            .ok()?;
        Some(2 + total)
    }

    /// Decrypt a ciphertext+tag buffer in-place. Returns plaintext length.
    pub fn decrypt(&mut self, ct: &mut [u8]) -> Option<usize> {
        if ct.len() < TAG_LEN {
            return None;
        }
        let nonce = Self::make_nonce(self.rx_counter);
        self.rx_counter += 2;
        self.cipher.open_in_place(&nonce, &[], ct, ct.len()).ok()
    }
}

impl Drop for SecureChannel {
    fn drop(&mut self) {
        self.tx_counter = 0;
        self.rx_counter = 0;
    }
}

// ── Handshake ───────────────────────────────────────────────────────────

/// Perform client-side VSC v1 handshake.
///
/// 1. Read server hello: `VSC\x01` + server_pubkey (36 bytes)
/// 2. Generate ephemeral X25519 keypair
/// 3. Send client pubkey (32 bytes)
/// 4. Derive shared_secret → session_key = SHA-256(shared_secret)
pub fn client_handshake(stream: &TcpStream) -> Option<SecureChannel> {
    let mut reader = stream;

    // Read 36-byte server hello.
    let mut hello = [0u8; 36];
    if let Err(e) = reader.read_exact(&mut hello) {
        eprintln!("[vsc] handshake: read server hello failed: {}", e);
        return None;
    }
    if hello[..4] != VSC_MAGIC {
        eprintln!("[vsc] handshake: bad magic: {:02x} {:02x} {:02x} {:02x}",
            hello[0], hello[1], hello[2], hello[3]);
        return None;
    }
    let mut server_pk = [0u8; 32];
    server_pk.copy_from_slice(&hello[4..36]);

    // Seed RNG from OS entropy.
    let mut seed = [0u8; 32];
    os_random(&mut seed);
    let mut rng = ChaChaRng::from_seed(seed);
    zeroize(&mut seed);

    // Generate ephemeral keypair.
    let (mut sk, pk) = x25519_keypair(&mut rng);

    // Send client pubkey.
    let mut writer = stream;
    writer.write_all(&pk).ok()?;

    // Derive shared secret.
    let ss = match x25519_diffie_hellman(&sk, &server_pk) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[vsc] handshake: DH failed: {:?}", e);
            zeroize(&mut sk);
            return None;
        }
    };
    zeroize(&mut sk);

    // Session key = SHA-256(shared_secret).
    let digest = Sha256::digest(&ss);
    let mut session_key = [0u8; 32];
    session_key.copy_from_slice(&digest.bytes[..32]);

    let cipher = ChaCha20Poly1305::new(&session_key);
    zeroize(&mut session_key);

    Some(SecureChannel {
        cipher,
        tx_counter: 1, // client sends on odd
        rx_counter: 0, // client receives on even
    })
}

// ── Frame I/O ───────────────────────────────────────────────────────────

/// Read exactly `n` bytes from a TcpStream.
/// Retries on `WouldBlock` (non-blocking socket).
pub fn read_exact(stream: &TcpStream, buf: &mut [u8]) -> bool {
    let mut offset = 0;
    while offset < buf.len() {
        let mut reader = stream;
        match reader.read(&mut buf[offset..]) {
            Ok(0) => return false,
            Ok(n) => offset += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            Err(ref e) => {
                eprintln!("[vsc] read error: {}", e);
                return false;
            }
        }
    }
    true
}

/// Receive and decrypt one frame. Returns plaintext in `buf[..len]`.
pub fn recv_frame(stream: &TcpStream, ch: &mut SecureChannel, buf: &mut [u8]) -> Option<usize> {
    // Read 2-byte length header.
    let mut hdr = [0u8; 2];
    if !read_exact(stream, &mut hdr) {
        eprintln!("[vsc] recv: header EOF");
        return None;
    }
    let ct_len = u16::from_le_bytes(hdr) as usize;
    if ct_len < TAG_LEN || ct_len > MAX_FRAME_CT {
        eprintln!("[vsc] recv: bad frame len {}", ct_len);
        return None;
    }

    // Read ciphertext+tag.
    let mut ct = [0u8; MAX_FRAME_CT];
    if !read_exact(stream, &mut ct[..ct_len]) {
        eprintln!("[vsc] recv: body EOF ct_len={}", ct_len);
        return None;
    }

    // Decrypt in-place.
    let ct_snap = [ct[0], ct[1], ct[2], ct.get(3).copied().unwrap_or(0)];
    let pt_len = match ch.decrypt(&mut ct[..ct_len]) {
        Some(n) => n,
        None => {
            eprintln!("[vsc] recv: decrypt failed rx_ctr={} ct_len={} hdr=[{:02x},{:02x}] ct[..4]={:02x?}",
                ch.rx_counter.wrapping_sub(2), ct_len, hdr[0], hdr[1], ct_snap);
            return None;
        }
    };
    buf[..pt_len].copy_from_slice(&ct[..pt_len]);
    Some(pt_len)
}

/// Encrypt and send one frame.
pub fn send_frame(stream: &TcpStream, ch: &mut SecureChannel, data: &[u8]) {
    // Split into MAX_FRAME_PT chunks.
    let mut offset = 0;
    while offset < data.len() {
        let end = std::cmp::min(offset + MAX_FRAME_PT, data.len());
        let chunk = &data[offset..end];

        let mut frame = [0u8; MAX_FRAME_CT + 2];
        if let Some(n) = ch.encrypt(chunk, &mut frame) {
            let mut writer = stream;
            let _ = writer.write_all(&frame[..n]);
        }
        offset = end;
    }
}

// ── OS entropy ──────────────────────────────────────────────────────────

/// Fill buffer with OS-provided random bytes.
fn os_random(buf: &mut [u8]) {
    // Linux: /dev/urandom.  Extensible to Windows (BCryptGenRandom)
    // and macOS (SecRandomCopyBytes).
    use std::fs::File;
    let mut f = File::open("/dev/urandom").expect("failed to open /dev/urandom");
    f.read_exact(buf).expect("failed to read entropy");
}
