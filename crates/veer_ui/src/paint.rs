//! Software painter: fills, thick lines, circles, 8×16 text, icons.
//!
//! Writes XRGB8888 pixels through a raw pointer (framebuffer or test
//! buffer); all coordinates are clipped. Integer-only and deterministic.

use crate::font;
use crate::{Color, Widget};

/// Pixel sink: `base` points at `width × height` XRGB8888 pixels with
/// `pitch_px` pixels per row.
pub struct Painter {
    base: *mut u32,
    pub width: u32,
    pub height: u32,
    pitch_px: u32,
}

impl Painter {
    /// # Safety
    /// `base` must point at `height × pitch_px` writable u32 pixels for
    /// the painter's lifetime.
    pub unsafe fn new(base: *mut u32, width: u32, height: u32, pitch_px: u32) -> Self {
        Self {
            base,
            width,
            height,
            pitch_px,
        }
    }

    #[inline]
    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        unsafe {
            self.base
                .add((y as u32 * self.pitch_px + x as u32) as usize)
                .write_volatile(color.0);
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w as i32).min(self.width as i32);
        let y1 = (y + h as i32).min(self.height as i32);
        for row in y0..y1 {
            for col in x0..x1 {
                unsafe {
                    self.base
                        .add((row as u32 * self.pitch_px + col as u32) as usize)
                        .write_volatile(color.0);
                }
            }
        }
    }

    /// Filled disc (thickness stamps + dots).
    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: u32, color: Color) {
        let r = r as i32;
        let mut y = -r;
        while y <= r {
            let half = isqrt((r * r - y * y) as u32) as i32;
            self.fill_rect(cx - half, cy + y, (half * 2 + 1) as u32, 1, color);
            y += 1;
        }
    }

    /// 1 px circle outline (midpoint).
    pub fn stroke_circle(&mut self, cx: i32, cy: i32, r: u32, color: Color) {
        let mut x = r as i32;
        let mut y = 0i32;
        let mut err = 1 - x;
        while x >= y {
            for (px, py) in [
                (x, y),
                (y, x),
                (-x, y),
                (-y, x),
                (-x, -y),
                (-y, -x),
                (x, -y),
                (y, -x),
            ] {
                self.set_pixel(cx + px, cy + py, color);
            }
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }

    /// Thick line: Bresenham spine with disc stamps (`thick` px diameter).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color, thick: u32) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        let rad = thick / 2;
        loop {
            if thick <= 1 {
                self.set_pixel(x, y, color);
            } else {
                self.fill_circle(x, y, rad, color);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn frac(&self, size: u32, num: f32) -> i32 {
        (size as f32 * num) as i32
    }

    fn iline(
        &mut self,
        x: i32,
        y: i32,
        u0: f32,
        v0: f32,
        u1: f32,
        v1: f32,
        s: u32,
        c: Color,
        t: u32,
    ) {
        self.line(
            x + self.frac(s, u0),
            y + self.frac(s, v0),
            x + self.frac(s, u1),
            y + self.frac(s, v1),
            c,
            t,
        );
    }

    /// Draw one icon in a `size × size` box at `(x, y)`.
    pub fn icon(&mut self, x: i32, y: i32, size: u32, symbol: crate::IconSymbol, color: Color) {
        use crate::IconSymbol::*;
        let t = (size / 8).max(2);
        let s = size;
        match symbol {
            Check => {
                self.iline(x, y, 0.22, 0.55, 0.45, 0.75, s, color, t);
                self.iline(x, y, 0.45, 0.75, 0.78, 0.25, s, color, t);
            }
            Close => {
                self.iline(x, y, 0.24, 0.24, 0.76, 0.76, s, color, t);
                self.iline(x, y, 0.76, 0.24, 0.24, 0.76, s, color, t);
            }
            Plus => {
                self.iline(x, y, 0.5, 0.22, 0.5, 0.78, s, color, t);
                self.iline(x, y, 0.22, 0.5, 0.78, 0.5, s, color, t);
            }
            Minus => {
                self.iline(x, y, 0.22, 0.5, 0.78, 0.5, s, color, t);
            }
            ChevronRight => {
                self.iline(x, y, 0.38, 0.2, 0.62, 0.5, s, color, t);
                self.iline(x, y, 0.62, 0.5, 0.38, 0.8, s, color, t);
            }
            ChevronDown => {
                self.iline(x, y, 0.2, 0.38, 0.5, 0.62, s, color, t);
                self.iline(x, y, 0.5, 0.62, 0.8, 0.38, s, color, t);
            }
            ArrowUp => {
                self.iline(x, y, 0.5, 0.8, 0.5, 0.2, s, color, t);
                self.iline(x, y, 0.5, 0.2, 0.28, 0.44, s, color, t);
                self.iline(x, y, 0.5, 0.2, 0.72, 0.44, s, color, t);
            }
            ArrowDown => {
                self.iline(x, y, 0.5, 0.2, 0.5, 0.8, s, color, t);
                self.iline(x, y, 0.5, 0.8, 0.28, 0.56, s, color, t);
                self.iline(x, y, 0.5, 0.8, 0.72, 0.56, s, color, t);
            }
            Dot => {
                let r = (size / 5).max(2);
                self.fill_circle(x + size as i32 / 2, y + size as i32 / 2, r, color);
            }
            Alert => {
                self.iline(x, y, 0.5, 0.16, 0.82, 0.78, s, color, t);
                self.iline(x, y, 0.82, 0.78, 0.18, 0.78, s, color, t);
                self.iline(x, y, 0.18, 0.78, 0.5, 0.16, s, color, t);
                self.iline(x, y, 0.5, 0.38, 0.5, 0.58, s, color, t);
            }
            Info => {
                let (cx, cy) = (x + size as i32 / 2, y + size as i32 / 2);
                self.stroke_circle(cx, cy, size / 2 - 1, color);
                self.iline(x, y, 0.5, 0.42, 0.5, 0.68, s, color, t);
                self.fill_circle(x + size as i32 / 2, y + self.frac(s, 0.3), t / 2 + 1, color);
            }
            Net => {
                let (cx, cy) = (x + size as i32 / 2, y + size as i32 / 2);
                let r = size / 2 - 1;
                self.stroke_circle(cx, cy, r, color);
                self.fill_circle(cx, cy, t / 2 + 1, color);
                let rr = r as f32;
                for (dx, dy) in [(-0.7f32, -0.7f32), (0.7, -0.7), (0.0, 1.0)] {
                    let ex = cx + (rr * dx) as i32;
                    let ey = cy + (rr * dy) as i32;
                    self.line(cx, cy, ex, ey, color, 1);
                    self.fill_circle(ex, ey, t / 2 + 1, color);
                }
            }
        }
    }

    /// Draw one glyph (ASCII; others render as block).
    pub fn draw_char(&mut self, x: i32, y: i32, ch: u8, color: Color, scale: u8) {
        let g = font::glyph(ch);
        let s = scale.max(1) as i32;
        for (row, bits) in g.iter().enumerate() {
            for col in 0..8 {
                if bits & (0x80 >> col) != 0 {
                    self.fill_rect(
                        x + col as i32 * s,
                        y + row as i32 * s,
                        s as u32,
                        s as u32,
                        color,
                    );
                }
            }
        }
    }

    /// Draw a string; returns advance width in px.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: Color, scale: u8) -> u32 {
        let s = scale.max(1);
        let mut cx = x;
        for b in text.bytes() {
            self.draw_char(cx, y, b, color, s);
            cx += 8 * s as i32;
        }
        (cx - x).max(0) as u32
    }

    /// Intrinsic (tight) size of a widget.
    pub fn intrinsic(&self, w: &Widget) -> crate::Size {
        use crate::Size;
        match w {
            Widget::Empty => Size::new(0, 0),
            Widget::Text(t) => Size::new(
                t.content.len() as u32 * 8 * t.scale.max(1) as u32,
                16 * t.scale.max(1) as u32,
            ),
            Widget::Column(c) => {
                let (mut mw, mut mh) = (0u32, 0u32);
                for k in c.items {
                    let s = self.intrinsic(k);
                    mw = mw.max(s.w);
                    mh += s.h;
                }
                if !c.items.is_empty() {
                    mh += c.spacing * (c.items.len() as u32 - 1);
                }
                Size::new(
                    mw + c.padding.left + c.padding.right,
                    mh + c.padding.top + c.padding.bottom,
                )
            }
            Widget::Row(r) => {
                let (mut mw, mut mh) = (0u32, 0u32);
                for k in r.items {
                    let s = self.intrinsic(k);
                    mw += s.w;
                    mh = mh.max(s.h);
                }
                if !r.items.is_empty() {
                    mw += r.spacing * (r.items.len() as u32 - 1);
                }
                Size::new(
                    mw + r.padding.left + r.padding.right,
                    mh + r.padding.top + r.padding.bottom,
                )
            }
            Widget::Spacer(h) => Size::new(0, *h),
            Widget::Divider => Size::new(0, 9),
            Widget::Icon(i) => Size::new(i.size, i.size),
        }
    }

    /// Paint a widget with top-left at `(x, y)` inside `avail_w` px.
    pub fn paint(&mut self, w: &Widget, x: i32, y: i32, avail_w: u32) {
        match w {
            Widget::Empty => {}
            Widget::Text(t) => {
                self.draw_text(x, y, t.content, t.color, t.scale);
            }
            Widget::Column(c) => {
                let mut cy = y + c.padding.top as i32;
                for k in c.items {
                    self.paint(k, x + c.padding.left as i32, cy, avail_w);
                    cy += self.intrinsic(k).h as i32 + c.spacing as i32;
                }
            }
            Widget::Row(r) => {
                let mut cx = x + r.padding.left as i32;
                for k in r.items {
                    self.paint(k, cx, y + r.padding.top as i32, avail_w);
                    cx += self.intrinsic(k).w as i32 + r.spacing as i32;
                }
            }
            Widget::Spacer(_) => {}
            Widget::Divider => {
                self.fill_rect(x, y + 4, avail_w, 1, crate::theme::MUTED);
            }
            Widget::Icon(i) => {
                self.icon(x, y, i.size, i.symbol, i.color);
            }
        }
    }
}

/// Integer square root (Newton; deterministic, no floats).
fn isqrt(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    fn canvas(w: u32, h: u32) -> (Vec<u32>, Painter) {
        let mut buf = vec![0u32; (w * h) as usize];
        let ptr = buf.as_mut_ptr();
        // SAFETY: `buf` outlives the painter within each test.
        let p = unsafe { Painter::new(ptr, w, h, w) };
        (buf, p)
    }

    fn painted(buf: &[u32], bg: u32) -> usize {
        buf.iter().filter(|p| **p != bg).count()
    }

    #[test]
    fn rect_and_line_paint() {
        let (buf, mut p) = canvas(64, 64);
        p.fill_rect(4, 4, 8, 8, Color(1));
        assert_eq!(painted(&buf, 0), 64);
        p.line(0, 0, 63, 63, Color(2), 1);
        assert!(painted(&buf, 0) > 64);
    }

    #[test]
    fn every_icon_paints() {
        use crate::IconSymbol;
        for s in IconSymbol::ALL {
            let (buf, mut p) = canvas(48, 48);
            p.icon(8, 8, 32, s, Color(0xFFFFFF));
            assert!(painted(&buf, 0) > 0, "{s:?} painted nothing");
        }
    }

    #[test]
    fn text_draws_glyphs() {
        let (buf, mut p) = canvas(128, 32);
        let w = p.draw_text(0, 0, "VeerOS", Color(0xFFFFFF), 1);
        assert_eq!(w, 6 * 8);
        assert!(painted(&buf, 0) > 50);
    }
}
