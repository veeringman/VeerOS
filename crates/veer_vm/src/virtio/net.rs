//! Virtio-net device — TAP-backed bridge to the Linux host.
//!
//! Implements VIRTIO 1.0 *legacy* semantics, matching the guest driver in
//! `crates/soc/qemu_pc/src/virtio_net.rs`:
//!
//!   * Queue 0 = receiveq, Queue 1 = transmitq.
//!   * Each packet carries a 10-byte `virtio_net_hdr` (no MRG_RXBUF).
//!   * Only `VIRTIO_NET_F_MAC` is advertised — guest reads the 6-byte MAC
//!     from device config at offset 0.
//!
//! The TAP interface must be pre-created on the host (e.g. `ip tuntap add
//! dev tap0 mode tap user $USER`). The VMM opens it as non-blocking;
//! transmission drains the TX ring on each `QUEUE_NOTIFY`, and a dedicated
//! reader thread copies frames from TAP into waiting RX descriptors.

use anyhow::{bail, Context, Result};
use std::os::unix::io::RawFd;

use crate::memory::GuestMem;
use super::{
    pop_avail, push_used, walk_chain, VirtioDevice, VirtioTransport,
};

/// Legacy virtio_net_hdr (10 bytes — no VIRTIO_NET_F_MRG_RXBUF).
const NET_HDR_LEN: usize = 10;
/// Max Ethernet frame we'll copy into an RX descriptor.
const MAX_ETH_FRAME: usize = 1600;

const VIRTIO_NET_F_MAC: u32 = 1 << 5;

pub struct VirtioNet {
    transport: VirtioTransport,
    mac: [u8; 6],
    /// TAP file descriptor (owned). Opened non-blocking; reader thread
    /// polls on it, writers call `write(2)` on it.
    tap_fd: RawFd,
    /// Name of the TAP interface (for logging).
    tap_name: String,
}

impl Drop for VirtioNet {
    fn drop(&mut self) {
        if self.tap_fd >= 0 {
            unsafe { libc::close(self.tap_fd); }
        }
    }
}

impl VirtioNet {
    /// Open `/dev/net/tun` and attach to an existing TAP interface by name.
    pub fn open_tap(ifname: &str, mac: [u8; 6]) -> Result<Self> {
        if ifname.len() >= 16 {
            bail!("TAP interface name '{ifname}' too long (max 15 bytes)");
        }
        let fd = unsafe {
            libc::open(
                b"/dev/net/tun\0".as_ptr() as *const libc::c_char,
                libc::O_RDWR | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            bail!("open /dev/net/tun: {e} (is the tun module loaded? run `sudo modprobe tun`)");
        }

        // struct ifreq: 16-byte name, then union; first u16 of union is ifr_flags.
        #[repr(C)]
        struct IfReqFlags {
            ifr_name: [u8; 16],
            ifr_flags: u16,
            _pad: [u8; 22],
        }
        let mut req = IfReqFlags {
            ifr_name: [0; 16],
            ifr_flags: (libc::IFF_TAP | libc::IFF_NO_PI) as u16,
            _pad: [0; 22],
        };
        req.ifr_name[..ifname.len()].copy_from_slice(ifname.as_bytes());

        // TUNSETIFF = _IOW('T', 202, int) => 0x400454ca
        const TUNSETIFF: libc::c_ulong = 0x400454ca;
        let rc = unsafe { libc::ioctl(fd, TUNSETIFF, &mut req as *mut IfReqFlags) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd); }
            bail!(
                "TUNSETIFF on '{ifname}': {e} (create it first with `ip tuntap add dev {ifname} mode tap user $USER`)"
            );
        }

