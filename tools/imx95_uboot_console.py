#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of the Verdin iMX95 typed U-Boot one-shot from the eMMC.

The route is the one the board sessions typed by hand (docs/notes/
pre-study-imx95.md section 6, docs/standalone-boot.md "Toradex Verdin
iMX95"): Linux stages the image under /var/yakogl on the eMMC's root file
system (U-Boot's mmc 0:2, Linux's /dev/mmcblk0p2); at U-Boot's prompt the
image is loaded with ext4load, its md5 compared with the host's, and entered
with "dcache off; icache flush; go". Nothing is stored: no saveenv, no
bootcmd change, no write to the eMMC (its boot areas or any partition).

    imx95_uboot_console.py capture HOST:PORT|DEVICE RAW STAMPED
        Writes the console bytes unchanged to RAW and every completed line as
        "<unix time> <line>" to STAMPED. Runs until killed or the console
        closes. Exit 4: busy or unreachable.
    imx95_uboot_console.py probe HOST:PORT|DEVICE
        Opens the console once and closes it. 0: free. 3: unreachable.
        4: "busy". 5: accepted and closed at once.
    imx95_uboot_console.py oneshot HOST:PORT|DEVICE RAW STAMPED FILE MD5
                           [--part D:P] [--addr 0xADDR] [--window 0xSIZE]
        "capture", and it drives the one-shot (events go to stderr). It sends
        STOP_KEY every KEY_PERIOD s from U-Boot's "Net: " line (or "Hit any
        key", if the capture started late) until the prompt PROMPT comes,
        then types the load line (FILE, an absolute path in the partition,
        ext4size and the window checked by U-Boot, ext4load, md5sum). When
        U-Boot's md5 is MD5 it types the go line; otherwise it types the
        refusal line, which echoes why and runs bootcmd (Linux). Defaults:
        --part 0:2, --addr 0xd0000000, --window 0x10000000 (the DTS's
        zephyr,sram; driver/boards/toradex/verdin_imx95).
    imx95_uboot_console.py lines FILE MD5 [--part D:P] [--addr A] [--window S]
        Prints the load line, the go line and the refusal line "oneshot"
        would type, one per line (for a --dry-run). Exit 2 if one is refused.
    imx95_uboot_console.py typed HOST:PORT|DEVICE RAW STAMPED LINE...
        As oneshot, but types the given LINEs in order, waiting for the
        prompt between them.
    imx95_uboot_console.py check LINE...
        Exit 0 when every LINE may be typed, 2 (and why on stderr) if not.
    imx95_uboot_console.py scan LOG [--end-re RE] [--json]
        Prints the phases reached and one verdict. Exit 0 only for "done".
    imx95_uboot_console.py scan LOG --back
        Prints nothing. Exit 0 once the cycle is over: Linux started in a
        boot cycle after the image's end, after a refusal, or in a cycle in
        which U-Boot was not stopped.
    imx95_uboot_console.py --self-test

Every line typed, by any subcommand, passes check_line() first, again right
before its bytes are sent: only the commands of the one-shot are allowed
(echo, if/then/else/fi, ext4size, ext4load, md5sum, itest, test, dcache,
icache, go, and "run bootcmd"), and anything that stores the environment or
writes a boot partition, the eMMC or a fuse (saveenv, env save, setenv
bootcmd, mmc write, mmc dev with a hardware partition, ext4write, run
update_uboot, ...) is refused with the reason, before the console is opened.

A typing failure (wrong echo ECHO_TRIES times) ends with Ctrl-C and the one
line "run bootcmd": U-Boot's prompt has no watchdog on this board (no wdt
command), so the board would stay there; the line boots Linux, nothing more.
No prompt after a line (NEXT_BOUND): nothing more is typed.

