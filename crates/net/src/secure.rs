//! VeerOS Secure Connect (VSC) — encrypted channel for remote shell
//! and file transfer.
//!
//! Protocol (VSC v1):
//!   1. Server → Client:  "VSC\x01" + server_ephemeral_pubkey (32 bytes)  [36 bytes]
//!   2. Client → Server:  client_ephemeral_pubkey (32 bytes)
//!   3. Both sides:       shared_secret = X25519(our_sk, their_pk)
//!                         session_key   = SHA-256(shared_secret)
//!   4. Client sends mode frame: `[mode_byte]`
//!      - 0x00 = shell (interactive terminal)
//!      - 0x01 = push  (upload file to server)
//!      - 0x02 = pull  (download file from server)
//!   5. All subsequent data:  ChaCha20-Poly1305 encrypted frames
//!
//! Frame format (after handshake):
//!   [2-byte LE length] [ciphertext + 16-byte tag]
//!   length = plaintext_len + TAG_LEN (16)
//!
//! Nonce management:  12-byte nonce = [4-byte zero] [8-byte LE counter]
//!   Server uses even counters (0, 2, 4, ...); client uses odd (1, 3, 5, ...)
//!   This prevents nonce reuse between directions.

use arch::Serial;
use core::cell::UnsafeCell;
use crypto::sha256::Sha256;
use crypto::chacha20::ChaCha20Poly1305;
use crypto::x25519::{x25519_keypair, x25519_diffie_hellman, X25519PublicKey};
use crypto::rng::ChaChaRng;
use crypto::{Aead, Hash, zeroize};

/// VSC protocol magic + version.
const VSC_MAGIC: [u8; 4] = [b'V', b'S', b'C', 0x01];

/// Maximum plaintext per frame (keeps stack usage low on embedded).
pub const MAX_FRAME_PLAINTEXT: usize = 256;

/// Tag size for ChaCha20-Poly1305.
const TAG_LEN: usize = 16;

/// Maximum frame ciphertext = plaintext + tag.
const MAX_FRAME_CT: usize = MAX_FRAME_PLAINTEXT + TAG_LEN;

// ── Mode bytes (sent by client after handshake) ─────────────────────────

/// Interactive shell mode.
pub const MODE_SHELL: u8 = 0x00;
/// File upload (client → server).
pub const MODE_PUSH: u8 = 0x01;
/// File download (server → client).
pub const MODE_PULL: u8 = 0x02;

/// Transfer status codes.
pub const STATUS_OK: u8 = 0x00;
pub const STATUS_ERR: u8 = 0x01;

/// Server-side secure channel state.
pub struct SecureChannel {
    cipher: ChaCha20Poly1305,
    /// Server send counter (even: 0, 2, 4, ...).
    tx_counter: u64,
    /// Server recv counter (odd: 1, 3, 5, ...).
    rx_counter: u64,
}

impl SecureChannel {
    /// Build a 12-byte nonce from a counter value.
    fn make_nonce(counter: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        let bytes = counter.to_le_bytes();
        nonce[4..12].copy_from_slice(&bytes);
        nonce
    }

    /// Encrypt `plaintext` into `out_buf`.
    /// Returns the frame bytes to send: [2-byte LE len][ciphertext+tag].
    /// `out_buf` must be at least `plaintext.len() + TAG_LEN + 2`.
    pub fn encrypt_frame<'a>(
        &mut self,
        plaintext: &[u8],
        out_buf: &'a mut [u8],
    ) -> Option<&'a [u8]> {
        let ct_len = plaintext.len() + TAG_LEN;
        if ct_len > MAX_FRAME_CT || out_buf.len() < ct_len + 2 {
            return None;
        }

        // Length header (2 bytes LE).
        let len_bytes = (ct_len as u16).to_le_bytes();
        out_buf[0] = len_bytes[0];
        out_buf[1] = len_bytes[1];

        // Copy plaintext into payload area.
        out_buf[2..2 + plaintext.len()].copy_from_slice(plaintext);

        let nonce = Self::make_nonce(self.tx_counter);
        self.tx_counter += 2; // even increments for server

        let total = self.cipher.seal_in_place(
            &nonce, &[], &mut out_buf[2..], plaintext.len()
        ).ok()?;

        Some(&out_buf[..2 + total])
    }

    /// Decrypt a received frame (ciphertext+tag) in-place.
    /// `frame` must contain exactly the ciphertext+tag (no length header).
    /// Returns the plaintext length on success, or None if auth fails.
    pub fn decrypt_frame(&mut self, frame: &mut [u8]) -> Option<usize> {
        if frame.len() < TAG_LEN {
            return None;
        }

        let nonce = Self::make_nonce(self.rx_counter);
        self.rx_counter += 2; // odd increments for client

        self.cipher.open_in_place(&nonce, &[], frame, frame.len()).ok()
    }
}

