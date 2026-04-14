//! VIRTIO network device driver (virtio-net-pci, legacy interface).
//!
//! Uses two queues:
//!   - Queue 0: receiveq (device → driver)
//!   - Queue 1: transmitq (driver → device)
//!
//! Each packet on the wire is preceded by a `VirtioNetHeader` (10 or 12 bytes
//! depending on VIRTIO_NET_F_MRG_RXBUF). The header carries checksum and GSO
//! metadata; we zero it for simple passthrough.

use crate::mm::FrameAllocator;
use crate::virtio::{self, Virtqueue, VRING_DESC_F_WRITE, VRING_DESC_F_NEXT};

/// Maximum packet payload size.
pub const MAX_PACKET_SIZE: usize = 1514;  // standard Ethernet MTU
/// VIRTIO net header size (without mergeable rx buffer feature).
pub const NET_HEADER_SIZE: usize = 10;
/// Total buffer size per RX descriptor.
pub const RX_BUF_SIZE: usize = NET_HEADER_SIZE + MAX_PACKET_SIZE;
/// Number of pre-posted RX buffers.
pub const RX_RING_SIZE: usize = 16;

/// VIRTIO net features.
#[allow(dead_code)]
const VIRTIO_NET_F_CSUM: u32 = 1 << 0;
#[allow(dead_code)]
const VIRTIO_NET_F_MAC: u32 = 1 << 5;
#[allow(dead_code)]
const VIRTIO_NET_F_STATUS: u32 = 1 << 16;
#[allow(dead_code)]
const VIRTIO_NET_F_MRG_RXBUF: u32 = 1 << 15;

/// VIRTIO net header (10 bytes, legacy without merge-able RX buffers).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtioNetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}

