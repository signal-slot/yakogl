#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The read-only facts of the NanoPC-T6 at its U-Boot prompt.

    prompt-reads.py run TTY RECORD_DIR [BACK]
        Opens the board's debug UART (UART2, 1500000 8N1) and waits, writing
        nothing, for U-Boot's "Hit any key to stop autoboot". The caller
        reboots the board (prompt-reads.sh does). At that line it sends one
        space, and at the prompt the commands of plan() one by one, each
        typed, compared with U-Boot's echo and only then entered. The last
        command is `reset`: the board boots Armbian as after a power-on.
        RECORD_DIR gets uart.log (every byte read), typed.log (every byte
        written, with its time) and answers.txt (each command's output).
        A wait for Linux's login that ends without it closes uart.log with
        one line that starts with "####" and says where the stream went
        silent: what the board printed after that point is not recorded.
        BACK is a file the caller makes when Armbian answers over ssh with
        a new boot_id. A wait for the autoboot line or for Linux's login
        then ends once the UART has also been silent for 30 s, not at its
        full time: what it waits for cannot come any more (issue #11943).
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

Three steps depend on what was read before them:

  - The QoS words of the GPU sit behind PD_GPU, and reading an unpowered
    domain's block can fault the bus. They are read only when the PMU says
    the domain is on (PWR_GATE bit 0 clear and REPAIR_STATUS bit 1 set).
  - WDT0's words are read only when CLKGATE_CON(15) says both its gates
    are open: a read of a block whose clock is gated can hang the bus.
  - `pmic dump` is entered only when `dm tree` shows the RK806 probed
    already ("+"): U-Boot's probe of it writes SYS_CFG3, and this run must
    not be the one that causes a probe.

Exit: 0 done (every command answered, `reset` entered, the Linux login seen
again). 12 no autoboot line in time, or until BACK on a silent UART
(nothing written; not 3, which prompt-reads.sh keeps for "refused before
the reboot"). 4 the autoboot was
missed (one space written, Linux booted). 5 stopped at the prompt (an echo
that never matched, or a command without an answer): the board stays at the
U-Boot prompt until someone resets it. 6 `reset` entered but no Linux login
on the UART in time, or until BACK on a silent UART (the UART alone cannot
say Linux is not back:
prompt-reads.sh asks the board over ssh). 7 U-Boot left the prompt during a command (a fault resets the
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
# Why a run stops with 12 (prompt-go.py's too).
NO_AUTOBOOT = "no autoboot line in %d s after the reboot: nothing was written"
NO_AUTOBOOT_BACK = ("no autoboot line, the UART is silent and Armbian answers over ssh with a new boot_id: "
                    "the board booted without the line reaching this tool, nothing was written")
KERNEL = b"Starting kernel"
LOGIN = b"login:"
PROMPT = b"=> "
TRIES = 3

# PMU and CRU words of docs/notes/nanopc-t6-uboot-handover.md, section 3.
PMU = 0xFD8D8000
PWR_GATE, IDLE_REQ, IDLE_ACK, IDLE_ST, REPAIR_ST = 0x14C, 0x10C, 0x118, 0x120, 0x290
QOS_BLOCKS = (0xFDF35000, 0xFDF35200, 0xFDF35400, 0xFDF35600)
# WDT0, the watchdog the image arms itself (section 6): its two gates,
# PCLK_WDT0 and TCLK_WDT0, are CLKGATE_CON(15) bits 0 and 1 (soc::rk3588);
# the CRU's global reset status and configuration, GLB_RST_ST and
# GLB_RST_CON (nanopc-t6.sh reads them under Linux before and after a run).
CLKGATE_CON15 = 0xFD7C083C
GLB_RST_ST, GLB_RST_CON = 0xFD7C0C04, 0xFD7C0C10
WDT0 = 0xFEAF0000
# UART2, the debug console (rk3588-base.dtsi serial@feb50000;
# board::nanopc_t6_early::UART2_PHYS; driver/docs/nanopc-t6-mali.md table).
# DW_apb_uart, 32-bit registers at reg-shift 2: LCR is +0x0c, USR +0x7c. Both
# reads are free of side effects. Never read RBR (+0), IIR (+8), LSR (+0x14)
# or MSR (+0x18): those reads clear state. DLL/DLM share +0 and +4 with RBR/IER
# behind LCR.DLAB, and reaching them is a write to LCR that would break
# U-Boot's own console, so the divisor is not read.
UART2 = 0xFEB50000
UART2_LCR, UART2_USR = UART2 + 0x0C, UART2 + 0x7C
# UART2's clock in the CRU, Linux v6.12 drivers/clk/rockchip/clk-rk3588.c:
# CLKSEL_CON(x) is CRU + 0x300 + 4x (CLKGATE_CON(x) is +0x800 + 4x, as above).
#   :509 clk_uart2_p = { clk_uart2_src, clk_uart2_frac, xin24m }
#   :631-633 clk_uart2 mux CLKSEL_CON(45)[1:0] (0 src, 1 frac, 2 xin24m)
#   :1247-1249 clk_uart2_src: gpll_cpll_p, CLKSEL_CON(43) mux bit 7 (0 gpll,
#     1 cpll), divider bits 6:2 (div + 1), gate CLKGATE_CON(12) bit 14
#   :1250-1253 clk_uart2_frac: CLKSEL_CON(44), numerator [31:16] and
#     denominator [15:0] (clk.h COMPOSITE_FRACMUX div_shift 16; rate =
#     parent * n / d), gate CLKGATE_CON(12) bit 15
#   :1221 pclk_uart2 gate CLKGATE_CON(12) bit 3; :1254-1255 sclk_uart2 gate
#     CLKGATE_CON(13) bit 0
# PLL rate: clk-pll.c:896-921 (m, p, s and k).
UART2_SEL = 0xFD7C0000 + 0x300 + 4 * 43       # CLKSEL_CON(43), (44), (45) follow
UART2_GATE12 = 0xFD7C0000 + 0x800 + 4 * 12    # CLKGATE_CON(12), (13) follow

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
        # WDT0's gates and the global reset words.
        md(CLKGATE_CON15),
        md(GLB_RST_ST),
        md(GLB_RST_CON),
        # UART2's format, status and clock (decode: uart2_decode). No RBR,
        # IIR, LSR, MSR; no DLL/DLM (they need LCR.DLAB, a write).
        md(UART2_LCR),
        md(UART2_USR),
        md(UART2_SEL, 3),
        md(UART2_GATE12, 2),
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


def wdt0_clocked(answers):
    """True only when CLKGATE_CON(15) was read with both WDT0 gates open."""
    w = md_word(answers.get(md(CLKGATE_CON15), ""), CLKGATE_CON15)
    return w is not None and w & 3 == 0


def later_commands(answers):
    """The commands that depend on the answers so far, and why/why not."""
    cmds, notes = [], []
    if wdt0_clocked(answers):
        # WDT_CR and WDT_TORR, then COMP_PARAM_1, COMP_VERSION and COMP_TYPE.
        # Never WDT_EOI (+0x14): a read of it clears the interrupt.
        notes.append("WDT0's gates read open: WDT0's control and identity words are read")
        cmds += [md(WDT0, 2), md(WDT0 + 0xF4, 3)]
    else:
        notes.append("WDT0's gates do not read open: WDT0 is not read")
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


def pll_hz(answers, base):
    """A PLL's rate in Hz (clk-pll.c:896-921, fractional k included) from
    its PLL_CON words, or None. The PLL mode is not checked here."""
    w = md_words(answers.get(md(base, 7), ""), base, 7)
    if w is None or w[1] & 0x3f == 0:
        return None
    m, p, s, k = w[0] & 0x3ff, w[1] & 0x3f, (w[1] >> 6) & 7, w[2] & 0xffff
    return (24000000 * m // p + (24000000 * k // (p * 65535) if k else 0)) >> s


def uart2_decode(answers):
    """The lines that say what UART2 reads at the prompt: format, clock
    source, input clock and the baud divisor 1 gives. [] when the reads
    were not made. The divisor itself is not read."""
    lcr = md_word(answers.get(md(UART2_LCR), ""), UART2_LCR)
    usr = md_word(answers.get(md(UART2_USR), ""), UART2_USR)
    sel = md_words(answers.get(md(UART2_SEL, 3), ""), UART2_SEL, 3)
    gates = md_words(answers.get(md(UART2_GATE12, 2), ""), UART2_GATE12, 2)
    if lcr is None and sel is None:
        return []
    out = ["UART2: the divisor (DLL/DLM) is not read: it sits behind LCR.DLAB, and setting that "
           "is a write that breaks U-Boot's console; the baud below is what divisor 1 would give"]
    if lcr is not None:
        par = "N" if not lcr & 8 else ("O" if not lcr & 0x10 else "E")
        out.append("UART2 LCR 0x%08x: %d%s%d, DLAB %s%s" % (
            lcr, 5 + (lcr & 3), par, 2 if lcr & 4 else 1,
            "SET (the divisor latch is open: not a normal state)" if lcr & 0x80 else "clear",
            "" if lcr & 0xff == 3 else " (not 8N1 with DLAB clear)"))
    if usr is not None:
        out.append("UART2 USR 0x%08x: BUSY %d, TFNF %d, TFE %d, RFNE %d, RFF %d" % (
            usr, usr & 1, usr >> 1 & 1, usr >> 2 & 1, usr >> 3 & 1, usr >> 4 & 1))
    if sel is None:
        out.append("UART2 clock: CLKSEL_CON(43..45) not read")
        return out
    src, frac, mux = sel
    hz, how = None, "reserved mux value 3"
    if mux & 3 == 2:
        hz, how = 24000000, "xin24m (24 MHz OSC)"
    elif mux & 3 < 2:
        pll, base = ("cpll", 0xFD7C01A0) if src >> 7 & 1 else ("gpll", 0xFD7C01C0)
        phz = pll_hz(answers, base)
        div = (src >> 2 & 0x1f) + 1
        how = "%s / %d" % (pll, div)
        hz = None if phz is None else phz // div
        if phz is None:
            how += " (PLL words not read)"
        if mux & 3 == 1:
            n, d = frac >> 16, frac & 0xffff
            if n and d:
                how += " * %d / %d (fractional divider)" % (n, d)
                hz = None if hz is None else hz * n // d
            else:
                how += " (fractional divider 0: pass-through)"
    out.append("UART2 clock select: CLKSEL_CON(43) 0x%08x, (44) 0x%08x, (45) 0x%08x: source %s" % (src, frac, mux, how))
    if gates is not None:
        g12, g13 = gates
        names = [("pclk_uart2", g12, 3), ("clk_uart2_src", g12, 14), ("clk_uart2_frac", g12, 15), ("sclk_uart2", g13, 0)]
        out.append("UART2 gates (1 = gated): CLKGATE_CON(12) 0x%08x, (13) 0x%08x: %s" % (
            g12, g13, ", ".join("%s %d" % (n, w >> b & 1) for n, w, b in names)))
    if hz is not None:
        out.append("UART2 input clock %d Hz; divisor 1 gives %d baud (clk / 16); the PLL mode is not applied" % (hz, hz // 16))
    return out


class Stop(Exception):
    def __init__(self, code, why):
        super().__init__(why)
        self.code = code
        self.why = why


class Console:
    """The UART: every byte read goes to uart.log, every write to typed.log."""

    def __init__(self, fd, raw, typed, gap=0.003, tool="prompt-reads", back=None, quiet=30.0):
        self.fd, self.raw, self.typed, self.gap, self.tool = fd, raw, typed, gap, tool
        # back: a file the shell runner makes when Armbian answers over ssh
        # with a new boot_id (None: no such witness); quiet: how long the
        # UART must have been silent then for a wait to end on that word.
        self.back, self.quiet = back, quiet
        self.told = False         # the last wait_for ended on the runner's word
        self.waited = 0.0         # how long the last wait_login waited
        self.buf = b""
        self.kernel = False       # "Starting kernel" was read: never write again
        self.armed = False        # the autoboot line was read
        self.t0 = time.monotonic()
        self.nread = 0            # the bytes in uart.log
        self.last = self.t0       # when the last of them was read

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
            self.nread += len(b)
            self.last = time.monotonic()
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

    def wait_for(self, needle, timeout, back=False):
        """Reads until needle; False when timeout seconds passed without it.

        back: the wait is for something that cannot come once Linux runs
        again (the autoboot line, Linux's login itself). It then also ends,
        False and with self.told set, when the runner's file says Armbian
        answers over ssh with a new boot_id and the UART has been silent
        for self.quiet seconds (issue #11943). Both are needed: a stream
        that still runs brings its bytes by itself, and a stopped one
        would keep the run waiting for minutes after Linux is back."""
        self.told = False
        end = time.monotonic() + timeout
        while needle not in self.buf:
            now = time.monotonic()
            if now >= end:
                return False
            if back and self.back and now - self.last >= self.quiet and os.path.exists(self.back):
                self.told = True
                return False
            self.read(min(end - now, 0.2))
        return True

    def wait_login(self, timeout):
        """Reads until Linux's login, writing nothing; True when it came.

        The UART is not the only witness of Linux: the network serial
        adapter can stop its stream for good in the middle of the kernel's
        boot output (issue #11874), and a login printed then is never read.
        So a wait that ends without the login closes uart.log with one line
        that starts with "####" and says where the stream went silent (the
        hole in the record), and says the same in typed.log. Whether Linux
        is back is decided by the shell runner around the tool, over ssh
        (prompt-lib.sh, exit_when_back: a new boot_id).

        The runner tells the tool as soon as it sees that (wait_for's
        back), so a silent UART does not hold the run for the whole
        timeout after Linux is back. The line in uart.log then says that
        this is why the wait ended."""
        start = time.monotonic()
        came = self.wait_for(LOGIN, timeout, back=True)
        now = time.monotonic()
        self.waited = now - start
        if came:
            return True
        hole = "no byte was read after %.3f s (byte %d of uart.log); the wait for Linux's login ended at %.3f s, " \
               "%.1f s later" % (self.last - self.t0, self.nread, now - self.t0, now - self.last)
        if self.told:
            hole += (", because the runner saw Armbian over ssh with a new boot_id while the UART was silent: "
                     "what the board printed after this point is not in this record")
        else:
            hole += (": the UART stream stopped or the board printed nothing, and what it printed after this "
                     "point is not in this record")
        self.raw.write(b"\r\n#### %s: %s\r\n" % (self.tool.encode(), hole.encode()))
        self.raw.flush()
        self.log("no Linux login on the UART: " + hole)
        return False

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


def enter(con, cmd, timeout, ok=allowed, wait=True):
    """Type cmd, compare the echo, enter it, return its output.

    ok decides what may be typed (prompt-go.py passes its own list). With
    wait False nothing is read after the Enter (`go`: from then on the
    image owns the UART).
    """
    if not ok(cmd):
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
    if cmd == "reset" or not wait:
        return ""
    if not con.wait_prompt(timeout):
        if con.kernel:
            raise Stop(7, "U-Boot left the prompt during %r and started Linux: nothing was written after it" % cmd)
        raise Stop(5, "no prompt %d s after %r" % (timeout, cmd))
    out = clean(con.buf)
    out = out[:out.rfind("=> ")] if out.endswith("=> ") else out
    return out.lstrip("\n")


def run(fd, rec, boot_wait=240, login_wait=180, gap=0.003, cmd_wait=30, back=None, quiet=30.0):
    os.makedirs(rec, exist_ok=True)
    answers = {}
    with open(os.path.join(rec, "uart.log"), "wb") as raw, \
            open(os.path.join(rec, "typed.log"), "w") as typed, \
            open(os.path.join(rec, "answers.txt"), "w") as ans:
        con = Console(fd, raw, typed, gap, back=back, quiet=quiet)

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
            if not con.wait_for(AUTOBOOT, boot_wait, back=True):
                raise Stop(12, NO_AUTOBOOT_BACK if con.told else NO_AUTOBOOT % boot_wait)
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
            for n in uart2_decode(answers):
                say(n)
                ans.write("# %s\n" % n)
            say("%d commands answered; entering reset" % len(answers))
            one("reset")
            # From here the board must not be written to: disarm.
            con.armed = False
            con.buf = b""
            t = time.monotonic()
            if not con.wait_login(login_wait):
                raise Stop(6, "reset entered, no Linux login on the UART in %d s" % con.waited)
            say("done: %d commands, the Linux login %d s after reset" % (len(answers) - 1, time.monotonic() - t))
            return 0
        except Stop as s:
            con.armed = False
            if s.code in (4, 7):
                # Read on, written nothing more, so the record shows the boot.
                con.wait_login(login_wait)
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

    def __init__(self, master, mode="ok", words=None, dm=DM_PROBED, garble=0, kernel_stop=False):
        super().__init__(daemon=True)
        self.m, self.mode, self.dm = master, mode, dm
        # The network adapter's stream stops in the kernel's boot output: the
        # login after it never reaches the tool (issue #11874).
        self.kernel_stop = kernel_stop
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
        if self.kernel_stop:
            self.out("Starting kernel ...\r\n\r\n[    0.000000] Booting Linux on physical CPU 0x0000000000\r\n[    0.0")
        else:
            self.out("Starting kernel ...\r\n\r\nboard login: root (automatic login)\r\n# ")
        end = time.monotonic() + 0.6
        while time.monotonic() < end:
            self.shell += self.key(0.1)

    def after_enter(self, cmd):
        """A subclass plays what follows cmd itself and returns True (the
        prompt loop ends); here every command is answered at the prompt."""
        return False

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
                    if self.after_enter(cmd):
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


# The replay of a captured prompt session (testdata/prompt-reads/): the
# prediction of section 1 from the words first_commands() reads. A model of
# soc::rk3588::gpu_clock_set's rule for the self-test only; the tool never
# uses it.
GPU_OPP_MHZ = (300, 400, 500, 600, 700, 800, 900, 1000)
CPLL_SET_MHZ = 1500


def parse_transcript(text):
    """{command: output} of a captured session: lines `=> CMD`, then the
    output up to the next `=> `; `#` lines before the first command are
    the fixture's header and are dropped."""
    answers, cmd = {}, None
    for line in text.splitlines():
        if line.startswith("=> "):
            cmd = line[3:].strip()
            answers[cmd] = ""
        elif cmd is not None:
            answers[cmd] += line + "\n"
    return answers


def md_words(answer, addr, count):
    """The count words `md.l` printed from addr, or None."""
    words = []
    for line in answer.splitlines():
        m = re.match(r"([0-9a-f]{8}): ((?:[0-9a-f]{8} ?)+)", line)
        if m and int(m.group(1), 16) == addr + 4 * len(words):
            words += [int(w, 16) for w in m.group(2).split()]
    return words[:count] if len(words) >= count else None


def pll_mhz(answers, base, mode_shift):
    """A PLL's rate in MHz from its PLL_CON words and MODE_CON0 (normal mode
    only), or None."""
    w = md_words(answers.get(md(base, 7), ""), base, 7)
    mode = md_word(answers.get(md(0xFD7C0280), ""), 0xFD7C0280)
    if w is None or mode is None or (mode >> mode_shift) & 3 != 1:
        return None
    m, p, s = w[0] & 0x3ff, w[1] & 0x3f, (w[1] >> 6) & 7
    return None if p == 0 or w[2] & 0xffff else (24 * m // p) >> s


def clock_prediction(answers):
    """'keep', 'move' or 'refuse', as section 1's rule for the words read."""
    sel158 = md_word(answers.get(md(0xFD7C0578), ""), 0xFD7C0578)
    sel159 = md_word(answers.get(md(0xFD7C057C), ""), 0xFD7C057C)
    cpll = pll_mhz(answers, 0xFD7C01A0, 8)
    gpll = pll_mhz(answers, 0xFD7C01C0, 2)
    if sel158 is None or sel159 is None:
        return "refuse"
    div = (sel158 & 0x1f) + 1
    parent = None if sel158 & 0x4000 else {0: gpll, 1: cpll}.get((sel158 >> 5) & 7)
    if parent is not None and parent % div == 0 and parent // div in GPU_OPP_MHZ and sel159 & 0x1f == 0:
        return "keep"
    return "move" if cpll == CPLL_SET_MHZ else "refuse"


class Checks:
    """A self-test's checks, with no count kept by hand: every line of the
    test function that calls check() must have run by the end (a check in a
    loop counts once it has run), else the summary names the lines."""

    def __init__(self, fn):
        self.fn, self.ran, self.failed, self.n = fn, set(), [], 0

    def __call__(self, cond, what):
        self.n += 1
        self.ran.add(sys._getframe(1).f_lineno)
        if not cond:
            self.failed.append(what)
            print("FAIL: " + what)

    def lines(self):
        """The lines of the test function that call check()."""
        code = self.fn.__code__
        with open(code.co_filename) as f:
            src = f.read().splitlines()
        out = []
        for i in range(code.co_firstlineno, len(src)):
            if src[i][:1] not in ("", " ", "#"):
                break                       # the next top-level statement
            if re.match(r"\s*check\(", src[i]):
                out.append(i + 1)
        return out

    def summary(self, name):
        not_run = [n for n in self.lines() if n not in self.ran]
        print("%s self-test: %d checks run, %d failed%s" % (
            name, self.n, len(self.failed),
            ", check lines not run: " + " ".join(map(str, not_run)) if not_run else ""))
        return 0 if not self.failed and self.n > 0 and not not_run else 1


def self_test():
    import pty
    import tempfile
    check = Checks(self_test)

    def case(back=False, **kw):
        """back: the runner's file (Armbian answers over ssh with a new
        boot_id) is there from the start."""
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
                flag = os.path.join(d, "linux-back")
                if back:
                    open(flag, "w").close()
                fake.start()
                rc = run(slave, d, boot_wait=3, login_wait=3, gap=0, cmd_wait=2,
                         back=flag if back else None, quiet=1.0)
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
    wdt = [md(WDT0, 2), md(WDT0 + 0xF4, 3)]
    want = first_commands() + wdt + ["pmic list", "pmic dev pmic@0", "pmic dump", "reset"]
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
    check(b"####" not in files["uart.log"], "ok: the login came, uart.log has no line about a hole")

    # 1b. The UART stream stops in the kernel's output after reset: exit 6,
    # nothing written after reset, uart.log closed with one line that says
    # where the hole starts (its byte offset is the end of what was read).
    rc, f, files = case(kernel_stop=True)
    log = files["uart.log"]
    at = log.find(b"\r\n#### prompt-reads: no byte was read after ")
    m = re.search(rb"\(byte (\d+) of uart\.log\)", log[at:])
    check(rc == 6 and f.entered == want and f.shell == b"",
          "kernel stop: exit 6, the plan entered, nothing written after reset (rc %d, %r)" % (rc, f.shell))
    check(at > 0 and log.endswith(b"not in this record\r\n") and log.count(b"####") == 1
          and m is not None and int(m.group(1)) == at and log[:at].endswith(b"[    0.0"),
          "kernel stop: uart.log ends with one '####' line at the hole's byte (%r)" % log[-160:])
    check(b"no Linux login on the UART: no byte was read after" in files["typed.log"],
          "kernel stop: typed.log says the same")
    check(b"because the runner saw Armbian" not in log, "kernel stop: no word of ssh without the runner's file")

    # 1c. The same with the runner's file (Armbian answers over ssh, issue
    # #11943): the wait ends once the UART has been silent for quiet, still
    # exit 6 and one '####' line at the hole's byte, and the line says why.
    rc, f, files = case(kernel_stop=True, back=True)
    log = files["uart.log"]
    at = log.find(b"\r\n#### prompt-reads: no byte was read after ")
    m = re.search(rb"\(byte (\d+) of uart\.log\)", log[at:])
    check(rc == 6 and f.entered == want and f.shell == b"",
          "kernel stop, ssh sees Linux: exit 6, nothing written after reset (rc %d, %r)" % (rc, f.shell))
    check(at > 0 and log.endswith(b"not in this record\r\n") and log.count(b"####") == 1
          and m is not None and int(m.group(1)) == at and log[:at].endswith(b"[    0.0")
          and b"because the runner saw Armbian over ssh with a new boot_id while the UART was silent" in log[at:],
          "kernel stop, ssh sees Linux: one '####' line at the hole's byte, and it says why the wait ended (%r)"
          % log[-200:])
    w = re.search(rb"no Linux login on the UART in (\d+) s", files["typed.log"])
    check(w is not None and int(w.group(1)) < 3,
          "kernel stop, ssh sees Linux: typed.log gives the time waited, under the whole login wait (%r)"
          % (w.group(0) if w else None))

    # 1d. The runner's file alone ends nothing: with it there from the start
    # and the UART alive, the run is the whole run and the login is read.
    rc, f, files = case(back=True)
    check(rc == 0 and f.entered == want and b"####" not in files["uart.log"],
          "ssh sees Linux, the UART alive: exit 0, the login read, no line about a hole (rc %d)" % rc)

    # 2. PD_GPU on: the four QoS blocks are read, five words each.
    rc, f, _ = case(words=PMU_ON)
    qos = [c for c in f.entered if c.startswith("md.l 0xfdf35")]
    check(rc == 0 and qos == [md(b + 8, 5) for b in QOS_BLOCKS], "PD_GPU on: the QoS words are read (%r)" % qos)

    # 2b. WDT0's gates: read open (the fake's 0), the block is read after the
    # gate word; either gate closed, it is not read at all.
    rc, f, _ = case()
    check(f.entered.index(md(WDT0, 2)) > f.entered.index(md(CLKGATE_CON15)), "WDT0 open: read after its gate word")
    for gate in (1, 2, 3):
        rc, f, _ = case(words={**PMU_OFF, CLKGATE_CON15: gate})
        check(rc == 0 and not any(c.startswith("md.l 0xfeaf") for c in f.entered),
              "WDT0 gate word %#x: WDT0 is not read" % gate)

    # 3. The pmic not probed: no pmic command at all.
    rc, f, _ = case(dm=DM_UNPROBED)
    check(rc == 0 and not any(c.startswith("pmic") for c in f.entered), "pmic not probed: no pmic command")

    # 4. The autoboot is missed: one space, then nothing, whatever Linux prints.
    rc, f, _ = case(mode="missed")
    check(rc == 4 and f.got == b" " and f.entered == [], "missed: exit 4, one space written in all (%r)" % f.got)

    # 5. No autoboot line: nothing is written.
    rc, f, files = case(mode="noautoboot")
    check(rc == 12 and f.got == b"", "no autoboot line: exit 12, nothing written (%r)" % f.got)
    check(b"STOP (12): no autoboot line in 3 s after the reboot" in files["typed.log"],
          "no autoboot line: typed.log says the whole wait passed")

    # 5b. The same when ssh sees Linux: the wait for the line ends once the
    # UART is silent, and the stop says so.
    rc, f, files = case(mode="noautoboot", back=True)
    check(rc == 12 and f.got == b"" and b"STOP (12): " + NO_AUTOBOOT_BACK.encode() in files["typed.log"],
          "no autoboot line, ssh sees Linux: exit 12, nothing written, the stop names both witnesses (%r)" % f.got)

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

    # 11. Replay of captured prompt sessions: every command of a fixture is
    # one the tool enters, and the decoded decisions are the expected ones.
    here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata", "prompt-reads")
    plan_cmds = set(first_commands())
    expect = {"section4-2026-11-12.txt": ("move", False, 0xa003, False),
              "synthetic-keep-pd-on.txt": ("keep", True, 0xa000, True)}
    for name, (clock, pd_on, gate15, open_) in expect.items():
        with open(os.path.join(here, name)) as f:
            text = f.read()
        ans = parse_transcript(text)
        pd = "on" if pd_on else "off"
        check(set(ans) <= plan_cmds and all(allowed(c) for c in ans),
              name + ": every command is in the plan and allowed")
        check(clock_prediction(ans) == clock, name + ": gpu_clock_set predicted " + clock)
        check(pd_gpu_on(ans) == pd_on, name + ": PD_GPU reads " + pd)
        check(md_word(ans[md(CLKGATE_CON15)], CLKGATE_CON15) == gate15,
              name + ": CLKGATE_CON(15) is 0x%08x" % gate15)
        check(wdt0_clocked(ans) == open_, name + ": WDT0's gates read " + ("open" if open_ else "closed"))
        cmds, _ = later_commands(ans)
        check((md(WDT0, 2) in cmds) == open_, name + ": WDT0 is read only with both gates open")
        check(("expect clock=%s pd_gpu=%s " % (clock, pd)) in text,
              name + ": the header's expectation says the same")
        check("SYNTHETIC" in text, name + ": the header marks synthetic words")
        check(not re.search(r"(?i)/dev/|tty|serial|ethaddr|[0-9a-f]{2}(:[0-9a-f]{2}){5}|\.(local|lan)\b", text),
              name + ": no tty path, serial or MAC in the fixture")
    with open(os.path.join(here, "section4-2026-11-12.txt")) as f:
        sec4 = parse_transcript(f.read())
    check(pll_mhz(sec4, 0xFD7C01A0, 8) == 1500 and pll_mhz(sec4, 0xFD7C01C0, 2) == 1188,
          "replay: CPLL 1500 MHz and GPLL 1188 MHz, both in normal mode")
    off_rate = dict(sec4)
    off_rate[md(0xFD7C01A0, 7)] = off_rate[md(0xFD7C01A0, 7)].replace("000000fa", "000000c8")
    check(clock_prediction(off_rate) == "refuse", "replay: a CPLL at another rate predicts refuse")
    pvt = dict(sec4)
    pvt[md(0xFD7C0578)] = "fd7c0578: 00005f24\n"
    check(clock_prediction(pvt) == "move", "replay: the PVTPLL bit predicts move")

    # 12. UART2's reads and their decode. Only reads with no side effect; the
    # divisor is not read; DLAB, the clock source and the computed rate.
    u2cmds = [md(UART2_LCR), md(UART2_USR), md(UART2_SEL, 3), md(UART2_GATE12, 2)]
    check(all(c in plan_cmds and allowed(c) for c in u2cmds), "uart2: its reads are in the plan and allowed")
    check(all(a not in (UART2 + 0x00, UART2 + 0x08, UART2 + 0x14, UART2 + 0x18) for a in
              [int(c.split()[1], 16) for c in u2cmds]),
          "uart2: no RBR, IIR, LSR or MSR read")
    check(md(UART2_LCR) == "md.l 0xfeb5000c 1" and md(UART2_SEL, 3) == "md.l 0xfd7c03ac 3"
          and md(UART2_GATE12, 2) == "md.l 0xfd7c0830 2", "uart2: the addresses")
    check(not any(re.match(r"(?:mw|mm|nm|cp|setenv|saveenv)\b", c) for c in first_commands()),
          "uart2: the plan has no write command")
    with open(os.path.join(here, "synthetic-uart2-pll-frac.txt")) as f:
        utext = f.read()
    u2 = parse_transcript(utext)
    check(set(u2) <= plan_cmds and all(allowed(c) for c in u2), "uart2 fixture: every command is in the plan and allowed")
    check("SYNTHETIC" in utext and "expect uart2_hz=100000000 baud=6250000 dlab=clear" in utext,
          "uart2 fixture: marked synthetic, header says the expectation")
    check(not re.search(r"(?i)/dev/|tty|serial|ethaddr|[0-9a-f]{2}(:[0-9a-f]{2}){5}|\.(local|lan)\b", utext),
          "uart2 fixture: no tty path, serial or MAC")
    d = "\n".join(uart2_decode(u2))
    check("not read" in d and "DLAB" in d and "LCR.DLAB" in d, "uart2: the decode says the divisor is not read, and why")
    check("8N1, DLAB clear" in d and "cpll / 5 * 1 / 3 (fractional divider)" in d
          and "input clock 100000000 Hz; divisor 1 gives 6250000 baud" in d,
          "uart2: a PLL source with a fractional divider decodes to the computed rate")
    check(pll_hz(u2, 0xFD7C01A0) == 1500000000 and pll_hz(u2, 0xFD7C01C0) == 1188000000, "uart2: PLL Hz")
    osc = dict(u2)
    osc[md(UART2_SEL, 3)] = "fd7c03ac: 00000090 00010003 00000002\n"
    d = "\n".join(uart2_decode(osc))
    check("xin24m (24 MHz OSC)" in d and "input clock 24000000 Hz; divisor 1 gives 1500000 baud" in d,
          "uart2: 24 MHz with divisor 1 decodes to 1500000")
    dlab = dict(u2)
    dlab[md(UART2_LCR)] = "feb5000c: 00000083                             ....\n"
    d = "\n".join(uart2_decode(dlab))
    check("DLAB SET" in d and "not 8N1" in d, "uart2: DLAB set is flagged")
    gpll = dict(u2)
    gpll[md(UART2_SEL, 3)] = "fd7c03ac: 00000008 00000000 00000000\n"
    check("input clock 396000000 Hz" in "\n".join(uart2_decode(gpll)) and uart2_decode(sec4) == [],
          "uart2: the gpll source with the plain divider; no UART2 reads, no decode")

    return check.summary("prompt-reads")


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if argv[1:] == ["plan"]:
        print("\n".join(first_commands()))
        print("# then, if CLKGATE_CON(15) reads both WDT0 gates open: %s; %s" % (md(WDT0, 2), md(WDT0 + 0xF4, 3)))
        print("# then, if PD_GPU reads on: " + "; ".join(md(b + 8, 5) for b in QOS_BLOCKS))
        print("# then, if dm tree shows the pmic probed: pmic list; pmic dev <its name>; pmic dump")
        print("reset")
        return 0
    if len(argv) in (4, 5) and argv[1] == "run":
        fd = open_tty(argv[2])
        try:
            return run(fd, argv[3], back=argv[4] if len(argv) == 5 else None)
        finally:
            os.close(fd)
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
