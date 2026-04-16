//! SSH-2 Binary Packet Protocol (RFC 4253 §6).
//!
//! Handles packet framing, encryption, and MAC for the SSH connection.
//! Before key exchange completes, packets are unencrypted.
//! After NEWKEYS, packets use chacha20-poly1305@openssh.com.

use arch::Serial;
use crypto::Aead;
use crypto::chacha20::{ChaCha20Poly1305, chacha20_xor};
use crypto::zeroize;
use crate::{MAX_PACKET, MAX_PAYLOAD, get_u32, put_u32};

/// Transport state — tracks encryption keys and sequence numbers.
pub struct Transport {
    /// Client → server sequence number.
    rx_seq: u32,
    /// Server → client sequence number.
    tx_seq: u32,
    /// Encryption keys (set after NEWKEYS).
    keys: Option<TransportKeys>,
}

/// Session keys derived from key exchange.
pub struct TransportKeys {
    /// Client-to-server main key (ChaCha20-Poly1305 K_1).
    pub rx_key: [u8; 32],
    /// Client-to-server header key (ChaCha20 K_2 for length encryption).
    pub rx_header_key: [u8; 32],
    /// Server-to-client main key.
    pub tx_key: [u8; 32],
    /// Server-to-client header key.
    pub tx_header_key: [u8; 32],
}

impl Transport {
    pub fn new() -> Self {
        Self {
            rx_seq: 0,
            tx_seq: 0,
            keys: None,
        }
    }

    /// Install session keys after successful key exchange + NEWKEYS.
    pub fn set_keys(&mut self, keys: TransportKeys) {
        self.keys = Some(keys);
    }

    /// Read one SSH packet from the serial device.
    ///
    /// Returns the number of payload bytes written into `payload_buf`.
    /// Returns 0 on connection error (EOF / bad packet).
    pub fn read_packet<S: Serial>(
        &mut self,
        serial: &S,
        payload_buf: &mut [u8; MAX_PAYLOAD],
    ) -> usize {
        match &self.keys {
            None => self.read_packet_plain(serial, payload_buf),
            Some(_) => self.read_packet_encrypted(serial, payload_buf),
        }
    }

    /// Write one SSH packet to the serial device.
    pub fn write_packet<S: Serial>(
        &mut self,
        serial: &S,
        payload: &[u8],
    ) {
        match &self.keys {
            None => self.write_packet_plain(serial, payload),
            Some(_) => self.write_packet_encrypted(serial, payload),
        }
    }

    // ── Plaintext (pre-NEWKEYS) ─────────────────────────────────────────

    fn read_packet_plain<S: Serial>(
        &mut self,
        serial: &S,
        payload_buf: &mut [u8; MAX_PAYLOAD],
    ) -> usize {
        // Read 4-byte packet_length
        let mut hdr = [0u8; 4];
        for i in 0..4 {
            hdr[i] = serial.read_byte();
        }
        let packet_length = get_u32(&hdr) as usize;

        if packet_length < 2 || packet_length > MAX_PACKET - 4 {
            return 0;
        }

        // Read padding_length
        let padding_length = serial.read_byte() as usize;
        let payload_length = packet_length - padding_length - 1;

        if payload_length > MAX_PAYLOAD {
            return 0;
        }

        // Read payload
        for i in 0..payload_length {
            payload_buf[i] = serial.read_byte();
        }

        // Read and discard padding
        for _ in 0..padding_length {
            let _ = serial.read_byte();
        }

        self.rx_seq = self.rx_seq.wrapping_add(1);
        payload_length
    }

    fn write_packet_plain<S: Serial>(
        &mut self,
        serial: &S,
        payload: &[u8],
    ) {
        // Padding must be at least 4 bytes, total (payload + padding + 1)
        // must be multiple of 8 (or cipher block size).
        let block_size = 8;
        let min_packet = 1 + payload.len() + 4; // padding_len + payload + min_padding
        let padding_len = block_size - (min_packet % block_size);
        let padding_len = if padding_len < 4 { padding_len + block_size } else { padding_len };
        let packet_length = 1 + payload.len() + padding_len;

        // Write packet_length (u32 BE)
        let mut hdr = [0u8; 4];
        put_u32(&mut hdr, packet_length as u32);
        serial.write_bytes(&hdr);

        // Write padding_length (u8)
        serial.write_byte(padding_len as u8);

        // Write payload
        serial.write_bytes(payload);

        // Write padding (zeros — before NEWKEYS, no need for random)
        for _ in 0..padding_len {
            serial.write_byte(0);
        }

        self.tx_seq = self.tx_seq.wrapping_add(1);
    }

