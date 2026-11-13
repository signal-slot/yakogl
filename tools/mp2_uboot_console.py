#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Console side of the STM32MP257F-DK typed U-Boot one-shot.

    mp2_uboot_console.py capture HOST:PORT|DEVICE RAW STAMPED
        Writes the console bytes unchanged to RAW and every completed line as
        "<unix time> <line>" to STAMPED. Runs until killed or the console
        closes. Exit 4: busy or unreachable.
    mp2_uboot_console.py probe HOST:PORT|DEVICE
        Opens the console once and closes it. 0: free. 3: unreachable.
        4: "busy". 5: accepted and closed at once.
    mp2_uboot_console.py typed HOST:PORT|DEVICE RAW STAMPED LINE...
        "capture", and it also types (events go to stderr). It sends STOP_KEY
        every KEY_PERIOD s from the first BL2 line (or the U-Boot banner, if
        the capture started late) until U-Boot's prompt PROMPT comes, then
        types each LINE once, in order, waiting for the prompt between them.
        It sends nothing else: no CR outside the lines, no "boot" fallback.
        A wrong echo gets a Ctrl-C (U-Boot drops the half line) and the same
        LINE again, ECHO_TRIES times, then typing stops; nothing is typed
        after a miss, and BL2's watchdog (about 32 s) ends a stuck prompt.
        No LINE may write the U-Boot environment (the caller's rule).
    mp2_uboot_console.py scan LOG [--json]
        Prints the phases reached and one verdict. Exit 0 only for "done".
    mp2_uboot_console.py scan LOG --back
        Prints nothing. Exit 0 once the cycle is over: Linux started in a
        boot cycle after the image's end, or after a refusal (or in a cycle
        that never stopped U-Boot).
    mp2_uboot_console.py --self-test

Phases (what one cycle prints; the "mp2z: " lines are the script's, see
driver/tools/uboot-mp2):

  bl2        TF-A "NOTICE:  BL2" / "Reset reason"
  optee      "I/TC:"
  uboot      "U-Boot 20xx.xx"
  stopped    the prompt, i.e. autoboot was stopped
  script     a line "mp2z: ..."
  refused    "mp2z: refused: <why>"
  go         "mp2z: go 0x<addr>"
  image      "*** Booting Zephyr"
  image_end  "mp2: ... PSCI SYSTEM_RESET at <n> ms"
  watchdog   a new BL2 cycle after go, without an image_end
  linux      "Starting kernel ..." / "Linux version"

