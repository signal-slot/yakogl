#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2_uboot_redact_check.py - refuse a DK U-Boot run record, or text quoted
from one, that still holds board identity (plan #357, item uboot-redact-check).

    tools/mp2_uboot_redact_check.py [--allow FILE] PATH...

PATH is a record directory (scanned recursively; symlinks skipped) or text
files, for instance records/stm32mp257f-dk/<stamp>-<name>-<sha8>/ and
docs/notes/stm32mp257f-dk-uboot-boot.md. Every file is scanned, binary or
not (console.raw may hold control bytes). Findings, one per line, on stdout:

    file:line: kind (masked match)

The match is never printed in full. Kinds:

    ipv4        any IPv4 address except 0.0.0.0 and 127.0.0.0/8
    tailnet-ip  an address in 100.64.0.0/10 (also reported instead of ipv4)
    tailnet     a *.ts.net name
    mac         a MAC address (aa:bb:cc:dd:ee:ff or with '-')
    tty         /dev/tty*, /dev/ttyUSB*, /dev/serial/by-*/*, /dev/cu.*
    usb-serial  the value after iSerial / SerialNumber / Serial Number /
                ID_SERIAL(_SHORT), and a usb-<vendor>_<product>_<id>-ifNN name
    env         ethaddr, ethNaddr, serverip, ipaddr, serial# or board_id as
                'name=value', 'name: value' or 'setenv name value' with a
                value other than <redacted> (quotes ignored). An empty value
                and a variable reference ($x, ${x}) are not a leak.
    ssh         user@host after ssh, scp, sftp or rsync
    env-value   the value of BOARD_HOST, BOARD_UART, BOARD_USER,
                NETBOOT_SERVER when set (six characters or longer)

Not flagged: hex memory addresses (0xa0000000), sha256 and other digests,
version numbers. --allow FILE is a reviewed allowlist: one entry per line,
'#' comments, blank lines ignored. An entry is the exact matched text, or
'kind:text' to allow it for that kind only. The allowlist is for names that
are public (a documentation address in a quoted example); review each entry.

Exit 0 clean, 1 findings (or an unreadable path inside a record), 2 usage.
Python 3 standard library only.
"""
import ipaddress
import os
import re
import sys

ENV_VARS = ("BOARD_HOST", "BOARD_UART", "BOARD_USER", "NETBOOT_SERVER")

RE_V4 = re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w]|\.\d)")
RE_MAC = re.compile(r"(?<![0-9A-Za-z:-])(?:[0-9A-Fa-f]{2}[:-]){5}"
                    r"[0-9A-Fa-f]{2}(?![0-9A-Za-z:-])")
RE_TTY = re.compile(r"/dev/(?:serial/by-[\w-]+/|tty|cu\.)[^\s'\"`,;)<>]*")
RE_TAILNET = re.compile(r"(?<![\w.-])(?:[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?\.)+"
                        r"ts\.net(?![\w-])", re.I)
RE_USBSER = re.compile(r"\b(?:iSerial|SerialNumber|Serial Number|ID_SERIAL_SHORT|"
                       r"ID_SERIAL)\b[\s:=]*(?:\d+\s+)?([A-Za-z0-9_.-]{4,})", re.I)
RE_USBNAME = re.compile(r"\busb-[A-Za-z0-9]+_[\w.+-]+-if\d\d\b")
RE_ENV = re.compile(
    r"(?<![\w])(?:(?:setenv|env\s+set)\s+)?"
    r"(?P<k>eth\d*addr|serverip|ipaddr|serial#|board_id)"
    r"(?:\s*[=:]\s*|\s+(?=\S))(?P<v>[^\s;]*)", re.I)
RE_SSH = re.compile(r"\b(?:ssh|scp|sftp|rsync)\b[^\n|;&]*?"
                    r"(?<![\w.-])([A-Za-z_][\w.-]*@[A-Za-z0-9][\w.:-]*)")
OK_VALUE = re.compile(r"^(?:<redacted>|\$\{?\w+\}?|)$")


def env_values():
    out = []
    for name in ENV_VARS:
        v = os.environ.get(name, "")
        if len(v) >= 6:
            out.append((name, re.compile(r"(?<![\w.-])" + re.escape(v) + r"(?![\w-])")))
    return out


def find_line(line, envs):
    """[(start, end, kind)] for one line, overlaps resolved (earlier kind wins)."""
    raw = []
    for m in RE_ENV.finditer(line):
        raw_v = m.group("v")
        v = raw_v.strip("'\"`")
        key = m.group("k")
        # 'ipaddr' followed by a bare word in prose is only a value after '=',
        # ':' or setenv; a plain space form is checked only for setenv/env set
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
        if a in ipaddress.IPv4Network("100.64.0.0/10"):
            raw.append((m.start(), m.end(), "tailnet-ip"))
        else:
            raw.append((m.start(), m.end(), "ipv4"))
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


def scan(path, label, envs, allow, out):
    with open(path, "rb") as f:
        text = f.read().decode("utf-8", "replace")
    n = 0
    for i, line in enumerate(text.replace("\r\n", "\n").replace("\r", "\n").split("\n")):
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