impl Drop for SecureChannel {
    fn drop(&mut self) {
        self.tx_counter = 0;
        self.rx_counter = 0;
    }
}

/// Perform the server side of the VSC handshake over a Serial-like
/// transport (TCP socket wrapped as TcpSerial).
///
/// 1. Generate ephemeral X25519 keypair
/// 2. Send VSC magic + pubkey
/// 3. Read client pubkey
/// 4. Derive shared secret → session key
///
/// Returns `Some(SecureChannel)` on success.
pub fn server_handshake<S: Serial>(
    serial: &S,
    seed: [u8; 32],
) -> Option<SecureChannel> {
    let mut rng = ChaChaRng::from_seed(seed);

    // Generate ephemeral keypair.
    let (mut sk, pk) = x25519_keypair(&mut rng);

    // Send: VSC\x01 + server pubkey (36 bytes) as a single write.
    let mut hello = [0u8; 36];
    hello[..4].copy_from_slice(&VSC_MAGIC);
    hello[4..36].copy_from_slice(&pk);
    serial.write_bytes(&hello);
    serial.flush();

    // Read client pubkey (32 bytes).
    let mut client_pk: X25519PublicKey = [0u8; 32];
    for byte in client_pk.iter_mut() {
        *byte = serial.read_byte();
    }

    // Derive shared secret.
    let ss = match x25519_diffie_hellman(&sk, &client_pk) {
        Ok(s) => s,
        Err(_) => {
            zeroize(&mut sk);
            return None;
        }
    };

    // Wipe secret key immediately.
    zeroize(&mut sk);

    // Derive session key = SHA-256(shared_secret).
    let digest = Sha256::digest(&ss);
    let mut session_key = [0u8; 32];
    session_key.copy_from_slice(&digest.bytes[..32]);

    let cipher = ChaCha20Poly1305::new(&session_key);
    zeroize(&mut session_key);

    Some(SecureChannel {
        cipher,
        tx_counter: 0, // server sends on even nonces
        rx_counter: 1, // server receives on odd nonces
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// SecureSerial — wraps a Serial + SecureChannel as a new Serial
// ═══════════════════════════════════════════════════════════════════════════

/// A Serial adapter that encrypts/decrypts through a SecureChannel.
///
/// Outgoing bytes are buffered and flushed as encrypted frames.
/// Incoming frames are decrypted and yielded byte-by-byte.
pub struct SecureSerial<S: Serial> {
    inner: UnsafeCell<SecureSerialInner<S>>,
}

struct SecureSerialInner<S: Serial> {
    serial: S,
    channel: SecureChannel,
    tx_buf: [u8; MAX_FRAME_PLAINTEXT],
    tx_len: usize,
    rx_buf: [u8; MAX_FRAME_PLAINTEXT],
    rx_pos: usize,
    rx_len: usize,
}

impl<S: Serial> SecureSerial<S> {
    pub fn new(inner: S, channel: SecureChannel) -> Self {
        Self {
            inner: UnsafeCell::new(SecureSerialInner {
                serial: inner,
                channel,
                tx_buf: [0u8; MAX_FRAME_PLAINTEXT],
                tx_len: 0,
                rx_buf: [0u8; MAX_FRAME_PLAINTEXT],
                rx_pos: 0,
                rx_len: 0,
            }),
        }
    }

    /// Consume the SecureSerial and return the underlying serial and channel.
    pub fn into_parts(self) -> (S, SecureChannel) {
        let inner = self.inner.into_inner();
        (inner.serial, inner.channel)
    }
}

impl<S: Serial> SecureSerialInner<S> {
    /// Flush the outgoing buffer as an encrypted frame.
    fn flush_tx(&mut self) {
        if self.tx_len == 0 {
            return;
        }
        let mut frame = [0u8; MAX_FRAME_CT + 2];
        if let Some(data) = self.channel.encrypt_frame(
            &self.tx_buf[..self.tx_len],
            &mut frame,
        ) {
            self.serial.write_bytes(data);
            self.serial.flush();
        }
        self.tx_len = 0;
    }

    /// Read and decrypt the next incoming frame.
    fn fill_rx(&mut self) {
        // Read 2-byte LE length header.
        let lo = self.serial.read_byte();
        let hi = self.serial.read_byte();
        let ct_len = u16::from_le_bytes([lo, hi]) as usize;

        if ct_len < TAG_LEN || ct_len > MAX_FRAME_CT {
            return; // invalid frame, skip
        }

        // Read ciphertext + tag.
        let mut ct = [0u8; MAX_FRAME_CT];
        for i in 0..ct_len {
            ct[i] = self.serial.read_byte();
        }

        // Decrypt in-place.
        if let Some(pt_len) = self.channel.decrypt_frame(&mut ct[..ct_len]) {
            self.rx_buf[..pt_len].copy_from_slice(&ct[..pt_len]);
            self.rx_pos = 0;
            self.rx_len = pt_len;
        }
    }
}

impl<S: Serial> Serial for SecureSerial<S> {
    fn read_byte(&self) -> u8 {
        let s = unsafe { &mut *self.inner.get() };
        if s.rx_pos >= s.rx_len {
            s.fill_rx();
        }
        if s.rx_pos < s.rx_len {
            let b = s.rx_buf[s.rx_pos];
            s.rx_pos += 1;
            b
        } else {
            0 // EOF / error
        }
    }

    fn write_byte(&self, byte: u8) {
        let s = unsafe { &mut *self.inner.get() };
        s.tx_buf[s.tx_len] = byte;
        s.tx_len += 1;
        // Flush on newline or when buffer is full.
        if byte == b'\n' || s.tx_len >= MAX_FRAME_PLAINTEXT {
            s.flush_tx();
        }
    }

    fn flush(&self) {
        let s = unsafe { &mut *self.inner.get() };
        s.flush_tx();
    }

    fn has_data(&self) -> bool {
        let s = unsafe { &mut *self.inner.get() };
        s.rx_pos < s.rx_len || s.serial.has_data()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Mode negotiation and file transfer
// ═══════════════════════════════════════════════════════════════════════════

/// Read the client's mode byte from the first encrypted frame after handshake.
/// Returns the mode byte, or `None` if the frame couldn't be read.
pub fn read_mode<S: Serial>(serial: &S, channel: &mut SecureChannel) -> Option<u8> {
    // Read a single encrypted frame.
    let lo = serial.read_byte();
    let hi = serial.read_byte();
    let ct_len = u16::from_le_bytes([lo, hi]) as usize;

    // Debug: check the length header.
    // Expected: 17 (1 plaintext + 16 tag).
    if ct_len < TAG_LEN || ct_len > MAX_FRAME_CT {
        return None;
    }
    let mut ct = [0u8; MAX_FRAME_CT];
    for i in 0..ct_len {
        ct[i] = serial.read_byte();
    }
    let pt_len = channel.decrypt_frame(&mut ct[..ct_len])?;
    if pt_len >= 1 {
        Some(ct[0])
    } else {
        None
    }
}

/// Read one raw encrypted frame from the serial. Returns decrypted data
/// in `buf` and the plaintext length.
fn recv_frame<S: Serial>(serial: &S, ch: &mut SecureChannel, buf: &mut [u8]) -> Option<usize> {
    let lo = serial.read_byte();
    let hi = serial.read_byte();
    let ct_len = u16::from_le_bytes([lo, hi]) as usize;
    if ct_len < TAG_LEN || ct_len > MAX_FRAME_CT {
        return None;
    }
    let mut ct = [0u8; MAX_FRAME_CT];
    for i in 0..ct_len {
        ct[i] = serial.read_byte();
    }
    let pt_len = ch.decrypt_frame(&mut ct[..ct_len])?;
    buf[..pt_len].copy_from_slice(&ct[..pt_len]);
    Some(pt_len)
}

/// Send an encrypted frame of raw data over serial.
fn send_frame<S: Serial>(serial: &S, ch: &mut SecureChannel, data: &[u8]) {
    let mut frame = [0u8; MAX_FRAME_CT + 2];
    if let Some(out) = ch.encrypt_frame(data, &mut frame) {
        serial.write_bytes(out);
        serial.flush();
    }
}

/// Handle a file upload (push) from the client.
///
/// Protocol:
///   1. Auth already done by caller (or skipped).
///   2. Read header frame: `[path_len:u16 LE][path bytes][file_size:u32 LE]`
///   3. Read file data frames until `file_size` bytes received.
///   4. Write to VFS via `write_fn`.
///   5. Send status frame: `[0x00]` OK or `[0x01][error msg]`.
pub fn handle_push<S: Serial>(
    serial: &S,
    ch: &mut SecureChannel,
    write_fn: fn(&str, &[u8], bool) -> bool,
) {
    // Read header frame.
    let mut hdr = [0u8; MAX_FRAME_PLAINTEXT];
    let hdr_len = match recv_frame(serial, ch, &mut hdr) {
        Some(n) => n,
        None => {
            send_frame(serial, ch, &[STATUS_ERR, b'h', b'd', b'r']);
            return;
        }
    };

    if hdr_len < 6 {
        // Need at least 2 (path_len) + 0 (path) + 4 (file_size).
        send_frame(serial, ch, &[STATUS_ERR, b'h', b'd', b'r']);
        return;
    }

    let path_len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
    if path_len == 0 || 2 + path_len + 4 > hdr_len {
        send_frame(serial, ch, &[STATUS_ERR, b'p', b'a', b't', b'h']);
        return;
    }

    let path_bytes = &hdr[2..2 + path_len];
    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => {
            send_frame(serial, ch, &[STATUS_ERR, b'u', b't', b'f', b'8']);
            return;
        }
    };

    let size_off = 2 + path_len;
    let file_size = u32::from_le_bytes([
        hdr[size_off], hdr[size_off + 1], hdr[size_off + 2], hdr[size_off + 3],
    ]) as usize;

    // Read file data.  Accumulate in a stack buffer.
    // For embedded targets, limit to 8 KiB max file size.
    const MAX_FILE: usize = 8192;
    if file_size > MAX_FILE {
        send_frame(serial, ch, &[STATUS_ERR, b'b', b'i', b'g']);
        return;
    }

    let mut file_buf = [0u8; MAX_FILE];
    let mut received = 0usize;

    while received < file_size {
        let mut chunk = [0u8; MAX_FRAME_PLAINTEXT];
        let n = match recv_frame(serial, ch, &mut chunk) {
            Some(n) => n,
            None => {
                send_frame(serial, ch, &[STATUS_ERR, b'i', b'o']);
                return;
            }
        };
        let take = if received + n > file_size { file_size - received } else { n };
        file_buf[received..received + take].copy_from_slice(&chunk[..take]);
        received += take;
    }

    // Write to VFS.
    if write_fn(path, &file_buf[..file_size], false) {
        send_frame(serial, ch, &[STATUS_OK]);
    } else {
        send_frame(serial, ch, &[STATUS_ERR, b'v', b'f', b's']);
    }
}

/// Handle a file download (pull) from the client.
///
/// Protocol:
///   1. Auth already done by caller (or skipped).
///   2. Read request frame: `[path_len:u16 LE][path bytes]`
///   3. Read from VFS via `read_fn`.
///   4. Send response: `[status:u8][file_size:u32 LE]`
///   5. Send file data frames.
pub fn handle_pull<S: Serial>(
    serial: &S,
    ch: &mut SecureChannel,
    read_fn: fn(&str, &mut [u8]) -> usize,
) {
    // Read request frame.
    let mut req = [0u8; MAX_FRAME_PLAINTEXT];
    let req_len = match recv_frame(serial, ch, &mut req) {
        Some(n) => n,
        None => {
            send_frame(serial, ch, &[STATUS_ERR, b'h', b'd', b'r']);
            return;
        }
    };

    if req_len < 2 {
        send_frame(serial, ch, &[STATUS_ERR, b'h', b'd', b'r']);
        return;
    }

    let path_len = u16::from_le_bytes([req[0], req[1]]) as usize;
    if path_len == 0 || 2 + path_len > req_len {
        send_frame(serial, ch, &[STATUS_ERR, b'p', b'a', b't', b'h']);
        return;
    }

    let path_bytes = &req[2..2 + path_len];
    let path = match core::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => {
            send_frame(serial, ch, &[STATUS_ERR, b'u', b't', b'f', b'8']);
            return;
        }
    };

    // Read file from VFS.
    const MAX_FILE: usize = 8192;
    let mut file_buf = [0u8; MAX_FILE];
    let file_size = read_fn(path, &mut file_buf);

    if file_size == 0 {
        // File not found or empty.
        send_frame(serial, ch, &[STATUS_ERR, b'n', b'o', b't', b'f']);
        return;
    }

    // Send response header: [status][file_size:u32 LE].
    let size_bytes = (file_size as u32).to_le_bytes();
    let resp = [STATUS_OK, size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]];
    send_frame(serial, ch, &resp);

    // Send file data in chunks.
    let mut sent = 0usize;
    while sent < file_size {
        let end = if sent + MAX_FRAME_PLAINTEXT > file_size {
            file_size
        } else {
            sent + MAX_FRAME_PLAINTEXT
        };
        send_frame(serial, ch, &file_buf[sent..end]);
        sent = end;
    }
}
