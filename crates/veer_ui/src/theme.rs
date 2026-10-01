//! VeeroS dark-first theme (matches the Veero Midnight skin and the
//! VeerFlow visual language: dark-first, neon accents, subtle motion).

use crate::Color;

/// Console background.
pub const BG: Color = Color(0x0014181F);
/// Elevated surface (tiles, pickers).
pub const SURFACE: Color = Color(0x00181D26);
/// Primary text.
pub const TEXT: Color = Color(0x00E6F0FF);
/// Secondary text.
pub const MUTED: Color = Color(0x0096A3B8);
/// Neon accent (confirmations, focus, live indicators).
pub const ACCENT: Color = Color(0x003DDC97);
/// Warnings.
pub const WARN: Color = Color(0x00F4A261);
/// Errors / destructive.
pub const DANGER: Color = Color(0x00E63946);
/// Informational.
pub const INFO: Color = Color(0x004CC9F0);
