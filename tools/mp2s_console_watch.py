#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Passive console logger of the STM32MP257F-DK across several boots.

    mp2s_console_watch.py watch HOST:PORT|DEVICE RAW STAMPED [--boots N]
                          [--timeout S] [--linger S]
        Writes the console bytes unchanged to RAW and every completed line
        as "<unix time> <line>" to STAMPED. It never sends a byte: a key at
        U-Boot's autoboot prompt would stop the standalone boot, so this
        tool only reads. With --boots N it ends LINGER s (default 20) after
        Linux has started in boot cycle N+1 (N image boots, then OpenSTLinux
        by itself); it also ends at TIMEOUT s (default 1800) or when the
        console closes. Exit 4: busy or unreachable; else the "scan" exit
        code of the final log when --boots was given, else 0.
    mp2s_console_watch.py scan LOG [--boots N] [--json]
        Prints one line per boot cycle and a verdict. Exit 0 only for "ok".
        LOG is a STAMPED file or a raw capture.
    mp2s_console_watch.py --self-test
        Scan verdicts on synthetic logs and a passive watch on a pseudo-
        terminal (the watcher must read everything and send nothing).

A boot cycle starts at TF-A's "NOTICE:  BL2" line. In a cycle the scan looks
for: "boots left <n>" (the product script's count), "*** Booting Zephyr"
(the image), "PSCI SYSTEM_RESET at <n> ms" (the image's end) and Linux
("Starting kernel ..." / "Linux version"). Verdict "ok": exactly N+1 cycles,
the first N each with the image and its end, the last with Linux and no
image, and the "boots left" values, where printed, counting down from N-1 to
0 (the first N cycles). Otherwise "bad_boots" (wrong cycle count or an image
missing), "no_linux" (the last cycle never reached Linux) or "bad_count"
(boots left not counting down). Without --boots the verdict is "ok" for any
log with at least one cycle.
"""
import io
import json
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
# Read only: the console helpers (open, receive, sink). Its typing is not used.
import mp2_uboot_console as _c  # noqa: E402

LINGER = 20.0
TIMEOUT = 1800.0

BL2_RX = re.compile(r"NOTICE:\s+BL2\b")
LEFT_RX = re.compile(r"boots left (\d+)")
IMAGE_RX = re.compile(r"\*\*\* Booting Zephyr")
END_RX = re.compile(r"PSCI SYSTEM_RESET at \d+ ms")
LINUX_RX = re.compile(r"Starting kernel \.\.\.|Linux version \d")


class Cycles:
    """The boot cycles of a stream of console lines, one line at a time."""

    def __init__(self):
        self.list = []

    def line(self, ln):
        if BL2_RX.search(ln):
            self.list.append({"left": [], "image": False, "end": False, "linux": False})
        if not self.list:
            return
        c = self.list[-1]
        m = LEFT_RX.search(ln)
        if m:
            c["left"].append(int(m.group(1)))
        c["image"] |= bool(IMAGE_RX.search(ln))
        c["end"] |= bool(END_RX.search(ln))
        c["linux"] |= bool(LINUX_RX.search(ln))


def verdict(cs, boots):
    if boots is None:
        return "ok" if cs else "bad_boots"
    if len(cs) != boots + 1:
        return "bad_boots"
    if not all(c["image"] and c["end"] for c in cs[:boots]) or cs[-1]["image"]:
        return "bad_boots"
    if not cs[-1]["linux"]:
        return "no_linux"
    left = [x for c in cs[:boots] for x in c["left"]]
    if left and left != list(range(boots - 1, -1, -1)):
        return "bad_count"
    return "ok"


def read_log(path):
    with open(path, "rb") as f:
        raw = f.read().decode("latin-1")
    cy = Cycles()
    for ln in raw.splitlines():
        m = re.match(r"^\d+\.\d+ (.*)$", ln)
        cy.line(m.group(1) if m else ln)
    return cy.list


def scan(path, boots, as_json=False, out=sys.stdout):
    try:
        cs = read_log(path)
    except OSError as e:
        print("scan: cannot read %s: %s" % (path, e.strerror), file=sys.stderr)
        return 2
    v = verdict(cs, boots)
    if as_json:
        print(json.dumps({"cycles": cs, "verdict": v}), file=out)
    else:
        for i, c in enumerate(cs, 1):
            print("boot %d: boots left %s, image %s, end %s, linux %s" % (
                i, ",".join(map(str, c["left"])) or "-", "yes" if c["image"] else "no",
                "yes" if c["end"] else "no", "yes" if c["linux"] else "no"), file=out)
        print("verdict: %s (%d cycle(s))" % (v, len(cs)), file=out)
    return 0 if v == "ok" else 1


def watch(addr, raw_path, st_path, boots=None, timeout=TIMEOUT, linger=LINGER):
    s = _c._open(addr, lambda m: _c._stderr("watch: " + m))
    if s is None:
        return 4
    s.settimeout(1.0)
    _c._term_exit()
    sink, head, t0 = _c._Sink(raw_path, st_path), b"", time.time()
    cy, pending, done_at = Cycles(), b"", None
    try:
        while True:
            now = time.time()
            if now - t0 > timeout or (done_at and now - done_at > linger):
                break
            chunk, closed = _c._recv(s)
            if closed:
                break
            if not chunk:
                continue
            sink.feed(chunk, time.time())
            if time.time() - t0 < 3 and len(head) < 256:
                head += chunk
                if b"busy" in head:
                    _c._stderr("watch: the console is busy (another client holds it)")
                    return 4
            pending += chunk
            *whole, pending = pending.split(b"\n")
            for ln in whole:
                cy.line(_c._clean(ln))
            if boots is not None and done_at is None and \
                    len(cy.list) == boots + 1 and cy.list[-1]["linux"]:
                done_at = time.time()
    finally:
        sink.close()
        s.close()
    return scan(st_path, boots) if boots is not None else 0


def _cyc(left, image, linux):
    t = ["NOTICE:  BL2: v2.10", "U-Boot 2023.10"]
    if left is not None:
        t.append("mp2z: boots left %d" % left)
    if image:
        t += ["*** Booting Zephyr OS ***", "mp2: done, PSCI SYSTEM_RESET at 1234 ms"]
    if linux:
        t.append("Starting kernel ...")
    return t


def self_test():
    import pty
    import select
    import shutil
    import subprocess
    import tempfile
    import tty
    failed = []

    def check(name, cond):
        print("%s %s" % ("ok  " if cond else "FAIL", name))
        if not cond:
            failed.append(name)

    here = os.path.abspath(__file__)
    tmp = tempfile.mkdtemp(prefix="mp2s-console-watch.")
    try:
        def run_scan(name, lines, boots, want_rc, want_v):
            path = os.path.join(tmp, "log")
            with open(path, "w") as f:
                f.write("\n".join(lines) + "\n")
            out = io.StringIO()
            rc = scan(path, boots, out=out)
            check(name, rc == want_rc and ("verdict: %s " % want_v) in out.getvalue())

        c = _cyc
        run_scan("2 boots then Linux", c(1, 1, 0) + c(0, 1, 0) + c(None, 0, 1), 2, 0, "ok")
        run_scan("0 boots, Linux", c(None, 0, 1), 0, 0, "ok")
        run_scan("image missing", c(1, 1, 0) + c(0, 0, 0) + c(None, 0, 1), 2, 1, "bad_boots")
        run_scan("Linux missing", c(1, 1, 0) + c(0, 1, 0) + c(None, 0, 0), 2, 1, "no_linux")
        run_scan("count not down", c(0, 1, 0) + c(1, 1, 0) + c(None, 0, 1), 2, 1, "bad_count")
        run_scan("too few cycles", c(1, 1, 0) + c(None, 0, 1), 2, 1, "bad_boots")
        run_scan("too many cycles",
                 c(1, 1, 0) + c(0, 1, 0) + c(None, 1, 1) + c(None, 0, 1), 2, 1, "bad_boots")
        run_scan("stamped log",
                 ["1700000000.000 " + x for x in c(1, 1, 0) + c(0, 1, 0) + c(None, 0, 1)],
                 2, 0, "ok")
        check("missing log: exit 2", scan(os.path.join(tmp, "none"), None, out=io.StringIO()) == 2)
        check("unreachable console: exit 4", watch(
            "/nonexistent/tty", os.path.join(tmp, "r0"), os.path.join(tmp, "s0")) == 4)

        # Passive watch on a pty: it must read all of it and send nothing.
        master, slave = pty.openpty()
        tty.setraw(slave)
        p = subprocess.Popen(
            [sys.executable, "-I", here, "watch", os.ttyname(slave),
             os.path.join(tmp, "raw"), os.path.join(tmp, "st"),
             "--boots", "2", "--linger", "1", "--timeout", "30"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        time.sleep(1)
        text = "".join(x + "\r\n" for x in c(1, 1, 0) + c(0, 1, 0) + c(None, 0, 1)).encode()
        os.write(master, text)
        back, end = b"", time.time() + 20
        while p.poll() is None and time.time() < end:
            r, _, _ = select.select([master], [], [], 0.2)
            if r:
                back += os.read(master, 4096)
        if p.poll() is None:
            p.kill()
        out, _err = p.communicate(timeout=5)
        with open(os.path.join(tmp, "raw"), "rb") as f:
            raw = f.read()
        check("watch: exit 0 and verdict ok", p.returncode == 0 and b"verdict: ok" in out)
        check("watch: raw capture is unchanged", raw == text)
        check("watch: nothing sent to the console", back == b"")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("%d failed" % len(failed) if failed else "all passed")
    return 1 if failed else 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    args, opt = [], {"boots": None, "timeout": TIMEOUT, "linger": LINGER, "json": False}
    it = iter(argv[1:])
    try:
        for a in it:
            if a == "--json":
                opt["json"] = True
            elif a in ("--boots", "--timeout", "--linger"):
                v = next(it)
                opt[a[2:]] = int(v) if a == "--boots" else float(v)
            else:
                args.append(a)
    except (StopIteration, ValueError):
        print(__doc__, file=sys.stderr)
        return 2
    if argv[:1] == ["watch"] and len(args) == 3:
        return watch(args[0], args[1], args[2], opt["boots"], opt["timeout"], opt["linger"])
    if argv[:1] == ["scan"] and len(args) == 1:
        return scan(args[0], opt["boots"], opt["json"])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
