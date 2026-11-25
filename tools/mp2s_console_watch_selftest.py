#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_console_watch_selftest.py - tools/mp2s_console_watch.py over a
pseudo-terminal (plan #362, item console-watch-selftest). No board.

    python3 -I tools/mp2s_console_watch_selftest.py

tools/check_selftests.py lists this file, so tools/check.sh gives it a row;
the tool's built-in --self-test alone is in no row. Each case runs the CLI
(python3 -I) as the board run does and checks its exit code, its output, the
RAW and STAMPED files and that the watcher sent no byte to the console:

- the built-in --self-test passes;
- watch --boots 2 on two image boots and then Linux: exit 0, verdict ok, RAW
  byte for byte, one stamped line per console line;
- watch --boots 2 with Linux never reached: ends at --timeout, exit 1 and
  verdict no_linux; with the second image missing: exit 1, bad_boots;
- a console that answers "busy": exit 4; a path that does not exist: exit 4;
- watch without --boots: ends at --timeout, exit 0;
- scan --boots 2 --json on the STAMPED file; usage errors: exit 2.

No case orders its writes by time. The console text goes into the pty's
input queue before the watcher starts: this process holds the slave open, so
the queue keeps the bytes, and the watcher's open (tcsetattr TCSANOW) does
not flush them. A late start of python on a loaded runner then delays the
read, never loses it. Times in the cases are bounds (--timeout, the wait for
exit), not synchronisation.
"""
import json
import os
import pty
import select
import subprocess
import sys
import tempfile
import time
import tty

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "mp2s_console_watch.py")
FAILED = []
# Linux's N_TTY input queue holds 4096 bytes; a write past it would block
QUEUE_MAX = 2048


def check(name, cond, detail=""):
    print("%s %s%s" % ("ok  " if cond else "FAIL", name, "" if cond else ": " + detail))
    if not cond:
        FAILED.append(name)


def cycle(left, image, linux):
    """One DK boot as its console prints it (three BL2 notices, one cycle)."""
    t = ["NOTICE:  BL2: v2.10-stm32mp2-r2.0(debug)", "NOTICE:  BL2: Built : 16:16:36",
         "NOTICE:  BL2: Booting BL31", "U-Boot 2023.10"]
    if left is not None:
        t.append("mp2z: boots left %d" % left)
    if image:
        t += ["*** Booting Zephyr OS ***", "mp2: done, PSCI SYSTEM_RESET at 1234 ms"]
    if linux:
        t.append("Starting kernel ...")
    return t


def console(lines):
    return "".join(x + "\r\n" for x in lines).encode()


def run(args, bound=60):
    p = subprocess.run([sys.executable, "-I", TOOL] + args, capture_output=True,
                       text=True, timeout=bound)
    return p.returncode, p.stdout + p.stderr


def watch_pty(tmp, name, text, opts, bound=60):
    """Run `watch` on a pty whose input queue already holds `text`.

    Returns (exit code, output, raw bytes, stamped text, bytes sent back,
    seconds the watcher ran)."""
    assert len(text) < QUEUE_MAX, "console text past the pty's input queue"
    raw, st = os.path.join(tmp, name + ".raw"), os.path.join(tmp, name + ".st")
    master, slave = pty.openpty()
    try:
        tty.setraw(slave)
        if text:
            os.write(master, text)
        t0 = time.time()
        rc, out = run(["watch", os.ttyname(slave), raw, st] + opts, bound)
        took = time.time() - t0
        back = b""
        while select.select([master], [], [], 0)[0]:
            got = os.read(master, 4096)
            if not got:
                break
            back += got
    finally:
        os.close(master)
        os.close(slave)

    def rd(path, mode):
        try:
            with open(path, mode) as f:
                return f.read()
        except OSError:
            return b"" if "b" in mode else ""
    return rc, out, rd(raw, "rb"), rd(st, "r"), back, took


def main():
    tmp = os.environ.get("TMPDIR") or None
    tmp = tempfile.mkdtemp(prefix="mp2s-console-watch-selftest.", dir=tmp)

    # the built-in self-test (scan verdicts and a pty watch)
    rc, out = run(["--self-test"], bound=120)
    check("--self-test: exit 0, all passed", rc == 0 and "all passed" in out, out)

    good = cycle(1, 1, 0) + cycle(0, 1, 0) + cycle(None, 0, 1)
    text = console(good)
    rc, out, raw, st, back, _ = watch_pty(
        tmp, "good", text, ["--boots", "2", "--linger", "1", "--timeout", "30"])
    check("watch --boots 2: exit 0, verdict ok", rc == 0 and "verdict: ok (3 cycle(s))" in out, out)
    check("watch --boots 2: boots left 1, then 0", "boot 1: boots left 1," in out
          and "boot 2: boots left 0," in out and "boot 3: boots left -, image no, end no, linux yes" in out, out)
    check("watch: RAW is the console byte for byte", raw == text, repr(raw[:80]))
    lines = st.splitlines()
    stamped_ok = len(lines) == len(good) and all(
        ln.split(" ", 1)[0].replace(".", "", 1).isdigit() and ln.split(" ", 1)[1] == want
        for ln, want in zip(lines, good))
    check("watch: one stamped line per console line", stamped_ok, st[:200])
    check("watch: nothing sent to the console", back == b"", repr(back))

    rc, out = run(["scan", os.path.join(tmp, "good.st"), "--boots", "2", "--json"])
    try:
        js = json.loads(out)
    except ValueError:
        js = {}
    check("scan --json on STAMPED: exit 0, verdict ok, 3 cycles",
          rc == 0 and js.get("verdict") == "ok" and len(js.get("cycles", [])) == 3
          and [c["left"] for c in js["cycles"]] == [[1], [0], []], out)

    rc, out, raw, _, back, took = watch_pty(
        tmp, "nolinux", console(cycle(1, 1, 0) + cycle(0, 1, 0) + cycle(None, 0, 0)),
        ["--boots", "2", "--linger", "1", "--timeout", "2"])
    check("watch, Linux never reached: ends at --timeout, exit 1, no_linux",
          rc == 1 and "verdict: no_linux" in out and took < 30, "%d %.1fs %s" % (rc, took, out))
    check("watch, Linux never reached: nothing sent", back == b"", repr(back))

    rc, out, _, _, back, _ = watch_pty(
        tmp, "noimage", console(cycle(1, 1, 0) + cycle(0, 0, 0) + cycle(None, 0, 1)),
        ["--boots", "2", "--linger", "1", "--timeout", "2"])
    check("watch, second image missing: exit 1, bad_boots",
          rc == 1 and "verdict: bad_boots" in out, out)

    rc, out, _, _, back, _ = watch_pty(
        tmp, "busy", b"console busy\r\n", ["--boots", "2", "--timeout", "30"])
    check("watch, console busy: exit 4", rc == 4 and "busy" in out, out)
    check("watch, console busy: nothing sent", back == b"", repr(back))

    rc, out = run(["watch", os.path.join(tmp, "no-such-tty"),
                   os.path.join(tmp, "x.raw"), os.path.join(tmp, "x.st")])
    check("watch, no such console: exit 4", rc == 4 and "cannot reach the console" in out, out)

    rc, out, raw, _, back, took = watch_pty(
        tmp, "noboots", text, ["--timeout", "2"])
    check("watch without --boots: ends at --timeout, exit 0",
          rc == 0 and raw == text and took < 30, "%d %.1fs %s" % (rc, took, out))
    check("watch without --boots: nothing sent", back == b"", repr(back))

    for name, args in (("no verb", []), ("watch without files", ["watch", "/dev/null"]),
                       ("--boots not a number", ["scan", "x", "--boots", "two"]),
                       ("--timeout without value", ["watch", "a", "b", "c", "--timeout"])):
        rc, _ = run(args)
        check("usage error (%s): exit 2" % name, rc == 2, str(rc))

    for f in os.listdir(tmp):
        os.unlink(os.path.join(tmp, f))
    os.rmdir(tmp)
    print("%d failed" % len(FAILED) if FAILED else "all passed")
    return 1 if FAILED else 0


if __name__ == "__main__":
    sys.exit(main())
