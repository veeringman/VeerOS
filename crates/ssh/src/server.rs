//! SSH-2 Server — main state machine.
//!
//! Drives the SSH connection lifecycle:
//!   1. Version exchange
//!   2. Key exchange (curve25519-sha256)
//!   3. User authentication (password)
//!   4. Channel open + shell request
//!   5. Interactive session (bridged via `SshSerial`)
//!
//! The server is generic over `arch::Serial` — it works with any byte
//! transport (UART, TcpSerial, etc.).

use arch::Serial;
use crypto::CryptoRng;
use crate::{MAX_PAYLOAD, VERSION_STRING, get_u32, get_string, put_u32, put_string};
use crate::transport::Transport;
use crate::kex::{KexConfig, KexState, build_kexinit, process_ecdh_init};
use crate::auth::{self, AuthResult};
use crate::channel::{ChannelManager, INITIAL_WINDOW, MAX_CHANNEL_PACKET};

/// SSH connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Waiting for client version string.
    VersionExchange,
    /// Sent our KEXINIT, waiting for client's.
    KexInit,
    /// Waiting for client's ECDH_INIT.
    KexEcdh,
    /// Key exchange done, waiting for NEWKEYS.
    KexNewKeys,
    /// Authenticated session — waiting for SERVICE_REQUEST.
    ServiceRequest,
    /// Waiting for USERAUTH_REQUEST.
    UserAuth,
    /// User authenticated — channel negotiation + interactive session.
    Authenticated,
    /// Connection should be closed.
    Disconnected,
}

/// SSH server configuration.
pub struct SshServerConfig {
    /// Ed25519 host key seed (32 bytes).
    pub host_seed: [u8; 32],
    /// Ed25519 host key public key (32 bytes).
    pub host_pubkey: [u8; 32],
    /// Password verification callback: `(username, password) -> accepted`.
    pub password_verify: fn(&[u8], &[u8]) -> bool,
}

/// Run the SSH-2 server over a serial (byte-stream) transport.
///
/// This function handles the entire SSH lifecycle. When the client
/// requests a shell, it returns `Some(SshShellBridge)` which implements
/// `arch::Serial` for bridging to the VeerOS shell.
///
/// Returns `None` if the connection fails at any point before shell setup.
pub fn run_ssh_handshake<S: Serial>(
    serial: &S,
    config: &SshServerConfig,
    rng: &mut dyn CryptoRng,
) -> Option<SshShellBridge> {
    run_ssh_handshake_with_trace(serial, config, rng, |_| {})
}

