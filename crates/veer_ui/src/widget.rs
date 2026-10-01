//! Widget IR — API-congruent with Veero's `veero-ui`, integer pixels,
//! borrowed strings, arena-built trees.

use crate::{Color, EdgeInsets};

/// Standalone text run. `scale` 1 = 8×16 px, 2 = 16×32 px.
#[derive(Clone, Copy, Debug)]
pub struct Text<'a> {
    pub content: &'a str,
    pub color: Color,
    pub scale: u8,
}

impl<'a> Text<'a> {
    pub const fn new(content: &'a str, color: Color) -> Self {
        Self {
            content,
            color,
            scale: 1,
        }
    }

    pub const fn scale(mut self, scale: u8) -> Self {
        self.scale = if scale < 1 {
            1
        } else if scale > 2 {
            2
        } else {
            scale
        };
        self
    }
}

/// Ordered children with spacing + padding.
#[derive(Clone, Copy, Debug)]
pub struct Children<'a> {
    pub items: &'a [Widget<'a>],
    pub spacing: u32,
    pub padding: EdgeInsets,
}

impl<'a> Children<'a> {
    pub const fn new(items: &'a [Widget<'a>]) -> Self {
        Self {
            items,
            spacing: 8,
            padding: EdgeInsets::zero(),
        }
    }
}

/// Vector icon drawn with strokes (Veero `IconSymbol` subset, integer box).
#[derive(Clone, Copy, Debug)]
pub struct Icon {
    pub symbol: IconSymbol,
    pub size: u32,
    pub color: Color,
}

impl Icon {
    pub const fn new(symbol: IconSymbol, size: u32, color: Color) -> Self {
        Self {
            symbol,
            size,
            color,
        }
    }
}

/// Icon vocabulary v1 (all drawable with lines + circles).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSymbol {
    Check,
    Close,
    Plus,
    Minus,
    ChevronRight,
    ChevronDown,
    ArrowUp,
    ArrowDown,
    Dot,
    Alert,
    Info,
    Net,
}

impl IconSymbol {
    /// All symbols (gallery + exhaustiveness tests).
    pub const ALL: [IconSymbol; 12] = [
        IconSymbol::Check,
        IconSymbol::Close,
        IconSymbol::Plus,
        IconSymbol::Minus,
        IconSymbol::ChevronRight,
        IconSymbol::ChevronDown,
        IconSymbol::ArrowUp,
        IconSymbol::ArrowDown,
        IconSymbol::Dot,
        IconSymbol::Alert,
        IconSymbol::Info,
        IconSymbol::Net,
    ];
}

/// The render-tree node.
#[derive(Clone, Copy, Debug)]
pub enum Widget<'a> {
    Empty,
    Text(Text<'a>),
    Column(Children<'a>),
    Row(Children<'a>),
    /// Vertical gap.
    Spacer(u32),
    /// Full-width 1 px rule (9 px tall incl. padding).
    Divider,
    Icon(Icon),
}
