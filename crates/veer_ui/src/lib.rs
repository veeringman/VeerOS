//! VeerOS native UI core — the View Runtime half of VeerFlow.
//!
//! `no_std`, zero-`alloc`: widgets borrow from a per-frame bump [`Arena`],
//! so a full view rebuild costs one arena reset. API-congruent with the
//! Veero framework (`veero-ui`/`veero-layout`/`veero-renderer`) so views
//! port 1:1 and the same design language (dark-first, neon accents,
//! 8-pt rhythm) holds from hosted prototypes to the bare-metal console.
//!
//! Pipeline per frame: `View::build` → [`intrinsic`] → [`Painter::paint`].

#![cfg_attr(not(test), no_std)]

pub mod arena;
pub mod color;
pub mod font;
pub mod geom;
pub mod paint;
pub mod theme;
pub mod view;
pub mod widget;

pub use arena::Arena;
pub use color::Color;
pub use geom::{EdgeInsets, Point, Rect, Size};
pub use paint::Painter;
pub use view::{render, View};
pub use widget::{Children, Icon, IconSymbol, Text, Widget};
