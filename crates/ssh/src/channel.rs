//! SSH-2 Channel Layer (RFC 4254).
//!
//! Handles channel open, data, window adjust, close, and channel requests
//! (pty-req, shell, exec). VeerOS supports a single session channel that
//! bridges to the shell via the `Serial` trait.

use crate::{get_string, get_u32, put_string, put_u32};

/// Maximum number of simultaneous channels (1 is enough for a shell).
pub const MAX_CHANNELS: usize = 1;

/// Initial window size (how many bytes the peer can send before we ack).
pub const INITIAL_WINDOW: u32 = 32768;

/// Maximum packet size we accept per channel data message.
pub const MAX_CHANNEL_PACKET: u32 = 4096;

/// Channel state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    Free,
    Open,
    Closing,
}

/// A single SSH channel.
#[derive(Clone, Copy)]
pub struct Channel {
    pub state: ChannelState,
    /// Our channel ID (server-side).
    pub server_id: u32,
    /// Peer's channel ID (client-side).
    pub client_id: u32,
    /// Our receive window remaining.
    pub rx_window: u32,
    /// Peer's receive window remaining.
    pub tx_window: u32,
    /// Peer's max packet size.
    pub tx_max_packet: u32,
    /// Whether a shell has been requested on this channel.
    pub shell_requested: bool,
    /// Whether an exec command has been requested.
    pub exec_requested: bool,
    /// Exec command buffer.
    pub exec_cmd: [u8; 128],
    /// Exec command length.
    pub exec_cmd_len: usize,
    /// Whether a PTY has been allocated.
    pub pty_allocated: bool,
    /// Terminal width / height (from pty-req).
    pub term_width: u32,
    pub term_height: u32,
}

impl Channel {
    pub const fn free() -> Self {
        Self {
            state: ChannelState::Free,
            server_id: 0,
            client_id: 0,
            rx_window: 0,
            tx_window: 0,
            tx_max_packet: 0,
            shell_requested: false,
            exec_requested: false,
            exec_cmd: [0u8; 128],
            exec_cmd_len: 0,
            pty_allocated: false,
            term_width: 80,
            term_height: 24,
        }
    }
}

/// Channel manager.
pub struct ChannelManager {
    pub channels: [Channel; MAX_CHANNELS],
    next_id: u32,
}

impl ChannelManager {
    pub const fn new() -> Self {
        Self {
            channels: [Channel::free(); MAX_CHANNELS],
            next_id: 0,
        }
    }

