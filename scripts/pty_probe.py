#!/usr/bin/env python3
"""Drive owt under a direct pty (no tmux: the sandbox reaps tmux servers)
and assert on exact screen state via a minimal ANSI grid parser.

Usage: python3 scripts/pty_probe.py [./target/debug/owt]
"""
import fcntl
import os
import pty
import re
import select
import struct
import sys
import termios
import time

OWT = sys.argv[1] if len(sys.argv) > 1 else "./target/debug/owt"
W, H = 100, 30
results = []


def check(name, cond, extra=""):
    results.append(cond)
    print(("PASS " if cond else "FAIL ") + name + (f"  <{extra}>" if extra and not cond else ""))


class Term:
    """Minimal ANSI grid: cursor addressing, erase, SGR/drop, alt screen."""

    def __init__(self, w, h):
        self.w, self.h = w, h
        self.grid = [[" "] * w for _ in range(h)]
        self.r, self.c = 0, 0

    def feed(self, data: bytes):
        i, n = 0, len(data)
        while i < n:
            b = data[i]
            if b == 0x1B and i + 1 < n and data[i + 1] == ord("["):
                m = re.match(rb"\x1b\[(\?)?([0-9;]*)([a-zA-Z])", data[i:])
                if not m:
                    i += 2
                    continue
                private = m.group(1) is not None
                params, cmd = m.group(2).decode(), m.group(3).decode()
                nums = [int(x) if x else 0 for x in params.split(";")]
                i += m.end()
                self.csi(cmd, nums, private)
            elif b == 0x0D:
                self.c = 0
                i += 1
            elif b == 0x0A:
                self.r = min(self.h - 1, self.r + 1)
                i += 1
            elif b == 0x08:
                self.c = max(0, self.c - 1)
                i += 1
            elif b == 0x07:
                i += 1
            elif b < 0x20:
                i += 1
            else:
                # UTF-8 char
                try:
                    ch = data[i:].decode("utf-8")[0]
                except Exception:
                    i += 1
                    continue
                width = 2 if ord(ch) > 0x1100 and unicodedata.east_asian_width(ch) in "WF" else 1
                if self.c < self.w:
                    self.grid[self.r][self.c] = ch
                    if width == 2 and self.c + 1 < self.w:
                        self.grid[self.r][self.c + 1] = ""
                self.c = min(self.w - 1, self.c + width)
                i += len(ch.encode("utf-8"))
        return self

    def csi(self, cmd, nums, private):
        def p(k, default):
            return nums[k] if len(nums) > k and nums[k] != 0 else default

        if private:
            if cmd in "hl" and 1049 in nums:
                if cmd == "h":
                    self.grid = [[" "] * self.w for _ in range(self.h)]
                    self.r, self.c = 0, 0
            return
        if cmd == "H" or cmd == "f":
            self.r = min(self.h - 1, p(0, 1) - 1)
            self.c = min(self.w - 1, p(1, 1) - 1)
        elif cmd == "A":
            self.r = max(0, self.r - p(0, 1))
        elif cmd == "B":
            self.r = min(self.h - 1, self.r + p(0, 1))
        elif cmd == "C":
            self.c = min(self.w - 1, self.c + p(0, 1))
        elif cmd == "D":
            self.c = max(0, self.c - p(0, 1))
        elif cmd == "G":
            self.c = min(self.w - 1, p(0, 1) - 1)
        elif cmd == "K":
            mode = p(0, 0)
            if mode == 0:
                for c in range(self.c, self.w):
                    self.grid[self.r][c] = " "
            elif mode == 1:
                for c in range(0, self.c + 1):
                    self.grid[self.r][c] = " "
            elif mode == 2:
                self.grid[self.r] = [" "] * self.w
        elif cmd == "J":
            if p(0, 0) == 2:
                self.grid = [[" "] * self.w for _ in range(self.h)]
                self.r, self.c = 0, 0
        elif cmd == "X":
            for c in range(self.c, min(self.w, self.c + p(0, 1))):
                self.grid[self.r][c] = " "

    def rows(self):
        return ["".join(r).rstrip() for r in self.grid]


import unicodedata  # noqa: E402  (after class for readability of feed)


