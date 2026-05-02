//! SSH-2 Client — minimal implementation.
//!
//! Drives the SSH client lifecycle:
//!   1. Version exchange
//!   2. Key exchange (curve25519-sha256) — client side
//!   3. User authentication (password)
//!   4. Channel open + shell/exec request
//!   5. Interactive session (bridged via `SshClientBridge`)

use crate::channel::{ChannelManager, INITIAL_WINDOW, MAX_CHANNEL_PACKET};
use crate::kex::build_kexinit;
use crate::transport::{Transport, TransportKeys};
use crate::{get_string, get_u32, put_mpint, put_string, put_u32, MAX_PAYLOAD, VERSION_STRING};
use arch::Serial;
use crypto::ed25519::ed25519_verify;
use crypto::sha256::Sha256;
use crypto::x25519::{x25519_diffie_hellman, x25519_keypair};
use crypto::CryptoRng;
use crypto::Hash;

/// SSH client configuration.
pub struct SshClientConfig<'a> {
    /// Username to authenticate as.
    pub username: &'a [u8],
    /// Password for authentication.
    pub password: &'a [u8],
}

/// Run the SSH-2 client handshake and return a bridge for interactive I/O.
///
/// Returns `None` if the connection fails at any point.
pub fn run_ssh_client<S: Serial>(
    serial: &S,
    config: &SshClientConfig,
    rng: &mut dyn CryptoRng,
) -> Option<SshClientBridge> {
    run_ssh_client_with_trace(serial, config, rng, |_| {})
}

