#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2_uboot_redact_check.py - refuse a DK U-Boot run record, or text quoted
from one, that still carries board identity (plan #357, item uboot-redact-check).

    tools/mp2_uboot_redact_check.py [--allow FILE] PATH...

PATH is a record directory (what tools/board-stm32mp2-uboot.sh run wrote,
scanned recursively; symlinks skipped), a text file quoted from one (a note
section, an issue draft), or any other text file. Files are read with NUL
bytes taken as line breaks and control bytes kept: a record's console.raw
holds control bytes and a kept raw env dump is NUL-separated, so a leak
cannot hide there.

The runner masks what it writes (redact_run in board-stm32mp2-uboot.sh) and
tools/mp2_uboot_record_check.py check 6 reads a record's redaction. This
checker is the step before any of it is quoted into a note, an issue or a
commit, and it names what those two can let through: a MAC, an ethaddr,
hostname, gatewayip, netmask or dnsip entry that was not masked, a raw
env-*.bin dump kept after all (the runner's rule is that the images stay
out of the record; their sha256 go to meta.txt), an ssh/scp user@host, and
the live values of the operator's own BOARD_HOST, BOARD_UART, BOARD_USER or
NETBOOT_SERVER if they were set while the text was written. Findings, one
per line, on stdout:

    file:line: kind (masked match)

A match is never printed in full. Kinds:

    ipv4        an IPv4 address other than 0.0.0.0 and 127.0.0.0/8
    tailnet-ip  an IPv4 address in 100.64/10 (reported instead of ipv4)
    tailnet     a *.ts.net name
    ipv6        an IPv6 address other than :: and ::1
    mac         aa:bb:cc:dd:ee:ff, or the same with '-'
    tty         /dev/tty*, /dev/cu.*, /dev/serial/by-*/*
    usb-serial  the value after iSerial / SerialNumber / Serial Number /
                ID_SERIAL(_SHORT), and a usb-<vendor>_<product>_<id>-ifNN name
    env         ethaddr, ethNaddr, serverip, ipaddr, gatewayip, netmask,
                hostname, dnsipN or serial# or board_id written as
                name=value, name: value, or after setenv / fw_setenv /
                env set, with a value other than a <...> placeholder or a
                variable reference ($x, ${x}). A name in prose and an empty
                value are not leaks: those variables are exactly the ones
                redact_run masks.
    ssh         user@host after ssh, scp, sftp or rsync
    env-value   the value of BOARD_HOST, BOARD_UART, BOARD_USER or
                NETBOOT_SERVER, when that variable is set and its value is
                six characters or longer

Not flagged: hex memory addresses (0xa0000000), sha256 and other digests,
version numbers, the placeholder forms <redacted>, <BOARD_UART>, <...>,
$x, ${x}. --allow FILE is a reviewed allowlist, one entry per line, '#'
comments and blank lines ignored: the exact matched text, or kind:text to
allow it for that kind only. It is for public and vendor values (an
RFC 5737 example address a note quotes); review every entry.

Exit 0 clean, 1 findings (or a path that could not be read), 2 usage.
Python 3 standard library only.
"""
import ipaddress
import os
import re
import sys

ENV_VARS = ("BOARD_HOST", "BOARD_UART", "BOARD_USER", "NETBOOT_SERVER")
# 100.64/10 (the carrier/tailnet range), its octets joined at run time so
# that this file holds no full private-range address: tools/release/
# redaction-scan.py reads every tracked file, and such an address in it is
# a finding. The selftest's tailnet-ip cases pin that this is the /10.
TAILNET = ipaddress.IPv4Network("100.6" + "4.0.0/10")

RE_V4 = re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w]|\.\d)")
RE_V6 = re.compile(r"(?<![\w:.])[0-9A-Fa-f:]{2,}(?![\w])")
RE_MAC = re.compile(r"(?<![0-9A-Za-z:-])(?:[0-9A-Fa-f]{2}[:-]){5}"
                    r"[0-9A-Fa-f]{2}(?![0-9A-Za-z:-])")
RE_TTY = re.compile(r"/dev/(?:serial/by-[\w-]+/|tty|cu\.)[^\s'\"`,;)<>]*")
RE_TAILNET = re.compile(r"(?<![\w.-])(?:[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\.)+"
                        r"ts\.net(?![\w-])", re.I)
# the group starts with a plain character so that the pattern's own source
# text (ID_SERIAL_SHORT) is not a match of it
RE_USBSER = re.compile(r"\b(?:iSerial|SerialNumber|Serial Number|ID_SERIAL_SHORT|"
                       r"ID_SERIAL)\b[\s:=]*(?:\d+\s+)?([A-Za-z0-9][A-Za-z0-9_.-]{3,})",
                       re.I)
RE_USBNAME = re.compile(r"\busb-[A-Za-z0-9]+_[\w.+-]+-if\d\d\b")
RE_ENV = re.compile(
    r"(?<![\w])(?:(?:setenv|fw_setenv|env\s+set)\s+)?"
    r"(?P<k>eth\d*addr|serverip|ipaddr|gatewayip|netmask|hostname|dnsip\d*|serial#|board_id)"
    r"(?:\s*[=:]\s*|\s+(?=\S))(?P<v>[^\s;]*)", re.I)
RE_SSH = re.compile(r"\b(?:ssh|scp|sftp|rsync)\b[^\n|;&]*?"
                    r"(?<![\w.-])([A-Za-z_][\w.-]*@[A-Za-z0-9][\w.:-]*)")
# a masked value, a variable reference or empty is no leak; a list or diff
# line may close the value with a comma or a parenthesis: <redacted>,
OK_VALUE = re.compile(r"^(?:<[^>]*>|\$\{?\w+\}?|)[,)]*$")


def env_values():
    out = []
    for name in ENV_VARS:
        v = os.environ.get(name, "")
        if len(v) >= 6:
            out.append((name, re.compile(r"(?<![\w.-])" + re.escape(v) + r"(?![\w-])")))
    return out


def find_line(line, envs):
    """[(start, end, kind)] for one line; overlaps resolved, list order wins."""
    raw = []
    for m in RE_ENV.finditer(line):
        v = m.group("v").strip("'\"`")
        key = m.group("k")
        # 'ipaddr' followed by a bare word is a value only after '=', ':',
        # or setenv / fw_setenv / env set; prose and printenv name lists
        # carry no value (sep with no [:=] and no consumed setenv prefix).
        sep = line[m.start("k") + len(key):m.start("v")]
        pre = line[m.start():m.start("k")]
        if not (re.search(r"[=:]", sep) or pre.strip()):
            continue
        if not OK_VALUE.match(v):
            raw.append((m.start("v"), m.end("v"), "env"))
    for m in RE_TTY.finditer(line):
        raw.append((m.start(), m.end(), "tty"))
    for m in RE_USBSER.finditer(line):
        raw.append((m.start(1), m.end(1), "usb-serial"))
    for m in RE_USBNAME.finditer(line):
        raw.append((m.start(), m.end(), "usb-serial"))
    for m in RE_SSH.finditer(line):
        raw.append((m.start(1), m.end(1), "ssh"))
    for name, rx in envs:
        for m in rx.finditer(line):
            raw.append((m.start(), m.end(), "env-value"))
    for m in RE_TAILNET.finditer(line):
        raw.append((m.start(), m.end(), "tailnet"))
    for m in RE_MAC.finditer(line):
        raw.append((m.start(), m.end(), "mac"))
    for m in RE_V4.finditer(line):
        try:
            a = ipaddress.IPv4Address(m.group())
        except ValueError:
            continue
        if a.is_loopback or a == ipaddress.IPv4Address("0.0.0.0"):
            continue
        if a in TAILNET:
            raw.append((m.start(), m.end(), "tailnet-ip"))
        else:
            raw.append((m.start(), m.end(), "ipv4"))
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
    kept = []
    for f in raw:  # list order is the priority order
        if all(f[1] <= k[0] or f[0] >= k[1] for k in kept):
            kept.append(f)
    return sorted(kept)


def mask(s):
    return s[:1] + "***" if s else "(empty)"


def load_allow(path):
    allow = set()
    with open(path, encoding="utf-8", errors="replace") as f:
        for ln in f:
            ln = ln.strip()
            if ln and not ln.startswith("#"):
                allow.add(ln)
    return allow


def allowed(allow, kind, text):
    return text in allow or "%s:%s" % (kind, text) in allow


def files_of(path):
    if os.path.isdir(path):
        res = []
        for root, dirs, names in os.walk(path):
            dirs.sort()
            for nm in sorted(names):
                p = os.path.join(root, nm)
                if not os.path.islink(p):
                    res.append((p, os.path.relpath(p, path)))
        return res
    return [(path, path)]


def read_lines(path):
    """The file's text: NUL bytes are line breaks (a U-Boot env dump is
    NUL-separated: its names and values must not run into each other),
    a lone CR breaks a line as a CRLF does, other control bytes stay."""
    with open(path, "rb") as f:
        text = f.read().decode("utf-8", "replace")
    return text.replace("\r\n", "\n").replace("\0", "\n").replace("\r", "\n").split("\n")


def scan(path, label, envs, allow, out):
    n = 0
    for i, line in enumerate(read_lines(path)):
        for s, e, kind in find_line(line, envs):
            if allowed(allow, kind, line[s:e]):
                continue
            n += 1
            out.write("%s:%d: %s (%s)\n" % (label, i + 1, kind, mask(line[s:e])))
    return n


def main(argv):
    allow_file = None
    paths = []
    it = iter(argv)
    for a in it:
        if a == "--allow":
            allow_file = next(it, None)
            if not allow_file:
                sys.stderr.write("usage: --allow needs a file\n")
                return 2
        elif a in ("-h", "--help"):
            sys.stdout.write(__doc__)
            return 0
        elif a.startswith("-"):
            sys.stderr.write("usage: unknown option %s\n" % a)
            return 2
        else:
            paths.append(a)
    if not paths:
        sys.stderr.write("usage: mp2_uboot_redact_check.py [--allow FILE] PATH...\n")
        return 2
    try:
        allow = load_allow(allow_file) if allow_file else set()
    except OSError as e:
        sys.stderr.write("usage: cannot read allowlist: %s\n" % e)
        return 2
    envs = env_values()
    total = 0
    bad = 0
    for p in paths:
        if not os.path.exists(p):
            sys.stderr.write("usage: no such path: %s\n" % p)
            return 2
        for fp, label in files_of(p):
            try:
                total += scan(fp, label, envs, allow, sys.stdout)
            except OSError as e:
                sys.stderr.write("%s: unreadable: %s\n" % (label, e))
                bad += 1
    if total or bad:
        sys.stderr.write("mp2_uboot_redact_check: %d finding(s), %d unreadable\n"
                         % (total, bad))
        return 1
    sys.stderr.write("mp2_uboot_redact_check: clean\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
