//! C runtime stubs required by the Espressif WiFi/BLE blob libraries.
//!
//! The blobs are compiled against Newlib and expect these symbols.
//! We provide minimal implementations sufficient for WiFi operation.

use core::ffi::{c_char, c_int, c_void};

// ═══════════════════════════════════════════════════════════════════════════
// String functions
// ═══════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    let mut len = 0;
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int {
    let mut i = 0;
    loop {
        let a = unsafe { *s1.add(i) } as u8;
        let b = unsafe { *s2.add(i) } as u8;
        if a != b {
            return a as c_int - b as c_int;
        }
        if a == 0 {
            return 0;
        }
        i += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        let a = unsafe { *s1.add(i) } as u8;
        let b = unsafe { *s2.add(i) } as u8;
        if a != b {
            return a as c_int - b as c_int;
        }
        if a == 0 {
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strcpy(dest: *mut c_char, src: *const c_char) -> *mut c_char {
    let mut i = 0;
    loop {
        let c = unsafe { *src.add(i) };
        unsafe { *dest.add(i) = c };
        if c == 0 {
            break;
        }
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strncpy(dest: *mut c_char, src: *const c_char, n: usize) -> *mut c_char {
    let mut i = 0;
    while i < n {
        let c = unsafe { *src.add(i) };
        unsafe { *dest.add(i) = c };
        if c == 0 {
            break;
        }
        i += 1;
    }
    while i < n {
        unsafe { *dest.add(i) = 0 };
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn strtol(s: *const c_char, endptr: *mut *mut c_char, base: c_int) -> i32 {
    let mut i = 0;
    // Skip leading whitespace.
    while unsafe { *s.add(i) } == b' ' as c_char || unsafe { *s.add(i) } == b'\t' as c_char {
        i += 1;
    }
    let neg = unsafe { *s.add(i) } == b'-' as c_char;
    if neg || unsafe { *s.add(i) } == b'+' as c_char {
        i += 1;
    }

    let radix = if base == 0 {
        if unsafe { *s.add(i) } == b'0' as c_char {
            i += 1;
            if unsafe { *s.add(i) } == b'x' as c_char || unsafe { *s.add(i) } == b'X' as c_char {
                i += 1;
                16
            } else {
                8
            }
        } else {
            10
        }
    } else {
        base as u32
    };

    let mut val: i32 = 0;
    loop {
        let c = unsafe { *s.add(i) } as u8;
        let digit = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => break,
        };
        if digit as u32 >= radix {
            break;
        }
        val = val.wrapping_mul(radix as i32).wrapping_add(digit as i32);
        i += 1;
    }

    if !endptr.is_null() {
        unsafe { *endptr = s.add(i) as *mut c_char };
    }
    if neg { -val } else { val }
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory functions
// ═══════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let s = src as *const u8;
    let mut i = 0;
    while i < n {
        unsafe { *d.add(i) = *s.add(i) };
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut c_void, src: *const c_void, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let s = src as *const u8;
    if (d as usize) < (s as usize) {
        let mut i = 0;
        while i < n {
            unsafe { *d.add(i) = *s.add(i) };
            i += 1;
        }
    } else {
        let mut i = n;
        while i > 0 {
            i -= 1;
            unsafe { *d.add(i) = *s.add(i) };
        }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    let d = dest as *mut u8;
    let val = c as u8;
    let mut i = 0;
    while i < n {
        unsafe { *d.add(i) = val };
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const c_void, s2: *const c_void, n: usize) -> c_int {
    let a = s1 as *const u8;
    let b = s2 as *const u8;
    for i in 0..n {
        let diff = unsafe { *a.add(i) } as c_int - unsafe { *b.add(i) } as c_int;
        if diff != 0 {
            return diff;
        }
    }
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// I/O stubs
// ═══════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub unsafe extern "C" fn puts(_s: *const c_char) -> c_int {
    // Blob log output — discard silently.
    0
}

// The blobs call snprintf/sprintf/vsnprintf primarily for logging.
// On stable Rust for riscv32, C-variadic functions are not supported,
// so we provide stub implementations that just null-terminate the buffer.
// The esp-wifi-sys crate also provides its own printf stub.

#[no_mangle]
pub unsafe extern "C" fn snprintf(
    buf: *mut c_char,
    size: usize,
    _fmt: *const c_char,
    // Variadic args follow in the C ABI — we ignore them.
    // This works because the C calling convention passes varargs on the stack
    // and the callee is not obligated to read them.
) -> c_int {
    if !buf.is_null() && size > 0 {
        unsafe { *buf = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sprintf(
    buf: *mut c_char,
    _fmt: *const c_char,
) -> c_int {
    if !buf.is_null() {
        unsafe { *buf = 0 };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn vsnprintf(
    buf: *mut c_char,
    size: usize,
    _fmt: *const c_char,
    _args: *mut c_void,
) -> c_int {
    if !buf.is_null() && size > 0 {
        unsafe { *buf = 0 };
    }
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// Assertion / abort
// ═══════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub unsafe extern "C" fn __assert_func(
    _file: *const c_char,
    _line: c_int,
    _func: *const c_char,
    _failedexpr: *const c_char,
) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[no_mangle]
pub unsafe extern "C" fn abort() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Number formatting helpers (kept for potential future use)
// ═══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
fn format_int(val: i32, buf: &mut [u8; 12]) -> &[u8] {
    let neg = val < 0;
    let mut uval = if neg { (val as i64).unsigned_abs() as u32 } else { val as u32 };
    let mut i = buf.len();
    if uval == 0 {
        i -= 1;
        buf[i] = b'0';
    } else {
        while uval > 0 {
            i -= 1;
            buf[i] = b'0' + (uval % 10) as u8;
            uval /= 10;
        }
    }
    if neg {
        i -= 1;
        buf[i] = b'-';
    }
    &buf[i..]
}

#[allow(dead_code)]
fn format_uint(mut val: u32, buf: &mut [u8; 11]) -> &[u8] {
    let mut i = buf.len();
    if val == 0 {
        i -= 1;
        buf[i] = b'0';
    } else {
        while val > 0 {
            i -= 1;
            buf[i] = b'0' + (val % 10) as u8;
            val /= 10;
        }
    }
    &buf[i..]
}

#[allow(dead_code)]
fn format_hex(mut val: u32, buf: &mut [u8; 9]) -> &[u8] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut i = buf.len();
    if val == 0 {
        i -= 1;
        buf[i] = b'0';
    } else {
        while val > 0 {
            i -= 1;
            buf[i] = HEX[(val & 0xF) as usize];
            val >>= 4;
        }
    }
    &buf[i..]
}