/// Run the SSH client handshake, emitting trace messages.
pub fn run_ssh_client_with_trace<S: Serial, F: FnMut(&str)>(
    serial: &S,
    config: &SshClientConfig,
    rng: &mut dyn CryptoRng,
    mut trace: F,
) -> Option<SshClientBridge> {
    let mut transport = Transport::new();
    let mut payload = [0u8; MAX_PAYLOAD];

    // Generate client ephemeral key pair for ECDH
    let (eph_sk, eph_pk) = x25519_keypair(rng);

    // ── Step 1: Version Exchange ─────────────────────────────────────
    // Send our version string
    serial.write_bytes(VERSION_STRING);
    serial.write_bytes(b"\r\n");
    trace("sent-version");

    // Read server's version string
    let mut server_version = [0u8; 256];
    let mut sv_len = 0;
    loop {
        let b = serial.read_byte();
        if b == 0x04 {
            return None;
        }
        if b == b'\n' {
            break;
        }
        if b == b'\r' {
            continue;
        }
        if sv_len < server_version.len() {
            server_version[sv_len] = b;
            sv_len += 1;
        }
    }
    if sv_len < 8 || &server_version[..8] != b"SSH-2.0-" {
        return None;
    }
    trace("got-server-version");

    // ── Step 2: Key Exchange Init ────────────────────────────────────
    // Send our (client) KEXINIT
    let mut cookie = [0u8; 16];
    rng.fill_bytes(&mut cookie);
    let kexinit_len = build_kexinit(&mut payload, &cookie);

    // Save client KEXINIT for exchange hash
    let mut client_kexinit = [0u8; MAX_PAYLOAD];
    client_kexinit[..kexinit_len].copy_from_slice(&payload[..kexinit_len]);
    let client_kexinit_len = kexinit_len;

    transport.write_packet(serial, &payload[..kexinit_len]);
    trace("sent-kexinit");

    // Read server's KEXINIT
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 20 {
        return None;
    }

    let mut server_kexinit = [0u8; MAX_PAYLOAD];
    server_kexinit[..n].copy_from_slice(&payload[..n]);
    let server_kexinit_len = n;
    trace("got-server-kexinit");

    // ── Step 3: ECDH Key Exchange (client side) ──────────────────────
    // Send KEX_ECDH_INIT with our ephemeral public key Q_C
    {
        let mut init = [0u8; 64];
        init[0] = 30; // SSH_MSG_KEX_ECDH_INIT
        let slen = put_string(&mut init[1..], &eph_pk);
        transport.write_packet(serial, &init[..1 + slen]);
    }
    trace("sent-ecdh-init");

    // Read server's KEX_ECDH_REPLY (msg type 31)
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 31 {
        return None;
    }
    trace("got-ecdh-reply");

    // Parse KEX_ECDH_REPLY:
    //   string  K_S (host key blob)
    //   string  Q_S (server ephemeral public key)
    //   string  signature of H
    let mut off = 1;

    // K_S: host key blob = string("ssh-ed25519") + string(pubkey)
    let (host_key_blob, consumed) = get_string(&payload[off..]);
    off += consumed;

    // Parse host key blob to extract public key
    let mut hk_off = 0;
    let (key_type, hk_consumed) = get_string(host_key_blob);
    hk_off += hk_consumed;
    if key_type != b"ssh-ed25519" {
        return None;
    }
    let (host_pubkey_bytes, _) = get_string(&host_key_blob[hk_off..]);
    if host_pubkey_bytes.len() != 32 {
        return None;
    }
    let mut host_pubkey = [0u8; 32];
    host_pubkey.copy_from_slice(host_pubkey_bytes);

    // Q_S: server ephemeral
    let (q_s_bytes, consumed) = get_string(&payload[off..]);
    off += consumed;
    if q_s_bytes.len() != 32 {
        return None;
    }
    let mut q_s = [0u8; 32];
    q_s.copy_from_slice(q_s_bytes);

    // Signature blob = string("ssh-ed25519") + string(sig_bytes)
    let (sig_blob, _consumed) = get_string(&payload[off..]);
    let mut sig_off = 0;
    let (sig_type, sig_consumed) = get_string(sig_blob);
    sig_off += sig_consumed;
    if sig_type != b"ssh-ed25519" {
        return None;
    }
    let (sig_bytes, _) = get_string(&sig_blob[sig_off..]);
    if sig_bytes.len() != 64 {
        return None;
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(sig_bytes);

    // Compute shared secret K = X25519(client_sk, server_pk)
    let shared_secret = match x25519_diffie_hellman(&eph_sk, &q_s) {
        Ok(ss) => ss,
        Err(_) => return None,
    };

    // Compute exchange hash H
    let h = compute_exchange_hash(
        VERSION_STRING,                        // V_C (our version)
        &server_version[..sv_len],             // V_S
        &client_kexinit[..client_kexinit_len], // I_C
        &server_kexinit[..server_kexinit_len], // I_S
        &host_pubkey,                          // K_S (raw pubkey)
        &eph_pk,                               // Q_C (our ephemeral)
        &q_s,                                  // Q_S (server ephemeral)
        &shared_secret,                        // K
    );

    // Verify server's signature over H
    if !ed25519_verify(&h, &sig, &host_pubkey) {
        trace("host-key-verify-failed");
        return None;
    }
    trace("host-key-verified");

    // Derive session keys (from server's perspective: C2S = rx, S2C = tx)
    let server_keys = derive_keys(&shared_secret, &h, &h);
    // Client swaps rx ↔ tx
    let client_keys = TransportKeys {
        rx_key: server_keys.tx_key,
        rx_header_key: server_keys.tx_header_key,
        tx_key: server_keys.rx_key,
        tx_header_key: server_keys.rx_header_key,
    };

    // ── Step 4: NEWKEYS ──────────────────────────────────────────────
    // Read server's NEWKEYS
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 21 {
        return None;
    }
    trace("got-server-newkeys");

    // Send our NEWKEYS
    payload[0] = 21;
    transport.write_packet(serial, &payload[..1]);
    trace("sent-newkeys");

    // Install encryption keys
    transport.set_keys(client_keys);
    trace("installed-keys");

    // ── Step 5: Service Request ──────────────────────────────────────
    let mut req = [0u8; 64];
    req[0] = 5; // SSH_MSG_SERVICE_REQUEST
    let slen = put_string(&mut req[1..], b"ssh-userauth");
    transport.write_packet(serial, &req[..1 + slen]);
    trace("sent-service-request");

    // Read SERVICE_ACCEPT
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 6 {
        return None;
    }
    trace("got-service-accept");

    // ── Step 6: User Authentication ──────────────────────────────────
    {
        let mut auth = [0u8; 256];
        let mut off = 0;
        auth[off] = 50; // SSH_MSG_USERAUTH_REQUEST
        off += 1;
        off += put_string(&mut auth[off..], config.username);
        off += put_string(&mut auth[off..], b"ssh-connection");
        off += put_string(&mut auth[off..], b"password");
        auth[off] = 0; // FALSE (no old password)
        off += 1;
        off += put_string(&mut auth[off..], config.password);
        transport.write_packet(serial, &auth[..off]);
    }
    trace("sent-userauth");

    // Read USERAUTH response
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 {
        return None;
    }
    if payload[0] != 52 {
        // 51 = USERAUTH_FAILURE, 52 = USERAUTH_SUCCESS
        trace("auth-failed");
        return None;
    }
    trace("auth-success");

    // ── Step 7: Channel Open + Shell ─────────────────────────────────
    let mut channels = ChannelManager::new();

    // CHANNEL_OPEN "session"
    {
        let mut msg = [0u8; 64];
        let mut off = 0;
        msg[off] = 90; // SSH_MSG_CHANNEL_OPEN
        off += 1;
        off += put_string(&mut msg[off..], b"session");
        put_u32(&mut msg[off..], 0); // sender channel
        off += 4;
        put_u32(&mut msg[off..], INITIAL_WINDOW); // initial window
        off += 4;
        put_u32(&mut msg[off..], MAX_CHANNEL_PACKET); // max packet
        off += 4;
        transport.write_packet(serial, &msg[..off]);
    }
    trace("sent-channel-open");

    // Read CHANNEL_OPEN_CONFIRMATION, skipping asynchronous messages
    // (e.g. SSH_MSG_GLOBAL_REQUEST "hostkeys-00@openssh.com" from OpenSSH).
    let n = loop {
        let n = transport.read_packet(serial, &mut payload);
        if n == 0 {
            return None;
        }
        match payload[0] {
            2 | 4 => continue, // SSH_MSG_IGNORE, SSH_MSG_DEBUG
            80 => {
                // SSH_MSG_GLOBAL_REQUEST — check want_reply
                let (_, name_end) = get_string(&payload[1..]);
                let want_reply_off = 1 + name_end;
                if want_reply_off < n && payload[want_reply_off] != 0 {
                    // Send SSH_MSG_REQUEST_FAILURE (82)
                    transport.write_packet(serial, &[82]);
                }
                continue;
            }
            91 => break n,    // SSH_MSG_CHANNEL_OPEN_CONFIRMATION
            _ => return None, // unexpected
        }
    };

    let server_channel_id = get_u32(&payload[5..]);
    let server_window = get_u32(&payload[9..]);
    let server_max_packet = get_u32(&payload[13..]);
    trace("got-channel-confirm");

    // Allocate our channel tracking
    let chan_idx = match channels.alloc_client(server_channel_id, server_window, server_max_packet)
    {
        Some(i) => i,
        None => return None,
    };

    // Request PTY
    {
        let mut msg = [0u8; 128];
        let mut off = 0;
        msg[off] = 98; // SSH_MSG_CHANNEL_REQUEST
        off += 1;
        put_u32(&mut msg[off..], server_channel_id);
        off += 4;
        off += put_string(&mut msg[off..], b"pty-req");
        msg[off] = 1; // want reply
        off += 1;
        off += put_string(&mut msg[off..], b"xterm");
        put_u32(&mut msg[off..], 80); // width
        off += 4;
        put_u32(&mut msg[off..], 24); // height
        off += 4;
        put_u32(&mut msg[off..], 0); // pixel width
        off += 4;
        put_u32(&mut msg[off..], 0); // pixel height
        off += 4;
        off += put_string(&mut msg[off..], b""); // terminal modes (empty)
        transport.write_packet(serial, &msg[..off]);
    }

    // Read PTY response, skipping async messages.
    let _n = loop {
        let n = transport.read_packet(serial, &mut payload);
        if n == 0 {
            return None;
        }
        match payload[0] {
            2 | 4 => continue,
            80 => {
                let (_, name_end) = get_string(&payload[1..]);
                let want_reply_off = 1 + name_end;
                if want_reply_off < n && payload[want_reply_off] != 0 {
                    transport.write_packet(serial, &[82]);
                }
                continue;
            }
            _ => break n,
        }
    };
    // Accept both success (99) and failure (100) for pty-req
    trace("got-pty-response");

    // Request shell
    {
        let mut msg = [0u8; 32];
        let mut off = 0;
        msg[off] = 98; // SSH_MSG_CHANNEL_REQUEST
        off += 1;
        put_u32(&mut msg[off..], server_channel_id);
        off += 4;
        off += put_string(&mut msg[off..], b"shell");
        msg[off] = 1; // want reply
        off += 1;
        transport.write_packet(serial, &msg[..off]);
    }
    trace("sent-shell-request");

    // Read shell response, skipping async messages.
    let _n = loop {
        let n = transport.read_packet(serial, &mut payload);
        if n == 0 {
            return None;
        }
        match payload[0] {
            2 | 4 => continue,
            80 => {
                let (_, name_end) = get_string(&payload[1..]);
                let want_reply_off = 1 + name_end;
                if want_reply_off < n && payload[want_reply_off] != 0 {
                    transport.write_packet(serial, &[82]);
                }
                continue;
            }
            100 => {
                trace("shell-request-failed");
                return None;
            }
            _ => break n,
        }
    };
    trace("shell-ready");

    Some(SshClientBridge {
        transport,
        channels,
        chan_idx,
        server_channel_id,
        rx_buf: [0u8; MAX_CHANNEL_PACKET as usize],
        rx_pos: 0,
        rx_len: 0,
        closed: false,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Exchange hash + key derivation (client side — same math as server)
// ═══════════════════════════════════════════════════════════════════════════

fn compute_exchange_hash(
    v_c: &[u8],
    v_s: &[u8],
    i_c: &[u8],
    i_s: &[u8],
    k_s: &[u8], // raw ed25519 host public key
    q_c: &[u8],
    q_s: &[u8],
    k: &[u8],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let mut tmp = [0u8; MAX_PAYLOAD];

    let n = put_string(&mut tmp, v_c);
    hasher.update(&tmp[..n]);
    let n = put_string(&mut tmp, v_s);
    hasher.update(&tmp[..n]);
    let n = put_string(&mut tmp, i_c);
    hasher.update(&tmp[..n]);
    let n = put_string(&mut tmp, i_s);
    hasher.update(&tmp[..n]);

    // K_S blob: string("ssh-ed25519") + string(pubkey)
    let mut ks_blob = [0u8; 128];
    let mut ks_len = 0;
    ks_len += put_string(&mut ks_blob[ks_len..], b"ssh-ed25519");
    ks_len += put_string(&mut ks_blob[ks_len..], k_s);
    let n = put_string(&mut tmp, &ks_blob[..ks_len]);
    hasher.update(&tmp[..n]);

    let n = put_string(&mut tmp, q_c);
    hasher.update(&tmp[..n]);
    let n = put_string(&mut tmp, q_s);
    hasher.update(&tmp[..n]);
    let n = put_mpint(&mut tmp, k);
    hasher.update(&tmp[..n]);

    let digest = hasher.finalize();
    let mut h = [0u8; 32];
    h.copy_from_slice(&digest.bytes[..32]);
    h
}

fn derive_keys(k: &[u8; 32], h: &[u8; 32], session_id: &[u8; 32]) -> TransportKeys {
    let c2s_key = derive_key_material(k, h, b'C', session_id);
    let s2c_key = derive_key_material(k, h, b'D', session_id);
    TransportKeys {
        rx_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&c2s_key[..32]);
            k
        },
        rx_header_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&c2s_key[32..64]);
            k
        },
        tx_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&s2c_key[..32]);
            k
        },
        tx_header_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&s2c_key[32..64]);
            k
        },
    }
}

