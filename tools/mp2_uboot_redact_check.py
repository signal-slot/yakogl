#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2_uboot_redact_check.py - refuse a DK U-Boot run record, or text quoted
from one, that still carries board or lab identity (plan #357, item
uboot-redact-check).

    python3 -I tools/mp2_uboot_redact_check.py [--allow FILE]... PATH...
    ("-" reads stdin)

PATH is a record directory (records/stm32mp257f-dk/<stamp>-<name>-<sha8>/:
meta.txt, console.raw, console.ts, typed.log, env-before.txt,
env-after.txt, env.diff, boot-sha256*.txt, boot.diff, verdict.txt,
INDEX.txt; read recursively) or a text file such as
docs/notes/stm32mp257f-dk-uboot-boot.md. Every file is read as text: NUL
bytes, carriage returns and ANSI escapes are removed first, so a raw
console capture is checked too and never skipped as binary. Symbolic links
are not followed (a note on stderr).

It is independent of the runner's redact_run (tools/board-stm32mp2-uboot.sh):
a second look, not the same code. Kinds:

    ipv4      an IPv4 address other than 0.0.0.0, 127.0.0.0/8 and
              192.0.2.0/24 (RFC 5737 TEST-NET-1, the range redact_run puts in
              place of every address it finds). The other RFC 5737 ranges
              are findings: a record never holds them. A dotted quad that
              continues with ".<digit>" is a version, not an address.
    tailnet   an address in 100.64/10 (CGNAT), or a *.ts.net name
    ipv6      an IPv6 address other than ::, ::1 and 2001:db8::/32
    mac       six hex pairs joined by ':' or '-' (not all-zero)
    tty       /dev/tty*, /dev/cu.* and the links under /dev/serial/by-id
              and by-path
    serial    the value after iSerial, SerialNumber, "Serial Number",
              ID_SERIAL(_SHORT), serial:, serial_number= and the like, when
              it holds a digit ("400e0000.serial: ..." is a driver, not one)
    env       a value of ethaddr, eth<N>addr, serverip, ipaddr, gatewayip,
              netmask, hostname, dnsip<N>, serial# or board_id (as KEY=value)
              other than <redacted>, <redacted:...>, a $-reference or empty
    login     <login>@<host> (ssh, scp, a shell prompt, ssh://): a devicetree
              node such as memory@a0000000 or ethernet@8000000port@1 (the
              part after "@" starts with a digit or is hex only) is not one,
              nor an RFC 2606 host (example.com, *.invalid, *.test)
    lanhost   a name under .local, .lan, .internal or .home.arpa
    envval    the value of BOARD_HOST, BOARD_UART, BOARD_USER or
              NETBOOT_SERVER, when that variable is set and 3 characters or
              longer

Memory addresses (0xa0000000, a0000000) and sha256 digests are not
findings: none of the shapes above matches them.

Each finding is one line "path:line: kind: masked" on stdout. The match is
shown masked (first character, "***", its length), never in full. Exit 0:
clean; 1: findings; 2: usage, I/O or allowlist error.

--allow FILE (repeatable) reads a reviewed allowlist. Each line other than
blank ones and "#" comments is

    kind | path glob | regex | reason

All four fields are required. A finding is dropped when its kind equals
the first field (or that field is "*"), the path matches the glob (fnmatch
over the path as printed), and the regex matches the whole matched text
(re.fullmatch). The reason is free text for the reviewer. A rule names a
pattern or a made-up value, never a real address. A malformed line stops
the check with exit 2 and its line number. Dropped findings are counted on
stderr.

tools/mp2_uboot_redact_selftest.sh checks this on records it generates.
"""
import fnmatch
import ipaddress
import os
import re
import sys

ENV_VARS = ("BOARD_HOST", "BOARD_UART", "BOARD_USER", "NETBOOT_SERVER")

PLACEHOLDER_NET = ipaddress.ip_network("192.0.2.0/24")     # redact_run's
TAILNET = ipaddress.ip_network(((100 << 24) | (64 << 16), 10))  # 100.64/10
V6_DOC = ipaddress.ip_network("2001:db8::/32")

RE_V4 = re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w]|\.\d)")
RE_V6 = re.compile(r"(?<![\w:.])[0-9A-Fa-f:]{2,}(?:%[\w]+)?(?![\w:])")
RE_MAC = re.compile(r"(?<![0-9A-Za-z:-])(?:[0-9A-Fa-f]{2}([:-]))"
                    r"(?:[0-9A-Fa-f]{2}\1){4}[0-9A-Fa-f]{2}(?![0-9A-Za-z:-])")
RE_TSNET = re.compile(r"(?<![\w.-])(?:[A-Za-z0-9-]+\.)+ts\.net(?![\w-])", re.I)
RE_LANHOST = re.compile(r"(?<![\w.-])(?:[A-Za-z0-9-]+\.)+"
                        r"(?:local|lan|internal|home\.arpa)(?![\w.-])", re.I)
RE_TTY = re.compile(r"/dev/(?:tty|cu\.|serial/by-(?:id|path)/)[^\s'\"`,;)\]>]*")
SERIAL_VAL = r"([A-Za-z0-9_.:-]{4,})"
RE_SERIAL = re.compile(
    r"\biSerial\b\s+(?:\d+\s+)?" + SERIAL_VAL + r"|"
    r"\b(?:SerialNumber|Serial[ _]Number|ID_SERIAL(?:_SHORT)?|serial_?no|"
    r"(?<![\w.@-])serial)\b\s*[:=]\s*" + SERIAL_VAL, re.I)
ENV_KEYS = (r"ethaddr|eth\d+addr|serverip|ipaddr|gatewayip|netmask|hostname|"
            r"dnsip\d*|serial#|board_id")
RE_ENV = re.compile(r"(?<![\w#$])(?:%s)=([^\s\"'`;,)]*)" % ENV_KEYS)
RE_ENV_OK = re.compile(r"^(?:|<redacted>|<redacted:[\w-]+>|\$\{?\w+\}?)$")
RE_LOGIN = re.compile(r"(?<![\w.%+-])([A-Za-z_][\w.-]*)@([A-Za-z0-9][\w-]*"
                      r"(?:\.[A-Za-z0-9][\w-]*)*)")
RE_HEX_UNIT = re.compile(r"^(?:[0-9A-Fa-f]+(?:,[0-9A-Fa-f]+)*$|[0-9])")
RE_EXAMPLE_HOST = re.compile(r"(?:^|\.)(?:example\.(?:com|net|org)|[\w-]+\.(?:invalid|example|test))$",
                             re.I)
RE_ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")

# overlapping matches: the earlier kind wins
ORDER = ("env", "serial", "envval", "tty", "login", "mac", "ipv6", "tailnet",
         "ipv4", "lanhost")


def env_values():
    out = []
    for name in ENV_VARS:
        v = os.environ.get(name, "")
        if len(v) >= 3:
            out.append(re.compile(r"(?<![\w])" + re.escape(v) + r"(?![\w])"))
    return out


def find_line(line, envs):
    """[(start, end, kind)] non-overlapping, for one line."""
    raw = []
    for m in RE_ENV.finditer(line):
        if not RE_ENV_OK.match(m.group(1)):
            raw.append((m.start(1), m.end(1), "env"))
    for m in RE_SERIAL.finditer(line):
        g = 1 if m.group(1) is not None else 2
        v = m.group(g)
        if v.startswith("<") or v.startswith("$") or not re.search(r"\d", v):
            continue
        raw.append((m.start(g), m.end(g), "serial"))
    for rx in envs:
        for m in rx.finditer(line):
            raw.append((m.start(), m.end(), "envval"))
    for m in RE_TTY.finditer(line):
        raw.append((m.start(), m.end(), "tty"))
    for m in RE_LOGIN.finditer(line):
        if not (RE_HEX_UNIT.match(m.group(2)) or RE_EXAMPLE_HOST.search(m.group(2))):
            raw.append((m.start(), m.end(), "login"))
    for m in RE_MAC.finditer(line):
        if set(m.group().replace(":", "").replace("-", "")) != {"0"}:
            raw.append((m.start(), m.end(), "mac"))
    for m in RE_V6.finditer(line):
        t = m.group().split("%")[0]
        if t.count(":") < 2:
            continue
        try:
            a = ipaddress.IPv6Address(t)
        except ValueError:
            continue
        if not (a.is_loopback or a.is_unspecified or a in V6_DOC):
            raw.append((m.start(), m.end(), "ipv6"))
    for m in RE_TSNET.finditer(line):
        raw.append((m.start(), m.end(), "tailnet"))
    for m in RE_V4.finditer(line):
        try:
            a = ipaddress.IPv4Address(m.group())
        except ValueError:
            continue
        if a in TAILNET:
            raw.append((m.start(), m.end(), "tailnet"))
        elif not (a.is_loopback or a.is_unspecified or a in PLACEHOLDER_NET):
            raw.append((m.start(), m.end(), "ipv4"))
    for m in RE_LANHOST.finditer(line):
        raw.append((m.start(), m.end(), "lanhost"))

    kept = []
    for f in sorted(raw, key=lambda f: (ORDER.index(f[2]), f[0])):
        if all(f[1] <= k[0] or f[0] >= k[1] for k in kept):
            kept.append(f)
    return sorted(kept)


def mask(s):
    return "%s***[%d]" % (s[:1], len(s))


class Allow:
    def __init__(self):
        self.rules = []
        self.dropped = 0

    def load(self, path):
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                s = line.strip()
                if not s or s.startswith("#"):
                    continue
                parts = [p.strip() for p in s.split("|", 3)]
                if len(parts) != 4 or not all(parts):
                    raise ValueError("%s:%d: want 'kind | path glob | regex | "
                                     "reason', all four" % (path, n))
                kind, glob, rx, _reason = parts
                if kind != "*" and kind not in ORDER:
                    raise ValueError("%s:%d: unknown kind" % (path, n))
                try:
                    crx = re.compile(rx)
                except re.error as e:
                    raise ValueError("%s:%d: bad regex: %s" % (path, n, e))
                self.rules.append((kind, glob, crx))

    def drops(self, path, kind, text):
        for k, glob, rx in self.rules:
            if (k == "*" or k == kind) and fnmatch.fnmatch(path, glob) \
                    and rx.fullmatch(text):
                self.dropped += 1
                return True
        return False


def clean(data):
    text = data.decode("utf-8", "replace")
    text = RE_ANSI.sub("", text).replace("\0", "").replace("\r", "")
    return text


def scan_text(text, name, envs, allow, out):
    n = 0
    for i, line in enumerate(text.split("\n"), 1):
        for s, e, kind in find_line(line, envs):
            if allow.drops(name, kind, line[s:e]):
                continue
            n += 1
            out.write("%s:%d: %s: %s\n" % (name, i, kind, mask(line[s:e])))
    return n


def files_of(path):
    if os.path.isdir(path):
        res = []
        for root, dirs, names in os.walk(path):
            dirs.sort()
            for nm in sorted(names):
                p = os.path.join(root, nm)
                if os.path.islink(p):
                    sys.stderr.write("not followed (symlink): %s\n" % p)
                else:
                    res.append(p)
        return res
    return [path]


def main(argv):
    allow = Allow()
    paths = []
    it = iter(argv)
    try:
        for a in it:
            if a == "--allow":
                f = next(it, None)
                if not f:
                    sys.stderr.write("usage: --allow needs a file\n")
                    return 2
                allow.load(f)
            elif a in ("-h", "--help"):
                sys.stdout.write(__doc__)
                return 0
            elif a.startswith("-") and a != "-":
                sys.stderr.write("usage: unknown option %s\n" % a)
                return 2
            else:
                paths.append(a)
    except (OSError, ValueError) as e:
        sys.stderr.write("allowlist error: %s\n" % e)
        return 2
    if not paths:
        sys.stderr.write("usage: mp2_uboot_redact_check.py [--allow FILE]... "
                         "PATH... (- for stdin)\n")
        return 2
    envs = env_values()
    total = 0
    try:
        for path in paths:
            if path == "-":
                total += scan_text(clean(sys.stdin.buffer.read()), "<stdin>",
                                   envs, allow, sys.stdout)
                continue
            if not os.path.exists(path):
                sys.stderr.write("error: %s: no such file or directory\n" % path)
                return 2
            for fp in files_of(path):
                with open(fp, "rb") as f:
                    total += scan_text(clean(f.read()), fp, envs, allow,
                                       sys.stdout)
    except OSError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2
    if allow.dropped:
        sys.stderr.write("allowlist: %d finding(s) dropped\n" % allow.dropped)
    if total:
        sys.stderr.write("mp2_uboot_redact_check: %d finding(s): refused\n" % total)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
