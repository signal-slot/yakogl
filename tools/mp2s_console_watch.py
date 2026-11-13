#!/usr/bin/env python3
"""Passive STM32MP257F-DK console logger: reads the UART, never writes it.

usage: mp2s_console_watch.py [--uart PATH] [--baud 115200] -o LOG
                             [--boots K] [--until REGEX] [--timeout SECONDS]

A key typed at U-Boot's autoboot prompt would stop the standalone boot, so
the UART is opened O_RDONLY|O_NOCTTY and no write is ever issued. Each line
goes to LOG with a "[+sss.mmm] " timestamp (seconds since start). A boot is
counted at BL2's banner or its 'Reset reason:' line (one count per boot: a
Reset reason line after a banner of the same boot is not counted again).
The run ends once K boots were seen and --until matched on a line of the
last boot (or later): exit 0. Timeout: exit 4. UART not openable: exit 2.
The UART path is never put in LOG; the header names it as $BOARD_UART.
"""
import argparse
import errno
import os
import re
import select
import sys
import termios
import time

BANNER_RE = re.compile(r"\bBL2:\s+v?\d")
REASON_RE = re.compile(r"Reset reason:")
OPEN_FLAGS = os.O_RDONLY | os.O_NOCTTY | os.O_NONBLOCK
ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
EOL_RE = re.compile(rb"\r\n|\n|\r")

BAUDS = {9600: termios.B9600, 19200: termios.B19200, 38400: termios.B38400,
         57600: termios.B57600, 115200: termios.B115200,
         230400: termios.B230400, 460800: termios.B460800,
         921600: termios.B921600}


def open_uart(path, baud):
    fd = os.open(path, OPEN_FLAGS)
    # read-only, no controlling tty: this process can only read
    assert OPEN_FLAGS & os.O_ACCMODE == os.O_RDONLY and OPEN_FLAGS & os.O_NOCTTY
    try:
        a = termios.tcgetattr(fd)
        a[0] = 0                                  # iflag
        a[1] = 0                                  # oflag
        a[2] = (a[2] & ~(termios.CSIZE | termios.PARENB | termios.CSTOPB)) \
            | termios.CS8 | termios.CREAD | termios.CLOCAL
        a[3] = 0                                  # lflag: raw, no echo
        a[4] = a[5] = BAUDS[baud]
        a[6][termios.VMIN] = 0
        a[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSANOW, a)
    except termios.error:
        pass                                      # not a tty: read anyway
    return fd


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--uart", default=os.environ.get("BOARD_UART"))
    ap.add_argument("--baud", type=int, default=115200, choices=sorted(BAUDS))
    ap.add_argument("-o", "--output", required=True)
    ap.add_argument("--boots", type=int, default=1)
    ap.add_argument("--until", default=None)
    ap.add_argument("--timeout", type=float, default=600.0)
    a = ap.parse_args(argv)
    until = re.compile(a.until) if a.until else None
    if not a.uart:
        print("mp2s_console_watch: no --uart and $BOARD_UART unset", file=sys.stderr)
        return 2
    try:
        fd = open_uart(a.uart, a.baud)
    except OSError as e:
        print("mp2s_console_watch: cannot open the UART: %s" % os.strerror(e.errno),
              file=sys.stderr)
        return 2

    t0 = time.monotonic()
    boots = lines = 0
    banner_open = False   # banner seen, its Reset reason line not yet
    matched = False
    buf = b""
    with open(a.output, "w", encoding="utf-8") as log:
        log.write("# mp2s_console_watch: uart=$BOARD_UART baud=%d boots=%d until=%s "
                  "timeout=%g (read-only, nothing is written to the console)\n"
                  % (a.baud, a.boots, a.until, a.timeout))

        def handle(raw):
            nonlocal boots, lines, banner_open, matched
            text = ANSI_RE.sub("", raw.decode("utf-8", "replace"))
            if BANNER_RE.search(text):
                boots += 1
                banner_open = True
            elif REASON_RE.search(text):
                if banner_open:
                    banner_open = False
                else:
                    boots += 1
            log.write("[+%07.3f] %s\n" % (time.monotonic() - t0, text))
            log.flush()
            lines += 1
            if until and boots >= a.boots and until.search(text):
                matched = True

        while not matched:
            left = a.timeout - (time.monotonic() - t0)
            if left <= 0:
                break
            r, _, _ = select.select([fd], [], [], min(left, 0.2))
            if not r:
                continue
            try:
                chunk = os.read(fd, 4096)
            except OSError as e:
                if e.errno in (errno.EAGAIN, errno.EINTR):
                    continue
                if e.errno != errno.EIO:          # EIO: pty peer gone / unplugged
                    raise
                chunk = b""
            if not chunk:
                time.sleep(0.05)                  # EOF: wait, do not spin
                continue
            buf += chunk
            while not matched:
                m = EOL_RE.search(buf)
                if not m:
                    break
                line, buf = buf[:m.start()], buf[m.end():]
                if line:
                    handle(line)
        if buf and not matched:
            handle(buf)
        log.write("# end: %s boots=%d lines=%d elapsed=%.1fs\n"
                  % ("until-matched" if matched else "timeout", boots, lines,
                     time.monotonic() - t0))
    os.close(fd)
    return 0 if matched else 4


if __name__ == "__main__":
    sys.exit(main())
