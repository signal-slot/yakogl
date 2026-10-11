#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""UART capture and phase scan for the AM62P U-Boot one-shot boot (TFTP or eMMC).

    am62p_netboot_uart.py capture HOST:PORT RAW STAMPED
        Connects to the serial bridge (a raw TCP stream of the board's debug
        UART), writes the bytes unchanged to RAW and every completed line as
        "<unix time> <line>" to STAMPED (CR and NUL removed there). RAW keeps
        NULs, so grep needs -a on it: without -a grep treats it as binary and
        reports no match, which looked like lost lines on 2026-10-03. Exits 4 when the bridge answers
        "busy" (held by another client) or cannot be reached; otherwise runs
        until killed (the runner kills it by PID) or the bridge closes.
    am62p_netboot_uart.py typed HOST:PORT RAW STAMPED LINE
        "capture", and it also types: the typed route, which writes nothing
        to the board's env (see "Typed route" below). Events go to stderr.
    am62p_netboot_uart.py scan LOG [--token T] [--json]
        Reads a RAW or STAMPED log and prints the phases it reached (with
        times relative to the first SPL banner when the log is stamped) and
        one verdict. Exit 0 only for "done".
    am62p_netboot_uart.py scan LOG --back
        Prints nothing; exit 0 when the cycle is over: a kernel started after
        the first SPL, and either no image was started or the image's reset
        came before that kernel ("back" in the --json result). The runner
        waits on this. (Its earlier test piped "scan --json", whose exit
        status is the verdict's, into python under pipefail, so a probe,
        whose verdict is never "done", waited out RUN_BOUND every time.)
    am62p_netboot_uart.py --self-test [SAVED_KEXEC_LOG]

