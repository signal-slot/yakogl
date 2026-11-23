#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_redact_check.py - keep board identity out of anything quoted from a
DK standalone record (plan #362, item redact-check).

    tools/mp2s_redact_check.py [--fix OUT] [PATH...]     ("-" or none: stdin)

Scans files, directories (recursively; symlinks and binary files skipped)
or stdin for what CLAUDE.md forbids in a file, commit, issue or pull
request:

    ipv4, ipv6, mac    addresses (loopback, 0.0.0.0, :: and ::1 are fine;
                       U-Boot hex such as 0xa0000000 is not an address)
    tailnet            *.ts.net names
    fqdn               other dotted names under a known TLD, except
                       github.com, kernel.org, st.com and their subdomains
    tty                /dev/tty*, /dev/cu.*, /dev/serial/by-*/*
    serial             the value after iSerial / SerialNumber / Serial Number
    secret             a password, passwd or token setting's value (for
                       password and passwd a colon counts as well)
    env:BOARD_HOST, env:BOARD_UART, env:BOARD_USER
                       the value of that variable, when it is set

Each finding is one line "path:line:kind:masked" on stdout (the match is
shown as its first character and "***", never in full). Exit 0 clean,
1 findings, 2 usage or I/O error.

--fix OUT writes a redacted copy ("<redacted:kind>" in place of each match)
to OUT: a file for one file or stdin, a directory for one directory. It
never writes in place: OUT may not be an input, nor inside or above one.
Binary files are not copied. The exit status still reflects the input.
"""
import ipaddress
import os
import re
import sys

ENV_VARS = ("BOARD_HOST", "BOARD_UART", "BOARD_USER")
FQDN_ALLOW = ("github.com", "kernel.org", "st.com")
TLDS = ("com net org io dev edu gov mil arpa jp de uk fr cn ru us "
        "local lan internal home corp intranet localdomain").split()

# ST's stock U-Boot env (stm32mp25 default, st-env/) carries exactly one
# address, "serverip=192.168.1.1": a vendor default, not a farm address.
# Only that text in that position is allowed; the same address elsewhere
# and any other serverip value are still findings.
ST_STOCK_V4 = re.compile(r"(?<![\w.])serverip=192\.168\.1\.1(?![\w.])")
RE_V4 = re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w]|\.\d)")
RE_V6 = re.compile(r"(?<![\w:.])[0-9A-Fa-f:]{2,}(?![\w])")
RE_MAC = re.compile(r"(?<![0-9A-Za-z:-])(?:[0-9A-Fa-f]{2}[:-]){5}"
                    r"[0-9A-Fa-f]{2}(?![0-9A-Za-z:-])")
RE_FQDN = re.compile(r"(?<![\w.-])(?:[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\.)+"
                     r"(?:%s)(?![\w-])" % "|".join(TLDS), re.I)
RE_TTY = re.compile(r"/dev/(?:serial/by-[\w-]+/|tty|cu\.)[^\s'\"`,;)]*")
RE_SERIAL = re.compile(r"\b(?:iSerial|SerialNumber|Serial Number)\b[\s:=]*"
                       r"(?:\d+\s+)?([A-Za-z0-9_.-]{4,})", re.I)
RE_SECRETS = re.compile(r"\b(?:password|passwd)\b\s*[=:]\s*([^\s]+)|"
                       r"\btoken=([^\s]+)", re.I)
MASKED = re.compile(r"^(<redacted|\*\*\*)")

# overlapping matches: the earlier kind wins
ORDER = ("secret", "serial", "env", "tty", "mac", "ipv6", "ipv4", "tailnet", "fqdn")


def env_values():
    out = []
    for name in ENV_VARS:
        v = os.environ.get(name, "")
        if v:
            out.append((name, re.compile(r"(?<![\w])" + re.escape(v) + r"(?![\w])")))
    return out


def find_line(line, envs):
    """[(start, end, kind)] non-overlapping, for one line."""
    raw = []
    for m in RE_SECRETS.finditer(line):
        g = 1 if m.group(1) is not None else 2
        if not MASKED.match(m.group(g)):
            raw.append((m.start(g), m.end(g), "secret"))
    for m in RE_SERIAL.finditer(line):
        raw.append((m.start(1), m.end(1), "serial"))
    for m in RE_TTY.finditer(line):
        raw.append((m.start(), m.end(), "tty"))
    for name, rx in envs:
        for m in rx.finditer(line):
            raw.append((m.start(), m.end(), "env:" + name))
    for m in RE_MAC.finditer(line):
        raw.append((m.start(), m.end(), "mac"))
    for m in RE_V6.finditer(line):
        t = m.group()
        if t.count(":") < 2:
            continue
        try:
            a = ipaddress.IPv6Address(t)
        except ValueError:
            continue
        if not (a.is_loopback or a.is_unspecified):
            raw.append((m.start(), m.end(), "ipv6"))
    stock = [m.span() for m in ST_STOCK_V4.finditer(line)]
    for m in RE_V4.finditer(line):
        if any(a <= m.start() and m.end() <= b for a, b in stock):
            continue
        try:
            a = ipaddress.IPv4Address(m.group())
        except ValueError:
            continue
        if not (a.is_loopback or a.is_unspecified):
            raw.append((m.start(), m.end(), "ipv4"))
    for m in RE_FQDN.finditer(line):
        h = m.group().lower()
        if h.endswith(".ts.net"):
            raw.append((m.start(), m.end(), "tailnet"))
        elif not any(h == d or h.endswith("." + d) for d in FQDN_ALLOW):
            raw.append((m.start(), m.end(), "fqdn"))

    def rank(f):
        return ORDER.index(f[2].split(":")[0])
    kept = []
    for f in sorted(raw, key=rank):
        if all(f[1] <= k[0] or f[0] >= k[1] for k in kept):
            kept.append(f)
    return sorted(kept)


def mask(s):
    return s[:1] + "***"


def scan_text(text, name, envs, out):
    """Print findings; return (count, redacted text)."""
    n = 0
    lines = text.split("\n")
    for i, line in enumerate(lines):
        fs = find_line(line, envs)
        for s, e, kind in fs:
            n += 1
            out.write("%s:%d:%s:%s\n" % (name, i + 1, kind, mask(line[s:e])))
        for s, e, kind in reversed(fs):
            line = line[:s] + "<redacted:%s>" % kind.split(":")[0] + line[e:]
        lines[i] = line
    return n, "\n".join(lines)


def read_text(path):
    with open(path, "rb") as f:
        data = f.read()
    if b"\0" in data[:8192]:
        return None
    return data.decode("utf-8", "replace")


def files_of(path):
    """[(file, relative name)] for a file or a directory tree."""
    if os.path.isdir(path):
        res = []
        for root, dirs, names in os.walk(path):
            dirs.sort()
            for nm in sorted(names):
                p = os.path.join(root, nm)
                if not os.path.islink(p):
                    res.append((p, os.path.relpath(p, path)))
        return res
    return [(path, os.path.basename(path))]


def inside(a, b):
    a, b = os.path.realpath(a), os.path.realpath(b)
    return a == b or a.startswith(b + os.sep)


def write_file(dst, text):
    os.makedirs(os.path.dirname(os.path.abspath(dst)), exist_ok=True)
    with open(dst, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def main(argv):
    fix = None
    paths = []
    it = iter(argv)
    for a in it:
        if a == "--fix":
            fix = next(it, None)
            if not fix:
                sys.stderr.write("usage: --fix needs an output path\n")
                return 2
        elif a in ("-h", "--help"):
            sys.stdout.write(__doc__)
            return 0
        elif a.startswith("-") and a != "-":
            sys.stderr.write("usage: unknown option %s\n" % a)
            return 2
        else:
            paths.append(a)
    paths = paths or ["-"]
    if fix is not None:
        if len(paths) != 1:
            sys.stderr.write("usage: --fix takes exactly one input\n")
            return 2
        if paths[0] != "-" and (inside(fix, paths[0]) or inside(paths[0], fix)):
            sys.stderr.write("refused: --fix never writes in place "
                             "(output overlaps the input)\n")
            return 2
    envs = env_values()
    total = 0
    try:
        if paths == ["-"]:
            text = sys.stdin.buffer.read().decode("utf-8", "replace")
            n, red = scan_text(text, "<stdin>", envs, sys.stdout)
            if fix:
                write_file(fix, red)
            return 1 if n else 0
        for path in paths:
            if not os.path.exists(path):
                sys.stderr.write("error: %s: no such file or directory\n" % path)
                return 2
            isdir = os.path.isdir(path)
            for fp, rel in files_of(path):
                text = read_text(fp)
                if text is None:
                    sys.stderr.write("skipped binary file: %s\n" % fp)
                    continue
                n, red = scan_text(text, fp, envs, sys.stdout)
                total += n
                if fix:
                    write_file(os.path.join(fix, rel) if isdir else fix, red)
    except OSError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
