#!/usr/bin/env python3
"""
VeerOS vi editor test — uses a background reader thread to prevent
USB-CDC buffer deadlock from vi's VT100 output.
"""
import serial
import time
import sys
import threading

PORT = sys.argv[1] if len(sys.argv) > 1 else "/dev/ttyACM1"
BAUD = 115200

ESC = b'\x1b'
CR  = b'\r'

class SerialTester:
    def __init__(self, port, baud):
        self.ser = serial.Serial(port, baud, timeout=0.1)
        self.buf = bytearray()
        self.lock = threading.Lock()
        self.running = True
        self.reader = threading.Thread(target=self._read_loop, daemon=True)
        self.reader.start()

    def _read_loop(self):
        while self.running:
            try:
                chunk = self.ser.read(1024)
                if chunk:
                    with self.lock:
                        self.buf.extend(chunk)
            except Exception:
                break

    def clear(self):
        with self.lock:
            self.buf.clear()

    def get_data(self):
        with self.lock:
            return bytes(self.buf)

    def send(self, data, delay=0.02):
        if isinstance(data, str):
            data = data.encode()
        for b in data:
            self.ser.write(bytes([b]))
            time.sleep(delay)

    def wait_for(self, pattern, timeout=5):
        if isinstance(pattern, str):
            pattern = pattern.encode()
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pattern in self.get_data():
                return True
            time.sleep(0.1)
        return False

    def close(self):
        self.running = False
        self.reader.join(timeout=2)
        self.ser.close()


def check(name, condition, detail=""):
    status = "PASS" if condition else "FAIL"
    msg = f"  [{status}] {name}"
    if detail and not condition:
        msg += f" — {detail}"
    print(msg, flush=True)
    return condition


def main():
    print(f"Connecting to {PORT}...", flush=True)
    t = SerialTester(PORT, BAUD)
    time.sleep(1)

    # Wait for boot and get prompt
    t.clear()
    t.send(CR)
    if not t.wait_for(b'veeros>', timeout=5):
        for _ in range(3):
            t.clear()
            t.send(CR)
            if t.wait_for(b'veeros>', timeout=3):
                break
        else:
            data = t.get_data()
            print(f"ERROR: No prompt. Got {len(data)} bytes: {repr(data[:200])}", flush=True)
            t.close()
            return 1

    print("Shell detected. Starting vi tests...\n", flush=True)
    passed = 0
    failed = 0
    total = 0

    # ── Test 1: Open vi (empty) and :q ───────────────────────────
    print("Test 1: Open vi (empty buffer) and :q", flush=True)
    t.clear()
    t.send(b'vi\r')
    time.sleep(1)

    t.clear()
    t.send(ESC)
    time.sleep(0.1)
    t.send(b':q\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("vi open + :q", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 2: Insert text, ESC, :q! ────────────────────────────
    print("\nTest 2: Insert mode + ESC + :q!", flush=True)
    t.clear()
    t.send(b'vi\r')
    time.sleep(1)

    t.send(b'ihello from test')
    time.sleep(0.2)
    t.send(ESC)
    time.sleep(0.1)
    t.clear()
    t.send(b':q!\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("Insert + ESC + :q!", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 3: :q on dirty buffer (should refuse) ───────────────
    print("\nTest 3: :q on dirty buffer (should refuse)", flush=True)
    t.clear()
    t.send(b'vi\r')
    time.sleep(1)

    t.send(b'imodified')
    time.sleep(0.1)
    t.send(ESC)
    time.sleep(0.1)
    t.clear()
    t.send(b':q\r')
    time.sleep(1)
    data = t.get_data()
    has_prompt = b'veeros>' in data
    total += 1
    if check(":q refused on dirty buffer", not has_prompt, repr(data[-200:])):
        passed += 1
    else:
        failed += 1

    t.send(ESC + b':q!\r')
    t.wait_for(b'veeros>', timeout=3)

    # ── Test 4: :wq ──────────────────────────────────────────────
    print("\nTest 4: Insert + :wq (save and quit)", flush=True)
    t.clear()
    t.send(b'vi\r')
    time.sleep(1)

    t.send(b'iVeerOS rocks!')
    time.sleep(0.1)
    t.send(ESC)
    time.sleep(0.1)
    t.clear()
    t.send(b':wq\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check(":wq save+quit", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 5: Movement (hjkl) + quit ───────────────────────────
    print("\nTest 5: Normal mode movement (hjkl)", flush=True)
    t.clear()
    t.send(b'vi some text here\r')
    time.sleep(1)

    t.send(b'llllhh')
    time.sleep(0.2)
    t.clear()
    t.send(ESC + b':q!\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("hjkl movement + quit", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 6: dd (delete line) ─────────────────────────────────
    print("\nTest 6: o + dd (open line, delete line)", flush=True)
    t.clear()
    t.send(b'vi first\r')
    time.sleep(1)

    t.send(b'osecond')
    time.sleep(0.1)
    t.send(ESC)
    time.sleep(0.1)
    t.send(b'kdd')
    time.sleep(0.2)
    t.clear()
    t.send(ESC + b':q!\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("o + dd + quit", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 7: ZZ ───────────────────────────────────────────────
    print("\nTest 7: ZZ (quick save+quit)", flush=True)
    t.clear()
    t.send(b'vi\r')
    time.sleep(1)

    t.clear()
    t.send(b'ZZ')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("ZZ", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 8: Search (/) ───────────────────────────────────────
    print("\nTest 8: / search then quit", flush=True)
    t.clear()
    t.send(b'vi the quick brown fox\r')
    time.sleep(1)

    t.send(b'/brown\r')
    time.sleep(0.3)
    t.clear()
    t.send(ESC + b':q!\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("/ search + quit", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Test 9: :set number ──────────────────────────────────────
    print("\nTest 9: :set number", flush=True)
    t.clear()
    t.send(b'vi hello\r')
    time.sleep(1)

    t.send(ESC + b':set number\r')
    time.sleep(0.5)
    data = t.get_data()
    has_prompt = b'veeros>' in data
    total += 1
    if check(":set number (stays in vi)", not has_prompt, repr(data[-150:])):
        passed += 1
    else:
        failed += 1

    t.send(ESC + b':q!\r')
    t.wait_for(b'veeros>', timeout=3)

    # ── Test 10: G command ───────────────────────────────────────
    print("\nTest 10: G command + gg", flush=True)
    t.clear()
    t.send(b'vi first line\r')
    time.sleep(1)

    t.send(b'osecond line')
    time.sleep(0.1)
    t.send(ESC)
    time.sleep(0.1)
    t.send(b'gg')
    time.sleep(0.1)
    t.send(b'G')
    time.sleep(0.2)
    t.clear()
    t.send(ESC + b':q!\r')
    found = t.wait_for(b'veeros>', timeout=3)
    total += 1
    if check("G + gg + quit", found, repr(t.get_data()[-100:])):
        passed += 1
    else:
        failed += 1
        t.send(ESC + b':q!\r')
        t.wait_for(b'veeros>', timeout=3)

    # ── Summary ──────────────────────────────────────────────────
    print(f"\n{'='*50}", flush=True)
    print(f"  Results: {passed}/{total} passed, {failed} failed", flush=True)
    print(f"{'='*50}", flush=True)

    t.close()
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