fn derive_key_material(k: &[u8; 32], h: &[u8; 32], label: u8, session_id: &[u8; 32]) -> [u8; 64] {
    let mut result = [0u8; 64];
    let mut tmp = [0u8; 37];

    let mut hasher = Sha256::new();
    let mpint_len = put_mpint(&mut tmp, k);
    hasher.update(&tmp[..mpint_len]);
    hasher.update(h);
    hasher.update(&[label]);
    hasher.update(session_id);
    let d = hasher.finalize();
    result[..32].copy_from_slice(&d.bytes[..32]);

    let mut hasher = Sha256::new();
    let mpint_len = put_mpint(&mut tmp, k);
    hasher.update(&tmp[..mpint_len]);
    hasher.update(h);
    hasher.update(&result[..32]);
    let d = hasher.finalize();
    result[32..64].copy_from_slice(&d.bytes[..32]);

    result
}

// ═══════════════════════════════════════════════════════════════════════════
// SshClientBridge — read/write over an encrypted SSH channel (client side)
// ═══════════════════════════════════════════════════════════════════════════

pub struct SshClientBridge {
    transport: Transport,
    channels: ChannelManager,
    chan_idx: usize,
    server_channel_id: u32,
    rx_buf: [u8; MAX_CHANNEL_PACKET as usize],
    rx_pos: usize,
    rx_len: usize,
    closed: bool,
}

