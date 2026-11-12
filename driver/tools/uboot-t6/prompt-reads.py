#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The read-only facts of the NanoPC-T6 at its U-Boot prompt.

    prompt-reads.py run TTY RECORD_DIR
        Opens the board's debug UART (UART2, 1500000 8N1) and waits, writing
        nothing, for U-Boot's "Hit any key to stop autoboot". The caller
        reboots the board (prompt-reads.sh does). At that line it sends one
        space, and at the prompt the commands of plan() one by one, each
        typed, compared with U-Boot's echo and only then entered. The last
        command is `reset`: the board boots Armbian as after a power-on.
        RECORD_DIR gets uart.log (every byte read), typed.log (every byte
        written, with its time) and answers.txt (each command's output).
    prompt-reads.py plan
        Prints the commands, nothing else.
    prompt-reads.py --self-test
        The same code against a made-up U-Boot on a pty. No board.

Why it is this careful. Armbian logs root in on this UART by itself, so a
byte written at the wrong moment is typed into a root shell. Three rules,
each held by the self-test:

  - Nothing is written before "Hit any key to stop autoboot" was read, and
    after it only the one space until a U-Boot prompt ("=> " at the start of
    a line) was read.
  - Once "Starting kernel" was read nothing is written again, whatever
    follows.
  - A command is typed without its Enter and U-Boot's echo is compared with
    it. On a difference the line is dropped with ctrl-c and typed again;
    after TRIES differences the run stops and nothing more is written. A
    line that U-Boot received damaged is therefore never entered.

Reads only. allowed() is the whole list of what can be entered: `md.l`,
`printenv`, `bdinfo`, `dm tree`, `clk dump`, the mmc and ext4 listings,
`pmic dump` and `reset`. There is no memory write, no boot command, no `go`,
no environment save, no storage write and no regulator command in it, and
allowed() takes one command per line (no `;`, no `&&`).

Two steps depend on what was read before them:

  - The QoS words of the GPU sit behind PD_GPU, and reading an unpowered
    domain's block can fault the bus. They are read only when the PMU says
    the domain is on (PWR_GATE bit 0 clear and REPAIR_STATUS bit 1 set).
  - `pmic dump` is entered only when `dm tree` shows the RK806 probed
    already ("+"): U-Boot's probe of it writes SYS_CFG3, and this run must
    not be the one that causes a probe.

Exit: 0 done (every command answered, `reset` entered, the Linux login seen
again). 3 no autoboot line in time (nothing written). 4 the autoboot was
missed (one space written, Linux booted). 5 stopped at the prompt (an echo
that never matched, or a command without an answer): the board stays at the
U-Boot prompt until someone resets it. 6 `reset` entered but no Linux login
in time. 7 U-Boot left the prompt during a command (a fault resets the
board) and started Linux; nothing was written after that command. 2 usage.
"""

import fcntl
import os
import re
import select
import sys
import termios
import threading
import time

AUTOBOOT = b"Hit any key to stop autoboot"
KERNEL = b"Starting kernel"
LOGIN = b"login:"
PROMPT = b"=> "
TRIES = 3

# PMU and CRU words of docs/notes/nanopc-t6-uboot-handover.md, section 3.
PMU = 0xFD8D8000
PWR_GATE, IDLE_REQ, IDLE_ACK, IDLE_ST, REPAIR_ST = 0x14C, 0x10C, 0x118, 0x120, 0x290
QOS_BLOCKS = (0xFDF35000, 0xFDF35200, 0xFDF35400, 0xFDF35600)

ALLOWED = re.compile(
    r"(?:version|help|printenv|bdinfo|dm tree|clk dump|wdt list"
    r"|mmc list|mmc dev 0|mmc info|mmc part"
    r"|ext4ls mmc 0:1 /(?:[A-Za-z0-9._-]+/?)*"
    r"|ext4size mmc 0:1 /(?:[A-Za-z0-9._-]+/?)+"
    r"|pmic list|pmic dev [A-Za-z0-9@._-]+|pmic dump"
    r"|md\.l 0x[0-9a-f]{8} [0-9a-f]{1,2}"
    r"|reset)"
)


def allowed(cmd):
    """True for the one-line, read-only commands this tool may enter."""
    return ALLOWED.fullmatch(cmd) is not None


def md(addr, count=1):
    return "md.l 0x%08x %x" % (addr, count)


def first_commands():
    """The commands that depend on nothing read before them, in order."""
    return [
        "version",
        "help",
        "printenv",
        "bdinfo",
        "dm tree",
        # The PMU: is PD_GPU on, and what the idle handshake reads.
        md(PMU + PWR_GATE),
        md(PMU + IDLE_REQ),
        md(PMU + IDLE_ACK),
        md(PMU + IDLE_ST),
        md(PMU + REPAIR_ST),
        # The CRU: CLKSEL_CON(158), (159), CLKGATE_CON(66), MODE_CON0, the
        # CPLL and GPLL PLL_CON words.
        md(0xFD7C0578),
        md(0xFD7C057C),
        md(0xFD7C0908),
        md(0xFD7C0280),
        md(0xFD7C01A0, 7),
        md(0xFD7C01C0, 7),
        "clk dump",
        "wdt list",
        # The eMMC is mmc 0 (U-Boot's own "MMC:" line; `mmc list` shows it
        # again). `mmc dev` selects and writes nothing.
        "mmc list",
        "mmc dev 0",
        "mmc info",
        "mmc part",
        "ext4ls mmc 0:1 /",
        "ext4ls mmc 0:1 /boot",
    ]


def md_word(answer, addr):
    """The first word `md.l` printed at addr, or None."""
    m = re.search(r"^%08x: ([0-9a-f]{8})\b" % addr, answer, re.M)
    return int(m.group(1), 16) if m else None


def pd_gpu_on(answers):
    """True only when both PMU words were read and say PD_GPU is on."""
    gate = md_word(answers.get(md(PMU + PWR_GATE), ""), PMU + PWR_GATE)
    rep = md_word(answers.get(md(PMU + REPAIR_ST), ""), PMU + REPAIR_ST)
    if gate is None or rep is None:
        return False
    return gate & 1 == 0 and rep & 2 == 2


def probed_pmic(dm_tree):
    """The name of the pmic `dm tree` shows probed, or None."""
    for line in dm_tree.splitlines():
        m = re.match(r"\s*pmic\s+\d+\s+\[ \+ \]\s+\S+\s+[|`\- ]*(\S+)\s*$", line)
        if m and allowed("pmic dev " + m.group(1)):
            return m.group(1)
    return None


def later_commands(answers):
    """The commands that depend on the answers so far, and why/why not."""
    cmds, notes = [], []
    if pd_gpu_on(answers):
        notes.append("PD_GPU reads on: the GPU QoS words are read")
        for base in QOS_BLOCKS:
            cmds.append(md(base + 0x08, 5))
    else:
        notes.append("PD_GPU does not read on: the GPU QoS words are not read")
    name = probed_pmic(answers.get("dm tree", ""))
    if name:
        notes.append("dm tree shows %s probed: pmic dump is entered" % name)
        cmds += ["pmic list", "pmic dev " + name, "pmic dump"]
    else:
        notes.append("dm tree shows no probed pmic: no pmic command is entered")
    return cmds, notes


class Stop(Exception):
    def __init__(self, code, why):
        super().__init__(why)
        self.code = code
        self.why = why


class Console:
    """The UART: every byte read goes to uart.log, every write to typed.log."""

    def __init__(self, fd, raw, typed, gap=0.003):
        self.fd, self.raw, self.typed, self.gap = fd, raw, typed, gap
        self.buf = b""
        self.kernel = False       # "Starting kernel" was read: never write again
        self.armed = False        # the autoboot line was read
        self.t0 = time.monotonic()

    def log(self, what):
        self.typed.write("%8.3f %s\n" % (time.monotonic() - self.t0, what))
        self.typed.flush()

    def read(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if not r:
            return b""
        try:
            b = os.read(self.fd, 4096)
        except OSError:
            return b""
        if b:
            self.raw.write(b)
            self.raw.flush()
            self.buf += b
            if KERNEL in self.buf:
                self.kernel = True
        return b

    def write(self, b):
        if not self.armed:
            raise Stop(5, "a write before the autoboot line: refused")
        if self.kernel:
            raise Stop(4, "a write after 'Starting kernel': refused")
        for i in range(len(b)):
            os.write(self.fd, b[i:i + 1])
            time.sleep(self.gap)
        self.log("wrote %r" % b)

    def at_prompt(self):
        t = self.buf
        return t.endswith(PROMPT) and (len(t) == len(PROMPT) or t[-len(PROMPT) - 1:-len(PROMPT)] in b"\r\n")

    def wait_for(self, needle, timeout):
        end = time.monotonic() + timeout
        while needle not in self.buf:
            left = end - time.monotonic()
            if left <= 0:
                return False
            self.read(min(left, 0.2))
        return True

    def wait_prompt(self, timeout, quiet=0.15):
        """True when the stream ends in a prompt and stays silent for quiet."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if self.kernel:
                return False
            if self.at_prompt() and not self.read(quiet):
                return True
            if not self.at_prompt():
                self.read(0.2)
        return False


def clean(b):
    s = b.decode("ascii", "replace")
    s = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", s)
    return s.replace("\r", "")


def enter(con, cmd, timeout):
    """Type cmd, compare the echo, enter it, return its output."""
    if not allowed(cmd):
        raise Stop(5, "not in the allowed list, not typed: %r" % cmd)
    line = cmd.encode("ascii")
    for attempt in range(1, TRIES + 1):
        if not con.at_prompt():
            raise Stop(5, "no prompt before %r" % cmd)
        con.buf = b""
        con.write(line)
        end = time.monotonic() + 1.0
        while con.buf != line and time.monotonic() < end and line.startswith(con.buf):
            con.read(0.05)
        if con.buf == line:
            break
        con.log("echo %r is not %r (try %d)" % (con.buf, line, attempt))
        if attempt == TRIES:
            raise Stop(5, "the echo of %r never matched: not entered" % cmd)
        # Drop the line. U-Boot answers ctrl-c with a fresh prompt.
        con.buf = b""
        con.write(b"\x03")
        if not con.wait_prompt(3):
            raise Stop(5, "no prompt after ctrl-c on %r" % cmd)
    con.buf = b""
    con.write(b"\r")
    if cmd == "reset":
        return ""
    if not con.wait_prompt(timeout):
        if con.kernel:
            raise Stop(7, "U-Boot left the prompt during %r and started Linux: nothing was written after it" % cmd)
        raise Stop(5, "no prompt %d s after %r" % (timeout, cmd))
    out = clean(con.buf)
    out = out[:out.rfind("=> ")] if out.endswith("=> ") else out
    return out.lstrip("\n")


def run(fd, rec, boot_wait=240, login_wait=180, gap=0.003, cmd_wait=30):
    os.makedirs(rec, exist_ok=True)
    answers = {}
    with open(os.path.join(rec, "uart.log"), "wb") as raw, \
            open(os.path.join(rec, "typed.log"), "w") as typed, \
            open(os.path.join(rec, "answers.txt"), "w") as ans:
        con = Console(fd, raw, typed, gap)

        def say(msg):
            con.log(msg)
            print("prompt-reads: " + msg, flush=True)

        def one(cmd):
            out = enter(con, cmd, cmd_wait)
            answers[cmd] = out
            ans.write("## %s\n%s" % (cmd, out if out.endswith("\n") or not out else out + "\n"))
            ans.flush()

        try:
            say("listening, nothing written (waiting up to %d s for the autoboot line)" % boot_wait)
            if not con.wait_for(AUTOBOOT, boot_wait):
                raise Stop(3, "no autoboot line in %d s: nothing was written" % boot_wait)
            con.armed = True
            con.buf = b""
            con.write(b" ")
            if not con.wait_prompt(5):
                raise Stop(4, "no U-Boot prompt after the one space: the autoboot was missed")
            say("at the U-Boot prompt")
            for cmd in first_commands():
                one(cmd)
            more, notes = later_commands(answers)
            for n in notes:
                say(n)
                ans.write("# %s\n" % n)
            for cmd in more:
                one(cmd)
            say("%d commands answered; entering reset" % len(answers))
            one("reset")
            # From here the board must not be written to: disarm.
            con.armed = False
            con.buf = b""
            t = time.monotonic()
            if not con.wait_for(LOGIN, login_wait):
                raise Stop(6, "reset entered, no Linux login in %d s" % login_wait)
            say("done: %d commands, the Linux login %d s after reset" % (len(answers) - 1, time.monotonic() - t))
            return 0
        except Stop as s:
            con.armed = False
            if s.code in (4, 7):
                # Read on, written nothing more, so the record shows the boot.
                con.wait_for(LOGIN, login_wait)
            say("STOP (%d): %s" % (s.code, s.why))
            return s.code


def open_tty(path):
    fd = os.open(path, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    fcntl.ioctl(fd, termios.TIOCEXCL)
    a = termios.tcgetattr(fd)
    a[0] = 0                                             # iflag: raw, no flow control
    a[1] = 0                                             # oflag
    a[2] = termios.CS8 | termios.CREAD | termios.CLOCAL  # cflag: 8N1
    a[3] = 0                                             # lflag: no echo, no canon
    a[4] = a[5] = termios.B1500000
    a[6][termios.VMIN], a[6][termios.VTIME] = 0, 0
    termios.tcsetattr(fd, termios.TCSANOW, a)
    termios.tcflush(fd, termios.TCIOFLUSH)
    return fd


# --------------------------------------------------------------------------
# Self-test: a made-up U-Boot on a pty.

PMU_ON = {PMU + PWR_GATE: 0x0000FFFE, PMU + REPAIR_ST: 0xFAD50003}
PMU_OFF = {PMU + PWR_GATE: 0x0000FFFF, PMU + REPAIR_ST: 0xFAD50001}
DM_PROBED = (
    " Class     Seq    Probed  Driver                Name\n"
    " spi           2  [ + ]   rockchip_spi          |-- spi@feb20000\n"
    " pmic          0  [ + ]   rockchip_rk8xx        |   `-- pmic@0\n"
    " regulator     0  [   ]   rk8xx_buck            |       |-- dcdc-reg1\n"
)
DM_UNPROBED = DM_PROBED.replace("0  [ + ]   rockchip_rk8xx", "0  [   ]   rockchip_rk8xx")


class FakeBoard(threading.Thread):
    """Plays a boot on the pty master and answers like U-Boot's prompt."""

    def __init__(self, master, mode="ok", words=None, dm=DM_PROBED, garble=0):
        super().__init__(daemon=True)
        self.m, self.mode, self.dm = master, mode, dm
        self.words = dict(PMU_OFF if words is None else words)
        self.garble = garble          # how many typed lines lose a byte
        self.got = b""                # every byte the tool wrote
        self.entered = []             # every line U-Boot was given with Enter
        self.shell = b""              # bytes that reached the "root shell"
        self.done = threading.Event()

    def out(self, s):
        os.write(self.m, s.encode() if isinstance(s, str) else s)

    def key(self, timeout):
        r, _, _ = select.select([self.m], [], [], timeout)
        if not r:
            return b""
        b = os.read(self.m, 1)
        self.got += b
        return b

    def linux(self):
        self.out("Starting kernel ...\r\n\r\nboard login: root (automatic login)\r\n# ")
        end = time.monotonic() + 0.6
        while time.monotonic() < end:
            self.shell += self.key(0.1)

    def answer(self, cmd):
        if cmd.startswith("md.l "):
            _, a, n = cmd.split()
            a, n = int(a, 16), int(n, 16)
            return "%08x:" % a + "".join(" %08x" % self.words.get(a + 4 * i, 0) for i in range(n)) + "\r\n"
        if cmd == "dm tree":
            return self.dm.replace("\n", "\r\n")
        if cmd == "printenv":
            # A line that holds the prompt's text in its middle and at its end.
            return "bootcmd=bootflow scan -lb\r\nps1==> \r\nx=a => b\r\n"
        return "answer to %s\r\n" % cmd

    def run(self):
        try:
            self.out("\r\nU-Boot SPL 2026.07\r\n\r\nU-Boot 2026.07\r\n\r\nModel: made up\r\n")
            if self.mode == "noautoboot":
                self.linux()
                return
            self.out("\x1b[2KHit any key to stop autoboot:  1 ")
            k = self.key(1.0)
            if not k or self.mode == "missed":
                self.out("\x1b[2KHit any key to stop autoboot:  0 \r\n")
                self.linux()
                return
            self.out("\r\n=> ")
            line = b""
            while True:
                k = self.key(3.0)
                if not k:
                    return
                if k == b"\x03":
                    line = b""
                    self.out("<INTERRUPT>\r\n=> ")
                elif k == b"\r":
                    cmd = line.decode()
                    line = b""
                    self.entered.append(cmd)
                    self.out("\r\n")
                    if cmd == "reset":
                        self.out("resetting ...\r\n\r\nU-Boot SPL 2026.07\r\n")
                        self.out("\x1b[2KHit any key to stop autoboot:  1 ")
                        self.key(0.3)
                        self.linux()
                        return
                    if self.mode == "hang" and cmd == "bdinfo":
                        continue
                    if self.mode == "fault" and cmd == "bdinfo":
                        self.out('"Synchronous Abort" handler, esr 0x96000010\r\nResetting CPU ...\r\n')
                        self.out("\r\nU-Boot SPL 2026.07\r\n\x1b[2KHit any key to stop autoboot:  1 ")
                        self.key(0.3)
                        self.out("\x1b[2KHit any key to stop autoboot:  0 \r\n")
                        self.linux()
                        return
                    self.out(self.answer(cmd) + "=> ")
                else:
                    if self.garble and len(line) == 2:
                        self.garble -= 1
                        line += b"#"            # a damaged byte, echoed as received
                        self.out(b"#")
                        continue
                    line += k
                    self.out(k)
        finally:
            self.done.set()


def self_test():
    import pty
    import tempfile
    failed = []
    count = [0]

    def check(cond, what):
        count[0] += 1
        if not cond:
            failed.append(what)
            print("FAIL: " + what)

    def case(**kw):
        master, slave = pty.openpty()
        a = termios.tcgetattr(slave)
        a[0] = a[1] = a[3] = 0
        a[6][termios.VMIN], a[6][termios.VTIME] = 0, 0
        termios.tcsetattr(slave, termios.TCSANOW, a)
        fake = FakeBoard(master, **kw)
        with tempfile.TemporaryDirectory() as d:
            out = sys.stdout
            sys.stdout = open(os.devnull, "w")
            try:
                fake.start()
                rc = run(slave, d, boot_wait=3, login_wait=3, gap=0, cmd_wait=2)
            finally:
                sys.stdout.close()
                sys.stdout = out
            fake.done.wait(5)
            files = {n: open(os.path.join(d, n), "rb").read() for n in os.listdir(d)}
        os.close(master)
        os.close(slave)
        return rc, fake, files

    # 1. The whole run, PD_GPU off, the pmic probed.
    rc, f, files = case()
    want = first_commands() + ["pmic list", "pmic dev pmic@0", "pmic dump", "reset"]
    check(rc == 0, "ok: exit 0 (got %d)" % rc)
    check(f.entered == want, "ok: U-Boot was given exactly the plan, reset last (%r)" % f.entered[-4:])
    check(f.got.startswith(b" version\r"), "ok: the first byte written is the one space")
    check(f.shell == b"", "ok: nothing reached the Linux shell after reset (%r)" % f.shell)
    check(not any(c.startswith("md.l 0xfdf35") for c in f.entered), "ok: PD_GPU off, no QoS word read")
    ans = files["answers.txt"].decode()
    check("## printenv\nbootcmd=bootflow scan -lb\nps1==> \nx=a => b\n## bdinfo\n" in ans,
          "ok: an answer that holds the prompt's text is kept whole")
    check(b"Hit any key" in files["uart.log"] and b"wrote b' '" in files["typed.log"],
          "ok: uart.log and typed.log hold the read and the written bytes")

    # 2. PD_GPU on: the four QoS blocks are read, five words each.
    rc, f, _ = case(words=PMU_ON)
    qos = [c for c in f.entered if c.startswith("md.l 0xfdf35")]
    check(rc == 0 and qos == [md(b + 8, 5) for b in QOS_BLOCKS], "PD_GPU on: the QoS words are read (%r)" % qos)

    # 3. The pmic not probed: no pmic command at all.
    rc, f, _ = case(dm=DM_UNPROBED)
    check(rc == 0 and not any(c.startswith("pmic") for c in f.entered), "pmic not probed: no pmic command")

    # 4. The autoboot is missed: one space, then nothing, whatever Linux prints.
    rc, f, _ = case(mode="missed")
    check(rc == 4 and f.got == b" " and f.entered == [], "missed: exit 4, one space written in all (%r)" % f.got)

    # 5. No autoboot line: nothing is written.
    rc, f, _ = case(mode="noautoboot")
    check(rc == 3 and f.got == b"", "no autoboot line: exit 3, nothing written (%r)" % f.got)

    # 6. One damaged line: dropped with ctrl-c, typed again, entered once.
    rc, f, _ = case(garble=1)
    check(rc == 0 and f.entered == want and f.got.count(b"\x03") == 1,
          "one damaged echo: ctrl-c, typed again, the plan entered unchanged")

    # 7. Every line damaged: never entered, the run stops.
    rc, f, _ = case(garble=99)
    check(rc == 5 and f.entered == [] and b"\r" not in f.got,
          "echo never matches: exit 5, no Enter was ever written (%r)" % f.entered)

    # 8. A command without an answer: the run stops, nothing more is entered.
    rc, f, _ = case(mode="hang")
    check(rc == 5 and f.entered[-1] == "bdinfo", "no answer: exit 5, nothing entered after it")

    # 8b. A command that faults: U-Boot resets and boots Linux. The second
    # autoboot line is not answered and nothing is written after the command.
    rc, f, _ = case(mode="fault")
    check(rc == 7 and f.got.endswith(b"bdinfo\r") and f.shell == b"",
          "fault: exit 7, nothing written after the command (%r)" % f.got[-12:])

    # 9. The allowed list.
    for c in first_commands() + [md(b + 8, 5) for b in QOS_BLOCKS] + ["pmic dev pmic@0", "pmic dump", "reset"]:
        check(allowed(c), "allowed: %s" % c)
    for c in ["mw.l 0xfd7c0578 0", "mw 0xfd7c0578 0", "go 0x10000000", "booti 0x10000000", "bootm", "boot",
              "bootflow scan -lb", "run bootcmd", "saveenv", "env save", "setenv a b", "env default -a",
              "mmc write 0x10000000 0 1", "mmc erase 0 1", "mmc dev 1", "mmc partconf 0 1 1 0",
              "ext4write mmc 0:1 0x10000000 /x 1", "ext4load mmc 0:1 0x10000000 /boot/x", "load mmc 0:1 0 /x",
              "regulator dev vdd_gpu_s0", "regulator enable", "regulator value 675000", "regulator status -a",
              "pmic write 0 0", "pmic dev a;reset", "sf probe", "sf erase 0 1000", "i2c mw 0 0 0",
              "md.l 0xfd8d814c 1; mw.l 0 0", "md.l 0xfd8d814c 1 && reset", "md.l 0xfd8d814c 100",
              "md.l fd8d814c 1", "mm 0", "nm 0", "cp.b 0 1 2", "source 0", "reset -w", "ext4ls mmc 0:1 /a b",
              "ext4ls mmc 1:1 /", "wdt start 1000", "wdt dev x", "clk setfreq x 1", "gpio set 1", "dcache off", ""]:
        check(not allowed(c), "not allowed: %r" % c)

    # 10. The decisions.
    check(pd_gpu_on({md(PMU + PWR_GATE): "fd8d814c: 0000fffe\n", md(PMU + REPAIR_ST): "fd8d8290: fad50003\n"}),
          "pd_gpu_on: gate bit 0 clear and repair bit 1 set reads on")
    check(not pd_gpu_on({md(PMU + PWR_GATE): "fd8d814c: 0000fffe\n", md(PMU + REPAIR_ST): "fd8d8290: fad50001\n"}),
          "pd_gpu_on: repair bit 1 clear reads off")
    check(not pd_gpu_on({md(PMU + PWR_GATE): "fd8d814c: 0000fffe\n"}), "pd_gpu_on: a word not read reads off")
    check(not pd_gpu_on({md(PMU + PWR_GATE): "garbage", md(PMU + REPAIR_ST): "fd8d8290: fad50003\n"}),
          "pd_gpu_on: an answer that is no md line reads off")
    check(probed_pmic(DM_PROBED) == "pmic@0" and probed_pmic(DM_UNPROBED) is None,
          "probed_pmic: only a pmic with '+'")
    check(probed_pmic(" pmic          0  [ + ]   rk8xx   `-- a;reset\n") is None,
          "probed_pmic: a name that would not be an allowed command is not used")

    planned = 97
    print("prompt-reads self-test: %d checks run (planned %d), %d failed" % (count[0], planned, len(failed)))
    return 0 if not failed and count[0] == planned else 1


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:] == ["plan"]:
        print("\n".join(first_commands()))
        print("# then, if PD_GPU reads on: " + "; ".join(md(b + 8, 5) for b in QOS_BLOCKS))
        print("# then, if dm tree shows the pmic probed: pmic list; pmic dev <its name>; pmic dump")
        print("reset")
        return 0
    if len(argv) == 4 and argv[1] == "run":
        fd = open_tty(argv[2])
        try:
            return run(fd, argv[3])
        finally:
            os.close(fd)
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
