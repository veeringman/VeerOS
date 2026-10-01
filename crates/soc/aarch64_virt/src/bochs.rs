//! Bochs display driver (`-device bochs-display` on QEMU `virt`).
//!
//! Programs the Bochs dispi registers through the PCI MMIO bar (BAR2 +
//! `0x500`, flat 16-bit registers) and exposes the linear framebuffer
//! (BAR0) via [`arch::DisplayDevice`]. Pixels are XRGB8888, matching the
//! `0x00RRGGBB` u32 packing used across VeerOS.
//!
//! The MMU is off on this target, so BAR physical addresses are directly
//! accessible — no page-table work needed.

use arch::DisplayDevice;

/// Dispi register indexes.
const DISPI_ID: u16 = 0;
const DISPI_XRES: u16 = 1;
const DISPI_YRES: u16 = 2;
const DISPI_BPP: u16 = 3;
const DISPI_ENABLE: u16 = 4;

/// `ENABLED | LFB`.
const DISPI_ON: u16 = 0x01 | 0x40;

/// Register block offset inside the MMIO BAR.
const DISPI_MMIO_OFFSET: usize = 0x500;

const MODE_W: u32 = 1024;
const MODE_H: u32 = 768;
const MODE_BPP: u16 = 32;

/// Pixel framebuffer on a Bochs display.
pub struct BochsDisplay {
    lfb: usize,
    mmio: usize,
    width: u32,
    height: u32,
    pitch: u32,
}

impl BochsDisplay {
    /// Probe PCI, assign BARs, and switch to `1024×768×32`.
    /// Returns `None` when no Bochs display is present.
    pub fn probe() -> Option<Self> {
        let dev = crate::pcie::find_bochs_display()?;
        let (lfb, mmio) = dev.assign_bars();
        let fb = Self {
            lfb,
            mmio,
            width: MODE_W,
            height: MODE_H,
            pitch: MODE_W * 4,
        };
        fb.reg_write(DISPI_ENABLE, 0x0000);
        fb.reg_write(DISPI_XRES, MODE_W as u16);
        fb.reg_write(DISPI_YRES, MODE_H as u16);
        fb.reg_write(DISPI_BPP, MODE_BPP);
        fb.reg_write(DISPI_ENABLE, DISPI_ON);
        Some(fb)
    }

    #[inline]
    fn reg_write(&self, index: u16, val: u16) {
        let addr = (self.mmio + DISPI_MMIO_OFFSET + ((index as usize) << 1)) as *mut u16;
        unsafe { core::ptr::write_volatile(addr, val) }
    }

    /// Raw framebuffer base (physical == virtual here). For DMA-style
    /// handoff to later compositor stages.
    pub fn framebuffer_base(&self) -> usize {
        self.lfb
    }

    /// Draw the bring-up test pattern: border + color bars + center tile.
    /// Proves the full path (PCI → dispi → LFB → screen) in one screendump.
    pub fn test_pattern(&self) {
        self.clear(0x0014181F);
        // Border.
        self.fill_rect(0, 0, self.width, 8, 0x003DDC97);
        self.fill_rect(0, self.height - 8, self.width, 8, 0x003DDC97);
        self.fill_rect(0, 0, 8, self.height, 0x003DDC97);
        self.fill_rect(self.width - 8, 0, 8, self.height, 0x003DDC97);
        // Color bars.
        const BARS: [u32; 6] = [
            0x00E63946, 0x00F4A261, 0x00E9C46A, 0x002A9D8F, 0x004CC9F0, 0x007B2CBF,
        ];
        let bw = self.width / BARS.len() as u32;
        for (i, c) in BARS.iter().enumerate() {
            self.fill_rect(i as u32 * bw, 64, bw, 96, *c);
        }
        // Center tile.
        let (cw, ch) = (320, 180);
        self.fill_rect(
            (self.width - cw) / 2,
            (self.height - ch) / 2,
            cw,
            ch,
            0x00181D26,
        );
        self.fill_rect(
            (self.width - cw) / 2,
            (self.height - ch) / 2,
            cw,
            6,
            0x003DDC97,
        );
    }
}

impl DisplayDevice for BochsDisplay {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn pitch(&self) -> u32 {
        self.pitch
    }
    fn bpp(&self) -> u32 {
        32
    }
    fn set_pixel(&self, x: u32, y: u32, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let addr = (self.lfb + (y * self.pitch + x * 4) as usize) as *mut u32;
        unsafe { core::ptr::write_volatile(addr, color) }
    }
    fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: u32) {
        let xe = (x + w).min(self.width);
        let ye = (y + h).min(self.height);
        for row in y..ye {
            let mut addr = (self.lfb + (row * self.pitch + x * 4) as usize) as *mut u32;
            for _ in x..xe {
                unsafe {
                    core::ptr::write_volatile(addr, color);
                    addr = addr.add(1);
                }
            }
        }
    }
    fn clear(&self, color: u32) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }
}