impl SshClientBridge {
    /// Read the next byte from the remote shell.
    pub fn read_byte_from<S: Serial>(&mut self, serial: &S) -> u8 {
        loop {
            if self.rx_pos < self.rx_len {
                let b = self.rx_buf[self.rx_pos];
                self.rx_pos += 1;
                return b;
            }
            if self.closed {
                return 0x04;
            }

            let mut payload = [0u8; MAX_PAYLOAD];
            let n = self.transport.read_packet(serial, &mut payload);
            if n == 0 {
                self.closed = true;
                return 0x04;
            }

            match payload[0] {
                94 => {
                    // CHANNEL_DATA
                    let (data, _) = get_string(&payload[5..]);
                    let copy_len = data.len().min(self.rx_buf.len());
                    self.rx_buf[..copy_len].copy_from_slice(&data[..copy_len]);
                    self.rx_pos = 0;
                    self.rx_len = copy_len;

                    // Window adjust
                    self.channels
                        .consume_rx_window(self.chan_idx, copy_len as u32);
                    if self.channels.channels[self.chan_idx].rx_window < INITIAL_WINDOW / 2 {
                        let adjust =
                            INITIAL_WINDOW - self.channels.channels[self.chan_idx].rx_window;
                        let mut adj_buf = [0u8; 64];
                        let adj_len =
                            self.channels
                                .build_window_adjust(self.chan_idx, adjust, &mut adj_buf);
                        // Window adjust uses server's channel ID
                        put_u32(&mut adj_buf[1..], self.server_channel_id);
                        self.transport.write_packet(serial, &adj_buf[..adj_len]);
                        self.channels.channels[self.chan_idx].rx_window = INITIAL_WINDOW;
                    }
                }
                93 => {
                    // WINDOW_ADJUST from server
                    let bytes = get_u32(&payload[5..]);
                    self.channels.channels[self.chan_idx].tx_window = self.channels.channels
                        [self.chan_idx]
                        .tx_window
                        .saturating_add(bytes);
                }
                96 => {
                    self.closed = true;
                    return 0x04;
                }
                97 => {
                    // CHANNEL_CLOSE — send close back
                    let mut close_buf = [0u8; 16];
                    close_buf[0] = 97;
                    put_u32(&mut close_buf[1..], self.server_channel_id);
                    self.transport.write_packet(serial, &close_buf[..5]);
                    self.closed = true;
                    return 0x04;
                }
                98 => {
                    // CHANNEL_REQUEST (e.g. exit-status)
                    // Parse want_reply and send success if needed
                    let mut roff = 1;
                    let _recipient = get_u32(&payload[roff..]);
                    roff += 4;
                    let (_req_type, consumed) = get_string(&payload[roff..]);
                    roff += consumed;
                    if roff < n && payload[roff] != 0 {
                        // want_reply = true → send CHANNEL_SUCCESS
                        let mut reply = [0u8; 8];
                        reply[0] = 99;
                        put_u32(&mut reply[1..], self.server_channel_id);
                        self.transport.write_packet(serial, &reply[..5]);
                    }
                }
                2 => {} // IGNORE
                4 => {} // DEBUG
                80 => {
                    // SSH_MSG_GLOBAL_REQUEST — reply failure if wanted
                    let (_, name_end) = get_string(&payload[1..]);
                    let want_reply_off = 1 + name_end;
                    if want_reply_off < n && payload[want_reply_off] != 0 {
                        self.transport.write_packet(serial, &[82]);
                    }
                }
                _ => {}
            }
        }
    }

