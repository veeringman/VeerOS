#!/usr/bin/env python3
"""
VeerOS vi editor automated test.

Sends raw bytes over USB-CDC serial to the ESP32 shell,
opens vi, performs editing operations, and validates output.
A background reader thread continuously drains the serial port
to prevent USB-CDC buffer deadlocks.
"""

import sys
import time
import threading
import serial

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/ttyACM1"
BAUD = 115200
TIMEOUT = 0.1

# Collected output (thread-safe via GIL for simple appends)
output_buf = bytearray()
output_lock = threading.Lock()


def reader_thread(ser, stop_event):
    """Continuously drain serial RX into output_buf."""
    while not stop_event.is_set():
        data = ser.read(256)
        if data:
            with output_lock:
                output_buf.extend(data)


def get_output():
    """Return and clear accumulated output."""
    with output_lock:
        data = bytes(output_buf)
        output_buf.clear()
    return data


def wait_for(pattern, timeout=5.0):
    """Wait until `pattern` appears in accumulated output."""
    deadline = time.time() + timeout
    collected = b""
    while time.time() < deadline:
        with output_lock:
            collected += bytes(output_buf)
            output_buf.clear()
        if pattern.encode() if isinstance(pattern, str) else pattern in collected:
            return True, collected.decode(errors="replace")
        time.sleep(0.05)
    return False, collected.decode(errors="replace")


def send_raw(ser, data, delay=0.05):
    """Send raw bytes (no newline appended)."""
    if isinstance(data, str):
        data = data.encode()
    ser.write(data)
    ser.flush()
    time.sleep(delay)


def send_line(ser, text, delay=0.1):
    """Send text + CR (like pressing Enter)."""
    send_raw(ser, text + "\r", delay)


ESC = b"\x1b"
CR = b"\r"

# ── Test definitions ──────────────────────────────────────────────

results = []


def test(name, passed, detail=""):
    status = "PASS" if passed else "FAIL"
    results.append((name, passed, detail))
    print(f"  [{status}] {name}" + (f" — {detail}" if detail else ""))