    // ── Encrypted (chacha20-poly1305@openssh.com) ───────────────────────

    fn read_packet_encrypted<S: Serial>(
        &mut self,
        serial: &S,
        payload_buf: &mut [u8; MAX_PAYLOAD],
    ) -> usize {
        let keys = self.keys.as_ref().unwrap();

        // Read 4-byte encrypted length
        let mut enc_len = [0u8; 4];
        for i in 0..4 {
            enc_len[i] = serial.read_byte();
        }

        // Decrypt length using header key (K_2, nonce = seqno, counter = 0)
        let mut nonce = [0u8; 12];
        put_u32(&mut nonce[8..12], self.rx_seq);
        chacha20_xor(&keys.rx_header_key, 0, &nonce, &mut enc_len);

        let packet_length = get_u32(&enc_len) as usize;
        if packet_length < 2 || packet_length > MAX_PACKET - 4 {
            return 0;
        }

        // Read the rest of the packet + 16-byte Poly1305 tag
        let total = packet_length + 16;
        let mut buf = [0u8; MAX_PACKET];
        for i in 0..total {
            if i >= buf.len() { return 0; }
            buf[i] = serial.read_byte();
        }

        // Verify Poly1305 tag.
        // AAD = encrypted length bytes (4 bytes).
        // Ciphertext = buf[..packet_length].
        // Tag = buf[packet_length..packet_length+16].
        let aead = ChaCha20Poly1305::new(&keys.rx_key);
        let mut verify_buf = [0u8; MAX_PACKET];
        // Pack: enc_len || ciphertext || tag for AEAD open
        verify_buf[..4].copy_from_slice(&enc_len);

        // Actually, chacha20-poly1305@openssh.com uses a different scheme:
        // - K_2 encrypts the 4-byte length field (nonce = seqno, counter = 0)
        // - K_1 encrypts the payload (nonce = seqno, counter = 0, skip block 0 for poly key)
        // - Poly1305 key = first 32 bytes of K_1 stream (counter = 0)
        // - Poly1305 authenticates: encrypted_length (4 bytes) || ciphertext
        // - Tag is appended after ciphertext

        // Generate Poly1305 key from K_1 stream block 0
        let mut poly_key = [0u8; 32];
        let poly_block = crypto::chacha20::chacha20_block(&keys.rx_key, 0, &nonce);
        poly_key.copy_from_slice(&poly_block[..32]);

        // Verify tag
        let tag = &buf[packet_length..packet_length + 16];
        // Compute Poly1305 over enc_len || ciphertext
        let mut auth_data = [0u8; MAX_PACKET];
        let mut re_enc_len = enc_len;
        // Re-encrypt length to get original encrypted form for auth
        chacha20_xor(&keys.rx_header_key, 0, &nonce, &mut re_enc_len);
        auth_data[..4].copy_from_slice(&re_enc_len);
        auth_data[4..4 + packet_length].copy_from_slice(&buf[..packet_length]);
        let auth_len = 4 + packet_length;

        // Poly1305 verify
        let mut computed_tag = [0u8; 16];
        poly1305_mac(&poly_key, &auth_data[..auth_len], &mut computed_tag);
        
        let mut diff: u8 = 0;
        for i in 0..16 {
            diff |= computed_tag[i] ^ tag[i];
        }
        if diff != 0 {
            return 0; // MAC failure
        }

        // Decrypt payload (counter starts at 1 to skip poly key block)
        chacha20_xor(&keys.rx_key, 1, &nonce, &mut buf[..packet_length]);

        // Parse: padding_length || payload || padding
        let padding_length = buf[0] as usize;
        let payload_length = packet_length - padding_length - 1;
        if payload_length > MAX_PAYLOAD {
            return 0;
        }
        payload_buf[..payload_length].copy_from_slice(&buf[1..1 + payload_length]);

        zeroize(&mut poly_key);
        self.rx_seq = self.rx_seq.wrapping_add(1);
        payload_length
    }