Phases (what one cycle prints; "vi95z: " lines are the typed lines' echoes):

  spl        "U-Boot SPL 20xx.xx" (a new boot cycle)
  uboot      "U-Boot 20xx.xx"
  stopped    the prompt, i.e. autoboot was stopped
  script     a line "vi95z: ..."
  refused    "vi95z: refused: <why>"
  loaded     "vi95z: loaded <size>"
  go         "vi95z: go 0x<addr>"
  image      "*** Booting Zephyr"
  armed      "verdin_imx95: WDOG3 armed" (the image's WDOG3 self-arm)
  result     the image's end line: "gl_test: all: ", "glmark2: score N: ",
             "deqp: summary: ", or --end-re's
  image_end  "verdin_imx95: <why>: PSCI SYSTEM_RESET at <n> ms"
  watchdog   a new SPL cycle after go without an image_end, or the System
             Manager's "LM Boot reason: fccu" after go
  linux      "Starting kernel ..." / "Linux version"

Verdicts: done (go, image, armed, image_end, then Linux in a new cycle;
with --end-re also the result), refused (the load line or the helper
refused: missing file, too big, md5 mismatch), no_prompt (U-Boot ran on to
Linux, or never offered the prompt), not_armed (the image started and ended
without the WDOG3 marker), no_result (as done, but --end-re's line never
came), watchdog_reset, no_go (stopped, but no jump), not_started (no U-Boot
in the log), hang (the image started but neither ended nor was reset, or it
ended and Linux never came: a capture cut off).
"""
import json
import os
import re
import select
import signal
import socket
import sys
import tempfile
import threading
import time

PROMPT_TEXT = "Verdin iMX95 # "    # Toradex's CONFIG_SYS_PROMPT (seen on the board)
PROMPT = PROMPT_TEXT.encode()
STOP_KEY = b" "                    # any key stops the autoboot; a space runs nothing
KEY_PERIOD = 0.05
KEY_BOUND = 30.0       # no prompt this long after the first key: missed
MISS_WINDOW = 3.0      # "Hit any key" came (bootdelay 1), no prompt this long after: missed
WAIT_START_BOUND = 180.0
QUIET = 0.5
QUIET_BOUND = 5.0
ECHO_BOUND = 5.0
ECHO_TRIES = 3
NEXT_BOUND = 90.0      # ext4load and md5sum of up to 256 MiB, then the prompt
CHUNK, CHUNK_GAP = 16, 0.015
LINE_MAX = 500         # well inside U-Boot's CONFIG_SYS_CBSIZE
BAUD = int(os.environ.get("IMX95_BAUD", "115200"))

DEF_PART = "0:2"
DEF_ADDR = "0xd0000000"
DEF_WINDOW = "0x10000000"
BACK_LINE = "echo vi95z: back to bootcmd; run bootcmd"

START_RX = re.compile(rb"Net: |Hit any key to stop autoboot")
HIT = b"Hit any key to stop autoboot"
KERNEL_RX = re.compile(rb"Starting kernel \.\.\.|Linux version \d")
MD5_RX = re.compile(r"md5 for [0-9a-fA-F]+ \.\.\. [0-9a-fA-F]+ ==> ([0-9a-f]{32})")
STAMP = re.compile(r"^(\d{9,}\.\d+) (.*)$")

END_LINES = r"^(gl_test: all: |glmark2: score [0-9]+: |deqp: summary: )"
PHASE_RX = [
    ("spl", re.compile(r"^U-Boot SPL 20\d\d\.\d+")),
    ("uboot", re.compile(r"^U-Boot 20\d\d\.\d+")),
    ("stopped", re.compile("^" + re.escape(PROMPT_TEXT.rstrip()))),
    ("script", re.compile(r"^vi95z: ")),
    ("refused", re.compile(r"^vi95z: refused:")),
    ("loaded", re.compile(r"^vi95z: loaded ")),
    ("go", re.compile(r"^vi95z: go 0x[0-9a-fA-F]+")),
    ("image", re.compile(r"\*\*\* Booting Zephyr")),
    ("armed", re.compile(r"verdin_imx95: WDOG3 armed")),
    ("result", re.compile(END_LINES)),
    ("image_end", re.compile(r"verdin_imx95: .*PSCI SYSTEM_RESET at \d+ ms")),
    ("fccu", re.compile(r"LM Boot reason: fccu")),
    ("linux", re.compile(r"Starting kernel \.\.\.|Linux version \d")),
]
ALL_PHASES = ["spl", "uboot", "stopped", "script", "refused", "loaded", "go", "image",
              "armed", "result", "image_end", "watchdog", "linux"]


# ---------------------------------------------------------------- the line check

# The commands a typed line may run. "run" only with bootcmd.
ALLOWED = {"echo", "ext4size", "ext4load", "md5sum", "itest", "test", "dcache",
           "icache", "go", "run", "true", "false"}
KEYWORDS = {"if", "then", "else", "elif", "fi", "!"}
# Named refusals, checked on the whole line first (so the reason is specific).
DENY = [
    (re.compile(r"\bsaveenv\b"), "saveenv stores the U-Boot environment"),
    (re.compile(r"\benv\s+(save|erase|default|import|export|set|delete|edit)\b"),
     "env subcommands change or store the environment"),
    (re.compile(r"\b(setenv|editenv|askenv)\b"), "no variable is set (bootcmd least of all)"),
    (re.compile(r"\bfw_setenv\b"), "fw_setenv stores the environment from Linux"),
    (re.compile(r"\b(ext[24]write|fatwrite|fatmkdir|fatrm|ext4mkdir)\b"), "a file system write"),
    (re.compile(r"\brun\s+(?!bootcmd\b)"), "run only runs bootcmd (update_uboot and the like write boot partitions)"),
    (re.compile(r"\brun\s+bootcmd\s*[^\s;]"), "run takes bootcmd alone"),
]
# Named refusals for a command word (mmc is fine as ext4load's argument).
DENY_CMD = {
    "mmc": "mmc (write, erase, dev with a boot area, partconf, ...) can write the eMMC",
    "save": "a file system write",
}
DENY_CMD.update((w, "a command that writes storage, fuses or memory") for w in (
    "gpt", "ums", "fastboot", "dfu", "sf", "nand", "mtd", "bootcount", "fuse", "ahab_close",
    "blob", "mw", "nm", "cp", "erase", "protect", "bmode", "mmcinfo"))


def check_line(line):
    """None if the line may be typed, else why not."""
    if not isinstance(line, str) or not line.strip():
        return "an empty line"
    if "\n" in line or "\r" in line:
        return "more than one line"
    if len(line) > LINE_MAX:
        return "longer than %d characters" % LINE_MAX
    if re.search(r"[^\x20-\x7e]", line):
        return "a control or non-ASCII character"
    if re.search(r"['\"`\\]", line):
        return "a quote or backslash"
    if re.search(r"\$(?!\{filesize\})", line):
        return "a variable other than ${filesize}"
    for rx, why in DENY:
        if rx.search(line):
            return "refused (%s): %s" % (rx.search(line).group(0).strip(), why)
    for seg in re.split(r";|&&|\|\|", line):
        words = seg.split()
        while words and words[0] in KEYWORDS:
            words = words[1:]
        if not words:
            continue
        if words[0] in DENY_CMD:
            return "refused (%s): %s" % (words[0], DENY_CMD[words[0]])
        if words[0] not in ALLOWED:
            return "refused: %s is not a command of the one-shot" % words[0]
        if words[0] == "run" and words[1:] != ["bootcmd"]:
            return "refused: run only runs bootcmd"
    return None


_PATH_RX = re.compile(r"^/[A-Za-z0-9._/-]+$")
_HEX_RX = re.compile(r"^0x[0-9a-fA-F]{1,8}$")


def oneshot_lines(path, md5, part=DEF_PART, addr=DEF_ADDR, window=DEF_WINDOW):
    """(load, go, refuse) lines, or raises ValueError with why."""
    if not _PATH_RX.match(path or "") or "/../" in path + "/" or "//" in path:
        raise ValueError("FILE must be an absolute path of [A-Za-z0-9._/-], no '..'")
    if not re.match(r"^[0-9a-f]{32}$", md5 or ""):
        raise ValueError("MD5 must be 32 lower-case hex digits")
    if not re.match(r"^\d:\d$", part or ""):
        raise ValueError("--part must be D:P")
    if not _HEX_RX.match(addr or "") or not _HEX_RX.match(window or ""):
        raise ValueError("--addr and --window must be 0x hex")
    src = "mmc %s" % part
    load = ("if ext4size %s %s && itest ${filesize} -lt %s && ext4load %s %s %s; "
            "then echo vi95z: loaded ${filesize}; md5sum %s ${filesize}; "
            "else echo vi95z: refused: %s missing, %s bytes or more, or not loaded; fi"
            % (src, path, window, src, addr, path, addr, path, window))
    go = "echo vi95z: go %s; dcache off; icache flush; go %s" % (addr, addr)
    refuse = "echo vi95z: refused: the md5 at the prompt is not the host one; run bootcmd"
    for ln in (load, go, refuse):
        why = check_line(ln)
        if why:
            raise ValueError("built line refused: %s" % why)
    return load, go, refuse


# ---------------------------------------------------------------- the console

class _Tty:
    """A serial device with the few socket calls the loops use."""

    def __init__(self, path):
        import termios
        self.fd = os.open(path, os.O_RDWR | os.O_NOCTTY)
        try:
            attr = termios.tcgetattr(self.fd)
            attr[0] = attr[1] = attr[3] = 0
            attr[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
            speed = getattr(termios, "B%d" % BAUD)
            attr[4] = attr[5] = speed
            attr[6][termios.VMIN], attr[6][termios.VTIME] = 0, 0
            termios.tcsetattr(self.fd, termios.TCSANOW, attr)
        except Exception:
            os.close(self.fd)
            raise
        self.t = None

    def settimeout(self, t):
        self.t = t

    def recv(self, n):
        r, _, _ = select.select([self.fd], [], [], self.t)
        if not r:
            raise socket.timeout()
        try:
            return os.read(self.fd, n)
        except OSError:
            return b""

    def sendall(self, b):
        while b:
            b = b[os.write(self.fd, b):]

    def close(self):
        os.close(self.fd)


def _is_device(addr):
    return addr.startswith("/")


def _open(addr, say):
    """A connection, or None after a message that does not name `addr`."""
    try:
        if _is_device(addr):
            return _Tty(addr)
        host, port = addr.rsplit(":", 1)
        return socket.create_connection((host, int(port)), timeout=10)
    except (OSError, ValueError, AttributeError) as e:
        say("cannot reach the console: %s" % (getattr(e, "strerror", None) or type(e).__name__))
        return None


def _stderr(msg):
    print(msg, file=sys.stderr, flush=True)


def probe(addr):
    s = _open(addr, lambda m: _stderr("probe: " + m))
    if s is None:
        return 3
    if _is_device(addr):
        s.close()
        return 0
    s.settimeout(1.5)
    try:
        data = s.recv(256)
    except socket.timeout:
        return 0
    except OSError as e:
        _stderr("probe: connection failed: %s" % (e.strerror or type(e).__name__))
        return 5
    finally:
        s.close()
    if b"busy" in data:
        _stderr("probe: busy (another client holds the console)")
        return 4
    if not data:
        _stderr("probe: accepted and closed at once")
        return 5
    return 0


def _clean(b):
    return b.replace(b"\r", b"").replace(b"\0", b"").decode("utf-8", "replace")


def _echo_text(b):
    """What U-Boot echoed, as text: CR, NUL and backspace-erase removed."""
    out = bytearray()
    for c in b.replace(b"\r", b"").replace(b"\0", b""):
        if c == 8:
            if out:
                out.pop()
        else:
            out.append(c)
    return out.decode("utf-8", "replace")


class _Sink:
    """RAW and STAMPED files, fed by chunks."""

    def __init__(self, raw_path, st_path):
        self.raw = open(raw_path, "ab", buffering=0)
        self.st = open(st_path, "a", buffering=1)
        self.pending = b""

    def feed(self, chunk, now):
        self.raw.write(chunk)
        self.pending += chunk
        while b"\n" in self.pending:
            ln, self.pending = self.pending.split(b"\n", 1)
            self.st.write("%.3f %s\n" % (now, _clean(ln)))

    def close(self):
        if self.pending:
            self.st.write("%.3f %s\n" % (time.time(), _clean(self.pending)))
            self.pending = b""
        self.raw.close()
        self.st.close()


def _term_exit():
    def on_term(signum, frame):
        raise SystemExit(0)
    if threading.current_thread() is threading.main_thread():
        signal.signal(signal.SIGTERM, on_term)


def _recv(s):
    """(chunk, closed)"""
    try:
        c = s.recv(4096)
        return c, not c
    except socket.timeout:
        return b"", False
    except OSError:
        return b"", True


def capture(addr, raw_path, st_path, log=None):
    say = log or _stderr
    s = _open(addr, lambda m: say("capture: " + m))
    if s is None:
        return 4
    s.settimeout(1.0)
    _term_exit()
    sink, head, t0 = _Sink(raw_path, st_path), b"", time.time()
    try:
        while True:
            chunk, closed = _recv(s)
            if closed:
                return 0
            if not chunk:
                continue
            sink.feed(chunk, time.time())
            if time.time() - t0 < 3 and len(head) < 256:
                head += chunk
                if b"busy" in head:
                    say("capture: the console is busy (another client holds it)")
                    return 4
    finally:
        sink.close()
        s.close()


# ---------------------------------------------------------------- the driver

class _Plan:
    """What to type: first() is the first line; after(n, out) the line to
    type once the prompt came back after n lines (out: U-Boot's output since
    the last CR), or None for nothing more."""

    def __init__(self, lines):
        self.lines = list(lines)

    def first(self):
        return self.lines[0]

    def after(self, n, out, ev):
        return self.lines[n] if n < len(self.lines) else None


class _OneShot(_Plan):
    def __init__(self, path, md5, **kw):
        self.load, self.go, self.refuse = oneshot_lines(path, md5, **kw)
        self.md5 = md5
        self.lines = [self.load]

    def after(self, n, out, ev):
        if n != 1:
            return None
        text = _clean(out)
        if re.search(r"(?m)^vi95z: refused:", text):
            ev("U-Boot refused the load (see the console); back to bootcmd")
            return BACK_LINE
        m = MD5_RX.search(text)
        if not re.search(r"(?m)^vi95z: loaded ", text) or not m:
            ev("REFUSED: no load or no md5 line in U-Boot's output")
            return self.refuse
        if m.group(1) != self.md5:
            ev("REFUSED: md5 at the prompt %s, the host's %s" % (m.group(1), self.md5))
            return self.refuse
        ev("md5 matches the host's: typing the go line")
        return self.go


def _run_plan(addr, raw_path, st_path, plan, log):
    def ev(msg):
        print("%.3f imx95-console: %s" % (time.time(), msg), file=log, flush=True)

    s = _open(addr, ev)
    if s is None:
        return 4
    s.settimeout(0.02)
    _term_exit()
    sink = _Sink(raw_path, st_path)
    try:
        return _loop(s, sink, plan, ev)
    finally:
        sink.close()
        s.close()


def typed(addr, raw_path, st_path, lines, log=None):
    """Capture and type LINEs. 2: a LINE is refused (nothing opened);
    4: busy or unreachable; else 0 when the console closes (or SIGTERM)."""
    log = log or sys.stderr
    if not lines:
        print("typed: no LINE", file=log)
        return 2
    for x in lines:
        why = check_line(x)
        if why:
            print("typed: line not typed, nothing opened: %s" % why, file=log)
            return 2
    return _run_plan(addr, raw_path, st_path, _Plan(lines), log)


def oneshot(addr, raw_path, st_path, path, md5, log=None, **kw):
    log = log or sys.stderr
    try:
        plan = _OneShot(path, md5, **kw)
    except ValueError as e:
        print("oneshot: nothing opened: %s" % e, file=log)
        return 2
    return _run_plan(addr, raw_path, st_path, plan, log)


def _loop(s, sink, plan, ev):
    st = {"state": "wait_start", "t": time.time(), "since": b"", "t_hit": None,
          "t_key": 0.0, "n": 0, "tries": 0, "t_rx": time.time(), "keys": 0,
          "line": plan.first(), "out": b""}
    t_open, head = time.time(), b""
    ev("connected, waiting for U-Boot's Net: line")
    while True:
        now = time.time()
        chunk, closed = _recv(s)
        if closed:
            ev("console closed in state %s" % st["state"])
            return 0
        if chunk:
            sink.feed(chunk, now)
            st["t_rx"] = now
            st["since"] += chunk
            if now - t_open < 3 and len(head) < 256:
                head += chunk
                if b"busy" in head:
                    ev("the console is busy (another client holds it)")
                    return 4
        try:
            _step(s, plan, ev, now, st)
        except OSError as e:
            ev("console error while sending in state %s: %s" % (st["state"], e.strerror or type(e).__name__))
            return 0


def _send_line(s, line):
    why = check_line(line)
    if why:                       # never reached for the built lines; the last guard
        raise ValueError(why)
    b = line.encode()
    for i in range(0, len(b), CHUNK):
        s.sendall(b[i:i + CHUNK])
        time.sleep(CHUNK_GAP)


def _step(s, plan, ev, now, st):
    state, since = st["state"], st["since"]

    def to(new):
        st["state"], st["t"], st["since"] = new, time.time(), b""

    def give_up(msg):
        s.sendall(b"\x03")
        time.sleep(0.2)
        try:
            _send_line(s, "run bootcmd")
            s.sendall(b"\r")
        except ValueError:
            pass
        to("aborted")
        ev("ABORTED: %s; Ctrl-C and run bootcmd typed, nothing more" % msg)

    if state == "wait_start":
        if now - st["t"] > WAIT_START_BOUND:
            to("expired")
            ev("EXPIRED: no U-Boot output within %.0f s of the start; capture only from now on" % WAIT_START_BOUND)
        elif START_RX.search(since):
            to("key")
            ev("U-Boot's Net: seen: sending the stop key every %.0f ms until the prompt" % (KEY_PERIOD * 1000))
    elif state == "key":
        if PROMPT in since:
            to("quiet")
            ev("prompt seen after %d key(s): autoboot stopped" % st["keys"])
        elif KERNEL_RX.search(since):
            to("missed")
            ev("MISSED: the kernel started; nothing typed")
        else:
            if st["t_hit"] is None and HIT in since:
                st["t_hit"] = now
            if st["t_hit"] is not None:
                after = since.split(HIT, 1)[1].split(b"\n")[1:-1]
                if any(_clean(x).strip() for x in after):
                    to("missed")
                    ev("MISSED: bootcmd output after 'Hit any key'; nothing typed")
                    return
            if (st["t_hit"] is not None and now - st["t_hit"] > MISS_WINDOW) or now - st["t"] > KEY_BOUND:
                to("missed")
                ev("MISSED: no prompt; nothing typed")
            elif now - st["t_key"] >= KEY_PERIOD:
                s.sendall(STOP_KEY)
                st["t_key"] = now
                st["keys"] += 1
    elif state == "quiet":
        if now - st["t_rx"] >= QUIET or now - st["t"] > QUIET_BOUND:
            st["tries"] += 1
            try:
                _send_line(s, st["line"])
            except ValueError as e:
                to("aborted")
                ev("ABORTED: line %d refused before sending: %s" % (st["n"] + 1, e))
                return
            to("echo")
            ev("typed line %d attempt %d (%d bytes), waiting for the echo" % (
                st["n"] + 1, st["tries"], len(st["line"])))
    elif state == "echo":
        if st["line"] in _echo_text(since):
            s.sendall(b"\r")
            st["n"] += 1
            st["tries"] = 0
            ev("echo exact: CR sent for line %d" % st["n"])
            to("next")
        elif now - st["t"] > ECHO_BOUND:
            if st["tries"] < ECHO_TRIES:
                ev("echo wrong or incomplete: %r; Ctrl-C and again" % _echo_text(since)[-80:])
                s.sendall(b"\x03")
                to("quiet")
            else:
                give_up("echo wrong %d times" % st["tries"])
    elif state == "next":
        if PROMPT in since:
            out = since.split(PROMPT, 1)[0]
            nxt = plan.after(st["n"], out, ev)
            if nxt is None:
                to("sent")
                ev("all lines typed; capture only from now on")
            else:
                st["line"] = nxt
                to("quiet")
        elif now - st["t"] > NEXT_BOUND:
            to("aborted")
            ev("ABORTED: no prompt within %.0f s of line %d; nothing more typed" % (NEXT_BOUND, st["n"]))


# ---------------------------------------------------------------- scan

def read_lines(path):
    """[(time or None, line)], NULs and CRs removed."""
    with open(path, "rb") as fh:
        raw = fh.read()
    text = raw.replace(b"\0", b"").replace(b"\r", b"").decode("utf-8", "replace")
    out = []
    for line in text.split("\n"):
        m = STAMP.match(line)
        t, body = (float(m.group(1)), m.group(2)) if m else (None, line)
        body = re.sub(r"^[^\x20-\x7e]+", "", body)   # a reset leaves bytes before a banner
        out.append((t, body))
    return out


def scan(lines, end_re=None):
    """Walks the lines once; a "U-Boot SPL" banner starts a new boot cycle.
    Returns {verdict, phases {name: time|True}, cycles, lines, back, result}."""
    end_rx = re.compile(end_re) if end_re else None
    got, cycles, result = {}, 0, None
    cycle_after_end = False
    back = False
    for t, line in lines:
        mark = t if t is not None else True
        names = [n for n, rx in PHASE_RX if rx.search(line)]
        if end_rx is not None and end_rx.search(line) and "result" not in names:
            names.append("result")
        for name in names:
            if name == "spl":
                cycles += 1
                got.setdefault("spl", mark)
                if "go" in got and "image_end" not in got:
                    got.setdefault("watchdog", mark)
                if "image_end" in got or "watchdog" in got:
                    cycle_after_end = True
            elif name == "fccu":
                if "go" in got and "image_end" not in got:
                    got.setdefault("watchdog", mark)
            elif name == "stopped":
                if "uboot" in got:
                    got.setdefault("stopped", mark)
            elif name in ("script", "refused", "loaded", "go"):
                if name == "refused" and "go" in got:
                    continue
                got.setdefault(name, mark)
                got.setdefault("script", mark)
            elif name in ("image", "armed", "result", "image_end"):
                if "go" in got and "watchdog" not in got:
                    got.setdefault(name, mark)
                    if name == "result" and result is None:
                        result = line.strip()
            elif name == "linux":
                got.setdefault("linux", mark)
                if "go" in got:
                    if cycle_after_end:
                        back = True
                elif "refused" in got or "stopped" not in got:
                    back = True
            else:
                got.setdefault(name, mark)
    nlines = sum(1 for _, ln in lines if ln.strip())
    if "go" in got:
        if "image_end" in got and cycle_after_end and back:
            if "armed" not in got:
                verdict = "not_armed"
            elif end_rx is not None and "result" not in got:
                verdict = "no_result"
            else:
                verdict = "done"
        elif "watchdog" in got:
            verdict = "watchdog_reset"
        elif "image" not in got:
            verdict = "no_go"
        else:
            verdict = "hang"
    elif "refused" in got:
        verdict = "refused"
    elif "uboot" not in got:
        verdict = "not_started"
    elif "stopped" not in got:
        verdict = "no_prompt"
    else:
        verdict = "no_go"
    return {"verdict": verdict, "phases": got, "cycles": cycles, "lines": nlines,
            "back": back, "result": result}


def report(res):
    ph = res["phases"]
    t0 = next((ph[k] for k in ("spl", "uboot") if isinstance(ph.get(k), float)), None)
    parts = []
    for name in ALL_PHASES:
        v = ph.get(name)
        if v is None:
            parts.append("%s=absent" % name)
        elif t0 is not None and isinstance(v, float):
            parts.append("%s=+%.1fs" % (name, v - t0))
        else:
            parts.append("%s=yes" % name)
    return "imx95-scan: verdict=%s back=%s lines=%d cycles=%d %s" % (
        res["verdict"], "yes" if res["back"] else "no", res["lines"], res["cycles"], " ".join(parts))


def scan_main(args):
    as_json, back, end_re, paths = False, False, None, []
    it = iter(args)
    for a in it:
        if a == "--json":
            as_json = True
        elif a == "--back":
            back = True
        elif a == "--end-re":
            end_re = next(it, None)
            if end_re is None:
                print(__doc__, file=sys.stderr)
                return 2
        elif a.startswith("--"):
            print(__doc__, file=sys.stderr)
            return 2
        else:
            paths.append(a)
    if len(paths) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        res = scan(read_lines(paths[0]), end_re)
    except OSError as e:
        print("scan: cannot read the log: %s" % (e.strerror or type(e).__name__), file=sys.stderr)
        return 2
    except re.error:
        print("scan: --end-re is not a regular expression", file=sys.stderr)
        return 2
    if back:
        return 0 if res["back"] else 1
    if as_json:
        print(json.dumps(res, sort_keys=True))
    else:
        print(report(res))
    return 0 if res["verdict"] == "done" else 1


# ---------------------------------------------------------------- self-test

SPL = ["U-Boot SPL 2024.04-7.8.0+git.b7c747d8c084 (Jan 01 2025 - 00:00:00 +0000)",
       "SOC: 0xa1009500", "LC: 0x2040010", "LM Boot reason: sw, origin: 2, errid: 1"]
SPL_WD = SPL[:3] + ["LM Boot reason: fccu, origin: -1, errid: 19"]
ATF = ["NOTICE:  BL31: v2.10.0  (release):lf-6.6.52-2.2.1-dirty"]
UBOOT = ["U-Boot 2024.04-7.8.0+git.b7c747d8c084 (Jan 01 2025 - 00:00:00 +0000)",
         "Model: Toradex 0000 Verdin iMX95 Hexa 8GB WB IT", "SM firmware Build 574, Commit cc7a8d2d",
         "Net:   eth0: ethernet@4ccb0000", "Hit any key to stop autoboot:  0 "]
LOAD_OUT = ["1234567 bytes read in 40 ms (29.4 MiB/s)", "vi95z: loaded 12d687"]
IMAGE = ["verdin_imx95: WDOG3 armed: CS 0x00002900 -> 0x000021a1 (on), TOVAL 0x400 -> 0xea60, 60 s",
         "*** Booting Zephyr OS build v4.0 ***", "gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out)",
         "verdin_imx95: done: PSCI SYSTEM_RESET at 15677 ms"]
KERNEL = ["Starting kernel ...", "[    0.000000] Linux version 6.6.157-7.8.0 (oe-user@oe-host)"]
H = "0123456789abcdef0123456789abcdef"


def _md5_line(h):
    return "md5 for d0000000 ... d012d686 ==> " + h


def _log(*parts):
    return "".join(x + "\r\n" for p in parts for x in p)


def _scan_text(text, end_re=None):
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "log")
        with open(p, "w") as f:
            f.write(text)
        return scan(read_lines(p), end_re)


def _cases():
    pr = PROMPT_TEXT
    stop = [pr + "if ext4size mmc 0:2 /var/yakogl/x.bin ... fi"]
    go = [pr + "echo vi95z: go 0xd0000000; dcache off; icache flush; go 0xd0000000",
          "vi95z: go 0xd0000000", "## Starting application at 0xD0000000 ..."]
    boot = SPL + ATF + UBOOT
    return {
        "done": _log(boot, stop, LOAD_OUT, [_md5_line(H)], go, IMAGE, ["\x00\x1b"], SPL, ATF, UBOOT[:4], KERNEL),
        "refused": _log(boot, stop, ["vi95z: refused: /var/yakogl/x.bin missing, 0x10000000 bytes or more, or not loaded"],
                        [pr + BACK_LINE, "vi95z: back to bootcmd"], KERNEL),
        "no_prompt": _log(boot, KERNEL),
        "not_armed": _log(boot, stop, LOAD_OUT, go, ["*** Booting Zephyr OS build v4.0 ***",
                                                     "verdin_imx95: watchdog refused: PSCI SYSTEM_RESET at 3 ms"],
                          SPL, ATF, UBOOT, KERNEL),
        "watchdog_reset": _log(boot, stop, LOAD_OUT, go, IMAGE[:2], SPL_WD, ATF, UBOOT, KERNEL),
        "no_go": _log(boot, stop, LOAD_OUT),
        "not_started": _log(["random noise", "login: "]),
        "hang": _log(boot, stop, LOAD_OUT, go, IMAGE[:2]),
    }


def scan_self_test(check):
    for name, text in _cases().items():
        for stamped in (False, True):
            if stamped:
                text = "".join("%.3f %s\n" % (1700000000.0 + i, x)
                               for i, x in enumerate(text.replace("\r", "").split("\n")))
            res = _scan_text(text)
            check("scan %s%s" % (name, " stamped" if stamped else ""), res["verdict"] == name,
                  "got %s" % res["verdict"])
    check("scan empty: not_started", _scan_text("")["verdict"] == "not_started")
    res = _scan_text(_cases()["done"])
    check("scan done phases", all(k in res["phases"] for k in
                                  ["spl", "uboot", "stopped", "script", "loaded", "go", "image", "armed",
                                   "result", "image_end", "linux"])
          and "watchdog" not in res["phases"] and "refused" not in res["phases"]
          and res["result"].startswith("gl_test: all: 0 of 50"), repr(res))
    res = _scan_text(_cases()["watchdog_reset"])
    check("scan watchdog phase", "watchdog" in res["phases"] and "image_end" not in res["phases"])
    # an echoed typed line (prompt first) is not a phase line
    res = _scan_text(_log(SPL, UBOOT, [PROMPT_TEXT + "echo vi95z: go 0xd0000000; go 0xd0000000"]))
    check("scan: the echo of the go line is not go", "go" not in res["phases"], repr(res["phases"]))
    # --end-re: glmark2's end line
    gl = _cases()["done"].replace("gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out)",
                                  "glmark2: score 123: 33 scenes")
    g = r"glmark2: score [0-9]+: "
    check("scan --end-re glmark2 done", _scan_text(gl, g)["verdict"] == "done")
    nores = _cases()["done"].replace("gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out)", "x")
    check("scan --end-re missing: no_result", _scan_text(nores, g)["verdict"] == "no_result")
    check("scan without --end-re: probe image done", _scan_text(nores)["verdict"] == "done")
    # the image ended but the next cycle has not reached Linux yet: hang, not back
    cut = _cases()["done"].split("Starting kernel")[0]
    r = _scan_text(cut)
    check("scan: ended, Linux not yet back: hang, back=no", r["verdict"] == "hang" and not r["back"], repr(r))
    # --back and exit codes
    for name, want in (("done", 0), ("refused", 0), ("no_prompt", 0), ("watchdog_reset", 0),
                       ("no_go", 1), ("hang", 1)):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "l")
            with open(p, "w") as f:
                f.write(_cases()[name])
            rc = scan_main([p, "--back"])
            check("scan --back %s exit %d" % (name, want), rc == want, "rc %d" % rc)
            with open(os.devnull, "w") as dn:
                old, sys.stdout = sys.stdout, dn
                try:
                    rc = scan_main([p])
                finally:
                    sys.stdout = old
            check("scan exit %s" % name, rc == (0 if name == "done" else 1), "rc %d" % rc)
    check("scan json", json.loads(json.dumps(_scan_text(_cases()["done"])))["verdict"] == "done")


def check_self_test(check):
    refused = [
        "saveenv",
        "setenv bootcmd run yako_hook; saveenv",
        "setenv bootcmd bootflow scan -b",
        "env save",
        "env set bootdelay 0",
        "echo x; saveenv",
        "mmc write 0xd0000000 0 0x800",
        "mmc dev 0 1",
        "mmc partconf 0 1 1 0",
        "mmc erase 0 0x800",
        "ext4write mmc 0:2 0xd0000000 /x 0x10",
        "fatwrite mmc 0:1 0xd0000000 x 10",
        "run update_uboot",
        "run bootcmd; saveenv",
        "fuse prog 2 2 0x1",
        "gpt write mmc 0 x",
        "ums 0 mmc 0",
        "fastboot 0",
        "fw_setenv bootcmd x",
        "go ${loadaddr}",
        "echo 'x'",
        "bootm 0xd0000000",
        "if true; then saveenv; fi",
        "",
        "echo a\rb",
        "echo " + "x" * LINE_MAX,
    ]
    for ln in refused:
        why = check_line(ln)
        check("check refuses %r" % ln[:40], why is not None, "allowed")
    for kw in ("saveenv", "mmc write", "env save"):
        why = check_line({"saveenv": "saveenv", "mmc write": "mmc write 0xd0000000 0 8",
                          "env save": "env save"}[kw])
        check("check names the reason for %s" % kw, why is not None and "refused" in why, repr(why))
    ok = list(oneshot_lines("/var/yakogl/gl_test.bin", H)) + [BACK_LINE, "run bootcmd"]
    for ln in ok:
        check("check allows %r" % ln[:48], check_line(ln) is None, repr(check_line(ln)))
    load, go, refuse = oneshot_lines("/var/yakogl/gl_test.bin", H)
    check("oneshot load line", load == (
        "if ext4size mmc 0:2 /var/yakogl/gl_test.bin && itest ${filesize} -lt 0x10000000 && "
        "ext4load mmc 0:2 0xd0000000 /var/yakogl/gl_test.bin; then echo vi95z: loaded ${filesize}; "
        "md5sum 0xd0000000 ${filesize}; else echo vi95z: refused: /var/yakogl/gl_test.bin missing, "
        "0x10000000 bytes or more, or not loaded; fi"), load)
    check("oneshot go line", go == "echo vi95z: go 0xd0000000; dcache off; icache flush; go 0xd0000000", go)
    for bad in (("../x", H), ("/var/yakogl/../../x", H), ("/x;saveenv", H), ("/x y", H), ("/x", "ABC"),
                ("/x", H.upper())):
        try:
            oneshot_lines(*bad)
            check("oneshot refuses %r" % (bad,), False, "built")
        except ValueError:
            check("oneshot refuses %r" % (bad,), True)
    for kw in ({"part": "0:2; saveenv"}, {"addr": "0xd0000000; saveenv"}, {"window": "-1"}):
        try:
            oneshot_lines("/x", H, **kw)
            check("oneshot refuses %r" % kw, False, "built")
        except ValueError:
            check("oneshot refuses %r" % kw, True)
    # the CLI forms
    with open(os.devnull, "w") as dn:
        old = sys.stderr
        sys.stderr = dn
        try:
            check("check CLI refuses saveenv", main(["check", "saveenv"]) == 2)
            check("check CLI refuses mmc write", main(["check", "mmc write 0xd0000000 0 8"]) == 2)
            check("check CLI refuses env save", main(["check", "env save"]) == 2)
            check("check CLI allows run bootcmd", main(["check", "run bootcmd"]) == 0)
        finally:
            sys.stderr = old


class _FakeVerdin:
    """The far end of a pty (the serial device the helper opens) or of a
    loopback socket, playing a Verdin iMX95 boot. It echoes what it gets, like
    U-Boot's line editor, and records the bytes. mode: "ok" (keys stop
    autoboot, the file loads, md5 H), "md5bad" (another md5), "missing" (the
    file is not there), "deaf" (keys ignored: Linux boots), "garble" (every
    echo is wrong)."""

    def __init__(self, mode, tcp=False):
        import pty
        self.mode, self.rx, self.prompt_at, self.lines = mode, b"", None, []
        self.tcp = tcp
        if tcp:
            self.srv = socket.socket()
            self.srv.bind((_loopback(), 0))
            self.srv.listen(1)
            self.addr = "%s:%d" % (_loopback(), self.srv.getsockname()[1])
        else:
            import tty
            self.m, self.sl = pty.openpty()
            tty.setraw(self.sl)        # no line discipline echo before the helper opens it
            self.addr = os.ttyname(self.sl)
        self.t = threading.Thread(target=self.run, daemon=True)
        self.t.start()

    # the far end's I/O, the same for both
    def send(self, b):
        if self.tcp:
            self.c.sendall(b)
        else:
            os.write(self.m, b)

    def recv(self, t=0.05):
        if self.tcp:
            r, _, _ = select.select([self.c], [], [], t)
            return self.c.recv(256) if r else b""
        r, _, _ = select.select([self.m], [], [], t)
        try:
            return os.read(self.m, 256) if r else b""
        except OSError:
            return b""

    def out(self, ln, gap=0.01):
        self.send(ln.encode() + b"\r\n")
        time.sleep(gap)

    def close(self):
        time.sleep(0.3)
        if self.tcp:
            self.c.close()
            self.srv.close()
        else:
            os.close(self.m)
            os.close(self.sl)

    def run(self):
        try:
            if self.tcp:
                self.c, _ = self.srv.accept()
            else:
                time.sleep(0.3)        # the helper opens the device and sets it raw
            for ln in SPL + ATF + UBOOT[:4]:
                self.out(ln)
            self.send(b"Hit any key to stop autoboot:  1")
            t_end, stopped = time.time() + 1.0, False
            while time.time() < t_end and not stopped:
                d = self.recv()
                self.rx += d
                stopped = self.mode != "deaf" and bool(d) and not d.strip(b" ")
            if not stopped:
                self.send(b"\r\n")
                for ln in KERNEL:
                    self.out(ln)
                self.close()
                return
            self.send(b"\b\b\b 0 \r\n" + PROMPT)
            buf, t_end = b"", time.time() + 25
            while time.time() < t_end:
                d = self.recv()
                if not d:
                    continue
                if self.prompt_at is None and d.strip(b" "):
                    # keys sent just before the prompt arrive after it
                    self.prompt_at = len(self.rx) + len(d) - len(d.lstrip(b" "))
                self.rx += d
                for b in d:
                    if b == 13:
                        line = buf.decode().strip()
                        buf = b""
                        self.lines.append(line)
                        self.send(b"\r\n")
                        if self.react(line):
                            self.close()
                            return
                    elif b == 3:
                        buf = b""
                        self.send(b"<INTERRUPT>\r\n" + PROMPT)
                    elif b == 32 and not buf:
                        pass
                    else:
                        buf += bytes([b])
                        self.send(b"#" if self.mode == "garble" else bytes([b]))
            self.close()
        except OSError:
            pass

    def react(self, line):
        """Plays U-Boot's answer to one line; True when the console ends."""
        if line.startswith("if ext4size"):
            if self.mode == "missing":
                self.out("** File not found /var/yakogl/x.bin **")
                self.out("vi95z: refused: /var/yakogl/x.bin missing, 0x10000000 bytes or more, or not loaded")
            else:
                for ln in LOAD_OUT:
                    self.out(ln)
                self.out(_md5_line(H if self.mode == "ok" else "f" * 32))
            self.send(PROMPT)
            return False
        if line.startswith("echo vi95z: go "):
            self.out("vi95z: go 0xd0000000")
            self.out("## Starting application at 0xD0000000 ...")
            for ln in IMAGE + SPL + ATF + UBOOT[:4] + ["Hit any key to stop autoboot:  0"] + KERNEL:
                self.out(ln)
            return True
        if line.endswith("run bootcmd"):
            m = re.match(r"echo (vi95z: [^;]*);", line)
            if m:
                self.out(m.group(1))
            for ln in KERNEL:
                self.out(ln)
            return True
        self.out("Unknown command '%s' - try 'help'" % line.split()[0])
        self.send(PROMPT)
        return False


def _loopback():
    import struct
    return socket.inet_ntoa(struct.pack("!I", socket.INADDR_LOOPBACK))


def _drive(fake, fn):
    import io
    with tempfile.TemporaryDirectory() as d:
        raw, st = os.path.join(d, "raw"), os.path.join(d, "st")
        log, res = io.StringIO(), {}
        t = threading.Thread(target=lambda: res.setdefault("rc", fn(fake.addr, raw, st, log)))
        t.start()
        t.join(40)
        with open(st) as f:
            stamped = f.read()
        with open(raw, "rb") as f:
            rawb = f.read()
        return (None if t.is_alive() else res.get("rc")), log.getvalue(), stamped, rawb


def typed_self_test(check):
    path = "/var/yakogl/x.bin"
    load, go, refuse = oneshot_lines(path, H)

    def run_oneshot(a, raw, st, log):
        return oneshot(a, raw, st, path, H, log)

    for mode, tcp in (("ok", False), ("ok", True), ("md5bad", False), ("missing", False),
                      ("deaf", False), ("garble", False)):
        name = "%s%s" % (mode, " tcp" if tcp else "")
        fake = _FakeVerdin(mode, tcp)
        rc, ev, stamped, rawb = _drive(fake, run_oneshot)
        check("oneshot %s returns 0" % name, rc == 0, "rc %r; %s" % (rc, ev[-300:]))
        check("oneshot %s: no address in messages" % name,
              fake.addr not in ev and _loopback() not in ev)
        typed_lines = fake.lines
        stamped_v = _scan_text(stamped)["verdict"]
        if mode == "ok":
            check("oneshot %s: only the stop key before the prompt" % name,
                  fake.prompt_at is not None and fake.prompt_at >= 1
                  and fake.rx[:fake.prompt_at].strip(b" ") == b"", repr(fake.rx[:60]))
            after = fake.rx[fake.prompt_at:] if fake.prompt_at is not None else b""
            check("oneshot %s: load line then go line, once each, after the prompt" % name,
                  after == (load + "\r" + go + "\r").encode(), repr(after[:200]))
            check("oneshot %s: stamped log scans done" % name, stamped_v == "done", stamped_v)
            check("oneshot %s: raw log written" % name, b"Booting Zephyr" in rawb)
        elif mode in ("md5bad", "missing"):
            back = refuse if mode == "md5bad" else BACK_LINE
            check("oneshot %s: load line, then the refusal, no go" % name,
                  typed_lines == [load, back] and b"go 0xd0000000;" not in fake.rx, repr(typed_lines))
            check("oneshot %s: scan refused" % name, stamped_v == "refused", stamped_v)
            check("oneshot %s: REFUSED event" % name, mode == "missing" or "REFUSED" in ev, ev[-200:])
        elif mode == "deaf":
            check("oneshot deaf: nothing but keys sent", fake.rx.strip(b" ") == b"", repr(fake.rx[:40]))
            check("oneshot deaf: MISSED event", "MISSED" in ev, ev[-200:])
            check("oneshot deaf: scan no_prompt", stamped_v == "no_prompt", stamped_v)
        elif mode == "garble":
            check("oneshot garble: no CR after a wrong echo, then run bootcmd",
                  typed_lines == ["run bootcmd"] and fake.rx.count(b"\x03") == ECHO_TRIES,
                  repr((typed_lines, fake.rx.count(b"\x03"))))
            check("oneshot garble: ABORTED event", "ABORTED" in ev, ev[-200:])
    # typed: refused lines are never sent; the console is not even opened
    import io
    for bad in ("saveenv", "env save", "mmc write 0xd0000000 0 0x800", "setenv bootcmd x",
                "echo ok; saveenv"):
        fake = _FakeVerdin("ok")
        log = io.StringIO()
        with tempfile.TemporaryDirectory() as d:
            rc = typed(fake.addr, os.path.join(d, "r"), os.path.join(d, "s"), ["echo vi95z: x", bad], log)
            opened = os.path.exists(os.path.join(d, "r"))
        time.sleep(0.4)
        check("typed refuses %r: exit 2, console not opened, nothing sent" % bad,
              rc == 2 and not opened and fake.rx == b"" and "nothing opened" in log.getvalue(),
              "rc %r opened %r rx %r" % (rc, opened, fake.rx[:20]))
    # the last guard: a refused line that reached the sender is not sent
    class _S:
        sent = b""

        def sendall(self, b):
            _S.sent += b
    try:
        _send_line(_S(), "saveenv")
        check("send guard refuses saveenv", False, "sent")
    except ValueError:
        check("send guard refuses saveenv", _S.sent == b"")
    check("oneshot bad md5: exit 2", oneshot("x:1", os.devnull, os.devnull, "/x", "nothex",
                                             open(os.devnull, "w")) == 2)
    # unreachable: 4 and no address
    s = socket.socket()
    s.bind((_loopback(), 0))
    port = s.getsockname()[1]
    s.close()
    log = io.StringIO()
    rc = typed("%s:%d" % (_loopback(), port), os.devnull, os.devnull, ["run bootcmd"], log)
    check("typed unreachable: 4, no address in the message",
          rc == 4 and _loopback() not in log.getvalue() and str(port) not in log.getvalue(), log.getvalue())


def self_test():
    fails = []

    def check(name, ok, why=""):
        print("%s %s%s" % ("ok  " if ok else "FAIL", name, "" if ok else ": " + why), flush=True)
        if not ok:
            fails.append(name)

    check_self_test(check)
    scan_self_test(check)
    typed_self_test(check)
    print("self-test: %d failed" % len(fails))
    return 1 if fails else 0


def _opts(args):
    kw, rest, it = {}, [], iter(args)
    for a in it:
        if a in ("--part", "--addr", "--window"):
            v = next(it, None)
            if v is None:
                raise ValueError("%s needs a value" % a)
            kw[a[2:]] = v
        elif a.startswith("--"):
            raise ValueError("unknown option %s" % a)
        else:
            rest.append(a)
    return kw, rest


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    cmd, args = (argv[0], argv[1:]) if argv else ("", [])
    if cmd == "scan" and args:
        return scan_main(args)
    if cmd == "probe" and len(args) == 1:
        return probe(args[0])
    if cmd == "capture" and len(args) == 3:
        return capture(*args)
    if cmd == "typed" and len(args) >= 4:
        return typed(args[0], args[1], args[2], args[3:])
    if cmd == "check" and args:
        bad = 0
        for ln in args:
            why = check_line(ln)
            if why:
                print("check: %r: %s" % (ln, why), file=sys.stderr)
                bad = 1
        return 2 if bad else 0
    if cmd in ("oneshot", "lines"):
        try:
            kw, rest = _opts(args)
        except ValueError as e:
            print("%s: %s" % (cmd, e), file=sys.stderr)
            return 2
        if cmd == "lines" and len(rest) == 2:
            try:
                print("\n".join(oneshot_lines(rest[0], rest[1], **kw)))
            except ValueError as e:
                print("lines: %s" % e, file=sys.stderr)
                return 2
            return 0
        if cmd == "oneshot" and len(rest) == 5:
            return oneshot(rest[0], rest[1], rest[2], rest[3], rest[4], **kw)
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