        // Advertise F_MAC only — guest ignores everything else.
        let transport = VirtioTransport::new(/*num_queues=*/2, VIRTIO_NET_F_MAC);
        Ok(Self {
            transport,
            mac,
            tap_fd: fd,
            tap_name: ifname.to_string(),
        })
    }

    pub fn mac(&self) -> [u8; 6] { self.mac }
    pub fn tap_name(&self) -> &str { &self.tap_name }

    /// Clone the underlying TAP fd (via `dup(2)`) so the RX reader thread
    /// can own its own reference independent of the device's lifetime.
    pub fn dup_tap_fd(&self) -> Result<RawFd> {
        let fd = unsafe { libc::dup(self.tap_fd) };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            bail!("dup TAP fd: {e}");
        }
        Ok(fd)
    }

    /// Drain the transmit queue (queue 1): for each avail chain, concatenate
    /// readable descriptors (starting after the 10-byte virtio_net_hdr) into
    /// a single Ethernet frame and `write(2)` it to the TAP fd. Push the
    /// chain's head onto the used ring so the guest can reclaim it.
    fn drain_tx(&mut self, mem: &GuestMem) -> Result<bool> {
        if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
            eprintln!("[veer-vm/net] tx-notify: drain_tx invoked");
        }
        let mut any = false;
        loop {
            let head = {
                let q = &mut self.transport.queues[1];
                if !q.is_ready() { return Ok(false); }
                match pop_avail(mem, q)? { Some(h) => h, None => break }
            };
            let chain = {
                let q = &self.transport.queues[1];
                walk_chain(mem, q, head)?
            };

            // Concatenate all readable descriptors. Drop the first 10 bytes
            // (virtio_net_hdr) — TAP wants just the raw Ethernet frame.
            let mut buf: Vec<u8> = Vec::with_capacity(MAX_ETH_FRAME + NET_HDR_LEN);
            for d in chain.iter().filter(|d| !d.writable) {
                let slice = mem.slice_mut(d.addr, d.len as usize)?;
                buf.extend_from_slice(slice);
            }
            if buf.len() > NET_HDR_LEN {
                let frame = &buf[NET_HDR_LEN..];
                if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
                    eprintln!("[veer-vm/net] tx: {} bytes to {}", frame.len(), self.tap_name);
                }
                let rc = unsafe {
                    libc::write(self.tap_fd, frame.as_ptr() as _, frame.len())
                };
                if rc < 0 {
                    let e = std::io::Error::last_os_error();
                    // EAGAIN: TAP buffer full; EIO: link down; just log and
                    // complete the descriptor so the guest isn't stuck.
                    eprintln!("[veer-vm] virtio-net: tap write failed: {e}");
                }
            }

            {
                let q = &mut self.transport.queues[1];
                push_used(mem, q, head, 0)?;
            }
            self.transport.isr_status |= 0x1;
            any = true;
        }
        Ok(any)
    }

    /// External entry point used by the RX reader thread: take a raw
    /// Ethernet frame, prepend a zeroed virtio_net_hdr, pop the next avail
    /// RX descriptor and copy the data into it. Returns `true` if the
    /// frame was delivered, `false` if the RX queue is not ready or no
    /// descriptor is available (caller should drop the frame).
    pub fn deliver_rx_frame(&mut self, mem: &GuestMem, frame: &[u8]) -> Result<bool> {
        let head = {
            let q = &mut self.transport.queues[0];
            if !q.is_ready() {
                if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
                    eprintln!("[veer-vm/net] rx drop: queue 0 not ready");
                }
                return Ok(false);
            }
            match pop_avail(mem, q)? {
                Some(h) => h,
                None => {
                    if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
                        eprintln!("[veer-vm/net] rx drop: no avail desc ({} bytes)", frame.len());
                    }
                    return Ok(false);
                }
            }
        };
        let chain = {
            let q = &self.transport.queues[0];
            walk_chain(mem, q, head)?
        };

        // Build header + payload and scatter across writable descriptors.
        let total_in = NET_HDR_LEN + frame.len();
        let mut remaining: Vec<u8> = Vec::with_capacity(total_in);
        remaining.extend_from_slice(&[0u8; NET_HDR_LEN]);
        remaining.extend_from_slice(frame);

        let mut written: u32 = 0;
        let mut src_off = 0usize;
        for d in chain.iter().filter(|d| d.writable) {
            if src_off >= remaining.len() { break; }
            let dst = mem.slice_mut(d.addr, d.len as usize)?;
            let n = core::cmp::min(dst.len(), remaining.len() - src_off);
            dst[..n].copy_from_slice(&remaining[src_off..src_off + n]);
            src_off += n;
            written += n as u32;
        }

        {
            let q = &mut self.transport.queues[0];
            push_used(mem, q, head, written)?;
        }
        self.transport.isr_status |= 0x1;
        if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
            eprintln!("[veer-vm/net] rx: delivered {} bytes (hdr+frame={})", written, total_in);
        }
        Ok(true)
    }
}

impl VirtioDevice for VirtioNet {
    fn config_read(&self, cfg_offset: u16, buf: &mut [u8]) {
        // Device config layout: [mac: [u8;6] @ 0, status:u16 @ 6, max_vq_pairs:u16 @ 8]
        let mut raw = [0u8; 12];
        raw[0..6].copy_from_slice(&self.mac);
        // status: VIRTIO_NET_S_LINK_UP (bit 0) — not negotiated unless F_STATUS;
        // we don't advertise F_STATUS so guest won't read it, but fill safe defaults.
        raw[6] = 0x01;
        let off = cfg_offset as usize;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = if off + i < raw.len() { raw[off + i] } else { 0 };
        }
    }

    fn transport(&self) -> &VirtioTransport { &self.transport }
    fn transport_mut(&mut self) -> &mut VirtioTransport { &mut self.transport }

    fn notify(&mut self, queue_idx: u16, mem: &GuestMem) -> Result<bool> {
        match queue_idx {
            0 => {
                if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
                    eprintln!("[veer-vm/net] rx-notify (guest posted RX buffers)");
                }
                Ok(false)
            }
            1 => self.drain_tx(mem).context("virtio-net: tx drain"),
            _ => Ok(false),
        }
    }
}