The phases, in the order one netboot cycle prints them on UART_3 (the U-Boot
lines are the ones tools/cts/log/board-sessions/am62p-netboot.md installs):

  spl1       "U-Boot SPL"                      the reboot from Linux reached SPL
  hook       "yakogl-netboot: once=<token>"    U-Boot found the one-shot armed
  cleared    "yakogl-netboot: cleared"         ... and saved the env without it
             or "yakogl-netboot: typed"        (typed route: nothing to clear)
  fetched    "Bytes transferred = "            the TFTP fetch completed
             or "<n> bytes read in "           (eMMC: U-Boot's ext4load completed; only
                                               before the hook's failure line, after which
                                               the normal bootcmd's own loads print it too)
  go         "## Starting application at"      U-Boot jumped to the image
  zephyr     "*** Booting Zephyr OS"           the image started
  rti_armed  "rti_arm: armed"                  the image armed the RTI (optional; NOT in
                                               order: Zephyr's log thread prints it BEFORE
                                               the "Booting Zephyr OS" banner, seen on the
                                               board 2026-10-03, so it counts anywhere
                                               between the jump and the reset)
  reset      "U-Boot SPL" after zephyr         the image ended in a SoC reset
  linux      "Starting kernel ..." after reset the next boot went to Linux

Verdicts, one per cause:

  done          every phase up to linux, the hook fired exactly once
  not_armed     SPL came but no hook line: the one-shot was not in the env
  not_cleared   the hook fired but "cleared" never came: saveenv failed, so
                U-Boot did not boot the image (by design)
  fetch_failed  cleared, but no jump: U-Boot printed its fetch-failed line
                (or an error this scan does not know; the log has it)
  load_failed   cleared, but no jump, and U-Boot printed "yakogl-netboot: mmc
                load failed": the eMMC route's ext4load failed (the file is
                absent: the eMMC probe's expected end)
  no_zephyr     U-Boot jumped but the Zephyr banner never came
  no_reset      Zephyr started and the SoC never reset within the capture (a
                hang no watchdog ended: the dangerous case, see the plan)
  no_linux      the SoC reset but Linux did not start afterwards
  looped        the hook fired twice: the one-shot did not clear
  wrong_token   a complete cycle, but for another run's token
  no_spl        the log has lines but no SPL banner (not a netboot cycle)
  empty         the log has no line at all (the capture saw nothing)

Typed route ("typed"): U-Boot's autoboot is stopped over the UART and one
command line is typed at the prompt, so the board's env is never written.
At bootdelay=0 U-Boot still checks once for a key (abortboot_single_key in
common/autoboot.c: "if (tstc())" before the delay loop; the board prints
"Hit any key to stop autoboot:  0", that function's format). So:
  1. capture until the first "U-Boot SPL" line (the runner reboots the board);
     none within WAIT_SPL_BOUND s: type nothing, ever ("typed: EXPIRED");
  2. send a CR every SPAM_PERIOD s (U-Boot proper resets the UART FIFO when it
     starts, so one early key is not enough) until the prompt PROMPT comes.
     If "Hit any key" came and no prompt followed within MISS_WINDOW s, or no
     prompt within SPAM_BOUND s of SPL, the autoboot was missed: stop sending
     and let the board boot as usual ("typed: MISSED");
  3. wait until the line is quiet (the CRs still in flight print prompts);
  4. type LINE without its CR, in small paced chunks, and compare U-Boot's
     echo with LINE. Only an exact echo gets the CR. A wrong echo gets Ctrl-C
     (U-Boot drops the line) and LINE again, at most ECHO_TRIES times; then
     "boot" (the normal bootcmd, i.e. Linux) instead;
  5. capture until killed or the bridge closes.
Risk the env route does not have: if the host or the bridge fails between
step 2 and step 4, U-Boot waits at its prompt (no watchdog, no bootretry in
Toradex's defconfig) until a person resets the board.
"""
import json
import os
import re
import signal
import socket
import sys
import tempfile
import threading
import time

PHASES = [
    ("spl1", re.compile(r"^U-Boot SPL ")),
    ("hook", re.compile(r"^yakogl-netboot: once=(\S+)")),
    ("cleared", re.compile(r"^yakogl-netboot: (?:cleared|typed)")),
    ("fetched", re.compile(r"^(?:Bytes transferred = (\d+)|(\d+) bytes read in )")),
    ("go", re.compile(r"^## Starting application at (0x[0-9a-fA-F]+)")),
    # the early boot markers ('H', 'K') can precede the banner on its line
    ("zephyr", re.compile(r"\*\*\* Booting Zephyr OS")),
    ("rti_armed", re.compile(r"rti_arm: armed")),
    ("reset", re.compile(r"^U-Boot SPL ")),
    ("linux", re.compile(r"^Starting kernel \.\.\.")),
]
# Phases that are not part of the ordered walk: see scan().
UNORDERED = {"rti_armed"}
SEQUENCE = [p for p in PHASES if p[0] not in UNORDERED]
RTI_ARMED = dict(PHASES)["rti_armed"]
HOOK = PHASES[1][1]
FETCH_FAILED = re.compile(r"^yakogl-netboot: fetch failed")
LOAD_FAILED = re.compile(r"^yakogl-netboot: mmc load failed")
STAMP = re.compile(r"^(\d{9,}\.\d+) (.*)$")


def read_lines(path):
    """[(time or None, line)], NULs and CRs removed."""
    with open(path, "rb") as fh:
        raw = fh.read()
    text = raw.replace(b"\0", b"").replace(b"\r", b"").decode("utf-8", "replace")
    out = []
    for line in text.split("\n"):
        m = STAMP.match(line)
        t, body = (float(m.group(1)), m.group(2)) if m else (None, line)
        # a reset leaves a partial byte or an escape before the next banner
        body = re.sub(r"^[^\x20-\x7e]+", "", body)
        out.append((t, body))
    return out


def scan(lines, token=None):
    """Walks the phases in order: a phase counts only after every earlier
    one. rti_armed is outside that order (the image prints it before its
    Zephyr banner): it counts after the jump and before the reset. Returns verdict, phases {name: time or True}, hooks
    (every once= token seen, in order), fetched_bytes and the line count."""
    got, hooks = {}, []
    fetch_failed, load_failed, fetched_bytes, idx = False, False, None, 0
    seen_spl, kernel_after_spl = False, False
    for t, line in lines:
        if PHASES[0][1].match(line):
            seen_spl = True
        elif seen_spl and PHASES[-1][1].match(line):
            kernel_after_spl = True  # the cycle ended in a kernel start, whichever path
        m = HOOK.match(line)
        if m:
            hooks.append(m.group(1))
        if FETCH_FAILED.match(line):
            fetch_failed = True
        if LOAD_FAILED.match(line):
            load_failed = True
        if "go" in got and "reset" not in got and "rti_armed" not in got and RTI_ARMED.search(line):
            got["rti_armed"] = t if t is not None else True
        if idx < len(SEQUENCE):
            name, rx = SEQUENCE[idx]
            mm = rx.search(line)
            # after the hook's own failure line, a load is the bootcmd's, not the image's
            if mm and not (name == "fetched" and (fetch_failed or load_failed)):
                got[name] = t if t is not None else True
                if name == "fetched":
                    fetched_bytes = int(mm.group(1) or mm.group(2))
                idx += 1
    nlines = sum(1 for _, line in lines if line.strip())
    if nlines == 0:
        verdict = "empty"
    elif len(hooks) > 1:
        verdict = "looped"
    elif "spl1" not in got:
        verdict = "no_spl"
    elif "hook" not in got:
        verdict = "not_armed"
    elif "cleared" not in got:
        verdict = "not_cleared"
    elif "go" not in got:
        verdict = "load_failed" if load_failed else "fetch_failed"
    elif "zephyr" not in got:
        verdict = "no_zephyr"
    elif "reset" not in got:
        verdict = "no_reset"
    elif "linux" not in got:
        verdict = "no_linux"
    elif token is not None and hooks[0] != token:
        verdict = "wrong_token"
    else:
        verdict = "done"
    back = kernel_after_spl and ("go" not in got or "linux" in got)
    return {"verdict": verdict, "phases": got, "hooks": hooks,
            "fetch_failed_line": fetch_failed, "load_failed_line": load_failed,
            "kernel_after_spl": kernel_after_spl, "back": back,
            "fetched_bytes": fetched_bytes, "lines": nlines}


def report(res):
    ph = res["phases"]
    t0 = ph.get("spl1") if isinstance(ph.get("spl1"), float) else None
    parts = []
    for name, _ in PHASES:
        v = ph.get(name)
        if v is None:
            parts.append("%s=absent" % name)
        elif t0 is not None and isinstance(v, float):
            parts.append("%s=+%.1fs" % (name, v - t0))
        else:
            parts.append("%s=yes" % name)
    fb = res["fetched_bytes"]
    return "netboot-scan: verdict=%s back=%s lines=%d hooks=%s fetched_bytes=%s %s" % (
        res["verdict"], "yes" if res["back"] else "no", res["lines"], ",".join(res["hooks"]) or "none",
        "absent" if fb is None else fb, " ".join(parts))


def _clean(b):
    return b.replace(b"\r", b"").replace(b"\0", b"").decode("utf-8", "replace")


def capture(addr, raw_path, stamped_path):
    host, port = addr.rsplit(":", 1)
    try:
        s = socket.create_connection((host, int(port)), timeout=10)
    except OSError as e:
        print("capture: cannot reach %s: %s" % (addr, e), file=sys.stderr)
        return 4
    s.settimeout(1.0)

    def on_term(signum, frame):
        raise SystemExit(0)

    # The runner stops the capture by its PID (SIGTERM); the unfinished last
    # line (a login prompt has no newline) must still reach the stamped file.
    signal.signal(signal.SIGTERM, on_term)
    with open(raw_path, "ab", buffering=0) as raw, open(stamped_path, "a", buffering=1) as st:
        state = {"pending": b""}
        try:
            return _capture_loop(s, addr, raw, st, state)
        finally:
            if state["pending"]:
                st.write("%.3f %s\n" % (time.time(), _clean(state["pending"])))


def _capture_loop(s, addr, raw, st, state):
    head, t_open = b"", time.time()
    while True:
        try:
            chunk = s.recv(4096)
        except socket.timeout:
            continue
        except OSError:
            return 0
        if not chunk:
            return 0
        raw.write(chunk)
        if time.time() - t_open < 3 and len(head) < 256:
            head += chunk
            if b"busy" in head:
                print("capture: the bridge %s is busy (another client holds it)" % addr, file=sys.stderr)
                return 4
        state["pending"] += chunk
        now = time.time()
        while b"\n" in state["pending"]:
            line, state["pending"] = state["pending"].split(b"\n", 1)
            st.write("%.3f %s\n" % (now, _clean(line)))


# The typed route (module docstring). The prompt is CONFIG_SYS_PROMPT of
# Toradex's verdin-am62p_a53_defconfig; the times are host-side bounds.
PROMPT = b"Verdin AM62P # "
WAIT_SPL_BOUND = 120.0  # typing is armed this long after the start: a typer
                        # left over from a killed runner must not stop a later
                        # reboot by someone else
SPAM_PERIOD = 0.05     # one CR every 50 ms from the first SPL banner
SPAM_BOUND = 60.0      # no prompt this long after SPL: missed
MISS_WINDOW = 3.0      # "Hit any key" came, no prompt this long after: missed
QUIET = 1.0            # quiet: nothing came for this long (every CR in flight has echoed)
QUIET_BOUND = 5.0
ECHO_BOUND = 5.0       # the echo of the typed line must be complete by then
ECHO_TRIES = 3
CHUNK, CHUNK_GAP = 16, 0.015   # typing pace: 16 bytes, then 15 ms


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


def typed(addr, raw_path, stamped_path, line, log=None):
    """Capture as capture() does and drive the typed route. Returns 4 for a
    busy or unreachable bridge, else 0 when the bridge closes (SIGTERM ends
    it too). Every step is logged to `log` (stderr) with the time."""
    log = log or sys.stderr
    if "\n" in line or "\r" in line or not line:
        print("typed: LINE must be one non-empty line", file=log)
        return 2
    host, port = addr.rsplit(":", 1)
    try:
        s = socket.create_connection((host, int(port)), timeout=10)
    except OSError as e:
        print("typed: cannot reach %s: %s" % (addr, e), file=log)
        return 4
    s.settimeout(0.02)

    def on_term(signum, frame):
        raise SystemExit(0)

    # (the self-test runs this in a thread, where signal() is not allowed)
    if threading.current_thread() is threading.main_thread():
        signal.signal(signal.SIGTERM, on_term)

    def ev(msg):
        print("%.3f typed: %s" % (time.time(), msg), file=log, flush=True)

    with open(raw_path, "ab", buffering=0) as raw, open(stamped_path, "a", buffering=1) as st:
        pend = {"pending": b""}
        try:
            return _typed_loop(s, addr, raw, st, pend, line.encode(), ev)
        finally:
            if pend["pending"]:
                st.write("%.3f %s\n" % (time.time(), _clean(pend["pending"])))


def _typed_loop(s, addr, raw, st, pend, line, ev):
    state, t_state = "wait_spl", time.time()
    t_open, head = time.time(), b""
    since = b""            # bytes received since the current step began
    t_hit, t_last_rx, t_cr = None, time.time(), 0.0
    tries = 0
    ev("connected, waiting for U-Boot SPL; line is %d bytes" % len(line))

    while True:
        now = time.time()
        try:
            chunk = s.recv(4096)
            if not chunk:
                ev("bridge closed in state %s" % state)
                return 0
        except socket.timeout:
            chunk = b""
        except OSError as e:
            ev("bridge error in state %s: %s" % (state, e))
            return 0
        if chunk:
            raw.write(chunk)
            t_last_rx = now
            if now - t_open < 3 and len(head) < 256:
                head += chunk
                if b"busy" in head:
                    ev("the bridge %s is busy (another client holds it)" % addr)
                    return 4
            pend["pending"] += chunk
            while b"\n" in pend["pending"]:
                ln, pend["pending"] = pend["pending"].split(b"\n", 1)
                st.write("%.3f %s\n" % (now, _clean(ln)))
            since += chunk

        try:
            state, t_state, since, t_hit, t_cr, tries = _typed_step(
                s, line, ev, now, state, t_state, since, t_hit, t_last_rx, t_cr, tries)
        except OSError as e:
            ev("bridge error while sending in state %s: %s" % (state, e))
            return 0


def _typed_step(s, line, ev, now, state, t_state, since, t_hit, t_last_rx, t_cr, tries):
    """One pass of the typed route's state machine (module docstring); returns
    the new (state, t_state, since, t_hit, t_cr, tries). A send that fails
    raises OSError to the caller."""

    def send(b):
        s.sendall(b)

    def type_line():
        for i in range(0, len(line), CHUNK):
            send(line[i:i + CHUNK])
            time.sleep(CHUNK_GAP)

    if state == "wait_spl":
        if now - t_state > WAIT_SPL_BOUND:
            state = "expired"
            ev("EXPIRED: no SPL within %.0f s of the start; capture only from now on" % WAIT_SPL_BOUND)
        elif b"U-Boot SPL " in since:
            state, t_state, since = "spam", now, b""
            ev("SPL seen: sending CR every %.0f ms until the prompt" % (SPAM_PERIOD * 1000))
    elif state == "spam":
        if PROMPT in since:
            state, t_state = "quiet", now
            ev("prompt seen: autoboot stopped; waiting for the line to be quiet")
        elif t_hit is None and b"Hit any key to stop autoboot" in since:
            t_hit = now
        if state == "spam" and t_hit is not None:
            # a stopped autoboot prints the prompt next; any other complete
            # line after "Hit any key" means bootcmd is already running
            after = since.split(b"Hit any key to stop autoboot", 1)[1].split(b"\n")[1:-1]
            if any(_clean(x).strip() for x in after):
                state = "missed"
                ev("MISSED: bootcmd output after 'Hit any key' (%r); nothing typed, the board boots as usual"
                   % _clean(next(x for x in after if _clean(x).strip()))[:60])
        if state == "spam":
            if (t_hit is not None and now - t_hit > MISS_WINDOW) or now - t_state > SPAM_BOUND:
                state = "missed"
                ev("MISSED: no prompt (%s); nothing typed, the board boots as usual" %
                   ("%.1f s after 'Hit any key'" % (now - t_hit) if t_hit else "%.0f s after SPL" % SPAM_BOUND))
            elif now - t_cr >= SPAM_PERIOD:
                send(b"\r")
                t_cr = now
    elif state == "quiet":
        if now - t_last_rx >= QUIET or now - t_state > QUIET_BOUND:
            tries += 1
            since = b""
            type_line()
            state, t_state = "echo", time.time()
            ev("typed attempt %d (%d bytes), waiting for the echo" % (tries, len(line)))
    elif state == "echo":
        if line.decode() in _echo_text(since):
            send(b"\r")
            state = "sent"
            ev("echo exact: CR sent")
        elif now - t_state > ECHO_BOUND:
            if tries < ECHO_TRIES:
                ev("echo wrong or incomplete: %r; Ctrl-C and again" % _echo_text(since)[-80:])
                send(b"\x03")
                state, t_state, since = "quiet", time.time(), b""
            else:
                ev("echo wrong %d times: typing 'boot' (the normal bootcmd) instead" % tries)
                send(b"\x03")
                time.sleep(0.3)
                send(b"boot\r")
                state = "fallback"
    return state, t_state, since, t_hit, t_cr, tries


# ---------------------------------------------------------------------------
# Self-test: a synthetic cycle built from the real U-Boot and Zephyr lines of
# records/am62p/netboot/uboot-boot-sample-uart-v16.log, plus the lines the
# plan's U-Boot hook echoes (not yet seen on the board: they are this repo's).

FIXTURE_DONE = """\
U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
Net:   eth0: ethernet@8000000port@1 [PRIME]Could not get PHY for mdio@f00: addr 7
Hit any key to stop autoboot:  0
yakogl-netboot: once=glt-abc123
Saving Environment to MMC... Writing to MMC(0)... OK
yakogl-netboot: cleared
TFTP from server 192.0.2.2; our IP address is 192.0.2.3
Filename 'yakogl/glt-abc123.bin'.
Load address: 0x82000000
Bytes transferred = 16107024 (f5c510 hex)
yakogl-netboot: go 0x82000000
## Starting application at 0x82000000 ...
HK*** Booting Zephyr OS build af17c0c8aa2a ***
I: rti_arm: armed
gl_test: done
I: sample done (0): resetting the SoC
\x1b
U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
Hit any key to stop autoboot:  0
SafeUI images not found, booting Linux
Starting kernel ...
"""


class _FakeUBoot:
    """A U-Boot on a local TCP port, for the typed route's self-test. It
    prints Linux noise until `reboot()`, then SPL and U-Boot banners, drops
    what it received so far (U-Boot proper resets the UART FIFO), and checks
    once for a key after "Hit any key ..." as abortboot_single_key does. At
    its prompt it echoes input; CR runs the line: an empty line prints the
    prompt again, Ctrl-C drops the line, "boot" boots Linux, and a line that
    starts with "echo yakogl-netboot: once=" prints what the board would
    (DHCP, then TFTP "File not found"). `executed` keeps every line run.
    keys=False ignores keys (an autoboot that cannot be stopped); drop=N
    loses the Nth typed byte of the first non-empty line (a noisy UART)."""

    def __init__(self, keys=True, drop=None):
        self.keys, self.drop, self.executed = keys, drop, []
        self.srv = socket.socket()
        self.srv.bind(("127.0.0.1", 0))
        self.srv.listen(1)
        self.addr = "127.0.0.1:%d" % self.srv.getsockname()[1]
        self.go = threading.Event()
        self.t = threading.Thread(target=self._run, daemon=True)
        self.t.start()

    def reboot(self):
        self.go.set()

    def _run(self):
        c, _ = self.srv.accept()
        c.settimeout(0.01)

        def drain():
            got = b""
            while True:
                try:
                    d = c.recv(4096)
                except socket.timeout:
                    return got
                except OSError:
                    return got
                if not d:
                    return got
                got += d

        c.sendall(b"verdin-am62p login: ")
        self.go.wait(10)
        c.sendall(b"\r\nreboot: Restarting system\r\n\r\nU-Boot SPL 2024.04-ti-g11cb0162a795\r\n")
        time.sleep(0.3)
        c.sendall(b"U-Boot SPL 2024.04-ti-g11cb0162a795\r\nU-Boot 2024.04-ti-g11cb0162a795\r\n")
        drain()                      # the FIFO reset at U-Boot's serial init
        time.sleep(0.3)
        c.sendall(b"Net:   eth0: ethernet@8000000port@1 [PRIME]\r\n\r\nHit any key to stop autoboot:  0 ")
        if not (self.keys and drain()):
            c.sendall(b"\r\nSafeUI images not found, booting Linux\r\nStarting kernel ...\r\n")
            time.sleep(1.0)
            drain()
            c.close()
            return
        c.sendall(b"\x08\x08\x08 0 \r\n" + PROMPT)
        buf, n_typed, dropped = b"", 0, False
        t_end = time.time() + 30
        while time.time() < t_end:
            for b in drain():
                b = bytes([b])
                if b == b"\r":
                    c.sendall(b"\r\n")
                    cmd = buf.decode()
                    buf = b""
                    if not cmd:
                        c.sendall(PROMPT)
                        continue
                    self.executed.append(cmd)
                    if cmd == "boot":
                        c.sendall(b"SafeUI images not found, booting Linux\r\nStarting kernel ...\r\n")
                        c.close()
                        return
                    m = re.match(r"echo yakogl-netboot: once=(\S+); echo yakogl-netboot: typed;", cmd)
                    if m:
                        c.sendall(("yakogl-netboot: once=%s\r\nyakogl-netboot: typed\r\n"
                                   "BOOTP broadcast 1\r\nDHCP client bound to address 192.0.2.3 (3 ms)\r\n"
                                   "TFTP from server 192.0.2.2; our IP address is 192.0.2.3\r\n"
                                   "TFTP error: 'File not found' (1)\r\n"
                                   "yakogl-netboot: fetch failed, booting Linux\r\n"
                                   "SafeUI images not found, booting Linux\r\nStarting kernel ...\r\n" % m.group(1)).encode())
                        c.close()
                        return
                    c.sendall(b"Unknown command\r\n" + PROMPT)
                elif b == b"\x03":
                    buf = b""
                    c.sendall(b"<INTERRUPT>\r\n" + PROMPT)
                else:
                    n_typed += 1
                    if self.drop is not None and not dropped and n_typed == self.drop:
                        dropped = True
                        continue
                    buf += b
                    c.sendall(b)
            time.sleep(0.005)
        c.close()


def _typed_case(fake, line, reboot_after=0.3):
    """Runs typed() against `fake`; returns (rc, events, scan result)."""
    import io
    d = tempfile.mkdtemp()
    raw, st = os.path.join(d, "raw"), os.path.join(d, "st")
    log = io.StringIO()
    out = {}
    t = threading.Thread(target=lambda: out.setdefault("rc", typed(fake.addr, raw, st, line, log)), daemon=True)
    t.start()
    time.sleep(reboot_after)
    fake.reboot()
    t.join(40)
    res = scan(read_lines(st))
    for f in (raw, st):
        os.unlink(f)
    os.rmdir(d)
    return out.get("rc"), log.getvalue(), res


def typed_self_test(check):
    line = ("echo yakogl-netboot: once=probe-none-1; echo yakogl-netboot: typed; "
            "setenv autoload no; if dhcp && tftpboot 0x82000000 192.0.2.2:yakogl/probe-none-1.bin; "
            "then go 0x82000000; fi; echo yakogl-netboot: fetch failed, booting Linux; boot")
    f = _FakeUBoot()
    rc, evs, r = _typed_case(f, line)
    check(rc == 0 and f.executed == [line] and r["verdict"] == "fetch_failed" and r["hooks"] == ["probe-none-1"],
          "typed: autoboot stopped, the line run once, scan fetch_failed (rc %s, ran %d, %s)"
          % (rc, len(f.executed), r["verdict"]))
    check("prompt seen" in evs and "echo exact" in evs, "typed: events name the prompt and the exact echo")
    f = _FakeUBoot(drop=40)
    rc, evs, r = _typed_case(f, line)
    check(f.executed == [line] and "Ctrl-C and again" in evs,
          "typed: a lost byte gets Ctrl-C and a second try, never a wrong line (ran %r)"
          % [x[:20] for x in f.executed])
    f = _FakeUBoot(keys=False)
    rc, evs, r = _typed_case(f, line)
    check(f.executed == [] and "MISSED" in evs and r["verdict"] == "not_armed" and r["kernel_after_spl"],
          "typed: autoboot not stopped: MISSED, nothing typed, Linux boots (%s)" % r["verdict"])
    global WAIT_SPL_BOUND
    saved, WAIT_SPL_BOUND = WAIT_SPL_BOUND, 0.5
    try:
        f = _FakeUBoot()
        rc, evs, r = _typed_case(f, line, reboot_after=1.5)
    finally:
        WAIT_SPL_BOUND = saved
    check(f.executed == [] and "EXPIRED" in evs and "SPL seen" not in evs,
          "typed: no SPL within the bound: EXPIRED, a later boot is not stopped (ran %d)" % len(f.executed))
    check(typed("127.0.0.1:1", "/dev/null", "/dev/null", line, open(os.devnull, "w")) == 4,
          "typed: unreachable bridge: rc 4")
    check(typed("127.0.0.1:1", "/dev/null", "/dev/null", "a\nb", open(os.devnull, "w")) == 2,
          "typed: a line with a newline is refused: rc 2")


# The first board probe (2026-10-03, env route): the real UART capture,
# addresses and serial numbers redacted. The hook fired, DHCP bound, the TFTP
# server answered "File not found" and U-Boot fell through to Linux.
REAL_PROBE = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                          "testdata", "am62p-netboot-probe-20261003.uart.ts")


def real_probe_self_test(check):
    import subprocess
    r = scan(read_lines(REAL_PROBE), "probe-none-1791009398")
    check(r["verdict"] == "fetch_failed" and r["hooks"] == ["probe-none-1791009398"]
          and r["fetch_failed_line"] and r["kernel_after_spl"] and r["back"],
          "real probe capture: fetch_failed, one hook, the kernel start found, back (%s)" % report(r)[:60])

    def back_rc(path):
        return subprocess.run([sys.executable, os.path.abspath(__file__), "scan", path, "--back"],
                              capture_output=True).returncode

    check(back_rc(REAL_PROBE) == 0, "scan --back on the real probe: exit 0 (the runner stops waiting)")
    with open(REAL_PROBE) as fh:
        upto = fh.read().split("Starting kernel ...")[0]
    fd, cut = tempfile.mkstemp()
    with os.fdopen(fd, "w") as fh:
        fh.write(upto)
    try:
        check(back_rc(cut) == 1, "scan --back before the kernel start: exit 1 (the runner keeps waiting)")
    finally:
        os.unlink(cut)


REAL_CAPTURES = [  # (file, token, bytes, rti line, phases that must be present)
    ("am62p-netboot-rtiproof-20261003.uart.ts", "rtiproof-691c3e985b1f068d-1791013366", 16238444,
     "fed for 0 s"),
    ("am62p-netboot-selfarm-20261003.uart.ts", "selfarmC-f047d34c0b11cb05-1791013634", 16234368,
     "fed for 150 s"),
]
VERDICT_PHASES = [n for n, _ in PHASES]


def real_capture_self_test(check):
    """The two 2026-10-03 image runs (redacted). Both printed the rti_arm line
    BEFORE the Zephyr banner; the old ordered scan reported rti_armed=absent."""
    here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata")
    for name, token, nbytes, fed in REAL_CAPTURES:
        lines = read_lines(os.path.join(here, name))
        r = scan(lines, token)
        missing = [n for n in VERDICT_PHASES if n not in r["phases"]]
        check(r["verdict"] == "done" and r["back"] and not missing and r["fetched_bytes"] == nbytes,
              "%s: done, back, every phase matched (missing %s)" % (name, missing or "none"))
        check(isinstance(r["phases"].get("rti_armed"), float)
              and any(fed in ln and RTI_ARMED.search(ln) for _, ln in lines),
              "%s: rti_armed has its timestamp, from the line '...%s'" % (name, fed))
        zi = next(i for i, (_, ln) in enumerate(lines) if "*** Booting Zephyr OS" in ln)
        ri = next(i for i, (_, ln) in enumerate(lines) if RTI_ARMED.search(ln))
        check(ri < zi, "%s: the capture has rti_arm BEFORE the banner (the case the old scan lost)" % name)
        check("rti_armed=+" in report(r) and "rti_armed=absent" not in report(r),
              "%s: the verdict line carries rti_armed" % name)


def rti_format_self_test(check):
    """The pattern is checked against the image's own format string, so a
    reworded printf fails here instead of printing rti_armed=absent."""
    src = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "driver", "zephyr", "lib",
                       "board", "am62p", "rti_arm.c")
    with open(src) as fh:
        fmts = re.findall(r'LOG_INF\("(rti_arm: armed[^"]*)"', fh.read())
    check(len(fmts) == 1, "rti_arm.c has exactly one 'rti_arm: armed' LOG_INF (found %d)" % len(fmts))
    if fmts:
        # fill every printf conversion with a plausible value, as the image does
        line = re.sub(r"%(?:0?x|llu|u|s)", "7", fmts[0])
        check(RTI_ARMED.search(line) is not None,
              "RTI_ARMED matches what the format string prints: %r" % line)
    # the runner's refusal literal: tools/board.sh's am62p case's MARKER
    with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "board.sh")) as fh:
        text = fh.read()
    am = text[text.find("\nam62p)"):text.find("\nimx8mp)")]
    m = re.search(r"^\s*MARKER='([^']*)'", am, re.M)
    check(m is not None and RTI_ARMED.search(m.group(1)) is not None,
          "tools/board.sh's am62p MARKER is matched by RTI_ARMED")


def capture_self_test(check):
    """capture() against a fake bridge: RAW must be exactly the bytes sent
    (NUL, CR, a lone 0xfd, a last line without newline included); STAMPED is the
    same bytes cleaned, one stamped row per line, the partial one included."""
    sent = (b"verdin login: \r\n\xfd\r\nU-Boot SPL 2024\r\x00\n"
            b"gl_test: all: 1 of 38 cases failed\r\n" + b"x" * 5000 + b"\r\nno newline at the end")
    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)

    def bridge():
        c, _ = srv.accept()
        for i in range(0, len(sent), 700):   # many small writes, split mid-line
            c.sendall(sent[i:i + 700])
            time.sleep(0.01)
        c.close()

    threading.Thread(target=bridge, daemon=True).start()
    d = tempfile.mkdtemp()
    raw, st = os.path.join(d, "raw"), os.path.join(d, "st")
    rc = capture("127.0.0.1:%d" % srv.getsockname()[1], raw, st)
    with open(raw, "rb") as fh:
        got = fh.read()
    stamped = [m.group(2) for m in (STAMP.match(x) for x in open(st, encoding="utf-8").read().split("\n")) if m]
    want = _clean(sent).split("\n")
    check(rc == 0 and got == sent, "capture: RAW == the bytes the bridge sent (%d of %d, rc %s)"
          % (len(got), len(sent), rc))
    check(stamped == want, "capture: STAMPED rows == the cleaned lines, last partial line kept (%d of %d)"
          % (len(stamped), len(want)))
    for f in (raw, st):
        os.unlink(f)
    os.rmdir(d)
    srv.close()


def self_test(saved_log=None):
    ok = True

    def check(cond, what):
        nonlocal ok
        print("  %s  %s" % ("PASS" if cond else "FAIL", what))
        ok = ok and cond

    def run(text, token=None, stamped=False):
        fd, path = tempfile.mkstemp()
        with os.fdopen(fd, "w") as fh:
            if stamped:
                for i, line in enumerate(text.split("\n")):
                    fh.write("%.3f %s\n" % (1790000000.0 + i, line))
            else:
                fh.write(text)
        try:
            return scan(read_lines(path), token)
        finally:
            os.unlink(path)

    def cut_at(marker):
        return FIXTURE_DONE.split(marker)[0]

    r = run(FIXTURE_DONE, "glt-abc123")
    check(r["verdict"] == "done" and r["fetched_bytes"] == 16107024,
          "full cycle: done, 16107024 bytes (%s)" % r["verdict"])
    r = run(FIXTURE_DONE, "glt-abc123", stamped=True)
    check(r["verdict"] == "done" and isinstance(r["phases"]["linux"], float), "stamped log: done, with times")
    check("linux=+23.0s" in report(r), "report: times relative to SPL (%s)" % report(r)[-40:])
    check(run(FIXTURE_DONE, "other")["verdict"] == "wrong_token", "another run's token: wrong_token")
    no_hook = "\n".join(l for l in FIXTURE_DONE.split("\n") if "yakogl-netboot" not in l)
    check(run(no_hook)["verdict"] == "not_armed", "no hook line: not_armed")
    check(run(cut_at("Saving"))["verdict"] == "not_cleared", "hook, no cleared: not_cleared")
    t = cut_at("TFTP from") + ("TFTP error: 'File not found' (1)\n"
                               "yakogl-netboot: fetch failed, booting Linux\nStarting kernel ...\n")
    r = run(t)
    check(r["verdict"] == "fetch_failed" and r["fetch_failed_line"], "TFTP refused: fetch_failed")
    # the eMMC route (yako_mmc): ext4load's line in place of TFTP's
    mmc = FIXTURE_DONE.split("TFTP from")[0] + "16107024 bytes read in 352 ms (43.6 MiB/s)\n" + \
        FIXTURE_DONE.split("Bytes transferred = 16107024 (f5c510 hex)\n")[1]
    r = run(mmc, "glt-abc123")
    check(r["verdict"] == "done" and r["fetched_bytes"] == 16107024, "eMMC cycle: done, 16107024 bytes read (%s)" % r["verdict"])
    # the probe's end: the file absent, then the normal bootcmd (SafeUI's
    # ext4load of its own files prints "bytes read" too: not the image's)
    t = cut_at("TFTP from") + ("** File not found /ostree/deploy/torizon/var/yakogl/once-p.bin **\n"
                               "yakogl-netboot: mmc load failed, booting Linux\n"
                               "65536 bytes read in 4 ms (15.6 MiB/s)\nSafeUI images not found, booting Linux\n"
                               "Starting kernel ...\n")
    r = run(t)
    check(r["verdict"] == "load_failed" and r["load_failed_line"] and r["fetched_bytes"] is None and r["back"],
          "eMMC file absent: load_failed, the bootcmd's later load not counted, back (%s, %s)"
          % (r["verdict"], r["fetched_bytes"]))
    check(run(cut_at("HK***"))["verdict"] == "no_zephyr", "jump, no banner: no_zephyr")
    check(run(cut_at("\x1b"))["verdict"] == "no_reset", "Zephyr, no reset: no_reset")
    check(run(FIXTURE_DONE.replace("Starting kernel ...\n", ""))["verdict"] == "no_linux", "reset, no Linux: no_linux")
    t = FIXTURE_DONE.replace("SafeUI images", "yakogl-netboot: once=glt-abc123\nSafeUI images")
    check(run(t)["verdict"] == "looped", "hook twice: looped")
    check(r["kernel_after_spl"], "fetch failed, Linux booted: kernel_after_spl set")
    check(not run(cut_at("\x1b"))["kernel_after_spl"], "Zephyr still running: kernel_after_spl clear")
    check(run("")["verdict"] == "empty", "empty log: empty, not a pass")
    check(run("\n\n")["verdict"] == "empty", "blank lines only: empty")
    r = run(FIXTURE_DONE.replace("I: rti_arm: armed\n", ""))
    check(r["verdict"] == "done" and "rti_armed" not in r["phases"],
          "no rti_arm line: done, and rti_armed reported absent")
    # a phase out of order does not count: a banner before any jump
    r = run("U-Boot SPL x\n*** Booting Zephyr OS build y ***\n")
    check("zephyr" not in r["phases"] and r["verdict"] == "not_armed", "banner before the hook: not counted")
    typed_self_test(check)
    real_probe_self_test(check)
    real_capture_self_test(check)
    rti_format_self_test(check)
    capture_self_test(check)
    if saved_log:
        # A real kexec cycle (Zephyr, then a reset, then Linux): no U-Boot
        # before the image, so no netboot phase may be claimed.
        r = scan(read_lines(saved_log))
        check(r["verdict"] == "not_armed" and "zephyr" not in r["phases"] and r["lines"] > 100,
              "saved kexec log %s (%d lines): %s" % (os.path.basename(saved_log), r["lines"], r["verdict"]))
        print("  " + report(r))
    print("am62p_netboot_uart self-test: %s" % ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test(argv[1] if len(argv) > 1 else None)
    if len(argv) == 4 and argv[0] == "capture":
        return capture(argv[1], argv[2], argv[3])
    if len(argv) == 5 and argv[0] == "typed":
        return typed(argv[1], argv[2], argv[3], argv[4])
    if len(argv) >= 2 and argv[0] == "scan":
        token = argv[argv.index("--token") + 1] if "--token" in argv else None
        res = scan(read_lines(argv[1]), token)
        if "--back" in argv:
            return 0 if res["back"] else 1
        print(json.dumps(res) if "--json" in argv else report(res))
        return 0 if res["verdict"] == "done" else 1
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