/// Run the SSH handshake and emit fixed stage markers through `trace`.
pub fn run_ssh_handshake_with_trace<S: Serial, F: FnMut(&str)>(
    serial: &S,
    config: &SshServerConfig,
    rng: &mut dyn CryptoRng,
    mut trace: F,
) -> Option<SshShellBridge> {
    let mut transport = Transport::new();
    let kex_config = KexConfig {
        host_seed: config.host_seed,
        host_pubkey: config.host_pubkey,
    };
    let mut kex_state = KexState::new(rng);
    let mut channels = ChannelManager::new();
    let mut state = State::VersionExchange;
    let mut payload = [0u8; MAX_PAYLOAD];
    let mut user = [0u8; 64];
    let mut auth_attempts: u8 = 0;

    // ── Step 1: Version exchange ─────────────────────────────────────────
    // Send our version string
    serial.write_bytes(VERSION_STRING);
    serial.write_bytes(b"\r\n");
    trace("sent-version");

    // Read client's version string (line ending with CR LF or LF)
    let mut version_buf = [0u8; 256];
    let mut vlen = 0;
    loop {
        let b = serial.read_byte();
        if b == 0x04 { return None; } // EOF
        if b == b'\n' { break; }
        if b == b'\r' { continue; }
        if vlen < version_buf.len() {
            version_buf[vlen] = b;
            vlen += 1;
        }
    }

    // Validate version starts with "SSH-2.0-"
    if vlen < 8 || &version_buf[..8] != b"SSH-2.0-" {
        return None;
    }

    kex_state.client_version[..vlen].copy_from_slice(&version_buf[..vlen]);
    kex_state.client_version_len = vlen;
    trace("got-client-version");

    // ── Step 2: Key Exchange Init ────────────────────────────────────────
    // Send our KEXINIT
    let mut cookie = [0u8; 16];
    rng.fill_bytes(&mut cookie);
    let kexinit_len = build_kexinit(&mut payload, &cookie);

    // Save our KEXINIT for exchange hash computation
    kex_state.server_kexinit[..kexinit_len].copy_from_slice(&payload[..kexinit_len]);
    kex_state.server_kexinit_len = kexinit_len;

    transport.write_packet(serial, &payload[..kexinit_len]);
    trace("sent-kexinit");

    // Read client's KEXINIT
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 20 { return None; } // not KEXINIT

    kex_state.client_kexinit[..n].copy_from_slice(&payload[..n]);
    kex_state.client_kexinit_len = n;
    trace("got-client-kexinit");

    // ── Step 3: ECDH Key Exchange ────────────────────────────────────────
    // Read client's KEX_ECDH_INIT (msg type 30)
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 30 { return None; }
    trace("got-ecdh-init");

    // Parse client's ephemeral public key Q_C
    let mut off = 1;
    let (q_c_bytes, consumed) = get_string(&payload[off..]);
    if q_c_bytes.len() != 32 { return None; }
    let mut q_c = [0u8; 32];
    q_c.copy_from_slice(q_c_bytes);

    // Process ECDH and build reply
    let (reply, reply_len, keys, exchange_hash, shared_secret, q_s) = match process_ecdh_init(&q_c, &mut kex_state, &kex_config) {
        Some(result) => result,
        None => return None,
    };

    // Debug: dump exchange hash H as hex
    {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut hex_buf = [0u8; 4 + 64]; // prefix + 64 hex chars
        // Dump H
        hex_buf[0] = b'H';
        hex_buf[1] = b':';
        for i in 0..32 {
            hex_buf[2 + i * 2] = HEX[(exchange_hash[i] >> 4) as usize];
            hex_buf[2 + i * 2 + 1] = HEX[(exchange_hash[i] & 0xf) as usize];
        }
        if let Ok(s) = core::str::from_utf8(&hex_buf[..66]) {
            trace(s);
        }
        // Dump Q_C (client ephemeral)
        hex_buf[0] = b'Q';
        hex_buf[1] = b'C';
        hex_buf[2] = b':';
        for i in 0..32 {
            hex_buf[3 + i * 2] = HEX[(q_c[i] >> 4) as usize];
            hex_buf[3 + i * 2 + 1] = HEX[(q_c[i] & 0xf) as usize];
        }
        if let Ok(s) = core::str::from_utf8(&hex_buf[..67]) {
            trace(s);
        }
        // Dump I_C length and I_S length
        hex_buf[0] = b'I';
        hex_buf[1] = b'C';
        hex_buf[2] = b'L';
        hex_buf[3] = b':';
        let ic_len = kex_state.client_kexinit_len;
        hex_buf[4] = HEX[(ic_len >> 12) & 0xf];
        hex_buf[5] = HEX[(ic_len >> 8) & 0xf];
        hex_buf[6] = HEX[(ic_len >> 4) & 0xf];
        hex_buf[7] = HEX[ic_len & 0xf];
        if let Ok(s) = core::str::from_utf8(&hex_buf[..8]) {
            trace(s);
        }
        hex_buf[0] = b'I';
        hex_buf[1] = b'S';
        hex_buf[2] = b'L';
        hex_buf[3] = b':';
        let is_len = kex_state.server_kexinit_len;
        hex_buf[4] = HEX[(is_len >> 12) & 0xf];
        hex_buf[5] = HEX[(is_len >> 8) & 0xf];
        hex_buf[6] = HEX[(is_len >> 4) & 0xf];
        hex_buf[7] = HEX[is_len & 0xf];
        if let Ok(s) = core::str::from_utf8(&hex_buf[..8]) {
            trace(s);
        }
        // Dump K (shared secret)
        hex_buf[0] = b'K';
        hex_buf[1] = b':';
        for i in 0..32 {
            hex_buf[2 + i * 2] = HEX[(shared_secret[i] >> 4) as usize];
            hex_buf[2 + i * 2 + 1] = HEX[(shared_secret[i] & 0xf) as usize];
        }
        if let Ok(s) = core::str::from_utf8(&hex_buf[..66]) {
            trace(s);
        }
        // Dump Q_S (server ephemeral)
        hex_buf[0] = b'Q';
        hex_buf[1] = b'S';
        hex_buf[2] = b':';
        for i in 0..32 {
            hex_buf[3 + i * 2] = HEX[(q_s[i] >> 4) as usize];
            hex_buf[3 + i * 2 + 1] = HEX[(q_s[i] & 0xf) as usize];
        }
        if let Ok(s) = core::str::from_utf8(&hex_buf[..67]) {
            trace(s);
        }
        // Dump V_C
        hex_buf[0] = b'V';
        hex_buf[1] = b'C';
        hex_buf[2] = b'L';
        hex_buf[3] = b':';
        let vc_len = kex_state.client_version_len;
        hex_buf[4] = HEX[(vc_len >> 12) & 0xf];
        hex_buf[5] = HEX[(vc_len >> 8) & 0xf];
        hex_buf[6] = HEX[(vc_len >> 4) & 0xf];
        hex_buf[7] = HEX[vc_len & 0xf];
        if let Ok(s) = core::str::from_utf8(&hex_buf[..8]) {
            trace(s);
        }
    }

    trace("built-ecdh-reply");

    // Send KEX_ECDH_REPLY
    transport.write_packet(serial, &reply[..reply_len]);
    trace("sent-ecdh-reply");

    // ── Step 4: NEWKEYS ──────────────────────────────────────────────────
    // Send our NEWKEYS
    payload[0] = 21; // SSH_MSG_NEWKEYS
    transport.write_packet(serial, &payload[..1]);
    trace("sent-newkeys");

    // Read client's NEWKEYS
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 21 { return None; }
    trace("got-client-newkeys");

    // Install encryption keys
    transport.set_keys(keys);
    trace("installed-keys");

    // ── Step 5: Service Request ──────────────────────────────────────────
    // Client sends SERVICE_REQUEST for "ssh-userauth"
    let n = transport.read_packet(serial, &mut payload);
    if n == 0 || payload[0] != 5 { return None; }
    trace("got-service-request");

    let (service_name, _) = get_string(&payload[1..]);
    if service_name != b"ssh-userauth" { return None; }

    // Send SERVICE_ACCEPT
    let mut accept = [0u8; 64];
    accept[0] = 6; // SSH_MSG_SERVICE_ACCEPT
    let accept_len = 1 + put_string(&mut accept[1..], b"ssh-userauth");
    transport.write_packet(serial, &accept[..accept_len]);
    trace("sent-service-accept");

    // ── Step 6: User Authentication ──────────────────────────────────────
    loop {
        let n = transport.read_packet(serial, &mut payload);
        if n == 0 { return None; }

        if payload[0] == 50 {
            // USERAUTH_REQUEST
            let (result, _ulen) = auth::check_userauth(&payload[..n], config.password_verify, &mut user);
            match result {
                AuthResult::Success => {
                    let slen = auth::build_userauth_success(&mut payload);
                    transport.write_packet(serial, &payload[..slen]);
                    trace("auth-success");
                    break;
                }
                AuthResult::Failure | AuthResult::Partial => {
                    auth_attempts += 1;
                    let slen = auth::build_userauth_failure(&mut payload);
                    transport.write_packet(serial, &payload[..slen]);
                    if auth_attempts >= 3 {
                        return None;
                    }
                }
            }
        } else {
            // Unexpected message during auth
            return None;
        }
    }

    // ── Step 7: Channel Setup ────────────────────────────────────────────
    // Now wait for CHANNEL_OPEN, pty-req, and shell request.
    let mut shell_channel: Option<usize> = None;

    loop {
        let n = transport.read_packet(serial, &mut payload);
        if n == 0 { return None; }

        match payload[0] {
            90 => {
                // CHANNEL_OPEN
                let (reply, rlen) = channels.handle_channel_open(&payload[..n]);
                transport.write_packet(serial, &reply[..rlen]);
            }
            98 => {
                // CHANNEL_REQUEST
                let (reply, rlen, shell_ready) = channels.handle_channel_request(&payload[..n]);
                if rlen > 0 {
                    transport.write_packet(serial, &reply[..rlen]);
                }
                if shell_ready {
                    shell_channel = channels.get_shell_channel();
                    trace("shell-ready");
                    break;
                }
            }
            93 => {
                // WINDOW_ADJUST — update peer's window
                let server_id = get_u32(&payload[1..]);
                let bytes_to_add = get_u32(&payload[5..]);
                if let Some(idx) = channels.find(server_id) {
                    channels.channels[idx].tx_window =
                        channels.channels[idx].tx_window.saturating_add(bytes_to_add);
                }
            }
            2 => {
                // IGNORE — skip
            }
            _ => {
                // Unknown — ignore during channel setup
            }
        }
    }

    let chan_idx = match shell_channel {
        Some(idx) => idx,
        None => return None,
    };

    Some(SshShellBridge {
        transport,
        channels,
        chan_idx,
        // Receive buffer for channel data from client
        rx_buf: [0u8; MAX_CHANNEL_PACKET as usize],
        rx_pos: 0,
        rx_len: 0,
        closed: false,
        auth_user: user,
        auth_user_len: {
            // Find the actual username length (set by check_userauth)
            let mut len = 0;
            while len < 64 && user[len] != 0 { len += 1; }
            len
        },
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// SshShellBridge — implements `arch::Serial` over an encrypted SSH channel
// ═══════════════════════════════════════════════════════════════════════════

/// Bridges an SSH channel to the VeerOS `Serial` trait.
///
/// After `run_ssh_handshake()` succeeds, the caller uses this struct
/// to create a `Console<SshShellBridge>` and run the shell over SSH.
///
/// **Important**: `SshShellBridge` needs a reference to the underlying
/// byte transport (TcpSerial) to read/write SSH packets. We store this
/// as function pointers set during construction.
pub struct SshShellBridge {
    transport: Transport,
    channels: ChannelManager,
    chan_idx: usize,
    rx_buf: [u8; MAX_CHANNEL_PACKET as usize],
    rx_pos: usize,
    rx_len: usize,
    closed: bool,
    /// Authenticated username (UTF-8 bytes, length in `auth_user_len`).
    auth_user: [u8; 64],
    auth_user_len: usize,
}

impl SshShellBridge {
    /// Read the next byte from the SSH channel.
    ///
    /// Reads SSH packets from the underlying transport, extracts channel
    /// data, and returns one byte at a time.
    pub fn read_byte_from<S: Serial>(&mut self, serial: &S) -> u8 {
        loop {
            // Return buffered data first
            if self.rx_pos < self.rx_len {
                let b = self.rx_buf[self.rx_pos];
                self.rx_pos += 1;
                return b;
            }

            if self.closed {
                return 0x04; // EOF
            }

            // Need more data — read an SSH packet
            let mut payload = [0u8; MAX_PAYLOAD];
            let n = self.transport.read_packet(serial, &mut payload);
            if n == 0 {
                self.closed = true;
                return 0x04;
            }

            match payload[0] {
                94 => {
                    // CHANNEL_DATA
                    let _recipient = get_u32(&payload[1..]);
                    let (data, _) = get_string(&payload[5..]);
                    if data.len() > self.rx_buf.len() {
                        self.closed = true;
                        return 0x04;
                    }

                    let copy_len = data.len();
                    self.rx_buf[..copy_len].copy_from_slice(data);
                    self.rx_pos = 0;
                    self.rx_len = copy_len;

                    // Consume window and send WINDOW_ADJUST if needed
                    self.channels.consume_rx_window(self.chan_idx, copy_len as u32);
                    if self.channels.channels[self.chan_idx].rx_window < INITIAL_WINDOW / 2 {
                        let adjust = INITIAL_WINDOW - self.channels.channels[self.chan_idx].rx_window;
                        let mut adj_buf = [0u8; 64];
                        let adj_len = self.channels.build_window_adjust(self.chan_idx, adjust, &mut adj_buf);
                        self.transport.write_packet(serial, &adj_buf[..adj_len]);
                        self.channels.channels[self.chan_idx].rx_window = INITIAL_WINDOW;
                    }
                }
                93 => {
                    // WINDOW_ADJUST
                    let _server_id = get_u32(&payload[1..]);
                    let bytes = get_u32(&payload[5..]);
                    self.channels.channels[self.chan_idx].tx_window =
                        self.channels.channels[self.chan_idx].tx_window.saturating_add(bytes);
                }
                96 => {
                    // CHANNEL_EOF
                    self.closed = true;
                    return 0x04;
                }
                97 => {
                    // CHANNEL_CLOSE
                    // Send close back
                    let mut close_buf = [0u8; 16];
                    let close_len = self.channels.build_channel_close(self.chan_idx, &mut close_buf);
                    self.transport.write_packet(serial, &close_buf[..close_len]);
                    self.channels.close(self.chan_idx);
                    self.closed = true;
                    return 0x04;
                }
                98 => {
                    // CHANNEL_REQUEST (e.g., window-change during session)
                    let (reply, rlen, _) = self.channels.handle_channel_request(&payload[..n]);
                    if rlen > 0 {
                        self.transport.write_packet(serial, &reply[..rlen]);
                    }
                }
                2 => {
                    // IGNORE
                }
                _ => {
                    // Unknown — skip
                }
            }
        }
    }

    /// Write a byte to the SSH channel.
    pub fn write_byte_to<S: Serial>(&mut self, serial: &S, byte: u8) {
        if self.closed { return; }

        let mut buf = [0u8; MAX_PAYLOAD];
        let data = [byte];
        let plen = self.channels.build_channel_data(self.chan_idx, &data, &mut buf);
        if plen > 0 {
            self.transport.write_packet(serial, &buf[..plen]);
        }
    }

    /// Write multiple bytes to the SSH channel (more efficient than one at a time).
    pub fn write_bytes_to<S: Serial>(&mut self, serial: &S, data: &[u8]) {
        if self.closed || data.is_empty() { return; }

        let mut buf = [0u8; MAX_PAYLOAD];
        let plen = self.channels.build_channel_data(self.chan_idx, data, &mut buf);
        if plen > 0 {
            self.transport.write_packet(serial, &buf[..plen]);
        }
    }

    /// Send EOF and CLOSE to cleanly shut down the channel.
    pub fn close_channel<S: Serial>(&mut self, serial: &S) {
        if self.closed { return; }
        self.closed = true;

        let mut buf = [0u8; 16];
        let n = self.channels.build_channel_eof(self.chan_idx, &mut buf);
        self.transport.write_packet(serial, &buf[..n]);

        let n = self.channels.build_channel_close(self.chan_idx, &mut buf);
        self.transport.write_packet(serial, &buf[..n]);
        self.channels.close(self.chan_idx);
    }

    /// Check if the channel is still open.
    pub fn is_open(&self) -> bool {
        !self.closed
    }

    /// Return the authenticated username bytes.
    pub fn authenticated_user(&self) -> &[u8] {
        &self.auth_user[..self.auth_user_len]
    }

    /// Return the exec command if the client requested exec instead of shell.
    pub fn exec_command(&self) -> Option<&[u8]> {
        let ch = &self.channels.channels[self.chan_idx];
        if ch.exec_requested && ch.exec_cmd_len > 0 {
            Some(&ch.exec_cmd[..ch.exec_cmd_len])
        } else {
            None
        }
    }
}