    /// Find a free slot and open a new channel.
    fn alloc(&mut self, client_id: u32, tx_window: u32, tx_max_packet: u32) -> Option<usize> {
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if ch.state == ChannelState::Free {
                ch.state = ChannelState::Open;
                ch.server_id = self.next_id;
                ch.client_id = client_id;
                ch.rx_window = INITIAL_WINDOW;
                ch.tx_window = tx_window;
                ch.tx_max_packet = tx_max_packet;
                ch.shell_requested = false;
                ch.exec_requested = false;
                ch.exec_cmd_len = 0;
                ch.pty_allocated = false;
                self.next_id += 1;
                return Some(i);
            }
        }
        None
    }

    /// Allocate a channel from the client side (we know the server's channel ID).
    pub fn alloc_client(
        &mut self,
        server_channel_id: u32,
        tx_window: u32,
        tx_max_packet: u32,
    ) -> Option<usize> {
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if ch.state == ChannelState::Free {
                ch.state = ChannelState::Open;
                ch.server_id = server_channel_id;
                ch.client_id = self.next_id;
                ch.rx_window = INITIAL_WINDOW;
                ch.tx_window = tx_window;
                ch.tx_max_packet = tx_max_packet;
                ch.shell_requested = true;
                ch.exec_requested = false;
                ch.exec_cmd_len = 0;
                ch.pty_allocated = false;
                self.next_id += 1;
                return Some(i);
            }
        }
        None
    }

    /// Find channel by server ID.
    pub fn find(&self, server_id: u32) -> Option<usize> {
        for (i, ch) in self.channels.iter().enumerate() {
            if ch.state != ChannelState::Free && ch.server_id == server_id {
                return Some(i);
            }
        }
        None
    }

    /// Find channel by client ID.
    pub fn find_by_client_id(&self, client_id: u32) -> Option<usize> {
        for (i, ch) in self.channels.iter().enumerate() {
            if ch.state != ChannelState::Free && ch.client_id == client_id {
                return Some(i);
            }
        }
        None
    }

    /// Process a CHANNEL_OPEN message.
    /// Returns the reply payload and its length.
    pub fn handle_channel_open(&mut self, payload: &[u8]) -> ([u8; 256], usize) {
        let mut reply = [0u8; 256];
        let mut off = 1; // skip msg type byte

        // channel type (string)
        let (chan_type, consumed) = get_string(&payload[off..]);
        off += consumed;

        // sender channel (client's ID)
        let client_id = get_u32(&payload[off..]);
        off += 4;

        // initial window size
        let init_window = get_u32(&payload[off..]);
        off += 4;

        // maximum packet size
        let max_packet = get_u32(&payload[off..]);

        // Only accept "session" channels
        if chan_type != b"session" {
            // CHANNEL_OPEN_FAILURE
            let mut roff = 0;
            reply[roff] = 92; // SSH_MSG_CHANNEL_OPEN_FAILURE
            roff += 1;
            put_u32(&mut reply[roff..], client_id);
            roff += 4;
            put_u32(&mut reply[roff..], 3); // reason: SSH_OPEN_UNKNOWN_CHANNEL_TYPE
            roff += 4;
            roff += put_string(&mut reply[roff..], b"unsupported channel type");
            roff += put_string(&mut reply[roff..], b""); // language tag
            return (reply, roff);
        }

        match self.alloc(client_id, init_window, max_packet) {
            Some(idx) => {
                let ch = &self.channels[idx];
                // CHANNEL_OPEN_CONFIRMATION
                let mut roff = 0;
                reply[roff] = 91; // SSH_MSG_CHANNEL_OPEN_CONFIRMATION
                roff += 1;
                put_u32(&mut reply[roff..], client_id); // recipient channel
                roff += 4;
                put_u32(&mut reply[roff..], ch.server_id); // sender channel
                roff += 4;
                put_u32(&mut reply[roff..], INITIAL_WINDOW); // initial window
                roff += 4;
                put_u32(&mut reply[roff..], MAX_CHANNEL_PACKET); // max packet
                roff += 4;
                (reply, roff)
            }
            None => {
                // No free channels
                let mut roff = 0;
                reply[roff] = 92;
                roff += 1;
                put_u32(&mut reply[roff..], client_id);
                roff += 4;
                put_u32(&mut reply[roff..], 4); // SSH_OPEN_RESOURCE_SHORTAGE
                roff += 4;
                roff += put_string(&mut reply[roff..], b"no free channels");
                roff += put_string(&mut reply[roff..], b"");
                (reply, roff)
            }
        }
    }

    /// Process a CHANNEL_REQUEST message.
    /// Returns an optional reply (CHANNEL_SUCCESS/FAILURE) and its length.
    pub fn handle_channel_request(&mut self, payload: &[u8]) -> ([u8; 64], usize, bool) {
        let mut reply = [0u8; 64];
        let mut off = 1; // skip msg type

        let server_id = get_u32(&payload[off..]);
        off += 4;

        let (req_type, consumed) = get_string(&payload[off..]);
        off += consumed;

        let want_reply = payload[off] != 0;
        off += 1;

        let idx = match self.find(server_id) {
            Some(i) => i,
            None => {
                if want_reply {
                    reply[0] = 100; // CHANNEL_FAILURE
                    put_u32(&mut reply[1..], server_id);
                    return (reply, 5, false);
                }
                return (reply, 0, false);
            }
        };

        let mut shell_ready = false;

        if req_type == b"pty-req" {
            // Parse terminal info (we mostly ignore it)
            // string TERM
            let (_term, consumed) = get_string(&payload[off..]);
            off += consumed;
            // uint32 width (chars)
            let width = get_u32(&payload[off..]);
            off += 4;
            // uint32 height (rows)
            let height = get_u32(&payload[off..]);
            off += 4;
            // skip pixel width, height, and terminal modes
            self.channels[idx].pty_allocated = true;
            self.channels[idx].term_width = width;
            self.channels[idx].term_height = height;

            if want_reply {
                reply[0] = 99; // CHANNEL_SUCCESS
                put_u32(&mut reply[1..], self.channels[idx].client_id);
                return (reply, 5, false);
            }
            return (reply, 0, false);
        }

        if req_type == b"shell" {
            self.channels[idx].shell_requested = true;
            shell_ready = true;
            if want_reply {
                reply[0] = 99; // CHANNEL_SUCCESS
                put_u32(&mut reply[1..], self.channels[idx].client_id);
                return (reply, 5, shell_ready);
            }
            return (reply, 0, shell_ready);
        }

        if req_type == b"exec" {
            // Parse the command string
            if off + 4 <= payload.len() {
                let (cmd, _consumed) = get_string(&payload[off..]);
                let copy_len = cmd.len().min(128);
                self.channels[idx].exec_cmd[..copy_len].copy_from_slice(&cmd[..copy_len]);
                self.channels[idx].exec_cmd_len = copy_len;
                self.channels[idx].exec_requested = true;
                self.channels[idx].shell_requested = true; // reuse shell_ready flow
                shell_ready = true;
            }
            if want_reply {
                reply[0] = 99; // CHANNEL_SUCCESS
                put_u32(&mut reply[1..], self.channels[idx].client_id);
                return (reply, 5, shell_ready);
            }
            return (reply, 0, shell_ready);
        }

        if req_type == b"env" {
            // Accept but ignore environment variables
            if want_reply {
                reply[0] = 99;
                put_u32(&mut reply[1..], self.channels[idx].client_id);
                return (reply, 5, false);
            }
            return (reply, 0, false);
        }

        if req_type == b"window-change" {
            // Update terminal dimensions
            if off + 8 <= payload.len() {
                self.channels[idx].term_width = get_u32(&payload[off..]);
                self.channels[idx].term_height = get_u32(&payload[off + 4..]);
            }
            // window-change never wants a reply
            return (reply, 0, false);
        }

        // Unknown request type
        if want_reply {
            reply[0] = 100; // CHANNEL_FAILURE
            put_u32(&mut reply[1..], self.channels[idx].client_id);
            return (reply, 5, false);
        }
        (reply, 0, false)
    }

    /// Build a CHANNEL_DATA message for sending data to the client.
    /// Returns bytes written into buf.
    pub fn build_channel_data(&mut self, chan_idx: usize, data: &[u8], buf: &mut [u8]) -> usize {
        let ch = &mut self.channels[chan_idx];
        if ch.state != ChannelState::Open || data.is_empty() {
            return 0;
        }

        let max_by_window = ch.tx_window as usize;
        let max_by_packet = ch.tx_max_packet.min(MAX_CHANNEL_PACKET) as usize;
        let max_by_buf = buf.len().saturating_sub(9);
        let chunk_len = data
            .len()
            .min(max_by_window)
            .min(max_by_packet)
            .min(max_by_buf);
        if chunk_len == 0 {
            return 0;
        }

        let mut off = 0;
        buf[off] = 94; // SSH_MSG_CHANNEL_DATA
        off += 1;
        put_u32(&mut buf[off..], ch.client_id);
        off += 4;
        off += put_string(&mut buf[off..], &data[..chunk_len]);
        ch.tx_window = ch.tx_window.saturating_sub(chunk_len as u32);
        off
    }

    /// Build a CHANNEL_WINDOW_ADJUST message.
    pub fn build_window_adjust(&self, chan_idx: usize, bytes_to_add: u32, buf: &mut [u8]) -> usize {
        let ch = &self.channels[chan_idx];
        let mut off = 0;
        buf[off] = 93; // SSH_MSG_CHANNEL_WINDOW_ADJUST
        off += 1;
        put_u32(&mut buf[off..], ch.client_id);
        off += 4;
        put_u32(&mut buf[off..], bytes_to_add);
        off += 4;
        off
    }

    /// Build a CHANNEL_EOF message.
    pub fn build_channel_eof(&self, chan_idx: usize, buf: &mut [u8]) -> usize {
        let ch = &self.channels[chan_idx];
        let mut off = 0;
        buf[off] = 96; // SSH_MSG_CHANNEL_EOF
        off += 1;
        put_u32(&mut buf[off..], ch.client_id);
        off += 4;
        off
    }

    /// Build a CHANNEL_CLOSE message.
    pub fn build_channel_close(&self, chan_idx: usize, buf: &mut [u8]) -> usize {
        let ch = &self.channels[chan_idx];
        let mut off = 0;
        buf[off] = 97; // SSH_MSG_CHANNEL_CLOSE
        off += 1;
        put_u32(&mut buf[off..], ch.client_id);
        off += 4;
        off
    }

    /// Close a channel.
    pub fn close(&mut self, idx: usize) {
        if idx < MAX_CHANNELS {
            self.channels[idx].state = ChannelState::Free;
        }
    }

    /// Consume window: track bytes received from client.
    pub fn consume_rx_window(&mut self, idx: usize, bytes: u32) {
        if idx < MAX_CHANNELS {
            self.channels[idx].rx_window = self.channels[idx].rx_window.saturating_sub(bytes);
        }
    }

    /// Get active shell channel index (if any).
    pub fn get_shell_channel(&self) -> Option<usize> {
        for (i, ch) in self.channels.iter().enumerate() {
            if ch.state == ChannelState::Open && ch.shell_requested {
                return Some(i);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_channel_data_caps_payload_to_peer_limits() {
        let mut mgr = ChannelManager::new();
        let idx = mgr.alloc(7, 12, 8).unwrap();
        let mut buf = [0u8; 64];
        let data = *b"abcdefghijklmnopqrstuvwxyz";

        let written = mgr.build_channel_data(idx, &data, &mut buf);

        assert_eq!(written, 17);
        assert_eq!(buf[0], 94);
        assert_eq!(get_u32(&buf[1..5]), 7);
        assert_eq!(get_u32(&buf[5..9]), 8);
        assert_eq!(&buf[9..17], b"abcdefgh");
        assert_eq!(mgr.channels[idx].tx_window, 4);
    }

    #[test]
    fn build_channel_data_returns_zero_when_window_is_exhausted() {
        let mut mgr = ChannelManager::new();
        let idx = mgr.alloc(3, 0, 32).unwrap();
        let mut buf = [0u8; 32];

        assert_eq!(mgr.build_channel_data(idx, b"hello", &mut buf), 0);
    }
}