impl VirtioNetHeader {
    pub const EMPTY: Self = Self {
        flags: 0,
        gso_type: 0,
        hdr_len: 0,
        gso_size: 0,
        csum_start: 0,
        csum_offset: 0,
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// Network device
// ═══════════════════════════════════════════════════════════════════════════

/// RX buffer pool (statically allocated).
#[repr(C, align(16))]
struct RxBuffers {
    bufs: [[u8; RX_BUF_SIZE]; RX_RING_SIZE],
}

static mut RX_POOL: RxBuffers = RxBuffers {
    bufs: [[0u8; RX_BUF_SIZE]; RX_RING_SIZE],
};

/// A VIRTIO network device.
pub struct VirtioNet {
    /// I/O base address (from PCI BAR0).
    pub io_base: u16,
    /// Receive queue.
    pub rxq: Option<Virtqueue>,
    /// Transmit queue.
    pub txq: Option<Virtqueue>,
    /// MAC address.
    pub mac: [u8; 6],
    /// Whether the device is initialized.
    pub active: bool,
    /// TX header (reused for each packet).
    tx_header: VirtioNetHeader,
}

impl VirtioNet {
    pub const fn new() -> Self {
        Self {
            io_base: 0,
            rxq: None,
            txq: None,
            mac: [0; 6],
            active: false,
            tx_header: VirtioNetHeader::EMPTY,
        }
    }

    /// Initialize the VIRTIO network device.
    ///
    /// `io_base` — I/O port base from PCI BAR0.
    /// `alloc` — frame allocator for virtqueue memory.
    pub fn init(&mut self, io_base: u16, alloc: &mut FrameAllocator) -> bool {
        self.io_base = io_base;

        // Reset device.
        virtio::device_reset(io_base);

        // Acknowledge + driver.
        virtio::device_set_status(io_base, virtio::STATUS_ACKNOWLEDGE);
        virtio::device_set_status(io_base, virtio::STATUS_DRIVER);

        // Read features and accept MAC feature only.
        let features = virtio::device_features(io_base);
        let accepted = features & VIRTIO_NET_F_MAC;
        virtio::guest_features(io_base, accepted);

        // Set up receiveq (queue 0).
        match Virtqueue::new(io_base, 0, alloc) {
            Some(q) => self.rxq = Some(q),
            None => {
                virtio::device_set_status(io_base, virtio::STATUS_FAILED);
                return false;
            }
        }

        // Set up transmitq (queue 1).
        match Virtqueue::new(io_base, 1, alloc) {
            Some(q) => self.txq = Some(q),
            None => {
                virtio::device_set_status(io_base, virtio::STATUS_FAILED);
                return false;
            }
        }

        // Read MAC address from device config (at offset 0, 6 bytes).
        if (features & VIRTIO_NET_F_MAC) != 0 {
            for i in 0..6 {
                self.mac[i] = virtio::config_read8(io_base, i as u16);
            }
        }

        // Mark driver OK.
        virtio::device_set_status(io_base, virtio::STATUS_DRIVER_OK);
        self.active = true;

        // Pre-post RX buffers.
        self.replenish_rx();

        true
    }

    /// Pre-post receive buffers to the RX queue.
    fn replenish_rx(&mut self) {
        let rxq = match self.rxq.as_mut() {
            Some(q) => q,
            None => return,
        };

        for i in 0..RX_RING_SIZE {
            if let Some(desc_idx) = rxq.alloc_desc() {
                let buf_ptr = unsafe { RX_POOL.bufs[i].as_mut_ptr() };
                unsafe {
                    let d = &mut *rxq.desc.add(desc_idx as usize);
                    d.addr = buf_ptr as u64;
                    d.len = RX_BUF_SIZE as u32;
                    d.flags = VRING_DESC_F_WRITE; // device writes to this buffer
                    d.next = 0;
                }
                rxq.submit(desc_idx);
            }
        }
    }

    /// Transmit a packet.
    ///
    /// `data` — Ethernet frame payload (without VIRTIO header).
    ///
    /// Returns `true` on success.
    pub fn send(&mut self, data: &[u8]) -> bool {
        if !self.active || data.len() > MAX_PACKET_SIZE {
            return false;
        }

        let txq = match self.txq.as_mut() {
            Some(q) => q,
            None => return false,
        };

        if txq.free_count < 2 {
            // Try to reclaim used descriptors.
            while let Some((head, _)) = txq.poll_used() {
                // Free the 2-descriptor chain.
                let next = unsafe { (*txq.desc.add(head as usize)).next };
                txq.free_desc(next);
                txq.free_desc(head);
            }
            if txq.free_count < 2 {
                return false;
            }
        }

        let d0 = match txq.alloc_desc() { Some(d) => d, None => return false };
        let d1 = match txq.alloc_desc() {
            Some(d) => d,
            None => { txq.free_desc(d0); return false }
        };

        // Descriptor 0: VIRTIO net header (device-readable).
        self.tx_header = VirtioNetHeader::EMPTY;
        unsafe {
            let desc = &mut *txq.desc.add(d0 as usize);
            desc.addr = &self.tx_header as *const VirtioNetHeader as u64;
            desc.len = NET_HEADER_SIZE as u32;
            desc.flags = VRING_DESC_F_NEXT;
            desc.next = d1;
        }

        // Descriptor 1: packet data (device-readable).
        unsafe {
            let desc = &mut *txq.desc.add(d1 as usize);
            desc.addr = data.as_ptr() as u64;
            desc.len = data.len() as u32;
            desc.flags = 0; // no more descriptors
            desc.next = 0;
        }

        txq.submit(d0);
        true
    }

    /// Receive a packet (non-blocking).
    ///
    /// `buf` — buffer to receive the Ethernet frame (without VIRTIO header).
    ///
    /// Returns the number of bytes received, or 0 if no packet is available.
    pub fn recv(&mut self, buf: &mut [u8]) -> usize {
        if !self.active {
            return 0;
        }

        let rxq = match self.rxq.as_mut() {
            Some(q) => q,
            None => return 0,
        };

        let (head, total_len) = match rxq.poll_used() {
            Some(x) => x,
            None => return 0,
        };

        let total = total_len as usize;
        if total <= NET_HEADER_SIZE {
            // No actual data — re-post the buffer.
            rxq.free_desc(head);
            return 0;
        }

        let data_len = total - NET_HEADER_SIZE;
        let copy_len = core::cmp::min(data_len, buf.len());

        // Copy data from the RX buffer (after the header) to the user buffer.
        let rx_buf = unsafe { &RX_POOL.bufs[head as usize % RX_RING_SIZE] };
        buf[..copy_len].copy_from_slice(&rx_buf[NET_HEADER_SIZE..NET_HEADER_SIZE + copy_len]);

        // Re-post the RX buffer.
        unsafe {
            let d = &mut *rxq.desc.add(head as usize);
            d.addr = rx_buf.as_ptr() as u64;
            d.len = RX_BUF_SIZE as u32;
            d.flags = VRING_DESC_F_WRITE;
            d.next = 0;
        }
        rxq.submit(head);

        copy_len
    }

    /// Format MAC address into a buffer. Returns bytes written.
    pub fn mac_fmt(&self, buf: &mut [u8; 18]) -> usize {
        fn hex(v: u8) -> u8 {
            if v < 10 { b'0' + v } else { b'a' + v - 10 }
        }
        let mut i = 0;
        for (j, &b) in self.mac.iter().enumerate() {
            buf[i] = hex(b >> 4); i += 1;
            buf[i] = hex(b & 0xF); i += 1;
            if j < 5 { buf[i] = b':'; i += 1; }
        }
        i
    }
}