    /// Write a byte to the remote shell.
    pub fn write_byte_to<S: Serial>(&mut self, serial: &S, byte: u8) {
        if self.closed {
            return;
        }
        let mut buf = [0u8; MAX_PAYLOAD];
        buf[0] = 94; // CHANNEL_DATA
        put_u32(&mut buf[1..], self.server_channel_id);
        let slen = put_string(&mut buf[5..], &[byte]);
        self.transport.write_packet(serial, &buf[..5 + slen]);
        self.channels.channels[self.chan_idx].tx_window = self.channels.channels[self.chan_idx]
            .tx_window
            .saturating_sub(1);
    }

    /// Write multiple bytes to the remote shell.
    pub fn write_bytes_to<S: Serial>(&mut self, serial: &S, data: &[u8]) {
        if self.closed || data.is_empty() {
            return;
        }
        let max_chunk = self.channels.channels[self.chan_idx]
            .tx_window
            .min(self.channels.channels[self.chan_idx].tx_max_packet)
            .min(MAX_CHANNEL_PACKET) as usize;
        let chunk_len = data.len().min(max_chunk);
        if chunk_len == 0 {
            return;
        }

        let mut buf = [0u8; MAX_PAYLOAD];
        buf[0] = 94;
        put_u32(&mut buf[1..], self.server_channel_id);
        let slen = put_string(&mut buf[5..], &data[..chunk_len]);
        self.transport.write_packet(serial, &buf[..5 + slen]);
        self.channels.channels[self.chan_idx].tx_window = self.channels.channels[self.chan_idx]
            .tx_window
            .saturating_sub(chunk_len as u32);
    }

    /// Close the SSH channel.
    pub fn close_channel<S: Serial>(&mut self, serial: &S) {
        if self.closed {
            return;
        }
        self.closed = true;

        let mut buf = [0u8; 16];
        buf[0] = 96; // CHANNEL_EOF
        put_u32(&mut buf[1..], self.server_channel_id);
        self.transport.write_packet(serial, &buf[..5]);

        buf[0] = 97; // CHANNEL_CLOSE
        put_u32(&mut buf[1..], self.server_channel_id);
        self.transport.write_packet(serial, &buf[..5]);
    }

    /// Check if channel is still open.
    pub fn is_open(&self) -> bool {
        !self.closed
    }
}
