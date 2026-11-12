#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Start a Zephyr image on the NanoPC-T6 from its U-Boot prompt.

    prompt-go.py run TTY RECORD_DIR NAME SIZE CRC PROBE IMAGE_WAIT
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
        verdict.txt. An answer that differs stops before `go`: `reset` is
        entered and the board boots Armbian.
        SIZE is decimal bytes, CRC eight hex digits (zlib's CRC-32, the one
        U-Boot's crc32 prints). RECORD_DIR also gets uart.log, typed.log and
        answers.txt, as prompt-reads.py's.
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

Exit: 0 the image ran, its verdict is PASS and Linux's login came back. 2
usage. 12 no autoboot line in time (nothing written; not 3, which
prompt-go.sh keeps for "refused before the reboot"). 4 the autoboot was
missed (one space written, Linux booted). 5 stopped at the prompt (an echo
that never matched, a command without an answer, the prompt lost): the board
stays at the U-Boot prompt until its power is cut. 6 an answer differed,
`reset` entered, no Linux login in time. 7 U-Boot left the prompt during a
command and started Linux. 8 an answer differed (size, load, magic, CRC):
`go` was not entered, `reset` was, and Linux's login came back. 9 `go`
entered and U-Boot did not answer it ("## Starting application" never
came): nothing more was written. 10 the image started, no Linux login in
time: the board needs a hand at its power switch. 11 Linux's login came back
and the verdict is FAIL (verdict.txt names the lines).
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
TYPES = ("t6_hello", "t6_gpu", "t6_mmu", "t6_fw", "t6_submit")


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


def judge(probe, log, out):
    """t6-verdict.sh PROBE LOG with T6_ROUTE=uboot into out: (rc, last line)."""
    env = dict(os.environ, T6_ROUTE="uboot", LC_ALL="C")
    r = subprocess.run(["bash", VERDICT, probe, log], env=env, capture_output=True, text=True)
    with open(out, "w") as f:
        f.write(r.stdout + r.stderr)
    lines = (r.stdout + r.stderr).strip().splitlines()
    return r.returncode, lines[-1] if lines else ""


def run(fd, rec, name, size, crc, probe, image_wait, boot_wait=240, login_wait=180, gap=0.003, scale=1.0):
    os.makedirs(rec, exist_ok=True)
    ok = go_allowed(name, size)
    with open(os.path.join(rec, "uart.log"), "wb") as raw, \
            open(os.path.join(rec, "typed.log"), "w") as typed, \
            open(os.path.join(rec, "answers.txt"), "w") as ans:
        con = P.Console(fd, raw, typed, gap)

        def say(msg):
            con.log(msg)
            print("prompt-go: " + msg, flush=True)

        def answered(cmd, out):
            ans.write("## %s\n%s" % (cmd, out if out.endswith("\n") or not out else out + "\n"))
            ans.flush()

        try:
            say("listening, nothing written (waiting up to %d s for the autoboot line)" % boot_wait)
            if not con.wait_for(P.AUTOBOOT, boot_wait):
                raise P.Stop(12, "no autoboot line in %d s after the reboot: nothing was written" % boot_wait)
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
                if not con.wait_for(P.LOGIN, login_wait):
                    raise P.Stop(6, "reset entered after the refusal, no Linux login in %d s" % login_wait)
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
            back = con.wait_for(P.LOGIN, image_wait + login_wait)
            log = os.path.join(rec, "image.log")
            with open(log, "wb") as f:
                f.write(con.buf)
            if not started:
                say("STOP (9): U-Boot did not answer %s in %d s; nothing more was written" % (go, 10 * scale))
                return 9
            vrc, vline = judge(probe, log, os.path.join(rec, "verdict.txt"))
            say("verdict: %s" % vline)
            if not back:
                say("STOP (10): the image started, no Linux login in %d s: the board needs a hand"
                    % (image_wait + login_wait))
                return 10
            if vrc != 0:
                say("STOP (11): Linux's login came back, the verdict is not PASS (verdict.txt)")
                return 11
            say("done: the verdict is PASS and Linux's login came back")
            return 0
        except P.Stop as s:
            con.armed = False
            if s.code in (4, 7):
                con.wait_for(P.LOGIN, login_wait)
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


class FakeGo(P.FakeBoard):
    """U-Boot that answers the go route's commands, then plays the image."""

    def __init__(self, master, name, size, crc, how="ok", lines=None):
        words = dict(P.PMU_OFF)
        words[LOAD + MAGIC_OFF] = 0x12345678 if how == "magic" else IMAGE_MAGIC
        mode = "noautoboot" if how == "noautoboot" else "ok"
        super().__init__(master, mode=mode, words=words)
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

    def case(how="ok", lines=None):
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
                fake.start()
                rc = run(slave, d, name, size, crc, "t6_hello", 1, boot_wait=3, login_wait=2, gap=0, scale=0.03)
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

    # 2. Each answer that differs: refused before go, reset entered, Linux back.
    for how, at in (("missing", 0), ("size-text", 0), ("size", 1), ("load", 2), ("load-n", 2), ("magic", 3),
                    ("crc", 4), ("crc-range", 4)):
        rc, f, files = case(how)
        check(rc == 8 and f.entered == cmds[:at + 1] + ["reset"],
              "%s: exit 8, %s refused, reset entered, no go (rc %d, %r)" % (how, cmds[at], rc, f.entered))
        check(f.shell == b"" and b"REFUSED before go" in files.get("typed.log", b""),
              "%s: the refusal is in typed.log, nothing reached Linux" % how)

    # 2b. No autoboot line after the reboot: nothing written, its own code.
    rc, f, _ = case("noautoboot")
    check(rc == 12 and f.got == b"", "no autoboot line: exit 12, nothing written (rc %d, %r)" % (rc, f.got))

    # 3. The prompt lost after ext4load: nothing more is typed, no go, no reset.
    rc, f, _ = case("lost")
    check(rc == 5 and f.entered == cmds[:3] and f.got.endswith(cmds[2].encode() + b"\r"),
          "prompt lost: exit 5, nothing typed after ext4load (rc %d, %r)" % (rc, f.entered))

    # 4. go answered by nothing: exit 9, nothing written after go.
    rc, f, _ = case("silent")
    check(rc == 9 and f.entered == cmds and f.shell == b"", "go silent: exit 9, nothing written after it (rc %d)" % rc)

    # 5. The verdict FAIL: a line of the record missing; Linux back.
    lines = [l for l in image_lines() if not l.startswith("t6_hello: 5 s")]
    rc, f, files = case(lines=lines)
    check(rc == 11 and b"VERDICT t6_hello FAIL (1 of 25)" in files.get("verdict.txt", b""),
          "verdict FAIL: exit 11, verdict.txt names it (rc %d)" % rc)

    # 6. The image ran, no login after it: exit 10, nothing written.
    rc, f, files = case("nologin")
    check(rc == 10 and f.shell == b"" and files.get("verdict.txt", b"").rstrip().endswith(b"PASS"),
          "no login: exit 10, the verdict still taken, nothing written (rc %d)" % rc)

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

    # 8. The CRC is zlib's.
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
    if len(argv) == 9 and argv[1] == "run":
        tty, rec, name, size, crc, probe, wait = argv[2:]
        why = args_ok(name, size, crc)
        if why or probe not in TYPES or not wait.isdigit():
            sys.stderr.write("prompt-go.py: %s\n" % (why or "PROBE or IMAGE_WAIT is wrong"))
            return 2
        fd = P.open_tty(tty)
        try:
            return run(fd, rec, name, int(size), int(crc, 16), probe, int(wait))
        finally:
            os.close(fd)
    sys.stderr.write(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
