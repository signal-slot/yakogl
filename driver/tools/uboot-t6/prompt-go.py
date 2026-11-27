#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Start a Zephyr image on the NanoPC-T6 from its U-Boot prompt.

    prompt-go.py run TTY RECORD_DIR NAME SIZE CRC PROBE IMAGE_WAIT [BACK]
        Opens the board's debug UART and waits, writing nothing, for
        U-Boot's autoboot line (the caller reboots the board: prompt-go.sh).
        At that line one space, and at the prompt one command at a time,
        each typed, its echo compared, entered, and its answer checked
        before the next:

          ext4size mmc 0:1 /boot/yakogl/NAME      no answer (an error is one)
          printenv filesize                      filesize=<SIZE in hex>
          ext4load mmc 0:1 0x10000000 /boot/yakogl/NAME
                                                 "<SIZE> bytes read in ..."
          md.l 0x10000038 1                      10000038: 644d5241 (ARM\\x64)
          crc32 0x10000000 <SIZE in hex>         ... ==> <CRC>
          go 0x10000000

        After `go` nothing is written to the UART again, whatever comes.
        The image's output and what follows go to image.log until Linux's
        login (IMAGE_WAIT seconds for the image, then 180 s for Linux), and
        t6-verdict.sh PROBE judges image.log (T6_ROUTE=uboot) into
        verdict.txt; PROBE gl_test or deqp is judged here instead
        (judge_sample: the T6:A and T6:G lines as t6-verdict.sh's uboot
        route has them, the sample's summary lines, the board glue's PSCI
        reset line), with the same last line. An answer that differs stops before `go`: `reset` is
        entered and the board boots Armbian.
        SIZE is decimal bytes, CRC eight hex digits (zlib's CRC-32, the one
        U-Boot's crc32 prints). RECORD_DIR also gets uart.log, typed.log and
        answers.txt, as prompt-reads.py's (a wait for Linux's login that
        ends without it closes uart.log with a "####" line that says where
        the stream went silent; image.log keeps only the bytes read).
        BACK is prompt-reads.py's: the file the caller makes when Armbian
        answers over ssh with a new boot_id, which ends a wait for the
        autoboot line or for Linux's login on a UART silent for 30 s.
    prompt-go.py plan NAME SIZE CRC
        Prints the commands and what each answer must be, nothing else.
    prompt-go.py --self-test
        The same code against a made-up U-Boot and image on a pty. No board.

The UART engine is prompt-reads.py's (loaded from beside this file): its
Console, its enter() with this file's list of what may be typed, its rules.
Nothing is written before the autoboot line, nothing after "Starting kernel"
and nothing after `go`; a line whose echo differs is never entered.

What may be typed, and nothing else (go_allowed): the six commands above for
this NAME and SIZE, and `reset`. NAME is a file name of letters, digits,
`.`, `_` and `-` ending in `.bin`, under /boot/yakogl; the load and entry
address is the image's link address, 0x10000000 (nanopc_t6.dts zephyr,sram;
board_lock_selftest.sh compares LOAD with it).