Verdicts: done (go, image, image_end, then Linux in a new cycle), refused,
watchdog_reset, not_stopped (U-Boot ran on to Linux or never offered the
prompt), no_go (stopped, but no jump: the typed line did not run, or the
image never started), not_started (no U-Boot in the log), and hang (the image
started but neither ended nor was reset, or ended and Linux never came: a
capture cut off, or the watchdog did not act).
"""
import importlib.util
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

PROMPT_TEXT = "STM32MP> "          # ST's CONFIG_SYS_PROMPT (assumed; override here)
PROMPT = PROMPT_TEXT.encode()
STOP_KEY = b" "                    # any key stops ST's autoboot; a space runs nothing
KEY_PERIOD = 0.05
KEY_BOUND = 60.0       # no prompt this long after the first key: missed
MISS_WINDOW = 4.0      # "Hit any key" came, no prompt this long after: missed
WAIT_START_BOUND = 120.0
QUIET = 0.5
QUIET_BOUND = 5.0
ECHO_BOUND = 5.0
ECHO_TRIES = 3
NEXT_BOUND = 20.0      # a typed line got its CR; the next prompt must come by then
CHUNK, CHUNK_GAP = 16, 0.015
BAUD = int(os.environ.get("MP2_BAUD", "115200"))

_here = os.path.dirname(os.path.abspath(__file__))


def _load_am62p():
    spec = importlib.util.spec_from_file_location(
        "am62p_netboot_uart", os.path.join(_here, "am62p_netboot_uart.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_am = _load_am62p()
read_lines = _am.read_lines
_clean = _am._clean
_echo_text = _am._echo_text

START_RX = re.compile(rb"NOTICE:\s+BL2|U-Boot 20\d\d\.\d+|Hit any key to stop autoboot")
HIT = b"Hit any key to stop autoboot"
KERNEL_RX = re.compile(rb"Starting kernel \.\.\.|Linux version \d")

PHASE_RX = [
    ("bl2", re.compile(r"NOTICE:\s+(BL2\b|Reset reason)")),
    ("optee", re.compile(r"^I/TC:")),
    ("uboot", re.compile(r"^U-Boot 20\d\d\.\d+")),
    ("stopped", re.compile("^" + re.escape(PROMPT_TEXT.rstrip()))),
    ("script", re.compile(r"^mp2z: ")),
    ("refused", re.compile(r"^mp2z: refused:")),
    ("go", re.compile(r"^mp2z: go 0x[0-9a-fA-F]+")),
    ("image", re.compile(r"\*\*\* Booting Zephyr")),
    ("image_end", re.compile(r"mp2: .*PSCI SYSTEM_RESET at \d+ ms")),
    ("linux", re.compile(r"Starting kernel \.\.\.|Linux version \d")),
]
PHASE_NAMES = [n for n, _ in PHASE_RX]
ALL_PHASES = PHASE_NAMES[:9] + ["watchdog", "linux"]


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
        return os.read(self.fd, n)

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
    except (OSError, ValueError) as e:
        say("cannot reach the console: %s" % (getattr(e, "strerror", None) or type(e).__name__))
        return None


def _stderr(msg):
    print(msg, file=sys.stderr, flush=True)


def probe(addr):
    if not _is_device(addr):
        return _am.probe(addr)
    try:
        t = _Tty(addr)
    except (OSError, KeyError, AttributeError) as e:
        _stderr("probe: cannot open: %s" % (getattr(e, "strerror", None) or type(e).__name__))
        return 3
    t.close()
    return 0


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


# ---------------------------------------------------------------- typed

def typed(addr, raw_path, st_path, lines, log=None):
    """Capture and drive the one-shot. 4: busy or unreachable; else 0 when the
    console closes (SIGTERM ends it too)."""
    log = log or sys.stderr
    if not lines or any((not x) or "\n" in x or "\r" in x for x in lines):
        print("typed: each LINE must be one non-empty line", file=log)
        return 2

    def ev(msg):
        print("%.3f typed: %s" % (time.time(), msg), file=log, flush=True)

    s = _open(addr, ev)
    if s is None:
        return 4
    s.settimeout(0.02)
    _term_exit()
    sink = _Sink(raw_path, st_path)
    try:
        return _typed_loop(s, sink, [x.encode() for x in lines], ev)
    finally:
        sink.close()


def _typed_loop(s, sink, lines, ev):
    st = {"state": "wait_start", "t": time.time(), "since": b"", "t_hit": None,
          "t_key": 0.0, "n": 0, "tries": 0, "t_rx": time.time(), "keys": 0}
    t_open, head = time.time(), b""
    ev("connected, waiting for BL2; %d line(s) to type" % len(lines))
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
            _typed_step(s, lines, ev, now, st)
        except OSError as e:
            ev("console error while sending in state %s: %s" % (st["state"], e.strerror or type(e).__name__))
            return 0


def _typed_step(s, lines, ev, now, st):
    state, since = st["state"], st["since"]

    def to(new):
        st["state"], st["t"], st["since"] = new, time.time(), b""

    def type_line(line):
        for i in range(0, len(line), CHUNK):
            s.sendall(line[i:i + CHUNK])
            time.sleep(CHUNK_GAP)

    if state == "wait_start":
        if now - st["t"] > WAIT_START_BOUND:
            to("expired")
            ev("EXPIRED: no boot output within %.0f s of the start; capture only from now on" % WAIT_START_BOUND)
        elif START_RX.search(since):
            to("key")
            ev("boot output seen: sending the stop key every %.0f ms until the prompt" % (KEY_PERIOD * 1000))
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
            since = b""
            type_line(lines[st["n"]])
            to("echo")
            ev("typed line %d/%d attempt %d (%d bytes), waiting for the echo" % (
                st["n"] + 1, len(lines), st["tries"], len(lines[st["n"]])))
    elif state == "echo":
        want = lines[st["n"]].decode()
        if want in _echo_text(since):
            s.sendall(b"\r")
            st["n"] += 1
            st["tries"] = 0
            ev("echo exact: CR sent for line %d/%d" % (st["n"], len(lines)))
            if st["n"] == len(lines):
                to("sent")
            else:
                to("next")
        elif now - st["t"] > ECHO_BOUND:
            if st["tries"] < ECHO_TRIES:
                ev("echo wrong or incomplete: %r; Ctrl-C and again" % _echo_text(since)[-80:])
                s.sendall(b"\x03")
                to("quiet")
            else:
                s.sendall(b"\x03")
                to("aborted")
                ev("ABORTED: echo wrong %d times; nothing more typed" % st["tries"])
    elif state == "next":
        if PROMPT in since:
            to("quiet")
        elif now - st["t"] > NEXT_BOUND:
            to("aborted")
            ev("ABORTED: no prompt after line %d; nothing more typed" % st["n"])


# ---------------------------------------------------------------- scan

def scan(lines):
    """Walks the lines once. A boot cycle is a run of lines from a BL2 marker
    on; a BL2 marker after the cycle has progressed past BL2 starts a new
    one. Returns {verdict, phases {name: time|True}, cycles, lines}."""
    got, cycles = {}, 0
    progressed = False
    after_end_cycle = False   # a BL2 marker came after image_end
    after_go_cycle = False
    linux_after_end = linux_after_refuse = linux_plain = False
    for t, line in lines:
        mark = t if t is not None else True
        for name, rx in PHASE_RX:
            if not rx.search(line):
                continue
            if name == "bl2":
                if cycles == 0 or progressed:
                    cycles += 1
                    progressed = False
                    if "go" in got and "image_end" not in got:
                        got.setdefault("watchdog", mark)
                    if "image_end" in got:
                        after_end_cycle = True
                    if "go" in got:
                        after_go_cycle = True
                got.setdefault("bl2", mark)
                continue
            progressed = True
            if name == "linux":
                got.setdefault("linux", mark)
                if "image_end" in got and after_end_cycle:
                    linux_after_end = True
                elif "refused" in got:
                    linux_after_refuse = True
                elif "go" not in got:
                    linux_plain = True
            elif name == "refused":
                got.setdefault("refused", mark)
                got.setdefault("script", mark)
            elif name == "script" or name == "go":
                got.setdefault(name, mark)
                if name == "go":
                    got.setdefault("script", mark)
            elif name == "stopped":
                if "uboot" in got:
                    got.setdefault("stopped", mark)
            elif name in ("image", "image_end"):
                if "go" in got:
                    got.setdefault(name, mark)
            else:
                got.setdefault(name, mark)
    nlines = sum(1 for _, ln in lines if ln.strip())
    if "go" in got:
        if "image_end" in got and linux_after_end:
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
        verdict = "not_stopped"
    else:
        verdict = "no_go"
    back = linux_after_end or linux_after_refuse or (linux_plain and "uboot" in got and "stopped" not in got)
    return {"verdict": verdict, "phases": got, "cycles": cycles, "lines": nlines, "back": back}


def report(res):
    ph = res["phases"]
    t0 = ph.get("bl2") if isinstance(ph.get("bl2"), float) else None
    parts = []
    for name in ALL_PHASES:
        v = ph.get(name)
        if v is None:
            parts.append("%s=absent" % name)
        elif t0 is not None and isinstance(v, float):
            parts.append("%s=+%.1fs" % (name, v - t0))
        else:
            parts.append("%s=yes" % name)
    return "mp2-scan: verdict=%s back=%s lines=%d cycles=%d %s" % (
        res["verdict"], "yes" if res["back"] else "no", res["lines"], res["cycles"], " ".join(parts))


def scan_main(args):
    as_json, back = "--json" in args, "--back" in args
    paths = [a for a in args if not a.startswith("--")]
    if len(paths) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        res = scan(read_lines(paths[0]))
    except OSError as e:
        print("scan: cannot read the log: %s" % (e.strerror or type(e).__name__), file=sys.stderr)
        return 2
    if back:
        return 0 if res["back"] else 1
    if as_json:
        print(json.dumps(res, sort_keys=True))
    else:
        print(report(res))
    return 0 if res["verdict"] == "done" else 1


# ---------------------------------------------------------------- self-test

BL2 = ["NOTICE:  CPU: STM32MP257FAK Rev.Y", "NOTICE:  Reset reason: Power-on Reset (PORRSTN)",
       "NOTICE:  BL2: v2.10-stm32mp2-r1.0(release):v2.10.0", "NOTICE:  BL2: Built : 10:00:00, Jan  1 2025"]
OPTEE = ["NOTICE:  BL31: v2.10", "I/TC: OP-TEE version: 4.0.0-stm32mp2", "I/TC: Primary CPU switching to normal world boot"]
UBOOT = ["U-Boot 2023.10-stm32mp2 (Jan 01 2025 - 10:00:00 +0000)", "Hit any key to stop autoboot:  0 "]
PROMPT_LN = PROMPT_TEXT
SCRIPT_OK = ["mp2z: checking image", "mp2z: go 0xa0000000"]
IMAGE = ["*** Booting Zephyr OS build v4.0 ***", "mp2: PSCI SYSTEM_RESET at 1234 ms"]
KERNEL = ["Starting kernel ...", "[    0.000000] Linux version 6.6.0 (user@host)"]


def _log(*parts, stamped=False):
    out = [x for p in parts for x in p]
    if stamped:
        return "".join("%.3f %s\n" % (1700000000.0 + i, x) for i, x in enumerate(out))
    return "".join(x + "\r\n" for x in out)


def _scan_text(text):
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "log")
        with open(p, "w") as f:
            f.write(text)
        return scan(read_lines(p)), p


def _cases():
    stop = [PROMPT_LN + "run x", ]
    return {
        "done": (_log(BL2, OPTEE, UBOOT, stop, SCRIPT_OK, IMAGE, ["\x00\x1b"], BL2, OPTEE, UBOOT, KERNEL), True),
        "refused": (_log(BL2, OPTEE, UBOOT, stop, ["mp2z: refused: bad magic"], KERNEL), False),
        "watchdog_reset": (_log(BL2, OPTEE, UBOOT, stop, SCRIPT_OK, IMAGE[:1], BL2, OPTEE, UBOOT, KERNEL), False),
        "not_stopped": (_log(BL2, OPTEE, UBOOT, KERNEL), False),
        "no_go": (_log(BL2, OPTEE, UBOOT, stop, ["mp2z: checking image"]), False),
        "not_started": (_log(BL2, OPTEE), False),
        "hang": (_log(BL2, OPTEE, UBOOT, stop, SCRIPT_OK, IMAGE[:1]), False),
    }


def scan_self_test(check):
    for name, (text, _) in _cases().items():
        for stamped in (False, True):
            if stamped:
                text = "".join("%.3f %s\n" % (1700000000.0 + i, x) for i, x in enumerate(text.replace("\r", "").split("\n")))
            res, p = _scan_text(text)
            check("scan %s%s" % (name, " stamped" if stamped else ""), res["verdict"] == name,
                  "got %s" % res["verdict"])
    check("scan empty", _scan_text("")[0]["verdict"] == "not_started")
    res, p = _scan_text(_cases()["done"][0])
    check("done phases", all(k in res["phases"] for k in
                             ["bl2", "optee", "uboot", "stopped", "script", "go", "image", "image_end", "linux"])
          and "watchdog" not in res["phases"] and "refused" not in res["phases"])
    res, p = _scan_text(_cases()["watchdog_reset"][0])
    check("watchdog phase", "watchdog" in res["phases"] and "image_end" not in res["phases"])
    # --back, and the CLI exit codes
    for name, back_exp in (("done", 0), ("refused", 0), ("not_stopped", 0), ("watchdog_reset", 1),
                           ("no_go", 1), ("hang", 1)):
        text = _cases()[name][0]
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "l")
            open(p, "w").write(text)
            rc = scan_main([p, "--back"])
            check("back %s" % name, rc == back_exp, "rc %d" % rc)
    # image ended but the next cycle has not reached Linux yet: not back
    text = _log(BL2, OPTEE, UBOOT, ["STM32MP> x"], SCRIPT_OK, IMAGE, BL2, OPTEE)
    check("back waits for linux", not _scan_text(text)[0]["back"])
    check("json", json.loads(json.dumps(_scan_text(_cases()["done"][0])[0]))["verdict"] == "done")


class _FakeDK:
    """A bridge on 127.0.0.1 that plays a DK boot and records what it received.
    mode: "ok" (key stops autoboot, two lines, image, reset, Linux) or
    "deaf" (keys are ignored: Linux boots)."""

    def __init__(self, mode):
        self.mode, self.rx, self.prompt_at, self.lines = mode, b"", None, []
        self.srv = socket.socket()
        self.srv.bind(("127.0.0.1", 0))
        self.srv.listen(1)
        self.port = self.srv.getsockname()[1]
        self.t = threading.Thread(target=self.run, daemon=True)
        self.t.start()

    def out(self, c, ln, gap=0.02):
        c.sendall(ln.encode() + b"\r\n")
        time.sleep(gap)

    def run(self):
        try:
            c, _ = self.srv.accept()
            c.settimeout(0.05)
            for ln in BL2 + OPTEE + UBOOT[:1]:
                self.out(c, ln)
            c.sendall(b"Hit any key to stop autoboot:  1")
            t_end, stopped = time.time() + 1.0, False
            while time.time() < t_end and not stopped:
                try:
                    d = c.recv(256)
                except socket.timeout:
                    continue
                self.rx += d
                stopped = self.mode == "ok" and bool(d.strip(b" ")) is False and bool(d)
            if not stopped:
                c.sendall(b"\r\n")
                self.out(c, KERNEL[0])
                self.out(c, KERNEL[1])
                time.sleep(0.2)
                c.close()
                return
            c.sendall(b"\b\b\b 0 \r\n")
            c.sendall(PROMPT)
            buf = b""
            while len(self.lines) < 2:
                try:
                    d = c.recv(256)
                except socket.timeout:
                    continue
                if not d:
                    return
                self.rx += d
                if self.prompt_at is None and d.strip(b" "):
                    # the helper keeps sending the stop key until it has seen the
                    # prompt, so keys sent just before it arrive after the prompt
                    # went out: the typed text starts at the first non-key byte
                    self.prompt_at = len(self.rx) - len(d.lstrip(b" "))
                for b in d:
                    if b == 13:
                        self.lines.append(buf.decode())
                        buf = b""
                        c.sendall(b"\r\n")
                        if len(self.lines) == 1:
                            self.out(c, "fake: loaded")
                            c.sendall(PROMPT)
                    elif b == 32 and not buf:
                        pass
                    else:
                        buf += bytes([b])
                        c.sendall(bytes([b]))
            for ln in SCRIPT_OK + IMAGE + BL2 + OPTEE + UBOOT[:1] + ["Hit any key to stop autoboot:  0"] + KERNEL:
                self.out(c, ln)
            time.sleep(0.3)
            c.close()
        except OSError:
            pass


def typed_self_test(check):
    import io
    lines = ["fake load a b c", "source a"]
    for mode in ("ok", "deaf"):
        fake = _FakeDK(mode)
        with tempfile.TemporaryDirectory() as d:
            raw, st = os.path.join(d, "raw"), os.path.join(d, "st")
            log = io.StringIO()
            res = {}
            t = threading.Thread(target=lambda: res.setdefault(
                "rc", typed("127.0.0.1:%d" % fake.port, raw, st, lines, log)))
            t.start()
            t.join(30)
            check("typed %s returns" % mode, not t.is_alive() and res.get("rc") == 0, "rc %r" % res.get("rc"))
            ev = log.getvalue()
            check("typed %s: no address in messages" % mode,
                  "127.0.0.1" not in ev and str(fake.port) not in ev)
            if mode == "ok":
                rx = fake.rx
                keys_ok = fake.prompt_at is not None and rx[:fake.prompt_at].strip(b" ") == b""
                check("typed ok: only the key before the prompt", keys_ok and fake.prompt_at >= 1)
                after = rx[fake.prompt_at:]
                check("typed ok: lines once, in order, after the prompt",
                      after == (lines[0] + "\r" + lines[1] + "\r").encode(), repr(after))
                check("typed ok: fake got the lines", fake.lines == lines, repr(fake.lines))
                r, _ = _scan_text(open(st).read())
                check("typed ok: stamped log scans done", r["verdict"] == "done", r["verdict"])
                check("typed ok: raw log written", b"Booting Zephyr" in open(raw, "rb").read())
            else:
                check("typed deaf: nothing but keys sent", fake.rx.strip(b" ") == b"", repr(fake.rx[:40]))
                check("typed deaf: MISSED event", "MISSED" in ev, ev[-200:])
                r, _ = _scan_text(open(st).read())
                check("typed deaf: scan not_stopped", r["verdict"] == "not_stopped", r["verdict"])
    # a refused LINE argument
    check("typed rejects an empty line", typed("127.0.0.1:1", os.devnull, os.devnull, [""], open(os.devnull, "w")) == 2)
    check("typed unreachable: 4 and no address",
          _unreachable_quiet())


def _unreachable_quiet():
    import io
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    log = io.StringIO()
    rc = typed("127.0.0.1:%d" % port, os.devnull, os.devnull, ["x"], log)
    return rc == 4 and "127.0.0.1" not in log.getvalue() and str(port) not in log.getvalue()


def tty_self_test(check):
    import pty
    m, sl = pty.openpty()
    try:
        t = _Tty(os.ttyname(sl))
    except (OSError, termios_error()) as e:
        check("tty open", False, str(e))
        return
    t.settimeout(0.2)
    os.write(m, b"abc")
    got = t.recv(16)
    t.sendall(b"xyz")
    time.sleep(0.1)
    back = os.read(m, 16)
    try:
        t.recv(1)
        timed = False
    except socket.timeout:
        timed = True
    check("tty roundtrip", got == b"abc" and b"xyz" in back and timed, repr((got, back)))
    t.close()
    os.close(m)
    os.close(sl)


def termios_error():
    import termios
    return termios.error


def self_test():
    fails = []

    def check(name, ok, why=""):
        print("%s %s%s" % ("ok  " if ok else "FAIL", name, "" if ok else ": " + why))
        if not ok:
            fails.append(name)

    scan_self_test(check)
    typed_self_test(check)
    tty_self_test(check)
    print("self-test: %d failed" % len(fails))
    return 1 if fails else 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if len(argv) >= 2 and argv[0] == "scan":
        return scan_main(argv[1:])
    if len(argv) == 2 and argv[0] == "probe":
        return probe(argv[1])
    if len(argv) == 4 and argv[0] == "capture":
        return capture(argv[1], argv[2], argv[3])
    if len(argv) >= 5 and argv[0] == "typed":
        return typed(argv[1], argv[2], argv[3], argv[4:])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
