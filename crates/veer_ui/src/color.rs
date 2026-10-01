//! Packed XRGB8888 color (`0x00RRGGBB` on the wire; alpha unused).

/// Packed color value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    /// Build from 8-bit channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }
}