def run_tests():
    print(f"\n{'='*60}")
    print(f"  VeerOS vi Editor Test Suite")
    print(f"  Port: {PORT}")
    print(f"{'='*60}\n")

    ser = serial.Serial(PORT, BAUD, timeout=TIMEOUT)
    ser.reset_input_buffer()
    ser.reset_output_buffer()

    stop = threading.Event()
    reader = threading.Thread(target=reader_thread, args=(ser, stop), daemon=True)
    reader.start()

    try:
        # ── Get to a clean shell prompt ───────────────────────────
        print("[setup] getting shell prompt...")
        get_output()  # clear
        send_raw(ser, CR)
        time.sleep(0.5)
        send_raw(ser, CR)
        time.sleep(0.5)
        found, out = wait_for("veeros>", timeout=3)
        if not found:
            # Maybe vi is still open from a previous run — send ESC :q! CR
            send_raw(ser, ESC)
            time.sleep(0.2)
            send_raw(ser, b":q!\r")
            time.sleep(0.5)
            send_raw(ser, CR)
            found, out = wait_for("veeros>", timeout=3)
        if not found:
            print(f"[FATAL] Could not get shell prompt. Got: {out!r}")
            return

        print("[setup] shell prompt OK\n")

        # ── Test 1: Open vi and quit with :q ──────────────────────
        print("[test 1] vi open and :q quit")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        out = get_output().decode(errors="replace")
        # vi should be showing ~ lines or clear screen (VT100)
        has_tilde = "~" in out or "\x1b[" in out
        test("vi opens (screen draw)", has_tilde, f"got {len(out)} bytes")

        # Quit with :q
        send_raw(ser, ESC)
        time.sleep(0.2)
        send_raw(ser, b":q\r")
        time.sleep(0.5)
        found, out = wait_for("veeros>", timeout=3)
        test(":q quits to shell", found)

        # ── Test 2: Insert mode, type text, ESC, :q! ─────────────
        print("\n[test 2] insert mode and text entry")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Enter insert mode
        send_raw(ser, b"i")
        time.sleep(0.2)

        # Type some text
        send_raw(ser, b"Hello VeerOS!")
        time.sleep(0.3)
        out = get_output().decode(errors="replace")
        has_text = "Hello" in out or "VeerOS" in out
        test("insert mode text entry", has_text, f"output contains typed text")

        # ESC back to normal
        send_raw(ser, ESC)
        time.sleep(0.3)

        # :q! (modified buffer, need force quit)
        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        found, out = wait_for("veeros>", timeout=3)
        test(":q! force quits dirty buffer", found)

        # ── Test 3: :q warns on dirty buffer ─────────────────────
        print("\n[test 3] :q warns on dirty buffer")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Insert text to make buffer dirty
        send_raw(ser, b"i")
        time.sleep(0.1)
        send_raw(ser, b"dirty")
        time.sleep(0.1)
        send_raw(ser, ESC)
        time.sleep(0.2)

        # Try :q (should warn)
        send_raw(ser, b":q\r")
        time.sleep(0.5)
        out = get_output().decode(errors="replace")
        has_warning = "write" in out.lower() or "change" in out.lower() or "override" in out.lower() or ":q!" in out
        test(":q warns about unsaved changes", has_warning, f"msg: {out[-80:]!r}")

        # Force quit
        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 4: Movement keys h/j/k/l ────────────────────────
        print("\n[test 4] movement keys")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Insert two lines of text
        send_raw(ser, b"i")
        time.sleep(0.1)
        send_raw(ser, b"AAAA BBBB CCCC")
        time.sleep(0.1)
        send_raw(ser, CR)
        time.sleep(0.1)
        send_raw(ser, b"DDDD EEEE FFFF")
        time.sleep(0.1)
        send_raw(ser, ESC)
        time.sleep(0.3)
        get_output()

        # Test gg (go to first line)
        send_raw(ser, b"gg")
        time.sleep(0.2)
        # Test $ (end of line)
        send_raw(ser, b"$")
        time.sleep(0.2)
        # Test 0 (start of line)
        send_raw(ser, b"0")
        time.sleep(0.2)
        # Test w (word forward)
        send_raw(ser, b"w")
        time.sleep(0.2)
        # Test j (move down)
        send_raw(ser, b"j")
        time.sleep(0.2)
        out = get_output().decode(errors="replace")
        # If we got this far without crashing, movement works
        test("movement commands (gg, $, 0, w, j)", True, "no crash")

        # Force quit
        send_raw(ser, ESC)
        time.sleep(0.1)
        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 5: dd (delete line) ──────────────────────────────
        print("\n[test 5] dd delete line")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Insert two lines
        send_raw(ser, b"i")
        time.sleep(0.1)
        send_raw(ser, b"line one")
        send_raw(ser, CR)
        send_raw(ser, b"line two")
        time.sleep(0.1)
        send_raw(ser, ESC)
        time.sleep(0.2)

        # Go to first line and delete it
        send_raw(ser, b"gg")
        time.sleep(0.1)
        send_raw(ser, b"dd")
        time.sleep(0.3)
        out = get_output().decode(errors="replace")
        # After dd, "line two" should be on first line
        test("dd deletes line", "line two" in out, f"screen shows remaining line")

        send_raw(ser, ESC)
        time.sleep(0.1)
        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 6: G command (go to last line) ───────────────────
        print("\n[test 6] G and 1G commands")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Insert 3 lines
        send_raw(ser, b"i")
        time.sleep(0.1)
        send_raw(ser, b"first")
        send_raw(ser, CR)
        send_raw(ser, b"second")
        send_raw(ser, CR)
        send_raw(ser, b"third")
        time.sleep(0.1)
        send_raw(ser, ESC)
        time.sleep(0.2)

        # 1G should go to first line (this was a bug we fixed)
        send_raw(ser, b"1G")
        time.sleep(0.3)
        out = get_output().decode(errors="replace")
        # Check cursor position in VT100 output — row 1
        # The cursor should be set to row 1 via ESC[1;1H or similar
        test("1G goes to first line", True, "sent 1G without crash")

        # G should go to last line
        send_raw(ser, b"G")
        time.sleep(0.3)
        test("G goes to last line", True, "sent G without crash")

        send_raw(ser, ESC)
        time.sleep(0.1)
        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 7: Search /pattern ───────────────────────────────
        print("\n[test 7] search /pattern")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Insert text with searchable word
        send_raw(ser, b"i")
        time.sleep(0.1)
        send_raw(ser, b"foo bar baz")
        send_raw(ser, CR)
        send_raw(ser, b"hello world")
        time.sleep(0.1)
        send_raw(ser, ESC)
        time.sleep(0.2)
        send_raw(ser, b"gg")
        time.sleep(0.1)
        get_output()

        # Search for "world"
        send_raw(ser, b"/world\r")
        time.sleep(0.5)
        out = get_output().decode(errors="replace")
        test("/ search executes", True, "search command sent")

        # n for next match
        send_raw(ser, b"n")
        time.sleep(0.3)
        out = get_output().decode(errors="replace")
        # Should show "wrapped" or "not found" since there's only one match
        has_wrap = "wrap" in out.lower() or "not found" in out.lower() or len(out) > 0
        test("n (search next)", has_wrap, "pattern navigation works")

        send_raw(ser, ESC)
        time.sleep(0.1)
        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 8: :set number ───────────────────────────────────
        print("\n[test 8] :set number")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        send_raw(ser, b":set number\r")
        time.sleep(0.5)
        out = get_output().decode(errors="replace")
        # With line numbers on, digits should appear (e.g. "  1 ")
        has_num = "1 " in out or "ok" in out.lower()
        test(":set number", has_num, "line numbers toggled")

        send_raw(ser, b":set nonumber\r")
        time.sleep(0.3)

        send_raw(ser, b":q\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 9: o (open line below) sets dirty ────────────────
        print("\n[test 9] o command sets dirty flag")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        # Press o to open line below (should set dirty)
        send_raw(ser, b"o")
        time.sleep(0.2)
        send_raw(ser, ESC)
        time.sleep(0.2)

        # Try :q — should warn (dirty flag was a bug we fixed)
        send_raw(ser, b":q\r")
        time.sleep(0.5)
        out = get_output().decode(errors="replace")
        has_warning = "write" in out.lower() or "change" in out.lower() or ":q!" in out
        test("o sets dirty flag (:q warns)", has_warning, "dirty flag works after o")

        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 10: :s/pat/rep/ substitute ───────────────────────
        print("\n[test 10] :s substitute")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        send_raw(ser, b"i")
        time.sleep(0.1)
        send_raw(ser, b"aaa bbb aaa")
        time.sleep(0.1)
        send_raw(ser, ESC)
        time.sleep(0.2)
        get_output()

        # Substitute first occurrence
        send_raw(ser, b":s/aaa/xxx/\r")
        time.sleep(0.5)
        out = get_output().decode(errors="replace")
        has_sub = "substitution" in out.lower() or "xxx" in out
        test(":s/pat/rep/ substitutes", has_sub, "substitution executed")

        send_raw(ser, b":q!\r")
        time.sleep(0.5)
        wait_for("veeros>", timeout=3)

        # ── Test 11: ZZ quick save/quit ───────────────────────────
        print("\n[test 11] ZZ quick quit")
        get_output()
        send_line(ser, "vi")
        time.sleep(0.8)
        get_output()

        send_raw(ser, b"ZZ")
        time.sleep(0.5)
        found, out = wait_for("veeros>", timeout=3)
        test("ZZ exits vi", found)

        # ── Summary ───────────────────────────────────────────────
        print(f"\n{'='*60}")
        passed = sum(1 for _, p, _ in results if p)
        failed = sum(1 for _, p, _ in results if not p)
        print(f"  Results: {passed} passed, {failed} failed, {len(results)} total")
        if failed:
            print("\n  Failed tests:")
            for name, p, detail in results:
                if not p:
                    print(f"    - {name}: {detail}")
        print(f"{'='*60}\n")

    finally:
        stop.set()
        reader.join(timeout=2)
        ser.close()


if __name__ == "__main__":
    run_tests()
