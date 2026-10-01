//! Per-frame bump arena: the only "allocation" in `veer_ui`.
//!
//! Views rebuild every frame; the arena hands out disjoint slices from a
//! caller-owned buffer and [`reset`](Arena::reset) reclaims everything at
//! once. No allocator, no fragmentation, deterministic bounds: building a
//! view larger than the buffer panics in debug (and is a loud, early
//! failure, not silent corruption).

use core::fmt;

/// Bump allocator over a caller-owned byte buffer.
pub struct Arena<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Arena<'a> {
    /// Wrap a backing buffer (e.g. a kernel `static mut` region).
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Reclaim the whole buffer for the next frame.
    pub fn reset(&mut self) {
        self.pos = 0;
    }

    /// Bytes consumed so far (for diagnostics / overflow budgets).
    pub fn used(&self) -> usize {
        self.pos
    }

    /// Copy `vals` into the arena, returning a disjoint mutable slice.
    ///
    /// Alignment accounts for the buffer's own base address, so even a
    /// byte-aligned backing buffer yields aligned slices.
    ///
    /// # Panics
    /// Panics when the buffer is exhausted.
    pub fn alloc_slice<T: Copy>(&mut self, vals: &[T]) -> &'a mut [T] {
        let align = core::mem::align_of::<T>();
        let base = self.buf.as_mut_ptr() as usize;
        let start = align_up(base + self.pos, align) - base;
        let bytes = vals.len() * core::mem::size_of::<T>();
        assert!(
            start.saturating_add(bytes) <= self.buf.len(),
            "veer_ui arena exhausted"
        );
        let ptr = unsafe { self.buf.as_mut_ptr().add(start) as *mut T };
        unsafe {
            core::ptr::copy_nonoverlapping(vals.as_ptr(), ptr, vals.len());
        }
        self.pos = start + bytes;
        // SAFETY: bump regions are disjoint and never reused until
        // `reset`, which requires `&mut self` (exclusive access).
        unsafe { &mut *core::ptr::slice_from_raw_parts_mut(ptr, vals.len()) }
    }

    /// Copy a string into the arena.
    pub fn alloc_str(&mut self, s: &str) -> &'a str {
        let bytes = self.alloc_slice(s.as_bytes());
        // SAFETY: copied from valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }

    /// Format arguments into the arena (for dynamic values: counts,
    /// addresses, ticks). Truncates silently at buffer end.
    pub fn alloc_fmt(&mut self, args: fmt::Arguments<'_>) -> &'a str {
        struct Writer<'w> {
            buf: &'w mut [u8],
            len: usize,
        }
        impl fmt::Write for Writer<'_> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                let free = self.buf.len().saturating_sub(self.len);
                let n = s.len().min(free);
                self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
                self.len += n;
                Ok(())
            }
        }
        let start = self.pos;
        let free = self.buf.len().saturating_sub(start);
        let mut w = Writer {
            buf: unsafe { core::slice::from_raw_parts_mut(self.buf.as_mut_ptr().add(start), free) },
            len: 0,
        };
        let _ = core::fmt::write(&mut w, args);
        let n = w.len;
        self.pos = start + n;
        // SAFETY: formatted from valid UTF-8 pieces, possibly truncated at
        // a byte boundary — truncation could split a char. `from_utf8`
        // check below keeps this safe (falls back to empty on split).
        match core::str::from_utf8(&self.buf[start..start + n]) {
            Ok(s) => {
                // Extend lifetime: region is disjoint (see `alloc_slice`).
                unsafe { &*(s as *const str) }
            }
            Err(_) => "",
        }
    }
}

const fn align_up(pos: usize, align: usize) -> usize {
    let mask = align - 1;
    (pos + mask) & !mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_are_disjoint_and_survive_reset() {
        let mut backing = [0u8; 256];
        let mut arena = Arena::new(&mut backing);
        let a = arena.alloc_slice(&[1u32, 2, 3]) as *mut [u32];
        let b = arena.alloc_str("hi");
        assert_eq!(unsafe { &*a }, &[1, 2, 3]);
        assert_eq!(b, "hi");
        assert!(arena.used() > 0);
        arena.reset();
        assert_eq!(arena.used(), 0);
        let c = arena.alloc_str("again");
        assert_eq!(c, "again");
    }

    #[test]
    fn slices_stay_aligned_on_misaligned_base() {
        let mut backing = [0u8; 257];
        let (_, tail) = backing.split_at_mut(1);
        let mut arena = Arena::new(tail);
        let s = arena.alloc_slice(&[1u64, 2, 3]);
        assert_eq!(s.as_ptr() as usize % core::mem::align_of::<u64>(), 0);
        assert_eq!(s, &[1, 2, 3]);
    }

    #[test]
    fn fmt_formats_dynamic_values() {
        let mut backing = [0u8; 128];
        let mut arena = Arena::new(&mut backing);
        let s = arena.alloc_fmt(format_args!("n={} x={:x}", 42, 255));
        assert_eq!(s, "n=42 x=ff");
    }

    /// Reproduce the SystemView build sequence (kernel panicked here).
    #[test]
    fn system_view_alloc_sequence() {
        use crate::{Children, Color, Icon, IconSymbol, Text, Widget};
        let mut backing = [0u8; 65536];
        let mut ui = Arena::new(&mut backing);

        let title = Widget::Text(Text::new("VeeroS", Color(0xE6F0FF)));
        let sub = Widget::Text(Text::new("sub", Color(0x96A3B8)));
        let disp_line = ui.alloc_fmt(format_args!("display {}", "1024x768x32"));
        let r_display = Widget::Row(Children::new(ui.alloc_slice(&[
            Widget::Icon(Icon::new(IconSymbol::Dot, 20, Color(1))),
            Widget::Text(Text::new(disp_line, Color(2))),
        ])));
        let mut strip: [Widget; 12] = [Widget::Empty; 12];
        for (i, s) in IconSymbol::ALL.iter().enumerate() {
            strip[i] = Widget::Icon(Icon::new(*s, 24, Color(3)));
        }
        let r_icons = Widget::Row(Children::new(ui.alloc_slice(&strip)));
        let body = ui.alloc_slice(&[title, sub, r_display, r_icons]);
        let tree = Widget::Column(Children::new(body));
        assert_eq!(ui.used() > 0, true);
        let _ = tree;
    }
}
