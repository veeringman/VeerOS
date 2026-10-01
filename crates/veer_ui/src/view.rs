//! View model: build a widget tree per frame, handle keys.
//!
//! Mirrors Veero's `AppModel` contract (`build` + `on_action`) reduced to
//! the console reality: integer keys in, pixels out.

use crate::{Arena, Painter, Widget};

/// A full-screen view: declarative tree + key handling.
pub trait View {
    /// Build the widget tree into the frame arena.
    fn build<'a>(&self, ui: &'a mut Arena) -> Widget<'a>;
    /// Handle a decoded key byte (see `uart_kbd` codes). Return `true`
    /// when the view changed and needs repaint.
    fn on_key(&mut self, key: u8) -> bool;
}

/// Build + paint one frame. The arena is reset first, so callers reuse a
/// single static buffer across frames.
pub fn render<V: View>(view: &V, ui: &mut Arena, p: &mut Painter) {
    ui.reset();
    let tree = view.build(ui);
    let w = p.width;
    p.paint(&tree, 0, 0, w);
}