The UART alone cannot say that Linux is not back: the network serial adapter
can stop its stream for good in the middle of the kernel's boot output
(issue #11874), and a login printed after that is never read. So 6 and 10
below mean "no Linux login on the UART"; prompt-go.sh then asks the board
over ssh, and only when that finds no new boot either does a 6 or a 10 stay
its exit (prompt-lib.sh, exit_when_back). With BACK the same answer over
ssh also ends the wait early (issue #11943): 6, 10 and 12 below are then
given as soon as the UART has been silent for 30 s.

Exit: 0 the image ran, its verdict is PASS and Linux's login came back. 2
usage. 12 no autoboot line in time (nothing written; not 3, which
prompt-go.sh keeps for "refused before the reboot"). 4 the autoboot was
missed (one space written, Linux booted). 5 stopped at the prompt (an echo
that never matched, a command without an answer, the prompt lost): the board
stays at the U-Boot prompt until its power is cut. 6 an answer differed,
`reset` entered, no Linux login on the UART in time. 7 U-Boot left the
prompt during a command and started Linux. 8 an answer differed (size,
load, magic, CRC): `go` was not entered, `reset` was, and Linux's login
came back. 9 `go`
entered and U-Boot did not answer it ("## Starting application" never
came): nothing more was written. 10 the image started, its verdict was
taken, no Linux login on the UART in time. 11 Linux's login came back
and the verdict is FAIL (verdict.txt names the lines; for gl_test that is
also a case failed or timed out, for a dEQP row no "deqp: done:" or no
"deqp: summary:" line).
"""

import importlib.util
import os
import re
import subprocess
import sys
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
# Loaded by path: no __pycache__ is left beside the tools.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("prompt_reads", os.path.join(HERE, "prompt-reads.py"))
P = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(P)

VERDICT = os.path.join(HERE, "..", "kexec-zephyr", "t6-verdict.sh")

# The image's link address: nanopc_t6.dts zephyr,sram, 256 MiB at 0x10000000.
LOAD = 0x10000000
DIR = "/boot/yakogl"
MAGIC_OFF = 0x38                    # the arm64 Image header's magic, byte 56
IMAGE_MAGIC = 0x644D5241            # "ARM\x64" read as a little-endian word
STARTED = b"## Starting application at 0x%08X" % LOAD
NAME_RE = re.compile(r"[A-Za-z0-9_][A-Za-z0-9._-]*\.bin")
# The probes t6-verdict.sh judges that U-Boot can start (prompt-go.sh's
# list; t6_display is not one: it adopts a picture only Linux leaves).
VERDICT_TYPES = ("t6_hello", "t6_gpu", "t6_mmu", "t6_fw", "t6_submit", "t6_cold", "triangle")
# The samples judged here by their own summary lines (judge_sample): gl_test
# (driver/samples/zephyr/gl_test, main_mali.c) and a dEQP row image
# (driver/samples/zephyr/deqp, tools/cts/t6-deqp-images.sh).
SAMPLE_TYPES = ("gl_test", "deqp")
TYPES = VERDICT_TYPES + SAMPLE_TYPES


def commands(name, size):
    """The commands for NAME and SIZE, in order; `go` last."""
    path = "%s/%s" % (DIR, name)
    return [
        "ext4size mmc 0:1 %s" % path,
        "printenv filesize",
        "ext4load mmc 0:1 0x%08x %s" % (LOAD, path),
        P.md(LOAD + MAGIC_OFF),
        "crc32 0x%08x %x" % (LOAD, size),
        "go 0x%08x" % LOAD,
    ]


def go_allowed(name, size):
    """The test of what may be typed for this NAME and SIZE."""
    ok = set(commands(name, size)) | {"reset"}
    return lambda cmd: cmd in ok


def checks(name, size, crc):
    """What each answer before `go` must be: (command, check, wait). A check
    returns None when the answer is right, otherwise why it is not."""
    c = commands(name, size)
    last = LOAD + size - 1

    def no_answer(out):
        return None if out.strip() == "" else "ext4size answered %r: the file is not there as named" % out.strip()

    def filesize(out):
        want = "filesize=%x" % size
        return None if out.strip() == want else "%r, not %r: the staged file is not the host's size" % (out.strip(), want)

    def loaded(out):
        m = re.search(r"^(\d+) bytes read in \d+ ms", out, re.M)
        if not m:
            return "ext4load answered %r, no 'bytes read' line" % out.strip()
        return None if int(m.group(1)) == size else "ext4load read %s bytes, not %d" % (m.group(1), size)

    def magic(out):
        w = P.md_word(out, LOAD + MAGIC_OFF)
        return None if w == IMAGE_MAGIC else "the word at byte 56 is %s, not 0x%08x (ARM\\x64)" % (
            "unread" if w is None else "0x%08x" % w, IMAGE_MAGIC)

    def crc32(out):
        m = re.search(r"^crc32 for ([0-9a-f]{8}) \.\.\. ([0-9a-f]{8}) ==> ([0-9a-f]{8})\s*$", out, re.M | re.I)
        if not m:
            return "crc32 answered %r" % out.strip()
        got = (int(m.group(1), 16), int(m.group(2), 16), m.group(3).lower())
        want = (LOAD, last, "%08x" % crc)
        return None if got == want else "crc32 %08x ... %08x ==> %s, not %08x ... %08x ==> %s" % (got + want)

    return [(c[0], no_answer, 30), (c[1], filesize, 30), (c[2], loaded, 120), (c[3], magic, 30), (c[4], crc32, 60)]


class Refused(Exception):
    pass


# SCTLR_EL2's M (bit 0), C (bit 2) and I (bit 12): the entry turns them off
# (t6-verdict.sh's SCTLR_MCI).
SCTLR_MCI = 0x1005
X16 = r"[0-9a-f]{16}"
T6A_RE = re.compile(r"T6:A EL2 HCR_EL2 0x%s -> 0x%s SCTLR_EL2 0x(%s) VBAR_EL2 0x%s -> 0x%s CNTFRQ_EL0 (\d+)" % (X16, X16, X16, X16, X16))
T6G_RE = re.compile(r"T6:G go: SCTLR_EL2 0x(%s) -> 0x(%s) \(MMU, D-cache, I-cache off\); 0x10000000-0x20000000 "
                    r"cleaned and invalidated to PoC, TLB and I-cache invalidated" % (X16, X16))
# The runner's all_line (driver/tools/pvrlab/runner/src/lib.rs, printed by every
# gl_test image): "gl_test: all: F of T cases failed (P passed; X timed out; first
# failure case A at index B; second failure case C; K skipped, first skip case D);
# precompiled hits H, compiler runs R". Groups: 1 F, 2 T, 3 P, 4 X, 5 K (skipped).
GL_ALL_RE = re.compile(r"gl_test: all: (-?\d+) of (\d+) cases failed \((\d+) passed; (\d+) timed out;"
                       r"[^)]*?; (\d+) skipped, first skip case -?\d+\)(?:; .*)?")
# The size of the regression set: gl_test's list mali_regress, 50 cases ("the list
# mali_regress (50 cases)"; tools/cts/RESULTS.md's T6 row: 50 of 50 at 9ad07f55f).
GL_TOTAL = 50
# deqp/src/main.cpp's "deqp: done: ...", then deqp_log.c's "deqp: summary: ..."
DEQP_DONE_RE = re.compile(r"deqp: done: -?\d+ passed, -?\d+ failed, -?\d+ not supported, -?\d+ warnings, "
                          r"complete=-?\d+")
DEQP_SUMMARY_RE = re.compile(r"deqp: summary: \d+ cases begun, \d+ with a result, \d+ without")
# nanopc_t6.c, t6_reset_to_linux: "t6: <why>: PSCI SYSTEM_RESET at N ms (F feeds)"
RESET_PFX = re.compile(r"t6: [^:]+: PSCI SYSTEM_RESET at ")
# t6-verdict.sh's banner_re: the next boot's first line after a WDT0 reset
BANNER_RE = re.compile(r"(DDR[ \t]V[0-9]|U-Boot[ \t][0-9])")


def judge_sample(probe, log, out):
    """gl_test or deqp: image.log judged by the go route's two boot lines,
    the sample's own summary lines and the board glue's reset line, in that
    order, into out in t6-verdict.sh's form: one line per expected line
    ("ok", "differs" or "missing"), then "VERDICT <probe> PASS" or
    "VERDICT <probe> FAIL (<n> of <m>)". Returns (rc, last line), rc 0 on PASS.

    The boot lines are the ones t6-verdict.sh checks with T6_ROUTE=uboot:
    T6:A's SCTLR_EL2 (SA) with M, C and I clear; T6:G's value as found (GF)
    with one of them set and its value read back (GL) with all three clear
    and equal to SA. gl_test then needs its one "gl_test: all:" line with 0
    failed, 0 timed out and at least one case; a dEQP row needs "deqp: done:"
    and then "deqp: summary:" (its cases' outcomes are tallied later from
    image.log, not here). Last the reset line "t6: <probe> done: PSCI
    SYSTEM_RESET at N ms (F feeds)", which the sample's own end calls."""
    with open(log, "rb") as f:
        lines = f.read().replace(b"\r", b"").replace(b"\0", b"").decode("latin-1").split("\n")
    res = []
    cur = [0]
    v = {}

    def step(what, pfx, full, test=None):
        """The first line at or after cur that starts with pfx (a string, or
        a compiled pattern matched at the start) is the candidate; "ok" when
        full matches all of it and test(match) is true."""
        for j in range(cur[0], len(lines)):
            l = lines[j]
            if (l.startswith(pfx) if isinstance(pfx, str) else pfx.match(l)):
                cur[0] = j + 1
                m = full.fullmatch(l)
                ok = m is not None and (test is None or test(m))
                res.append((ok, '%s expected %s; seen "%s" (line %d)' % (
                    "ok      " if ok else "differs ", what, l, j + 1)))
                return
        res.append((False, "missing  expected %s; seen nothing after line %d" % (what, cur[0])))

    def t6a(m):
        v["SA"] = m.group(1)
        return int(m.group(1), 16) & SCTLR_MCI == 0

    def t6g(m):
        gf, gl = m.group(1), m.group(2)
        return int(gf, 16) & SCTLR_MCI != 0 and int(gl, 16) & SCTLR_MCI == 0 and gl == v.get("SA")

    step("T6:A with SCTLR_EL2's M, C and I clear", "T6:A EL", T6A_RE, t6a)
    step("T6:G: SCTLR_EL2 found with M, C or I set, left with all three clear and equal to T6:A's", "T6:G ",
         T6G_RE, t6g)
    if probe == "gl_test":
        n_all = sum(1 for l in lines if l.startswith("gl_test: all: "))

        def all_ok(m):
            # one such line only: a second is a second boot (or a defect)
            return (int(m.group(1)) == 0 and int(m.group(4)) == 0 and int(m.group(5)) == 0
                    and int(m.group(2)) == GL_TOTAL and int(m.group(3)) == GL_TOTAL and n_all == 1)

        step("one \"gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; ...; 0 skipped ...)\" line", "gl_test: all: ",
             GL_ALL_RE, all_ok)
    else:
        step('"deqp: done: ... complete=C"', "deqp: done: ", DEQP_DONE_RE)
        step('"deqp: summary: N cases begun, M with a result, K without"', "deqp: summary: ", DEQP_SUMMARY_RE)
    # The return: the image's own PSCI line, or (WDT0 expiry prints nothing)
    # the next U-Boot banner after the summary, as t6-verdict.sh counts it.
    # A hung image has no summary, so its step above already failed.
    route = "psci"
    if not any(RESET_PFX.match(l) for l in lines[cur[0]:]):
        j = next((k for k in range(cur[0], len(lines)) if BANNER_RE.match(lines[k])), None)
        if j is not None and all(ok for ok, _ in res):
            route = "wdt0"
            res.append((True, 'ok       expected the return by WDT0: no PSCI line, the next U-Boot banner "%s" (line %d)' % (
                lines[j], j + 1)))
    if route == "psci":
        step('"t6: %s done: PSCI SYSTEM_RESET at N ms (F feeds)"' % probe, RESET_PFX,
             re.compile(r"t6: %s done: PSCI SYSTEM_RESET at \d+ ms \(\d+ feeds\)" % probe))
    seen = "route: " + route
    bad = sum(1 for ok, _ in res if not ok)
    last = "VERDICT %s PASS" % probe if bad == 0 else "VERDICT %s FAIL (%d of %d)" % (probe, bad, len(res))
    with open(out, "w") as f:
        f.write("".join(t + "\n" for _, t in res) + seen + "\n" + last + "\n")
    return (0 if bad == 0 else 1), last


def judge(probe, log, out):
    """t6-verdict.sh PROBE LOG with T6_ROUTE=uboot into out: (rc, last line).
    gl_test and deqp are judge_sample's."""
    if probe in SAMPLE_TYPES:
        return judge_sample(probe, log, out)
    env = dict(os.environ, T6_ROUTE="uboot", LC_ALL="C")
    r = subprocess.run(["bash", VERDICT, probe, log], env=env, capture_output=True, text=True)
    with open(out, "w") as f:
        f.write(r.stdout + r.stderr)
    lines = (r.stdout + r.stderr).strip().splitlines()
    return r.returncode, lines[-1] if lines else ""


CNTFRQ0_RE = re.compile(rb"^t6: CNTFRQ_EL0 0(?![0-9A-Za-z])", re.M)


def note_cause(log, out, vrc, vline):
    """An image that stopped on "t6: CNTFRQ_EL0 0" (the board glue found no
    counter frequency and refused to start the osal and firmware) is named
    in verdict.txt: a "cause:" line above the rest, and the verdict a
    failure (the image failure's own rc 1 -> exit 11), never a pass.
    Returns (rc, last line)."""
    with open(log, "rb") as f:
        data = f.read().replace(b"\r", b"").replace(b"\0", b"")
    if not CNTFRQ0_RE.search(data):
        return vrc, vline
    with open(out) as f:
        lines = f.read().splitlines()
    last = lines[-1] if lines else ""
    probe = last.split()[1] if last.startswith("VERDICT ") and len(last.split()) > 1 else ""
    if not (last.startswith("VERDICT ") and " FAIL" in last):
        last = ("VERDICT %s FAIL (cause: CNTFRQ_EL0 0)" % probe) if probe else "VERDICT FAIL (cause: CNTFRQ_EL0 0)"
    cause = "cause: CNTFRQ_EL0 0 at entry (U-Boot left the counter frequency unset)"
    lines = lines[:-1] + [cause, last]
    with open(out, "w") as f:
        f.write("\n".join(lines) + "\n")
    return (vrc or 1), last


RETURN_RE = re.compile(rb"PSCI SYSTEM_RESET at \d+ ms")
FEED_RE = re.compile(rb"yakogl-wdt: nanopc_t6 first feed at (\d+) ms")


def note_return(log, out):
    """Record how the image returned, as one line in verdict.txt just above
    the VERDICT line (which stays last). The exit code does not change: a
    PSCI SYSTEM_RESET is the image's own line; a WDT0 expiry prints nothing,
    so the absence of that line is the only trace of it."""
    with open(log, "rb") as f:
        data = f.read().replace(b"\r", b"")
    m = RETURN_RE.search(data)
    if m:
        ln = data[data.rfind(b"\n", 0, m.start()) + 1:].split(b"\n")[0].decode("latin-1")
        note = "return: PSCI SYSTEM_RESET (%s)" % ln
    else:
        note = "return: no PSCI SYSTEM_RESET line in image.log (WDT0 expiry or another reset: the image printed none)"
    fm = FEED_RE.search(data)
    feeder = "feeder: first feed at %s ms" % fm.group(1).decode() if fm else "feeder: no first-feed line"
    with open(out) as f:
        lines = f.read().splitlines()
    # informational only; above the return line, VERDICT stays last
    lines[max(len(lines) - 1, 0):max(len(lines) - 1, 0)] = [feeder, note]
    with open(out, "w") as f:
        f.write("\n".join(lines) + "\n")


def run(fd, rec, name, size, crc, probe, image_wait, boot_wait=240, login_wait=180, gap=0.003, scale=1.0,
        back=None):
    os.makedirs(rec, exist_ok=True)
    ok = go_allowed(name, size)
    with open(os.path.join(rec, "uart.log"), "wb") as raw, \
            open(os.path.join(rec, "typed.log"), "w") as typed, \
            open(os.path.join(rec, "answers.txt"), "w") as ans:
        con = P.Console(fd, raw, typed, gap, tool="prompt-go", back=back, quiet=30 * scale)

        def say(msg):
            con.log(msg)
            print("prompt-go: " + msg, flush=True)

        def answered(cmd, out):
            ans.write("## %s\n%s" % (cmd, out if out.endswith("\n") or not out else out + "\n"))
            ans.flush()

        try:
            say("listening, nothing written (waiting up to %d s for the autoboot line)" % boot_wait)
            if not con.wait_for(P.AUTOBOOT, boot_wait, back=True):
                raise P.Stop(12, P.NO_AUTOBOOT_BACK if con.told else P.NO_AUTOBOOT % boot_wait)
            con.armed = True
            con.buf = b""
            con.write(b" ")
            if not con.wait_prompt(5):
                raise P.Stop(4, "no U-Boot prompt after the one space: the autoboot was missed")
            say("at the U-Boot prompt")
            try:
                for cmd, check, wait in checks(name, size, crc):
                    out = P.enter(con, cmd, wait * scale, ok=ok)
                    answered(cmd, out)
                    why = check(out)
                    if why:
                        raise Refused(why)
                    say("%s: as expected" % cmd)
            except Refused as r:
                say("REFUSED before go: %s; entering reset" % r)
                P.enter(con, "reset", 0, ok=ok)
                con.armed = False
                con.buf = b""
                if not con.wait_login(login_wait):
                    raise P.Stop(6, "reset entered after the refusal, no Linux login on the UART in %d s" % con.waited)
                say("STOP (8): refused before go, reset entered, Linux's login came back")
                return 8
            go = commands(name, size)[-1]
            P.enter(con, go, 0, ok=ok, wait=False)
            # From here the image owns the UART: never write again.
            con.armed = False
            answered(go, "")
            say("%s entered: nothing is written from here on" % go)
            con.buf = b""
            started = con.wait_for(STARTED, 10 * scale)
            back = con.wait_login(image_wait + login_wait)
            log = os.path.join(rec, "image.log")
            with open(log, "wb") as f:
                f.write(con.buf)
            if not started:
                say("STOP (9): U-Boot did not answer %s in %d s; nothing more was written" % (go, 10 * scale))
                return 9
            vrc, vline = judge(probe, log, os.path.join(rec, "verdict.txt"))
            vrc, vline = note_cause(log, os.path.join(rec, "verdict.txt"), vrc, vline)
            note_return(log, os.path.join(rec, "verdict.txt"))
            say("verdict: %s" % vline)
            if not back:
                # Not "the board needs a hand": the UART may have stopped in
                # the kernel's output; prompt-go.sh asks the board over ssh.
                say("STOP (10): the image started, no Linux login on the UART in %d s (uart.log's last line "
                    "says where the stream went silent)" % con.waited)
                return 10
            if vrc != 0:
                say("STOP (11): Linux's login came back, the verdict is not PASS (verdict.txt)")
                return 11
            say("done: the verdict is PASS and Linux's login came back")
            return 0
        except P.Stop as s:
            con.armed = False
            if s.code in (4, 7):
                con.wait_login(login_wait)
            say("STOP (%d): %s" % (s.code, s.why))
            return s.code


def plan(name, size, crc):
    out = []
    for (cmd, _, _), want in zip(checks(name, size, crc), [
            "no answer", "filesize=%x" % size, "%d bytes read in ... ms" % size,
            "%08x: %08x" % (LOAD + MAGIC_OFF, IMAGE_MAGIC),
            "crc32 for %08x ... %08x ==> %08x" % (LOAD, LOAD + size - 1, crc)]):
        out.append("%-48s # %s" % (cmd, want))
    out.append("%-48s # then nothing is typed again" % commands(name, size)[-1])
    out.append("# any other answer: reset, and the board boots Armbian")
    return "\n".join(out)


# --------------------------------------------------------------------------
# Self-test: a made-up U-Boot and image on a pty.

FIXTURE = os.path.join(HERE, "..", "kexec-zephyr", "testdata", "t6_hello-uboot-pass.uart")


def image_lines():
    """The predicted go record's image part (the verdict's own fixture)."""
    with open(FIXTURE) as f:
        lines = [l.rstrip("\n") for l in f if not l.startswith("# ")]
    return lines[lines.index("## Starting application at 0x10000000 ...") + 1:]


def sample_lines(probe, how="ok"):
    """A made-up go record of a gl_test or dEQP row image: the fixture's boot
    lines up to Zephyr's banner, then the sample's lines and its reset line.
    how: "ok"; "fail1" (gl_test, one case failed); "nodone" (deqp, no
    "deqp: done:" line); "caches" (T6:G read back with the D-cache on)."""
    lines = image_lines()
    boot = lines[:[i for i, l in enumerate(lines) if l.startswith("*** Booting Zephyr")][0] + 1]
    if how == "caches":
        boot = [l.replace("-> 0x0000000030c50830 (MMU", "-> 0x0000000030c50834 (MMU") for l in boot]
    if probe == "gl_test":
        failed = 1 if how == "fail1" else 0
        own = ["t6_gl_test: build 51e4a974",
               "t6_gl_test: WDT0 period 22369 ms, fed until 300000 ms of uptime, alive window 20 s",
               "gl_test: the list mali_regress (50 cases) on one context",
               "gl_test: case 1 clear: PASS",
               "gl_test: case 2 triangle: %s" % ("FAIL" if failed else "PASS"),
               "gl_test: all: %d of 50 cases failed (%d passed; 0 timed out; first failure case %d at index %d; "
               "second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 529"
               % (failed, 50 - failed, 2 if failed else -1, 1 if failed else -1),
               "gl_test: %s (rc=%d)" % ("FAILED" if failed else "OK", failed),
               "t6: gl_test done: PSCI SYSTEM_RESET at 81234 ms (81 feeds)"]
    else:
        # a row with a failed case still PASSes: the cases are tallied later
        own = ["deqp_mali: build 51e4a974",
               "deqp: Pass dEQP-GLES2.info.vendor",
               "deqp: Fail dEQP-GLES2.functional.prerequisite.read_pixels",
               "deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1",
               "deqp: summary: 2 cases begun, 2 with a result, 0 without",
               "deqp: FAILED (rc=-1)",
               "t6: deqp done: PSCI SYSTEM_RESET at 412345 ms (412 feeds)"]
        if how == "nodone":
            own = [l for l in own if not l.startswith("deqp: done: ")]
    return boot + own


class FakeGo(P.FakeBoard):
    """U-Boot that answers the go route's commands, then plays the image.
    A how that ends in "-kstop" plays the UART's stream stopping in the
    kernel's output after the reset that follows (FakeBoard.kernel_stop)."""

    def __init__(self, master, name, size, crc, how="ok", lines=None):
        kstop = how.endswith("-kstop")
        how = how[:-len("-kstop")] if kstop else how
        words = dict(P.PMU_OFF)
        words[LOAD + MAGIC_OFF] = 0x12345678 if how == "magic" else IMAGE_MAGIC
        mode = how if how in ("noautoboot", "missed") else "ok"
        super().__init__(master, mode=mode, words=words, kernel_stop=kstop)
        self.name, self.size, self.crc, self.how = name, size, crc, how
        self.lines = image_lines() if lines is None else lines

    def answer(self, cmd):
        if cmd.startswith("ext4size "):
            if self.how == "size-text":
                return "Failed to mount ext2 filesystem...\r\n"
            return "** File not found %s **\r\n" % cmd.split()[-1] if self.how == "missing" else ""
        if cmd == "printenv filesize":
            return "filesize=%x\r\n" % (self.size + (4 if self.how == "size" else 0))
        if cmd.startswith("ext4load "):
            if self.how == "load":
                return "Failed to load '%s'\r\n" % cmd.split()[-1]
            return "%d bytes read in 31 ms (13.2 MiB/s)\r\n" % (self.size - (512 if self.how == "load-n" else 0))
        if cmd.startswith("crc32 "):
            crc = self.crc ^ (1 if self.how == "crc" else 0)
            # crc-range: the right CRC over another range (one byte short)
            last = LOAD + self.size - (2 if self.how == "crc-range" else 1)
            return "crc32 for %08x ... %08x ==> %08x\r\n" % (LOAD, last, crc)
        return super().answer(cmd)

    def after_enter(self, cmd):
        if self.how == "lost" and cmd.startswith("ext4load "):
            return True                     # the prompt never comes back
        if self.how == "uboot-boots" and cmd.startswith("ext4load "):
            self.out("\r\nU-Boot SPL 2026.07\r\n")   # a command made U-Boot reset and boot Linux
            self.linux()
            return True
        if not cmd.startswith("go "):
            return False
        if self.how == "silent":
            end = time.monotonic() + 1.0
            while time.monotonic() < end:
                self.shell += self.key(0.1)  # anything typed now is a fault
            return True
        self.out("## Starting application at 0x%08X ...\r\n" % LOAD)
        for l in self.lines:
            self.out(l + "\r\n")
        if self.how == "nologin":
            end = time.monotonic() + 1.0
            while time.monotonic() < end:
                self.shell += self.key(0.1)
            return True
        # The image's PSCI reset: U-Boot again, its autoboot not answered.
        self.out("\r\nU-Boot SPL 2026.07\r\n\x1b[2KHit any key to stop autoboot:  1 ")
        self.shell += self.key(0.3)
        self.out("\x1b[2KHit any key to stop autoboot:  0 \r\n")
        self.linux()
        return True


def self_test():
    import pty
    import tempfile
    import termios
    check = P.Checks(self_test)
    name, size, crc = "t6_hello.bin", 0x2a5c0, 0x1b2c3d4e

    def case(how="ok", lines=None, back=False, probe="t6_hello"):
        """back: the runner's file (Armbian answers over ssh with a new
        boot_id) is there from the start."""
        master, slave = pty.openpty()
        a = termios.tcgetattr(slave)
        a[0] = a[1] = a[3] = 0
        a[6][termios.VMIN], a[6][termios.VTIME] = 0, 0
        termios.tcsetattr(slave, termios.TCSANOW, a)
        fake = FakeGo(master, name, size, crc, how, lines)
        with tempfile.TemporaryDirectory() as d:
            out = sys.stdout
            sys.stdout = open(os.devnull, "w")
            try:
                flag = os.path.join(d, "linux-back")
                if back:
                    open(flag, "w").close()
                fake.start()
                rc = run(slave, d, name, size, crc, probe, 1, boot_wait=3, login_wait=2, gap=0, scale=0.03,
                         back=flag if back else None)
            finally:
                sys.stdout.close()
                sys.stdout = out
            fake.done.wait(5)
            files = {n: open(os.path.join(d, n), "rb").read() for n in os.listdir(d)}
        os.close(master)
        os.close(slave)
        return rc, fake, files

    cmds = commands(name, size)
    go = cmds[-1]

    # 1. The good path: the six commands in order, go last, nothing written
    # after it, the image's record judged PASS, Linux back.
    rc, f, files = case()
    check(rc == 0, "ok: exit 0 (got %d)" % rc)
    check(f.entered == cmds, "ok: U-Boot was given exactly the route, go last (%r)" % f.entered)
    check(f.got.endswith(go.encode() + b"\r") and f.shell == b"",
          "ok: nothing was written after go's Enter (%r)" % f.shell)
    check(b"T6:G go: SCTLR_EL2" in files.get("image.log", b""), "ok: image.log holds the image's lines")
    check(files.get("verdict.txt", b"").rstrip().endswith(b"VERDICT t6_hello PASS"), "ok: verdict.txt says PASS")
    check(b"####" not in files.get("uart.log", b"####"), "ok: the login came, uart.log has no line about a hole")

    def hole(files):
        """True when uart.log ends with one '####' line whose byte offset is
        where it starts, after the kernel's cut line, and image.log has none."""
        log = files.get("uart.log", b"")
        at = log.find(b"\r\n#### prompt-go: no byte was read after ")
        m = re.search(rb"\(byte (\d+) of uart\.log\)", log[at:])
        return (at > 0 and log.endswith(b"not in this record\r\n") and log.count(b"####") == 1
                and m is not None and int(m.group(1)) == at and log[:at].endswith(b"[    0.0")
                and b"####" not in files.get("image.log", b""))

    # 2. Each answer that differs: refused before go, reset entered, Linux back.
    for how, at in (("missing", 0), ("size-text", 0), ("size", 1), ("load", 2), ("load-n", 2), ("magic", 3),
                    ("crc", 4), ("crc-range", 4)):
        rc, f, files = case(how)
        check(rc == 8 and f.entered == cmds[:at + 1] + ["reset"],
              "%s: exit 8, %s refused, reset entered, no go (rc %d, %r)" % (how, cmds[at], rc, f.entered))
        check(f.shell == b"" and b"REFUSED before go" in files.get("typed.log", b""),
              "%s: the refusal is in typed.log, nothing reached Linux" % how)

    # 2a. A refusal, reset, and the UART stream stops in the kernel's output:
    # exit 6 (prompt-go.sh asks over ssh), nothing reached Linux, the hole said.
    rc, f, files = case("missing-kstop")
    check(rc == 6 and f.entered == cmds[:1] + ["reset"] and f.shell == b"" and hole(files),
          "refused, kernel stop: exit 6, nothing written after reset, uart.log says where the hole is (rc %d)" % rc)

    # 2b. No autoboot line after the reboot: nothing written, its own code.
    rc, f, files = case("noautoboot")
    check(rc == 12 and f.got == b"", "no autoboot line: exit 12, nothing written (rc %d, %r)" % (rc, f.got))
    check(b"STOP (12): no autoboot line in 3 s after the reboot" in files.get("typed.log", b""),
          "no autoboot line: typed.log says the whole wait passed")

    # 2c. The same when ssh sees Linux (issue #11943): the wait for the line
    # ends once the UART is silent, and the stop says so.
    rc, f, files = case("noautoboot", back=True)
    check(rc == 12 and f.got == b"" and b"STOP (12): " + P.NO_AUTOBOOT_BACK.encode() in files.get("typed.log", b""),
          "no autoboot line, ssh sees Linux: exit 12, nothing written, the stop names both witnesses (rc %d)" % rc)

    # 3. The prompt lost after ext4load: nothing more is typed, no go, no reset.
    rc, f, _ = case("lost")
    check(rc == 5 and f.entered == cmds[:3] and f.got.endswith(cmds[2].encode() + b"\r"),
          "prompt lost: exit 5, nothing typed after ext4load (rc %d, %r)" % (rc, f.entered))

    # 3a. Exit 4: the one space came too late, Linux booted, nothing typed.
    rc, f, _ = case("missed")
    check(rc == 4 and f.entered == [] and f.shell == b"", "autoboot missed: exit 4, nothing typed (rc %d, %r)" % (rc, f.entered))

    # 3b. Exit 7: a command made U-Boot leave the prompt and start Linux.
    rc, f, files = case("uboot-boots")
    check(rc == 7 and f.entered == cmds[:3] and f.shell == b"" and b"STOP (7)" in files.get("typed.log", b""),
          "U-Boot left the prompt: exit 7, nothing written after it (rc %d, %r)" % (rc, f.entered))

    # 3c. Exit 2 is the usage code of main(): a bad NAME, SIZE, CRC or verb,
    # before any port is opened. (1, 3, 40 and 41 are prompt-go.sh's.)
    err, sys.stderr = sys.stderr, open(os.devnull, "w")
    try:
        bad = [main(["x", "plan", "../x.bin", "1", "00000000"]), main(["x", "plan", name, "0", "00000000"]),
               main(["x", "plan", name, "1", "XYZ"]), main(["x", "bogus"]),
               main(["x", "run", "/dev/null", "/nonexistent", name, "1", "00000000", "t6_nonesuch", "1"])]
    finally:
        sys.stderr.close()
        sys.stderr = err
    check(bad == [2] * 5, "usage: exit 2 for each bad argument (%r)" % bad)

    # 4. go answered by nothing: exit 9, nothing written after go.
    rc, f, _ = case("silent")
    check(rc == 9 and f.entered == cmds and f.shell == b"", "go silent: exit 9, nothing written after it (rc %d)" % rc)

    # 5. The verdict FAIL: a line of the record missing; Linux back.
    lines = [l for l in image_lines() if not l.startswith("t6_hello: 5 s")]
    rc, f, files = case(lines=lines)
    check(rc == 11 and re.search(rb"VERDICT t6_hello FAIL \(1 of [0-9]+\)", files.get("verdict.txt", b"")),
          "verdict FAIL: exit 11, verdict.txt names it (rc %d)" % rc)

    # 5a. WDT0 against PSCI: the image's own reset line is the only
    # difference. With it cut (what a WDT0 expiry leaves) verdict.txt names
    # the return apart from the PSCI case; the return line sits above VERDICT.
    rc0, _, f0 = case()
    v0 = f0.get("verdict.txt", b"").decode().splitlines()
    wl = [l for l in image_lines() if "PSCI SYSTEM_RESET" not in l]
    rc1, _, f1 = case(lines=wl)
    v1 = f1.get("verdict.txt", b"").decode().splitlines()
    check(rc0 == 0 and v0[-2].startswith("return: PSCI SYSTEM_RESET (t6: t6_hello done: PSCI SYSTEM_RESET at")
          and v0[-1].startswith("VERDICT"), "return: PSCI reset is named, VERDICT stays last (%r)" % v0[-2:])
    check(v1[-2].startswith("return: no PSCI SYSTEM_RESET line") and "WDT0" in v1[-2] and v1[-2] != v0[-2]
          and v1[-1].startswith("VERDICT") and rc1 == (0 if v1[-1].endswith("PASS") else 11),
          "return: WDT0-style return is named apart; its exit follows the verdict (rc %d, %r)" % (rc1, v1[-2:]))

    # 5b. The WDT0 feeder's first-feed line: noted either way, no effect on
    # the verdict or the exit code.
    fl = "yakogl-wdt: nanopc_t6 first feed at 1002 ms"
    rcf, _, ff = case(lines=image_lines() + [fl])
    vf = ff.get("verdict.txt", b"").decode().splitlines()
    check(rcf == 0 and vf[-3] == "feeder: first feed at 1002 ms" and vf[-2].startswith("return:")
          and vf[-1].startswith("VERDICT"), "feeder: first-feed line is noted, VERDICT last (rc %d, %r)" % (rcf, vf[-3:]))
    check(rc0 == 0 and v0[-3] == "feeder: no first-feed line" and v0[-1].startswith("VERDICT")
          and rc1 == (0 if v1[-1].endswith("PASS") else 11) and v1[-3] == "feeder: no first-feed line",
          "feeder: no first-feed line is noted, verdict and exit unchanged (%r)" % v0[-3:])

    # 6. The image ran, no login after it: exit 10, nothing written.
    rc, f, files = case("nologin")
    check(rc == 10 and f.shell == b"" and files.get("verdict.txt", b"").rstrip().endswith(b"PASS"),
          "no login: exit 10, the verdict still taken, nothing written (rc %d)" % rc)
    check(files.get("uart.log", b"").count(b"####") == 1 and files["uart.log"].endswith(b"not in this record\r\n"),
          "no login: uart.log ends with the line that says where the stream went silent")

    # 6b. The image ran and reset the board, U-Boot booted Linux, and the
    # UART stream stopped in the kernel's output (issue #11874): exit 10
    # with the verdict PASS taken, nothing written after go, the hole said in
    # uart.log and typed.log, and no "needs a hand" (that is prompt-go.sh's to
    # say, after ssh finds no new boot either).
    rc, f, files = case("ok-kstop")
    typed = files.get("typed.log", b"")
    check(rc == 10 and f.entered == cmds and f.got.endswith(go.encode() + b"\r") and f.shell == b"",
          "kernel stop: exit 10, nothing written after go (rc %d, %r)" % (rc, f.shell))
    check(files.get("verdict.txt", b"").rstrip().endswith(b"VERDICT t6_hello PASS") and hole(files),
          "kernel stop: the verdict PASS taken, uart.log says where the hole is, image.log has only bytes read")
    check(b"STOP (10): the image started, no Linux login on the UART" in typed and b"needs a hand" not in typed
          and b"no Linux login on the UART: no byte was read after" in typed,
          "kernel stop: typed.log says no login on the UART and where it went silent, not 'needs a hand'")
    check(b"because the runner saw Armbian" not in files.get("uart.log", b""),
          "kernel stop: no word of ssh without the runner's file")

    # 6c. The same with the runner's file (issue #11943): Armbian answers over
    # ssh, so the wait ends once the UART has been silent for a while, not
    # after the image's and the login's whole time. Still exit 10, the
    # verdict taken, one '####' line at the hole's byte; the line says why
    # the wait ended and typed.log gives the time really waited.
    rc, f, files = case("ok-kstop", back=True)
    typed = files.get("typed.log", b"")
    check(rc == 10 and f.entered == cmds and f.shell == b""
          and files.get("verdict.txt", b"").rstrip().endswith(b"VERDICT t6_hello PASS") and hole(files),
          "kernel stop, ssh sees Linux: exit 10, the verdict PASS taken, the hole said (rc %d)" % rc)
    check(b"because the runner saw Armbian over ssh with a new boot_id while the UART was silent"
          in files.get("uart.log", b""),
          "kernel stop, ssh sees Linux: uart.log's last line says why the wait ended")
    w = re.search(rb"STOP \(10\): the image started, no Linux login on the UART in (\d+) s", typed)
    check(w is not None and int(w.group(1)) < 3,
          "kernel stop, ssh sees Linux: typed.log gives the time waited, under the whole 3 s (%r)"
          % (w.group(0) if w else None))

    # 6d. A refusal, reset, the stream stops, ssh sees Linux: exit 6 the same way.
    rc, f, files = case("missing-kstop", back=True)
    check(rc == 6 and f.entered == cmds[:1] + ["reset"] and f.shell == b"" and hole(files)
          and b"because the runner saw Armbian" in files.get("uart.log", b""),
          "refused, kernel stop, ssh sees Linux: exit 6, the hole said with its reason (rc %d)" % rc)

    # 6e. The runner's file alone ends nothing: with it there from the start
    # and the UART alive, the image's record is whole and the login is read.
    rc, f, files = case(back=True)
    check(rc == 0 and files.get("verdict.txt", b"").rstrip().endswith(b"VERDICT t6_hello PASS")
          and b"####" not in files.get("uart.log", b"####"),
          "ssh sees Linux, the UART alive: exit 0, verdict PASS, no line about a hole (rc %d)" % rc)

    # 6f. The samples judged by their summary lines (judge_sample), through
    # the whole route: gl_test PASS; one case failed; a dEQP row PASS (with a
    # failed case: the cases are tallied later); a row without "deqp: done:";
    # a gl_test image whose T6:G says the caches stayed on.
    for probe, how, want, why in (
            ("gl_test", "ok", 0, None),
            ("gl_test", "fail1", 11, rb"differs  expected one \"gl_test: all: "),
            ("deqp", "ok", 0, None),
            ("deqp", "nodone", 11, rb"missing  expected \"deqp: done: "),
            ("gl_test", "caches", 11, rb"differs  expected T6:G")):
        rc, f, files = case(lines=sample_lines(probe, how), probe=probe)
        v = files.get("verdict.txt", b"")
        last = v.rstrip().splitlines()[-1] if v.strip() else b""
        good = last == b"VERDICT %s PASS" % probe.encode() if want == 0 else (
            re.fullmatch(rb"VERDICT %s FAIL \(1 of [0-9]+\)" % probe.encode(), last) is not None
            and re.search(why, v) is not None)
        check(rc == want and good and f.entered == cmds and f.shell == b"",
              "%s %s: exit %d, verdict.txt's last line %r, nothing written after go (rc %d)" % (
                  probe, how, want, last, rc))

    # 6g. judge_sample alone on the records the route cannot fake: a gl_test
    # case timed out, no reset line, a second "gl_test: all:" line, a kexec
    # record (no T6:G line), the reset line of another sample, 0 cases.
    with tempfile.TemporaryDirectory() as d:
        log, out = os.path.join(d, "image.log"), os.path.join(d, "verdict.txt")

        def judged(lines):
            with open(log, "w") as f:
                f.write("\r\n".join(lines) + "\r\n")
            return judge("deqp" if any(l.startswith("deqp: ") for l in lines) else "gl_test", log, out)

        gl = sample_lines("gl_test")
        at = [i for i, l in enumerate(gl) if l.startswith("gl_test: all: ")][0]
        for what, lines in (
                ("a case timed out", gl[:at] + [gl[at].replace("(50 passed; 0 timed out", "(49 passed; 1 timed out")]
                 + gl[at + 1:]),
                ("no reset line", gl[:-1]),
                ("two gl_test: all: lines", gl[:at + 1] + gl[at:]),
                ("no T6:G line", [l for l in gl if not l.startswith("T6:G ")]),
                ("another sample's reset line", gl[:-1] + [gl[-1].replace("gl_test done", "deqp done")]),
                ("0 cases", gl[:at] + ["gl_test: all: 0 of 0 cases failed (0 passed; 0 timed out; first failure "
                                       "case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); "
                                       "precompiled hits 0, compiler runs 0"] + gl[at + 1:]),
                ("49 cases", gl[:at] + [gl[at].replace("0 of 50 cases failed (50 passed", "0 of 49 cases failed (49 passed")]
                 + gl[at + 1:]),
                ("a case skipped", gl[:at] + [gl[at].replace("(50 passed", "(49 passed").replace(
                    "0 skipped, first skip case -1", "1 skipped, first skip case 16")] + gl[at + 1:]),
                ("the summary truncated", gl[:at] + [gl[at][:60]] + gl[at + 1:]),
                ("a dEQP row without its summary", [l for l in sample_lines("deqp")
                                                    if not l.startswith("deqp: summary: ")])):
            rc, last = judged(lines)
            check(rc == 1 and last.startswith("VERDICT ") and " FAIL (1 of " in last,
                  "judge_sample, %s: FAIL (rc %d, %r)" % (what, rc, last))
        rc, last = judged(gl)
        check(rc == 0 and last == "VERDICT gl_test PASS", "judge_sample, the 50/50 record: PASS (%r)" % last)

        # WDT0 return: no PSCI line, the next U-Boot banner after the summary
        def route_of():
            with open(out) as f:
                return [l for l in f.read().splitlines() if l.startswith("route: ")]

        check(route_of() == ["route: psci"], "judge_sample, PASS + PSCI: route psci (%r)" % route_of())
        for banner in ("DDR V1.18 20220801", "U-Boot 2026.07 (Nov 12 2026)"):
            for probe in ("gl_test", "deqp"):
                rc, last = judged(sample_lines(probe)[:-1] + ["", banner])
                check(rc == 0 and last == "VERDICT %s PASS" % probe and route_of() == ["route: wdt0"],
                      "judge_sample, %s PASS + WDT0 return (%s): PASS, route wdt0 (%r)" % (probe, banner[:8], last))
        rc, last = judged(gl[:at] + gl[at + 1:-1] + ["DDR V1.18 20220801"])
        check(rc == 1 and " FAIL (" in last, "judge_sample, no summary + banner: FAIL (rc %d, %r)" % (rc, last))
        rc, last = judged(sample_lines("gl_test", "fail1")[:-1] + ["DDR V1.18 20220801"])
        check(rc == 1 and " FAIL (" in last, "judge_sample, FAIL summary + banner: FAIL (rc %d, %r)" % (rc, last))
        dq = [l for l in sample_lines("deqp") if not l.startswith("deqp: done: ")][:-1] + ["U-Boot 2026.07"]
        rc, last = judged(dq)
        check(rc == 1 and "FAIL" in last, "judge_sample, deqp no summary + banner: FAIL (rc %d, %r)" % (rc, last))
        rc, last = judged(gl[:-1] + ["U-Boot SPL 2026.07"])
        check(rc == 1, "judge_sample, an SPL line is no banner: FAIL (rc %d)" % rc)

    # 6h. A "t6: CNTFRQ_EL0 0" stop: named as its cause, a failure (the
    # image failure's rc: 11 with Linux back), feeder and return lines kept
    # just above VERDICT.
    fx = os.path.join(HERE, "testdata", "prompt-go", "cntfrq0-stop.txt")
    with open(fx) as f:
        stop = [l.rstrip("\n") for l in f if not l.startswith("#")]
    for probe in ("gl_test", "t6_hello"):
        rc, f, files = case(lines=stop, probe=probe)
        v = files.get("verdict.txt", b"").decode().splitlines()
        check(rc == 11, "cntfrq0 stop (%s): exit 11, the image failure (got %d)" % (probe, rc))
        check("cause: CNTFRQ_EL0 0 at entry (U-Boot left the counter frequency unset)" in v
              and v[-1].startswith("VERDICT %s FAIL" % probe) and v[-2].startswith("return:")
              and v[-3].startswith("feeder:") and v[-4].startswith("cause: CNTFRQ_EL0 0"),
              "cntfrq0 stop (%s): cause named, feeder and return above VERDICT (%r)" % (probe, v[-5:]))
    # a pass record without the line has no cause line
    rc, f, files = case()
    check(b"cause:" not in files.get("verdict.txt", b"cause:"), "cntfrq0: a good run has no cause line")

    # 7. What may be typed.
    ok = go_allowed(name, size)
    for c in cmds + ["reset"]:
        check(ok(c), "allowed: %s" % c)
    for c in ["go 0x10000004", "go 0x00200000", "booti 0x10000000", "bootm 0x10000000", "mw.l 0x10000000 0",
              "ext4load mmc 0:1 0x10000000 /boot/yakogl/other.bin", "ext4load mmc 1:1 0x10000000 /boot/yakogl/" + name,
              "ext4load mmc 0:1 0x02000000 /boot/yakogl/" + name, "crc32 0x10000000 %x" % (size + 1),
              "ext4write mmc 0:1 0x10000000 /boot/yakogl/" + name, "saveenv", "setenv bootcmd go 0x10000000",
              "go 0x10000000; reset", "md.l 0x10000038 2", "printenv", "dcache off", ""]:
        check(not ok(c), "not allowed: %r" % c)
    for n in ["t6_hello.bin", "t6_submit-2.bin"]:
        check(NAME_RE.fullmatch(n) is not None, "name: %s" % n)
    for n in ["../Image", "a/b.bin", "t6.img", ".bin", "a b.bin", "x.bin;reset", "-x.bin"]:
        check(NAME_RE.fullmatch(n) is None, "not a name: %r" % n)

    # 8. Every probe taken here is one t6-verdict.sh has a table for: an
    # empty record FAILs (exit 1); a name it does not know is usage (2).
    for t in VERDICT_TYPES + ("t6_nonesuch",):
        r = subprocess.run(["bash", VERDICT, t, os.devnull], env=dict(os.environ, T6_ROUTE="uboot", LC_ALL="C"),
                           capture_output=True)
        want = 2 if t == "t6_nonesuch" else 1
        check(r.returncode == want, "probe %s: t6-verdict.sh exits %d on an empty record (rc %d)" % (t, want, r.returncode))

    # 9. The CRC is zlib's.
    check("%08x" % zlib.crc32(b"123456789") == "cbf43926", "crc: zlib's CRC-32 of '123456789' is cbf43926")

    return check.summary("prompt-go")


def args_ok(name, size, crc):
    if NAME_RE.fullmatch(name) is None:
        return "NAME %r is not a file name ending in .bin" % name
    if not re.fullmatch(r"[1-9][0-9]*", size) or int(size) > 256 << 20:
        return "SIZE %r is not a decimal byte count up to 256 MiB" % size
    if not re.fullmatch(r"[0-9a-f]{8}", crc):
        return "CRC %r is not eight lower-case hex digits" % crc
    return None


def main(argv):
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) == 5 and argv[1] == "plan":
        why = args_ok(*argv[2:5])
        if why:
            sys.stderr.write("prompt-go.py: %s\n" % why)
            return 2
        print(plan(argv[2], int(argv[3]), int(argv[4], 16)))
        return 0
    if len(argv) in (9, 10) and argv[1] == "run":
        tty, rec, name, size, crc, probe, wait = argv[2:9]
        back = argv[9] if len(argv) == 10 else None
        why = args_ok(name, size, crc)
        if why or probe not in TYPES or not wait.isdigit():
            sys.stderr.write("prompt-go.py: %s\n" % (why or "PROBE or IMAGE_WAIT is wrong"))
            return 2
        fd = P.open_tty(tty)
        try:
            return run(fd, rec, name, int(size), int(crc, 16), probe, int(wait), back=back)
        finally:
            os.close(fd)
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