class Session:
    def __init__(self, w=W, h=H, argv=None):
        self.w, self.h = w, h
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.execv(OWT, [OWT, *(argv or [])])
        fcntl.ioctl(
            self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", h, w, 0, 0)
        )
        self.term = Term(w, h)
        self.buf = b""

    def pump(self, timeout=0.5):
        end = time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.time()))
            if not r:
                break
            try:
                chunk = os.read(self.fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            self.term.feed(chunk)

    def send(self, data: bytes):
        os.write(self.fd, data)

    def screen(self):
        self.pump(1.0)
        return self.term.rows()

    def resize(self, w, h):
        self.w, self.h = w, h
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", h, w, 0, 0))
        # Fresh grid: the app redraws fully on resize.
        self.term = Term(w, h)
        time.sleep(0.8)
        return self.screen()

    def close(self, sig=b"\x03\x03"):
        try:
            os.write(self.fd, sig)
            time.sleep(1.0)
        except OSError:
            pass
        try:
            _, status = os.waitpid(self.pid, os.WNOHANG)
            if status == 0:
                # Still alive (or already reaped): check properly.
                pass
        except ChildProcessError:
            return True
        try:
            pid, _ = os.waitpid(self.pid, os.WNOHANG)
            if pid != 0:
                return True
        except ChildProcessError:
            return True
        try:
            os.kill(self.pid, 9)
        except OSError:
            pass
        return False


def main():
    s = Session()
    # Wait for startup marker.
    started = False
    for _ in range(20):
        rows = s.screen()
        if any("retry_with_backoff" in r for r in rows):
            started = True
            break
    check("startup renders transcript", started)

    # Scroll up: top notice must appear.
    s.send(b"\x1b[5~")
    time.sleep(0.4)
    s.send(b"\x1b[5~")
    time.sleep(0.6)
    rows = s.screen()
    check("pgup scrolls up", any("Connected to the mock backend" in r for r in rows))

    # Wheel down x6 at 50,15 -> notice scrolls away; wheel up x12 -> back.
    for _ in range(6):
        s.send(b"\x1b[<65;50;15M")
        time.sleep(0.12)
    rows = s.screen()
    check(
        "wheel-down moves toward newer",
        not any("Connected to the mock backend" in r for r in rows),
    )
    for _ in range(12):
        s.send(b"\x1b[<64;50;15M")
        time.sleep(0.12)
    rows = s.screen()
    check("wheel-up moves toward older", any("Connected to the mock backend" in r for r in rows))

    # Bracketed paste of two lines.
    s.send(b"\x1b[200~pasted one\npasted two\x1b[201~")
    time.sleep(0.8)
    rows = s.screen()
    check(
        "bracketed paste inserts two prompt lines",
        any(r == "> pasted one" for r in rows) and any(r == "  pasted two" for r in rows),
    )
    s.send(b"\x03")
    time.sleep(0.4)

    # Ctrl-j newline.
    s.send(b"ab")
    time.sleep(0.3)
    s.send(b"\x0a")  # NOTE: raw \n; ctrl-j sends \x0a
    time.sleep(0.3)
    s.send(b"cd")
    time.sleep(0.6)
    rows = s.screen()
    check(
        "ctrl-j splits prompt line",
        any(r == "> ab" for r in rows) and any(r == "  cd" for r in rows),
    )
    s.send(b"\x03")
    time.sleep(0.3)

    # Streaming: submit and watch the answer grow across frames.
    s.send(b"stream me please\r")
    time.sleep(0.7)
    mid = s.screen()
    mid_text = " ".join(mid)
    time.sleep(2.5)
    late = s.screen()
    late_text = " ".join(late)
    check(
        "streaming renders progressively",
        "Found it" in mid_text
        and "honors Retry-After" not in mid_text
        and "honors Retry-After" in late_text,
    )

    # Scenario D: permission gate answers with 1.
    s.send(b"/demo d\r")
    time.sleep(1.2)
    rows = s.screen()
    check(
        "permission gate renders",
        any("Permission" in r for r in rows),
    )
    s.send(b"2")
    time.sleep(0.8)
    rows = s.screen()
    check("permission denial resolves", any("Denied" in r for r in rows))

    # Scenario E: question gate answers with 2.
    s.send(b"/demo e\r")
    time.sleep(1.2)
    s.send(b"1")
    time.sleep(0.8)
    rows = s.screen()
    check("question answer resolves", any("Equal jitter" in r for r in rows))

    # Cancel: start the long streaming scenario, ctrl-c stops it early.
    # (Timing-robust: ~27 events need ~1s+; cancel lands at ~0.3s.)
    s.send(b"/demo b\r")
    time.sleep(0.3)
    s.send(b"\x03")
    time.sleep(0.8)
    rows = s.screen()
    frozen = " ".join(rows)
    check(
        "ctrl-c cancels running stream",
        "full text." not in frozen and "Cancelled" in frozen,
    )

    # Ctrl-o opens a new session tab.
    s.send(b"\x0f")
    time.sleep(0.8)
    rows = s.screen()
    check("ctrl-o creates session", any("session 4" in r for r in rows))

    # Help overlay lists the Phase-3 keys.
    s.send(b"?")
    time.sleep(0.6)
    rows = s.screen()
    check(
        "help overlay documents keys",
        any("ctrl-o" in r for r in rows) and any("ctrl-j" in r for r in rows),
    )
    s.send(b"\x1b")
    time.sleep(0.4)

    exited = s.close()
    check("double ctrl-c exits", exited)

    # Resize sweep on a fresh instance: every size must keep all rows
    # within bounds with prompt + statusline visible.
    s2 = Session()
    try:
        for _ in range(20):
            rows = s2.screen()
            if any("retry_with_backoff" in r for r in rows):
                break
        for (w, h) in [(80, 24), (120, 40), (60, 20), (40, 15)]:
            rows = s2.resize(w, h)
            fits = all(len(r) <= w for r in rows) and len(rows) == h
            has_prompt = any(r.startswith(">") for r in rows)
            # Narrow widths truncate the statusline's right side (same as
            # Warp's truncate policy); the model tag must always survive.
            has_status = any("mock-sonnet" in r for r in rows)
            check(f"resize {w}x{h} keeps layout", fits and has_prompt and has_status)
    finally:
        s2.close()

    failed = sum(1 for ok in results if not ok)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
