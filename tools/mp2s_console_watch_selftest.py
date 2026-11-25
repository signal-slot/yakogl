#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Gate-visible selftest of mp2s_console_watch.py.

usage: tools/mp2s_console_watch_selftest.py

Runs the tool's built-in --self-test, then black-box cases of its real
interface (`watch DEVICE RAW STAMPED [--boots N] [--timeout S] [--linger S]`;
exit 4 is "console unreachable") against a pseudo-terminal this script
creates and feeds. No serial device, no BOARD_UART, no board. It is a .py so
that tools/check.sh's path map (tools/*.py) puts it in the tools scope.
Prints "N checks, M failed".
"""
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
RUN = [sys.executable, "-I", TOOL]
N = 0
BAD = 0


def check(name, ok):
    global N, BAD
    N += 1
    if not ok:
        BAD += 1
    print(("ok   " if ok else "FAIL ") + name)


def cyc(left, image, linux):
    t = ["NOTICE:  BL2: v2.10", "U-Boot 2023.10"]
    if left is not None:
        t.append("mp2z: boots left %d" % left)
    if image:
        t += ["*** Booting Zephyr OS ***", "mp2: done, PSCI SYSTEM_RESET at 1234 ms"]
    if linux:
        t.append("Starting kernel ...")
    return t


FULL = "".join(x + "\r\n" for x in cyc(1, 1, 0) + cyc(0, 1, 0) + cyc(None, 0, 1) + ["login: "]).encode()


def holds(pid, slave):
    try:
        return any(os.path.realpath("/proc/%d/fd/%s" % (pid, f)) == slave
                   for f in os.listdir("/proc/%d/fd" % pid))
    except OSError:
        return False


def watch(mode, d, extra):
    """Run `watch <slave> raw st extra`, feed MODE's bytes; (rc, secs, sent)."""
    m, s = pty.openpty()
    tty.setraw(s)
    slave = os.ttyname(s)
    p = subprocess.Popen(RUN + ["watch", slave, d + "/raw", d + "/st"] + extra,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    t0 = time.time()
    # wait until the tool holds the slave open (a loaded host starts python slowly)
    while not holds(p.pid, slave) and p.poll() is None and time.time() < t0 + 30:
        time.sleep(0.05)
    time.sleep(0.5)
    if mode == "split":
        for i in range(0, len(FULL), 7):  # splits lines mid-way
            os.write(m, FULL[i:i + 7])
            time.sleep(0.01)
    elif mode == "boots":
        os.write(m, FULL)
    back = b""
    while p.poll() is None and time.time() < t0 + 60:
        r, _, _ = select.select([m], [], [], 0.2)
        if r:
            back += os.read(m, 4096)
    if p.poll() is None:
        p.kill()
    p.communicate()
    os.close(m)
    os.close(s)
    return p.returncode, time.time() - t0, len(back)


def read(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError:
        return b""


def main():
    r = subprocess.run(RUN + ["--self-test"], capture_output=True)
    check("built-in --self-test", r.returncode == 0)

    with tempfile.TemporaryDirectory(prefix="mp2s_cw.") as tmp:
        def sub(n):
            d = os.path.join(tmp, n)
            os.mkdir(d)
            return d

        a = sub("a")
        rc, _, sent = watch("boots", a, ["--boots", "2", "--linger", "1", "--timeout", "40"])
        check("three boots + login: exit 0 (rc=%d)" % rc, rc == 0)
        check("three boots: raw log holds every byte", read(a + "/raw") == FULL)
        check("three boots: stamped log has the last count", b"boots left 0" in read(a + "/st"))
        check("three boots: nothing sent to the console", sent == 0)

        b = sub("b")
        rc, _, _ = watch("split", b, ["--boots", "2", "--linger", "1", "--timeout", "40"])
        check("split mid-line: exit 0 (rc=%d)" % rc, rc == 0)
        check("split mid-line: raw log holds every byte", read(b + "/raw") == FULL)
        check("split mid-line: lines reassembled", read(b + "/st").count(b"Starting kernel ...") == 1)

        c = sub("c")
        rc, secs, _ = watch("none", c, ["--boots", "2", "--timeout", "3"])
        check("no boots until timeout: exit 1 within bound (rc=%d, %ds)" % (rc, secs), rc == 1 and secs <= 40)
        rc, _, _ = watch("none", c, ["--timeout", "3"])
        check("no --boots: ends at timeout with exit 0 (rc=%d)" % rc, rc == 0)

        r = subprocess.run(RUN + ["watch", tmp + "/nonexistent", tmp + "/r", tmp + "/s"],
                           capture_output=True, text=True)
        check("missing device: exit 4", r.returncode == 4)
        check("missing device: message", "cannot reach the console" in r.stderr)
        r = subprocess.run(RUN + ["watch", "only-one"], capture_output=True)
        check("bad usage: exit 2", r.returncode == 2)

    print("%d checks, %d failed" % (N, BAD))
    return 1 if BAD else 0


if __name__ == "__main__":
    sys.exit(main())
