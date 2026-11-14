#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Host simulator of the STM32MP257F-DK standalone product script.

    tools/mp2s_script_sim.py run SCRIPT [options]
    tools/mp2s_script_sim.py faults SCRIPT --left N [options]

SCRIPT is the text of driver/tools/mp2-standalone/mp2-standalone.cmd (the
source, not the uImage). This runs the real script's text, boot after boot,
in a small interpreter for exactly the part of U-Boot v2023.10's hush that
the script uses (docs/notes/stm32mp257f-dk-standalone.md, section 7's lint):

  - if/then/elif/else/fi, ';', newlines, '&&', '||', '#' comments at the
    start of a word, double quotes, ${name} expansion (an unset name is
    empty; an empty unquoted expansion is no argument);
  - echo; setenv (set, delete); test with -z -n = != -eq -lt -gt -a -o,
    evaluated left to right as ST/cmd/test.c does, integers read with
    simple_strtol base 0; itest[.l] with -lt -gt -eq !=, operands read as
    hex (hextoul) or as 32-bit memory with '*addr'; setexpr[.l] NAME VALUE
    and NAME A +|- B, operands hex, the result written with "%lx"; size;
    load; saveenv; dcache/icache flush|off|on; go; exit (ends the script).

Anything else (another command, '$x' without braces, single quotes, a
backslash, a pipe, a usage error) stops the run with "sim: error: ..." and
exit 2: the simulator fails loudly rather than guess.

The board model:

  - the environment is stored in two redundant copies, each a flag byte and a
    CRC-valid or CRC-bad dictionary. A boot loads the valid copy with the
    higher flag (255 -> 0 wraps), or the compiled-in defaults when both are
    bad, which env_check then saves (ST/env/mmc.c, ST/env/common.c). A
    saveenv writes the whole RAM environment to the copy not in use with the
    flag one higher;
  - the start: copy A holds the stock environment (built in, or --env FILE
    in fw_printenv's name=value form) with flag 5, copy B the same plus the
    runner's "default" edit (section 7: mp2s_ver=1, mp2s_left,
    mp2s_prefixes=<boot_prefixes>, boot_prefixes="/yakogl/standalone/
    <boot_prefixes>") and --set/--unset, with flag 6 (--start-flags A,B);
  - the distro loop: boot_prefixes is split once at the start of the loop
    (hush builds the for list once); for "/yakogl/standalone/" the loop
    sets its working variables (devtype=mmc, devnum=0, distro_bootpart=8,
    bootfs being the card's eighth partition in section 1's list, prefix,
    script, filesize of the script, ...) and sources our script if it is on
    bootfs, then prints "SCRIPT FAILED: continuing..." when the script
    returns; any other prefix is ST's and boots Linux (OpenSTLinux). No
    prefix left: U-Boot stops at its prompt ("stuck");
  - bootfs holds /yakogl/standalone/boot.scr.uimg (--no-script removes it)
    and /yakogl/standalone/default.bin: by default a synthetic arm64 Image
    of 0x1000 bytes whose header gives image_size 0x20000; --image FILE takes
    a real file, --no-image removes it; --load-fails makes load fail;
  - go runs the image: it resets the SoC (the boot ends; the next boot
    starts) or, with --image-returns, returns 0 to the script.

run: boots until the first boot that reaches Linux (with --until-restored,
until the stored environment has no mp2s_* and no "/yakogl/standalone/"
prefix), or --max-boots (20). It prints every "mp2s: " console line and the
loop's "SCRIPT FAILED: continuing..." as the console would, and "sim: "
lines for the model (boots, saves, the image, Linux), then a result:

    sim: result: Linux at boot K, boots B, image runs R, saves S
    sim: result: no Linux within M boots, image runs R, saves S

and the stored environment at the end. It always checks that no variable
outside mp2s_*, boot_prefixes and the distro loop's working variables
(section 3: devtype, devnum, distro_bootpart, distro_bootpart_uuid,
bootfstype, devplist, prefix, script, target, filesize, fileaddr) changed,
and that boot_prefixes is either the start value or the recorded
mp2s_prefixes. --check-restored also requires that Linux was reached and
the env is restored (no mp2s_*, boot_prefixes the recorded value).

faults: runs the clean sequence once and counts the script's saveenv calls,
then once for each of them and each cut (before the write: nothing written;
torn: the target copy CRC-bad; after the write: written) cuts the power at
that saveenv: the boot ends there and the next boot starts. Each run must
reach Linux within N+1 boots, not counting the boot the power cut ended; run
the image at most N times; and end with the env restored and nothing else
changed. One line per case; exit 1 if any fails.

Exit 0: ran and every check held; 1: a check failed; 2: sim error or usage.
Python 3, standard library only. tools/mp2s_model.py is a separate abstract
model of the count; this file runs the script's own text.
"""
import argparse
import re
import struct
import sys

OUR_PREFIX = "/yakogl/standalone/"
SCRIPT_FILE = OUR_PREFIX + "boot.scr.uimg"
IMAGE_FILE = OUR_PREFIX + "default.bin"
DEVTYPE, DEVNUM, BOOTPART = "mmc", "0", "8"
LOOP_VARS = ("devtype", "devnum", "distro_bootpart", "distro_bootpart_uuid",
             "bootfstype", "devplist", "prefix", "script", "target",
             "filesize", "fileaddr")
KEYWORDS = ("if", "then", "elif", "else", "fi")
MASK64 = (1 << 64) - 1

# The stock environment when no --env is given: a model with the names the
# check watches, not values read from a board (TODO(board-env-read) in the
# design note; the st-env-fixture item's file can be passed with --env).
STOCK_ENV = {
    "bootcmd": "run bootcmd_stm32mp",
    "altbootcmd": "run bootcmd_stm32mp",
    "bootdelay": "1",
    "boot_prefixes": "/mmc0_",
    "boot_scripts": "boot.scr.uimg boot.scr",
    "boot_syslinux_conf": "extlinux/stm32mp257f-dk_extlinux.conf",
    "boot_targets": "mmc0",
}
# The compiled-in defaults that both copies bad fall back to: ST's
# boot_prefixes "/ /boot/" (config_distro_bootcmd.h), no mp2s_*.
DEFAULT_PREFIXES = "/ /boot/"


class SimError(Exception):
    """Something outside the modelled subset: fail loudly."""


class ScriptExit(Exception):
    """exit: the sourced script ends."""


class BootEnd(Exception):
    """The boot ends here (image reset or power cut)."""

    def __init__(self, why):
        super().__init__(why)
        self.why = why


# ---------------------------------------------------------------- numbers

def simple_strtoul(s, base):
    """U-Boot's simple_strtoul (lib/strto.c): no whitespace skipped, stops
    at the first character that is not a digit of the base."""
    if base == 0:
        if s[:1] == "0":
            base = 16 if (s[1:2].lower() == "x" and s[2:3] and
                          s[2] in "0123456789abcdefABCDEF") else 8
        else:
            base = 10
    if base == 16 and s[:2].lower() == "0x":
        s = s[2:]
    v = 0
    for c in s:
        d = int(c, 36) if c.isalnum() and c.isascii() else 99
        if d >= base:
            break
        v = (v * base + d) & MASK64
    return v


def simple_strtol(s, base):
    if s[:1] == "-":
        return -to_signed(simple_strtoul(s[1:], base))
    return to_signed(simple_strtoul(s, base))


def to_signed(v):
    v &= MASK64
    return v - (1 << 64) if v >> 63 else v


def hextoul(s):
    return simple_strtoul(s, 16)


# ---------------------------------------------------------------- parser

NAME_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")


def read_var(text, i, line):
    if text[i + 1:i + 2] != "{":
        raise SimError("line %d: '$' without '{name}' is not in the subset" % line)
    end = text.find("}", i + 2)
    name = text[i + 2:end] if end >= 0 else ""
    if end < 0 or not NAME_RE.match(name):
        raise SimError("line %d: bad ${...} expansion" % line)
    return end + 1, name


def tokenize(text):
    """Tokens: ("word", parts, line) with parts (kind, text, quoted), kind
    "lit" or "var"; ("op", ";"|"\\n"|"&&"|"||", line); ("eof", None, line)."""
    toks = []
    i, line, n = 0, 1, len(text)
    while i < n:
        c = text[i]
        if c in " \t\r":
            i += 1
        elif c == "\n":
            toks.append(("op", "\n", line))
            line += 1
            i += 1
        elif c == "#":
            while i < n and text[i] != "\n":
                i += 1
        elif c == ";":
            toks.append(("op", ";", line))
            i += 1
        elif c in "&|":
            if text[i:i + 2] not in ("&&", "||"):
                raise SimError("line %d: a single '%s' is not in the subset" % (line, c))
            toks.append(("op", text[i:i + 2], line))
            i += 2
        else:
            parts = []
            lit = ""
            while i < n and text[i] not in " \t\r\n;&|":
                c = text[i]
                if c == '"':
                    if lit:
                        parts.append(("lit", lit, False))
                        lit = ""
                    i += 1
                    buf = ""
                    while True:
                        if i >= n or text[i] == "\n":
                            raise SimError("line %d: unterminated or multi-line quote" % line)
                        c = text[i]
                        if c == '"':
                            i += 1
                            break
                        if c in "\\`":
                            raise SimError("line %d: '%s' in quotes is not in the subset" % (line, c))
                        if c == "$":
                            if buf:
                                parts.append(("lit", buf, True))
                                buf = ""
                            i, name = read_var(text, i, line)
                            parts.append(("var", name, True))
                            continue
                        buf += c
                        i += 1
                    # Even an empty "" makes an (empty) argument.
                    parts.append(("lit", buf, True))
                elif c == "$":
                    if lit:
                        parts.append(("lit", lit, False))
                        lit = ""
                    i, name = read_var(text, i, line)
                    parts.append(("var", name, False))
                elif c in "'\\`(){}<>":
                    raise SimError("line %d: '%s' is not in the subset" % (line, c))
                else:
                    lit += c
                    i += 1
            if lit:
                parts.append(("lit", lit, False))
            toks.append(("word", parts, line))
    toks.append(("eof", None, line))
    return toks


class Parser:
    def __init__(self, toks):
        self.t = toks
        self.i = 0

    def peek(self):
        return self.t[self.i]

    @staticmethod
    def keyword(tok):
        if tok[0] == "word" and len(tok[1]) == 1:
            kind, text, quoted = tok[1][0]
            if kind == "lit" and not quoted and text in KEYWORDS:
                return text
        return None

    def parse_list(self, stop):
        items = []
        while True:
            tok = self.peek()
            if tok[0] == "op" and tok[1] in (";", "\n"):
                self.i += 1
                continue
            if tok[0] == "eof":
                if stop:
                    raise SimError("line %d: end of script, expected %s" % (tok[2], "/".join(sorted(stop))))
                return items
            if self.keyword(tok) in stop:
                if not items:
                    raise SimError("line %d: empty list before '%s'" % (tok[2], self.keyword(tok)))
                return items
            items.append(self.parse_and_or())

    def parse_and_or(self):
        seq = [("", self.parse_command())]
        while True:
            tok = self.peek()
            if tok[0] == "op" and tok[1] in ("&&", "||"):
                self.i += 1
                while self.peek()[0] == "op" and self.peek()[1] == "\n":
                    self.i += 1
                seq.append((tok[1], self.parse_command()))
            elif tok[0] == "word":
                raise SimError("line %d: a word after 'fi' without a separator" % tok[2])
            else:
                return seq

    def parse_command(self):
        tok = self.peek()
        k = self.keyword(tok)
        if k == "if":
            return self.parse_if()
        if k:
            raise SimError("line %d: '%s' out of place" % (tok[2], k))
        if tok[0] != "word":
            raise SimError("line %d: expected a command, got %r" % (tok[2], tok[1]))
        words = []
        while self.peek()[0] == "word":
            words.append(self.peek()[1])
            self.i += 1
        return ("simple", words, tok[2])

    def parse_if(self):
        self.i += 1
        branches = []
        else_body = None
        while True:
            cond = self.parse_list({"then"})
            self.i += 1
            body = self.parse_list({"elif", "else", "fi"})
            branches.append((cond, body))
            k = self.keyword(self.peek())
            self.i += 1
            if k == "elif":
                continue
            if k == "else":
                else_body = self.parse_list({"fi"})
                self.i += 1
            return ("if", branches, else_body)


def parse(text):
    p = Parser(tokenize(text))
    return p.parse_list(set())


# ---------------------------------------------------------------- simulator

def default_image():
    img = bytearray(0x1000)
    struct.pack_into("<Q", img, 0x10, 0x20000)  # image_size
    struct.pack_into("<I", img, 0x38, 0x644D5241)  # "ARM\x64"
    return bytes(img)


def flag_newer(a, b):
    """True if copy flag a wins over b (ST/env/common.c env_import_redund)."""
    if a == 0 and b == 255:
        return True
    if a == 255 and b == 0:
        return False
    return a >= b


class Sim:
    def __init__(self, ast, script_len, stock, start, opts, cut=None, quiet=False):
        self.ast = ast
        self.script_len = script_len
        self.opts = opts
        self.cut = cut  # (n, "before"|"torn"|"after") or None
        self.quiet = quiet
        self.lines = []
        self.stock = dict(stock)
        self.start = dict(start)
        fa, fb = opts.start_flags
        # copies[i] = [flag, crc_ok, env]
        self.copies = [[fa, True, dict(stock)], [fb, True, dict(start)]]
        self.files = {}
        if not opts.no_script:
            self.files[SCRIPT_FILE] = b"<script>"
        if not opts.no_image:
            self.files[IMAGE_FILE] = opts.image_bytes
        self.saves = 0
        self.image_runs = 0
        self.cut_boot = None
        self.linux_boots = []
        self.boots = 0
        self.stuck = False

    def out(self, line):
        self.lines.append(line)
        if not self.quiet:
            print(line)

    # --- the stored environment
    def active(self):
        a, b = self.copies
        if a[1] and b[1]:
            return 0 if flag_newer(a[0], b[0]) else 1
        if a[1]:
            return 0
        if b[1]:
            return 1
        return None

    def stored_env(self):
        i = self.active()
        if i is None:
            env = dict(self.stock)
            env["boot_prefixes"] = DEFAULT_PREFIXES
            return env
        return dict(self.copies[i][2])

    def write_env(self, env):
        i = self.active()
        if i is None:
            target, flag = 0, 1
        else:
            target, flag = 1 - i, (self.copies[i][0] + 1) & 0xFF
        self.copies[target] = [flag, True, dict(env)]
        return target, flag

    # --- one boot
    def boot(self, k):
        self.k = k
        i = self.active()
        if i is None:
            env = self.stored_env()
            self.out("sim: boot %d: both env copies bad: the defaults; env_check saves them" % k)
            self.write_env(env)
        else:
            env = dict(self.copies[i][2])
            self.out("sim: boot %d: env copy %s (flag %d), boot_prefixes \"%s\""
                     % (k, "AB"[i], self.copies[i][0], env.get("boot_prefixes", "")))
        self.env = env
        self.mem = []
        for prefix in env.get("boot_prefixes", "").split():
            if prefix != OUR_PREFIX:
                self.out("sim: boot %d: prefix %s: ST's extlinux, booting Linux (OpenSTLinux)" % (k, prefix))
                return "linux"
            if SCRIPT_FILE not in self.files:
                self.out("sim: boot %d: prefix %s: no extlinux, no boot.scr.uimg, no boot.scr" % (k, prefix))
                continue
            self.out("sim: boot %d: prefix %s: source boot.scr.uimg" % (k, prefix))
            env.update(devtype=DEVTYPE, devnum=DEVNUM, distro_bootpart=BOOTPART,
                       bootfstype="ext4", devplist="8", target="mmc0", prefix=prefix,
                       script="boot.scr.uimg", filesize="%x" % self.script_len)
            if "scriptaddr" in env:
                env["fileaddr"] = env["scriptaddr"]
            try:
                self.run_list(self.ast)
            except ScriptExit:
                pass
            except BootEnd as e:
                return e.why
            self.out("SCRIPT FAILED: continuing...")
        self.out("sim: boot %d: the distro loop found nothing to boot: U-Boot stops at its prompt" % k)
        return "stuck"

    def run(self):
        for k in range(1, self.opts.max_boots + 1):
            self.boots = k
            why = self.boot(k)
            if why == "linux":
                self.linux_boots.append(k)
                if not self.opts.until_restored or self.restored(self.stored_env()):
                    break
            elif why == "stuck":
                self.stuck = True
                break

    def restored(self, env):
        return (not any(n.startswith("mp2s_") for n in env) and
                env.get("boot_prefixes") == self.start.get("mp2s_prefixes"))

    # --- the interpreter
    def run_list(self, items):
        st = 0
        for seq in items:
            st = self.run_command(seq[0][1])
            for op, cmd in seq[1:]:
                if (op == "&&") == (st == 0):
                    st = self.run_command(cmd)
        return st

    def run_command(self, c):
        if c[0] == "if":
            for cond, body in c[1]:
                if self.run_list(cond) == 0:
                    return self.run_list(body)
            return self.run_list(c[2]) if c[2] is not None else 0
        argv = self.expand(c[1], c[2])
        if not argv:
            return 0
        name, dot, suffix = argv[0].partition(".")
        fn = getattr(self, "cmd_" + name, None)
        if fn is None or (dot and name not in ("itest", "setexpr")):
            raise SimError("line %d: command '%s' is not in the subset" % (c[2], argv[0]))
        if dot and suffix != "l":
            raise SimError("line %d: width '.%s' is not in the subset" % (c[2], suffix))
        self.line = c[2]
        return fn(argv)

    def expand(self, words, line):
        argv = []
        for parts in words:
            cur = None
            for kind, text, quoted in parts:
                if kind == "lit":
                    cur = (cur or "") + text
                    continue
                v = self.env.get(text, "")
                # hush re-parses a command with an expansion; a value that
                # would read differently there is outside the model.
                if re.search(r'["$\\`]', v) or (not quoted and re.search(r"[\s'#;&|]", v)):
                    raise SimError("line %d: ${%s}=%r would be re-parsed by hush" % (line, text, v))
                if quoted or v:
                    cur = (cur or "") + v
            if cur is not None:
                argv.append(cur)
        return argv

    def usage(self, argv):
        raise SimError("line %d: usage error: %s" % (self.line, " ".join(argv)))

    def cmd_echo(self, argv):
        if argv[1:2] == ["-n"] or any("\\c" in a for a in argv):
            raise SimError("line %d: echo -n/\\c is not in the subset" % self.line)
        self.out(" ".join(argv[1:]))
        return 0

    def cmd_setenv(self, argv):
        if len(argv) < 2 or "=" in argv[1]:
            self.usage(argv)
        if len(argv) == 2:
            self.env.pop(argv[1], None)
            return 0
        value = " ".join(argv[2:])
        if not value:
            raise SimError("line %d: setenv %s to an empty value is not modelled" % (self.line, argv[1]))
        self.env[argv[1]] = value
        return 0

    TEST_OPS = {  # (argument index of the operator, arguments taken)
        "-o": (0, 1), "-a": (0, 1), "-z": (0, 2), "-n": (0, 2),
        "=": (1, 3), "!=": (1, 3), "-eq": (1, 3), "-lt": (1, 3), "-gt": (1, 3),
    }

    def cmd_test(self, argv):
        # ST/cmd/test.c do_test: left to right, no precedence.
        if len(argv) < 3:
            self.usage(argv)
        ap = argv[1:]
        expr, last_expr, last_binop = 0, -1, None
        while ap:
            op = None
            for o, (at, adv) in self.TEST_OPS.items():
                if len(ap) > at and ap[at] == o:
                    op = o
                    break
            if op is None or len(ap) < adv:
                raise SimError("line %d: test expression not in the subset at %r" % (self.line, ap))
            num = lambda s: simple_strtol(s, 0)
            if op == "-z":
                expr = int(ap[1] == "")
            elif op == "-n":
                expr = int(ap[1] != "")
            elif op == "=":
                expr = int(ap[0] == ap[2])
            elif op == "!=":
                expr = int(ap[0] != ap[2])
            elif op == "-eq":
                expr = int(num(ap[0]) == num(ap[2]))
            elif op == "-lt":
                expr = int(num(ap[0]) < num(ap[2]))
            elif op == "-gt":
                expr = int(num(ap[0]) > num(ap[2]))
            if op in ("-o", "-a"):
                last_expr, last_binop = expr, op
            else:
                if last_binop == "-o":
                    expr = int(last_expr or expr)
                elif last_binop == "-a":
                    expr = int(last_expr and expr)
                last_binop = None
            ap = ap[adv:]
        return 0 if expr else 1

    def mem_read(self, addr, w):
        for base, data in self.mem:
            if base <= addr and addr + w <= base + len(data):
                return int.from_bytes(data[addr - base:addr - base + w], "little")
        raise SimError("line %d: read of 0x%x, which nothing was loaded to" % (self.line, addr))

    def operand(self, s, w):
        if s.startswith("*"):
            return self.mem_read(hextoul(s[1:]), w)
        return hextoul(s)

    def cmd_itest(self, argv):
        # ST/cmd/itest.c: evalexp, then a signed long comparison.
        if len(argv) != 4:
            self.usage(argv)
        w = 4
        ops = {"-lt": lambda a, b: a < b, "-gt": lambda a, b: a > b,
               "-eq": lambda a, b: a == b, "!=": lambda a, b: a != b}
        if argv[2] not in ops:
            raise SimError("line %d: itest operator %r not in the subset" % (self.line, argv[2]))
        a, b = (to_signed(self.operand(s, w) & ((1 << (w * 8)) - 1)) for s in (argv[1], argv[3]))
        return 0 if ops[argv[2]](a, b) else 1

    def cmd_setexpr(self, argv):
        # ST/cmd/setexpr.c: get_arg (hex, or '*addr' of width w), env_set_hex.
        w = 4
        if len(argv) == 3:
            value = self.operand(argv[2], w)
        elif len(argv) == 5 and argv[3] in ("+", "-"):
            a, b = self.operand(argv[2], w), self.operand(argv[4], w)
            value = a + b if argv[3] == "+" else a - b
        else:
            raise SimError("line %d: setexpr form not in the subset: %s" % (self.line, " ".join(argv)))
        self.env[argv[1]] = "%x" % (value & MASK64)
        return 0

    def find(self, argv, n):
        if len(argv) != n:
            self.usage(argv)
        if argv[1] != DEVTYPE or argv[2] != "%s:%s" % (DEVNUM, BOOTPART):
            return None
        return self.files.get(argv[-1])

    def cmd_size(self, argv):
        data = self.find(argv, 4)
        if data is None:
            self.out("sim: boot %d: size %s: not found" % (self.k, argv[3]))
            return 1
        self.env["filesize"] = "%x" % len(data)
        return 0

    def cmd_load(self, argv):
        data = self.find(argv, 5)
        if data is None or self.opts.load_fails:
            self.out("sim: boot %d: load %s: fails" % (self.k, argv[4]))
            return 1
        addr = hextoul(argv[3])
        self.mem.append((addr, data))
        self.env["fileaddr"] = "%x" % addr
        self.env["filesize"] = "%x" % len(data)
        return 0

    def cmd_saveenv(self, argv):
        if len(argv) != 1:
            self.usage(argv)
        if self.opts.saveenv_fails:
            self.out("sim: boot %d: saveenv fails" % self.k)
            return 1
        self.saves += 1
        if self.cut and self.cut[0] == self.saves:
            kind = self.cut[1]
            if kind == "torn":
                i = self.active()
                self.copies[1 - i if i is not None else 0][1] = False
            elif kind == "after":
                self.write_env(self.env)
            self.cut_boot = self.k
            self.out("sim: boot %d: saveenv %d: power cut %s" % (
                self.k, self.saves, {"before": "before the write", "torn": "during the write (copy CRC-bad)",
                                     "after": "after the write"}[kind]))
            raise BootEnd("cut")
        target, flag = self.write_env(self.env)
        self.out("sim: boot %d: saveenv %d: copy %s, flag %d" % (self.k, self.saves, "AB"[target], flag))
        return 0

    def cmd_dcache(self, argv):
        if len(argv) != 2 or argv[1] not in ("flush", "off", "on"):
            self.usage(argv)
        return 0

    cmd_icache = cmd_dcache

    def cmd_go(self, argv):
        if len(argv) != 2:
            self.usage(argv)
        addr = hextoul(argv[1])
        if not any(base == addr for base, _ in self.mem):
            raise SimError("line %d: go 0x%x, where no image was loaded" % (self.line, addr))
        self.image_runs += 1
        if self.opts.image_returns:
            self.out("sim: boot %d: the image runs and returns 0" % self.k)
            return 0
        self.out("sim: boot %d: the image runs, then resets the SoC" % self.k)
        raise BootEnd("reset")

    def cmd_exit(self, argv):
        if len(argv) != 1:
            self.usage(argv)
        raise ScriptExit()

    # --- the checks on the stored environment
    def env_report(self):
        """Lines describing the stored env and a list of violations."""
        env = self.stored_env()
        bad = []
        other = sorted(n for n in set(env) | set(self.start)
                       if not n.startswith("mp2s_") and n != "boot_prefixes" and n not in LOOP_VARS
                       and env.get(n) != self.start.get(n))
        if other:
            bad.append("variables outside mp2s_* and boot_prefixes changed: %s" % ", ".join(
                "%s %r -> %r" % (n, self.start.get(n), env.get(n)) for n in other))
        allowed = {self.start.get("boot_prefixes"), self.start.get("mp2s_prefixes")}
        if env.get("boot_prefixes") not in allowed:
            bad.append("boot_prefixes is %r, neither the start value nor mp2s_prefixes" % env.get("boot_prefixes"))
        mp2s = sorted(n for n in env if n.startswith("mp2s_"))
        lines = ["sim: env: boot_prefixes \"%s\", %s" % (
            env.get("boot_prefixes", ""), ", ".join("%s=%s" % (n, env[n]) for n in mp2s) or "no mp2s_*"),
            "sim: env: other variables changed: %s" % (", ".join(other) or "none")]
        loop = [n for n in LOOP_VARS if env.get(n) != self.start.get(n)]
        if loop:
            lines.append("sim: env: distro-loop variables saved: %s" % ", ".join(loop))
        return env, lines, bad


def load_env_file(path):
    env = {}
    with open(path, encoding="utf-8") as f:
        for raw in f:
            raw = raw.rstrip("\n")
            if not raw:
                continue
            name, eq, value = raw.partition("=")
            if not eq or not name:
                raise SimError("%s: not a name=value line: %r" % (path, raw))
            env[name] = value
    return env


def build(args):
    try:
        with open(args.script, encoding="utf-8") as f:
            text = f.read()
    except OSError as e:
        raise SimError("cannot read %s: %s" % (args.script, e))
    ast = parse(text)
    stock = load_env_file(args.env) if args.env else dict(STOCK_ENV)
    start = dict(stock)
    if args.left != "none":
        if not re.fullmatch(r"[1-9]|forever", args.left):
            raise SimError("--left must be 1-9, forever or none (other values: --set mp2s_left=...)")
        if "boot_prefixes" not in stock:
            raise SimError("the stock env has no boot_prefixes")
        orig = stock["boot_prefixes"]
        start.update(mp2s_ver="1", mp2s_left=args.left, mp2s_prefixes=orig,
                     boot_prefixes=OUR_PREFIX + " " + orig)
    for s in args.set:
        name, eq, value = s.partition("=")
        if not eq or not name:
            raise SimError("--set wants NAME=VALUE: %r" % s)
        start[name] = value
    for name in args.unset:
        start.pop(name, None)
    if args.image:
        with open(args.image, "rb") as f:
            args.image_bytes = f.read()
    else:
        args.image_bytes = default_image()
    try:
        fa, fb = (int(x, 0) for x in args.start_flags.split(","))
        assert 0 <= fa <= 255 and 0 <= fb <= 255
    except (ValueError, AssertionError):
        raise SimError("--start-flags wants A,B in 0-255")
    args.start_flags = (fa, fb)
    return ast, len(text) + 72, stock, start


def cmd_run(args, ast, slen, stock, start):
    sim = Sim(ast, slen, stock, start, args)
    sim.run()
    if sim.linux_boots:
        sim.out("sim: result: Linux at boot %d, boots %d, image runs %d, saves %d" % (
            sim.linux_boots[0], sim.boots, sim.image_runs, sim.saves))
    elif sim.stuck:
        sim.out("sim: result: stuck at the U-Boot prompt at boot %d, image runs %d, saves %d" % (
            sim.boots, sim.image_runs, sim.saves))
    else:
        sim.out("sim: result: no Linux within %d boots, image runs %d, saves %d" % (
            sim.boots, sim.image_runs, sim.saves))
    env, lines, bad = sim.env_report()
    for line in lines:
        sim.out(line)
    if args.check_restored:
        if not sim.linux_boots:
            bad.append("Linux was not reached")
        if not sim.restored(env):
            bad.append("the env is not restored")
    for b in bad:
        sim.out("sim: FAIL: " + b)
    return 1 if bad else 0


def cmd_faults(args, ast, slen, stock, start):
    if not re.fullmatch(r"[1-9]", args.left):
        raise SimError("faults wants --left 1-9")
    n = int(args.left)
    args.until_restored = False
    failed = 0

    def check(sim):
        bad = []
        if not sim.linux_boots:
            bad.append("no Linux within %d boots" % sim.boots)
        else:
            boots = sim.linux_boots[0] - (1 if sim.cut_boot else 0)
            if boots > n + 1:
                bad.append("Linux at boot %d (%d without the cut boot), above N+1=%d" % (
                    sim.linux_boots[0], boots, n + 1))
        if sim.image_runs > n:
            bad.append("image runs %d, above N=%d" % (sim.image_runs, n))
        env, _, envbad = sim.env_report()
        bad += envbad
        if not sim.restored(env):
            bad.append("env not restored (boot_prefixes %r, mp2s_* %s)" % (
                env.get("boot_prefixes"), sorted(x for x in env if x.startswith("mp2s_"))))
        return bad

    clean = Sim(ast, slen, stock, start, args, quiet=True)
    clean.run()
    bad = check(clean)
    if clean.linux_boots and clean.linux_boots[0] != n + 1:
        bad.append("clean run: Linux at boot %d, expected %d" % (clean.linux_boots[0], n + 1))
    if clean.image_runs != n:
        bad.append("clean run: image runs %d, expected %d" % (clean.image_runs, n))
    print("sim: fault: none: Linux at boot %s, image runs %d, saves %d: %s" % (
        clean.linux_boots[0] if clean.linux_boots else "-", clean.image_runs, clean.saves,
        "ok" if not bad else "FAIL: " + "; ".join(bad)))
    if bad:
        failed += 1
        for line in clean.lines:
            print("     " + line)
    if clean.saves == 0:
        print("sim: FAIL: the clean run made no saveenv")
        return 1
    cases = 0
    for i in range(1, clean.saves + 1):
        for kind in ("before", "torn", "after"):
            sim = Sim(ast, slen, stock, start, args, cut=(i, kind), quiet=True)
            sim.run()
            cases += 1
            bad = check(sim)
            if sim.cut_boot is None:
                bad.append("the cut never happened")
            print("sim: fault: saveenv %d of %d, cut %s: Linux at boot %s, image runs %d: %s" % (
                i, clean.saves, kind, sim.linux_boots[0] if sim.linux_boots else "-",
                sim.image_runs, "ok" if not bad else "FAIL: " + "; ".join(bad)))
            if bad:
                failed += 1
                for line in sim.lines:
                    print("     " + line)
    print("sim: faults: N=%d, %d cuts, %d failed" % (n, cases, failed))
    return 1 if failed else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("mode", choices=("run", "faults"))
    ap.add_argument("script")
    ap.add_argument("--left", default="none", help="the runner's default: 1-9, forever or none")
    ap.add_argument("--env", help="stock env in fw_printenv name=value lines (default: built-in model)")
    ap.add_argument("--set", action="append", default=[], metavar="NAME=VALUE")
    ap.add_argument("--unset", action="append", default=[], metavar="NAME")
    ap.add_argument("--no-script", action="store_true")
    ap.add_argument("--no-image", action="store_true")
    ap.add_argument("--image", help="default.bin from this file (default: a synthetic arm64 Image)")
    ap.add_argument("--load-fails", action="store_true")
    ap.add_argument("--image-returns", action="store_true")
    ap.add_argument("--saveenv-fails", action="store_true")
    ap.add_argument("--start-flags", default="5,6", help="flags of copy A (stock) and B (edited)")
    ap.add_argument("--max-boots", type=int, default=20)
    ap.add_argument("--until-restored", action="store_true")
    ap.add_argument("--check-restored", action="store_true")
    args = ap.parse_args(argv)
    try:
        if args.max_boots < 1:
            raise SimError("--max-boots must be at least 1")
        built = build(args)
        return (cmd_run if args.mode == "run" else cmd_faults)(args, *built)
    except SimError as e:
        sys.stdout.flush()
        print("sim: error: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
