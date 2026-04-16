//! VeerOS SSH-2 Server — Minimal Implementation
//!
//! `no_std`, `no_alloc` SSH-2 server (RFC 4253/4254) for VeerOS.
//!
//! # Supported Algorithms
//!
//! | Type       | Algorithm                    |
//! |------------|------------------------------|
//! | KEX        | curve25519-sha256            |
//! | Host key   | ssh-ed25519                  |
//! | Cipher     | chacha20-poly1305@openssh.com |
//! | MAC        | (implicit in AEAD)           |
//! | Compress   | none                         |
//!
//! # Architecture
//!
//! The server is generic over `arch::Serial` — it reads/writes bytes
//! through the same trait used by UART and TcpSerial. The SSH protocol
//! state machine runs synchronously in a single task.

#![no_std]

pub mod transport;
pub mod kex;
pub mod auth;
pub mod channel;
pub mod server;

/// SSH-2 protocol version string.
pub const VERSION_STRING: &[u8] = b"SSH-2.0-VeerOS_1.0";

/// Maximum packet payload size (conservative for embedded).
pub const MAX_PAYLOAD: usize = 4096;

/// Maximum packet size including headers, padding, and MAC tag.
/// 4 (packet_length) + 1 (padding_length) + MAX_PAYLOAD + 255 (max padding) + 16 (tag).
pub const MAX_PACKET: usize = MAX_PAYLOAD + 280;

// ═══════════════════════════════════════════════════════════════════════════
// SSH-2 Message Types (RFC 4253 §12)
// ═══════════════════════════════════════════════════════════════════════════

/// SSH message type numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Disconnect = 1,
    Ignore = 2,
    Unimplemented = 3,
    Debug = 4,
    ServiceRequest = 5,
    ServiceAccept = 6,
    KexInit = 20,
    NewKeys = 21,
    KexEcdhInit = 30,
    KexEcdhReply = 31,
    UserauthRequest = 50,
    UserauthFailure = 51,
    UserauthSuccess = 52,
    UserauthBanner = 53,
    GlobalRequest = 80,
    RequestSuccess = 81,
    RequestFailure = 82,
    ChannelOpen = 90,
    ChannelOpenConfirmation = 91,
    ChannelOpenFailure = 92,
    ChannelWindowAdjust = 93,
    ChannelData = 94,
    ChannelExtendedData = 95,
    ChannelEof = 96,
    ChannelClose = 97,
    ChannelRequest = 98,
    ChannelSuccess = 99,
    ChannelFailure = 100,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<MsgType> {
        match v {
            1 => Some(MsgType::Disconnect),
            2 => Some(MsgType::Ignore),
            3 => Some(MsgType::Unimplemented),
            4 => Some(MsgType::Debug),
            5 => Some(MsgType::ServiceRequest),
            6 => Some(MsgType::ServiceAccept),
            20 => Some(MsgType::KexInit),
            21 => Some(MsgType::NewKeys),
            30 => Some(MsgType::KexEcdhInit),
            31 => Some(MsgType::KexEcdhReply),
            50 => Some(MsgType::UserauthRequest),
            51 => Some(MsgType::UserauthFailure),
            52 => Some(MsgType::UserauthSuccess),
            53 => Some(MsgType::UserauthBanner),
            80 => Some(MsgType::GlobalRequest),
            81 => Some(MsgType::RequestSuccess),
            82 => Some(MsgType::RequestFailure),
            90 => Some(MsgType::ChannelOpen),
            91 => Some(MsgType::ChannelOpenConfirmation),
            92 => Some(MsgType::ChannelOpenFailure),
            93 => Some(MsgType::ChannelWindowAdjust),
            94 => Some(MsgType::ChannelData),
            95 => Some(MsgType::ChannelExtendedData),
            96 => Some(MsgType::ChannelEof),
            97 => Some(MsgType::ChannelClose),
            98 => Some(MsgType::ChannelRequest),
            99 => Some(MsgType::ChannelSuccess),
            100 => Some(MsgType::ChannelFailure),
            _ => None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SSH-2 Disconnect Reason Codes
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum DisconnectReason {
    HostNotAllowed = 1,
    ProtocolError = 2,
    KeyExchangeFailed = 3,
    Reserved = 4,
    MacError = 5,
    CompressionError = 6,
    ServiceNotAvailable = 7,
    ProtocolVersionNotSupported = 8,
    HostKeyNotVerifiable = 9,
    ConnectionLost = 10,
    ByApplication = 11,
    TooManyConnections = 12,
    AuthCancelledByUser = 13,
    NoMoreAuthMethodsAvailable = 14,
    IllegalUserName = 15,
}

// ═══════════════════════════════════════════════════════════════════════════
// SSH-2 Wire Encoding Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Write a u32 in big-endian (network byte order) to a buffer.
pub fn put_u32(buf: &mut [u8], val: u32) {
    buf[0] = (val >> 24) as u8;
    buf[1] = (val >> 16) as u8;
    buf[2] = (val >> 8) as u8;
    buf[3] = val as u8;
}

/// Read a u32 in big-endian from a buffer.
pub fn get_u32(buf: &[u8]) -> u32 {
    ((buf[0] as u32) << 24)
        | ((buf[1] as u32) << 16)
        | ((buf[2] as u32) << 8)
        | (buf[3] as u32)
}

/// Write an SSH "string" (u32 length + data) into buffer, return bytes written.
pub fn put_string(buf: &mut [u8], data: &[u8]) -> usize {
    put_u32(&mut buf[0..4], data.len() as u32);
    buf[4..4 + data.len()].copy_from_slice(data);
    4 + data.len()
}

/// Read an SSH "string" from buffer. Returns (data_slice, bytes_consumed).
pub fn get_string(buf: &[u8]) -> (&[u8], usize) {
    let len = get_u32(&buf[0..4]) as usize;
    (&buf[4..4 + len], 4 + len)
}

/// Write an SSH "name-list" (comma-separated algorithm names).
pub fn put_name_list(buf: &mut [u8], names: &[u8]) -> usize {
    put_string(buf, names)
}

/// Write an SSH "mpint" (multi-precision integer in two's complement).
pub fn put_mpint(buf: &mut [u8], val: &[u8]) -> usize {
    // Skip leading zeros
    let mut start = 0;
    while start < val.len() && val[start] == 0 {
        start += 1;
    }
    if start == val.len() {
        // Zero value
        put_u32(&mut buf[0..4], 0);
        return 4;
    }

    // If high bit is set, prepend a zero byte
    let needs_pad = (val[start] & 0x80) != 0;
    let data_len = val.len() - start;
    let total_len = data_len + if needs_pad { 1 } else { 0 };

    put_u32(&mut buf[0..4], total_len as u32);
    let mut off = 4;
    if needs_pad {
        buf[off] = 0;
        off += 1;
    }
    buf[off..off + data_len].copy_from_slice(&val[start..]);
    4 + total_len
}
