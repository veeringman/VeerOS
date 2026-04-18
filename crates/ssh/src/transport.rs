//! SSH-2 Binary Packet Protocol (RFC 4253 §6).
//!
//! Handles packet framing, encryption, and MAC for the SSH connection.
//! Before key exchange completes, packets are unencrypted.
//! After NEWKEYS, packets use chacha20-poly1305@openssh.com.

use arch::Serial;
use crypto::Aead;
use crypto::chacha20::{chacha20_block, chacha20_xor};
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
        // RFC 4253 §6: total of (packet_length‖padding_length‖payload‖padding)
        // must be a multiple of block_size(8).  That total = 4 + packet_length,
        // so we align (4 + 1 + payload_len) and derive padding from that.
        let block_size = 8;
        let unpadded = 4 + 1 + payload.len(); // include 4-byte length field
        let padding_len = block_size - (unpadded % block_size);
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

    fn openssh_chacha_nonce(seq: u32) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        // OpenSSH stores the sequence number as a big-endian u64 IV via
        // POKE_U64(seqbuf, seqnr), then chacha_ivsetup() loads it as:
        //   state[14] = U8TO32_LITTLE(seqbuf[0..4])  — always 0 for u32 seq
        //   state[15] = U8TO32_LITTLE(seqbuf[4..8])  — seq in BE bytes
        //
        // Our RFC 8439 chacha20_block maps:
        //   state[13] = le32(nonce[0..4])
        //   state[14] = le32(nonce[4..8])
        //   state[15] = le32(nonce[8..12])
        //
        // state[13] maps to DJB counter_hi (0 since we pass counter as arg).
        // Put the BE seq bytes at nonce[8..12] so le32 produces the same
        // state[15] value as OpenSSH.
        nonce[8..12].copy_from_slice(&seq.to_be_bytes());
        nonce
    }

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
        let orig_enc_len = enc_len;

        // OpenSSH chacha20-poly1305 uses the packet sequence number as the
        // legacy 64-bit ChaCha nonce with counter 0 for the length field.
        let nonce = Self::openssh_chacha_nonce(self.rx_seq);
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
        // Generate Poly1305 key from K_1 stream block 0
        let mut poly_key = [0u8; 32];
        let poly_block = chacha20_block(&keys.rx_key, 0, &nonce);
        poly_key.copy_from_slice(&poly_block[..32]);

        // Verify tag
        let tag = &buf[packet_length..packet_length + 16];
        let mut auth_data = [0u8; MAX_PACKET];
        auth_data[..4].copy_from_slice(&orig_enc_len);
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

        // chacha20-poly1305: the 4-byte length is encrypted separately and
        // NOT included in the block alignment.  Only the encrypted body
        // (padding_length‖payload‖padding) = packet_length must be % 8 == 0.
        let block_size = 8;
        let unpadded = 1 + payload.len();
        let padding_len = block_size - (unpadded % block_size);
        let padding_len = if padding_len < 4 { padding_len + block_size } else { padding_len };
        let packet_length = 1 + payload.len() + padding_len;

        let nonce = Self::openssh_chacha_nonce(self.tx_seq);

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
        let poly_block = chacha20_block(&keys.tx_key, 0, &nonce);
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
    let le32 = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);

    let mut rb = [0u8; 16];
    rb.copy_from_slice(&key[..16]);
    rb[3] &= 0x0f;
    rb[7] &= 0x0f;
    rb[11] &= 0x0f;
    rb[15] &= 0x0f;
    rb[4] &= 0xfc;
    rb[8] &= 0xfc;
    rb[12] &= 0xfc;

    let t0 = le32(&rb[0..]);
    let t1 = le32(&rb[4..]);
    let t2 = le32(&rb[8..]);
    let t3 = le32(&rb[12..]);

    let r = [
        t0 & 0x3ff_ffff,
        ((t0 >> 26) | (t1 << 6)) & 0x3ff_ffff,
        ((t1 >> 20) | (t2 << 12)) & 0x3ff_ffff,
        ((t2 >> 14) | (t3 << 18)) & 0x3ff_ffff,
        t3 >> 8,
    ];
    let s = [le32(&key[16..]), le32(&key[20..]), le32(&key[24..]), le32(&key[28..])];

    let mut h = [0u32; 5];
    let mut off = 0;
    while off < data.len() {
        let take = (data.len() - off).min(16);
        let msg = &data[off..off + take];
        let mut n = [0u8; 17];
        n[..take].copy_from_slice(msg);
        n[take] = 1;

        let t0 = le32(&n[0..]);
        let t1 = le32(&n[4..]);
        let t2 = le32(&n[8..]);
        let t3 = le32(&n[12..]);
        let t4 = n[16] as u32;

        h[0] = h[0].wrapping_add(t0 & 0x3ff_ffff);
        h[1] = h[1].wrapping_add(((t0 >> 26) | (t1 << 6)) & 0x3ff_ffff);
        h[2] = h[2].wrapping_add(((t1 >> 20) | (t2 << 12)) & 0x3ff_ffff);
        h[3] = h[3].wrapping_add(((t2 >> 14) | (t3 << 18)) & 0x3ff_ffff);
        h[4] = h[4].wrapping_add((t3 >> 8) | (t4 << 24));

        let (r0, r1, r2, r3, r4) = (
            r[0] as u64, r[1] as u64, r[2] as u64, r[3] as u64, r[4] as u64,
        );
        let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
        let (h0, h1, h2, h3, h4) = (
            h[0] as u64, h[1] as u64, h[2] as u64, h[3] as u64, h[4] as u64,
        );

        let d0 = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1;
        let d1 = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2;
        let d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3;
        let d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4;
        let d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;

        let c0 = d0 >> 26;
        let mut h0 = (d0 & 0x3ff_ffff) as u32;
        let d1 = d1 + c0;
        let c1 = d1 >> 26;
        let h1 = (d1 & 0x3ff_ffff) as u32;
        let d2 = d2 + c1;
        let c2 = d2 >> 26;
        let h2 = (d2 & 0x3ff_ffff) as u32;
        let d3 = d3 + c2;
        let c3 = d3 >> 26;
        let h3 = (d3 & 0x3ff_ffff) as u32;
        let d4 = d4 + c3;
        let c4 = d4 >> 26;
        let h4 = (d4 & 0x3ff_ffff) as u32;

        h0 = h0.wrapping_add((c4 as u32) * 5);
        let carry = h0 >> 26;
        h0 &= 0x3ff_ffff;
        let h1 = h1.wrapping_add(carry);

        h = [h0, h1, h2, h3, h4];
        off += take;
    }

    let mut h0 = h[0];
    let mut h1 = h[1];
    let mut h2 = h[2];
    let mut h3 = h[3];
    let mut h4 = h[4];

    let c = h1 >> 26;
    h1 &= 0x3ff_ffff;
    h2 += c;
    let c = h2 >> 26;
    h2 &= 0x3ff_ffff;
    h3 += c;
    let c = h3 >> 26;
    h3 &= 0x3ff_ffff;
    h4 += c;
    let c = h4 >> 26;
    h4 &= 0x3ff_ffff;
    h0 += c * 5;
    let c = h0 >> 26;
    h0 &= 0x3ff_ffff;
    h1 += c;

    let mut g0 = h0.wrapping_add(5);
    let c = g0 >> 26;
    g0 &= 0x3ff_ffff;
    let mut g1 = h1.wrapping_add(c);
    let c = g1 >> 26;
    g1 &= 0x3ff_ffff;
    let mut g2 = h2.wrapping_add(c);
    let c = g2 >> 26;
    g2 &= 0x3ff_ffff;
    let mut g3 = h3.wrapping_add(c);
    let c = g3 >> 26;
    g3 &= 0x3ff_ffff;
    let g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);

    let mask = (g4 >> 31).wrapping_sub(1);
    h0 = (h0 & !mask) | (g0 & mask);
    h1 = (h1 & !mask) | (g1 & mask);
    h2 = (h2 & !mask) | (g2 & mask);
    h3 = (h3 & !mask) | (g3 & mask);
    h4 = (h4 & !mask) | (g4 & mask);

    let h_val = (h0 as u128)
        | ((h1 as u128) << 26)
        | ((h2 as u128) << 52)
        | ((h3 as u128) << 78)
        | ((h4 as u128) << 104);
    let s_val = (s[0] as u128)
        | ((s[1] as u128) << 32)
        | ((s[2] as u128) << 64)
        | ((s[3] as u128) << 96);
    let tag_val = h_val.wrapping_add(s_val);

    tag[..8].copy_from_slice(&(tag_val as u64).to_le_bytes());
    tag[8..].copy_from_slice(&((tag_val >> 64) as u64).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::poly1305_mac;

    #[test]
    fn poly1305_rfc8439_vector() {
        let key = [
            0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33,
            0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5, 0x06, 0xa8,
            0x01, 0x03, 0x80, 0x8a, 0xfb, 0x0d, 0xb2, 0xfd,
            0x4a, 0xbf, 0xf6, 0xaf, 0x41, 0x49, 0xf5, 0x1b,
        ];
        let msg = b"Cryptographic Forum Research Group";
        let expected = [
            0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6,
            0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01, 0x27, 0xa9,
        ];

        let mut tag = [0u8; 16];
        poly1305_mac(&key, msg, &mut tag);
        assert_eq!(tag, expected);
    }
}