    fn write_packet_encrypted<S: Serial>(
        &mut self,
        serial: &S,
        payload: &[u8],
    ) {
        let keys = self.keys.as_ref().unwrap();

        // Compute padding
        let block_size = 8;
        let min_packet = 1 + payload.len() + 4;
        let padding_len = block_size - (min_packet % block_size);
        let padding_len = if padding_len < 4 { padding_len + block_size } else { padding_len };
        let packet_length = 1 + payload.len() + padding_len;

        let mut nonce = [0u8; 12];
        put_u32(&mut nonce[8..12], self.tx_seq);

        // Build plaintext: padding_len || payload || padding
        let mut plain = [0u8; MAX_PACKET];
        plain[0] = padding_len as u8;
        plain[1..1 + payload.len()].copy_from_slice(payload);
        // Padding bytes: zeros (could be random for extra security)
        for i in 0..padding_len {
            plain[1 + payload.len() + i] = 0;
        }

        // Encrypt packet_length with header key K_2
        let mut enc_len = [0u8; 4];
        put_u32(&mut enc_len, packet_length as u32);
        chacha20_xor(&keys.tx_header_key, 0, &nonce, &mut enc_len);

        // Generate Poly1305 key from K_1 block 0
        let mut poly_key = [0u8; 32];
        let poly_block = crypto::chacha20::chacha20_block(&keys.tx_key, 0, &nonce);
        poly_key.copy_from_slice(&poly_block[..32]);

        // Encrypt payload with K_1 (counter starts at 1)
        chacha20_xor(&keys.tx_key, 1, &nonce, &mut plain[..packet_length]);

        // Compute Poly1305 tag over enc_len || ciphertext
        let mut auth_data = [0u8; MAX_PACKET];
        auth_data[..4].copy_from_slice(&enc_len);
        auth_data[4..4 + packet_length].copy_from_slice(&plain[..packet_length]);
        let auth_len = 4 + packet_length;

        let mut tag = [0u8; 16];
        poly1305_mac(&poly_key, &auth_data[..auth_len], &mut tag);

        // Send: enc_len || ciphertext || tag
        serial.write_bytes(&enc_len);
        serial.write_bytes(&plain[..packet_length]);
        serial.write_bytes(&tag);

        zeroize(&mut poly_key);
        self.tx_seq = self.tx_seq.wrapping_add(1);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Poly1305 standalone MAC (for SSH chacha20-poly1305 scheme)
// ═══════════════════════════════════════════════════════════════════════════

/// Compute a Poly1305 MAC using the given one-time key.
fn poly1305_mac(key: &[u8; 32], data: &[u8], tag: &mut [u8; 16]) {
    // Clamp r
    let mut r = [0u32; 5];
    let t0 = u32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let t1 = u32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    let t2 = u32::from_le_bytes([key[8], key[9], key[10], key[11]]);
    let t3 = u32::from_le_bytes([key[12], key[13], key[14], key[15]]);

    r[0] = t0 & 0x3ffffff;
    r[1] = ((t0 >> 26) | (t1 << 6)) & 0x3ffff03;
    r[2] = ((t1 >> 20) | (t2 << 12)) & 0x3ffc0ff;
    r[3] = ((t2 >> 14) | (t3 << 18)) & 0x3f03fff;
    r[4] = (t3 >> 8) & 0x00fffff;

    let s0 = u32::from_le_bytes([key[16], key[17], key[18], key[19]]);
    let s1 = u32::from_le_bytes([key[20], key[21], key[22], key[23]]);
    let s2 = u32::from_le_bytes([key[24], key[25], key[26], key[27]]);
    let s3 = u32::from_le_bytes([key[28], key[29], key[30], key[31]]);

    let mut h = [0u32; 5];
    let mut pos = 0;

    while pos < data.len() {
        let remaining = data.len() - pos;
        let chunk = remaining.min(16);
        let mut n = [0u8; 17];
        n[..chunk].copy_from_slice(&data[pos..pos + chunk]);
        n[chunk] = 1; // hibit

        // h += block
        let t0 = u32::from_le_bytes([n[0], n[1], n[2], n[3]]);
        let t1 = u32::from_le_bytes([n[4], n[5], n[6], n[7]]);
        let t2 = u32::from_le_bytes([n[8], n[9], n[10], n[11]]);
        let t3 = u32::from_le_bytes([n[12], n[13], n[14], n[15]]);
        let t4 = n[16] as u32;

        h[0] = h[0].wrapping_add(t0 & 0x3ffffff);
        h[1] = h[1].wrapping_add(((t0 >> 26) | (t1 << 6)) & 0x3ffffff);
        h[2] = h[2].wrapping_add(((t1 >> 20) | (t2 << 12)) & 0x3ffffff);
        h[3] = h[3].wrapping_add(((t2 >> 14) | (t3 << 18)) & 0x3ffffff);
        h[4] = h[4].wrapping_add((t3 >> 8) | (t4 << 24));

        // h *= r (mod 2^130 - 5)
        let r5 = [r[1] * 5, r[2] * 5, r[3] * 5, r[4] * 5];

        let d0 = (h[0] as u64) * (r[0] as u64)
            + (h[1] as u64) * (r5[3] as u64)
            + (h[2] as u64) * (r5[2] as u64)
            + (h[3] as u64) * (r5[1] as u64)
            + (h[4] as u64) * (r5[0] as u64);
        let d1 = (h[0] as u64) * (r[1] as u64)
            + (h[1] as u64) * (r[0] as u64)
            + (h[2] as u64) * (r5[3] as u64)
            + (h[3] as u64) * (r5[2] as u64)
            + (h[4] as u64) * (r5[1] as u64);
        let d2 = (h[0] as u64) * (r[2] as u64)
            + (h[1] as u64) * (r[1] as u64)
            + (h[2] as u64) * (r[0] as u64)
            + (h[3] as u64) * (r5[3] as u64)
            + (h[4] as u64) * (r5[2] as u64);
        let d3 = (h[0] as u64) * (r[3] as u64)
            + (h[1] as u64) * (r[2] as u64)
            + (h[2] as u64) * (r[1] as u64)
            + (h[3] as u64) * (r[0] as u64)
            + (h[4] as u64) * (r5[3] as u64);
        let d4 = (h[0] as u64) * (r[4] as u64)
            + (h[1] as u64) * (r[3] as u64)
            + (h[2] as u64) * (r[2] as u64)
            + (h[3] as u64) * (r[1] as u64)
            + (h[4] as u64) * (r[0] as u64);

        let mut c: u32;
        h[0] = d0 as u32 & 0x3ffffff;
        c = (d0 >> 26) as u32;
        let d1 = d1 + c as u64; h[1] = d1 as u32 & 0x3ffffff;
        c = (d1 >> 26) as u32;
        let d2 = d2 + c as u64; h[2] = d2 as u32 & 0x3ffffff;
        c = (d2 >> 26) as u32;
        let d3 = d3 + c as u64; h[3] = d3 as u32 & 0x3ffffff;
        c = (d3 >> 26) as u32;
        let d4 = d4 + c as u64; h[4] = d4 as u32 & 0x3ffffff;
        c = (d4 >> 26) as u32;
        h[0] = h[0].wrapping_add(c * 5);
        c = h[0] >> 26;
        h[0] &= 0x3ffffff;
        h[1] = h[1].wrapping_add(c);

        pos += 16;
    }

    // Final carry and freeze
    let mut c: u32;
    c = h[1] >> 26; h[1] &= 0x3ffffff; h[2] = h[2].wrapping_add(c);
    c = h[2] >> 26; h[2] &= 0x3ffffff; h[3] = h[3].wrapping_add(c);
    c = h[3] >> 26; h[3] &= 0x3ffffff; h[4] = h[4].wrapping_add(c);
    c = h[4] >> 26; h[4] &= 0x3ffffff; h[0] = h[0].wrapping_add(c * 5);
    c = h[0] >> 26; h[0] &= 0x3ffffff; h[1] = h[1].wrapping_add(c);

    // Compute h + -p
    let mut g = [0u32; 5];
    c = h[0].wrapping_add(5); g[0] = c & 0x3ffffff; c >>= 26;
    c = h[1].wrapping_add(c); g[1] = c & 0x3ffffff; c >>= 26;
    c = h[2].wrapping_add(c); g[2] = c & 0x3ffffff; c >>= 26;
    c = h[3].wrapping_add(c); g[3] = c & 0x3ffffff; c >>= 26;
    c = h[4].wrapping_add(c).wrapping_sub(1 << 26); g[4] = c & 0x3ffffff;

    // Select h or g
    let mask = (c >> 31).wrapping_sub(1); // all-ones if g >= 0 (no borrow)
    for i in 0..5 {
        h[i] = (h[i] & !mask) | (g[i] & mask);
    }

    // h = h + s
    let mut f: u64;
    f = h[0] as u64 + s0 as u64; h[0] = f as u32;
    f = h[1] as u64 + s1 as u64 + (f >> 32); h[1] = f as u32;
    f = h[2] as u64 + s2 as u64 + (f >> 32); h[2] = f as u32;
    f = h[3] as u64 + s3 as u64 + (f >> 32); h[3] = f as u32;

    // Output
    let h0 = ((h[0]) | (h[1] << 26)) as u32;
    let h1 = ((h[1] >> 6) | (h[2] << 20)) as u32;
    let h2 = ((h[2] >> 12) | (h[3] << 14)) as u32;
    let h3 = ((h[3] >> 18) | (h[4] << 8)) as u32;

    tag[0..4].copy_from_slice(&h0.to_le_bytes());
    tag[4..8].copy_from_slice(&h1.to_le_bytes());
    tag[8..12].copy_from_slice(&h2.to_le_bytes());
    tag[12..16].copy_from_slice(&h3.to_le_bytes());
}
