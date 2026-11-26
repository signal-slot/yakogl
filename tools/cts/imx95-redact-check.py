#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Rewrite or check Verdin iMX95 U-Boot console logs for board identifiers.

  imx95-redact-check.py PATH...          rewrite files (or directories) in place:
                                           MAC address                  -> <mac>
                                           env ethaddr, eth1addr, serial#, serverip, ipaddr,
                                           gatewayip, netmask values    -> <redacted>
                                           Toradex 'Serial#:' banner    -> <serial>
                                           dotted quad                  -> <addr>
                                           /dev/tty*                    -> <uart>
                                           /home/...                    -> <home>
  imx95-redact-check.py --check PATH...  print file:line:kind for every hit, never the value;
                                         exit 1 on any hit, 0 when clean

A dotted quad glued to a word, '.' or '-' is a version token (a version such as mesa-24.0.x or
the dEQP version token) and is kept. NUL bytes are stripped; symlinks are skipped.
"""
import os
import re
import sys

ENV_KEYS = r"(?:ethaddr|eth1addr|serial#|serverip|ipaddr|gatewayip|netmask)"
MAC = re.compile(rb"(?<![0-9A-Fa-f:])(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f:])")
ENV = re.compile(rb"(?m)^(\s*" + ENV_KEYS.encode() + rb"\s*=\s*)([^<\s]\S*)")
SERIAL = re.compile(rb"(Serial#:[ \t]*)([^<\s]\S*)")
QUAD = re.compile(rb"(?<![A-Za-z0-9_.\-])(?:[0-9]{1,3}\.){3}[0-9]{1,3}(?![A-Za-z0-9_\-]|\.[0-9A-Za-z])")
UART = re.compile(rb"/dev/tty[A-Za-z0-9_]*")
HOME = re.compile(rb"/home/[^\s\"'<>:;,)]+")

# (kind, regex, replacement); ENV and SERIAL keep their prefix (group 1).
RULES = [
    ("env", ENV, rb"\1<redacted>"),
    ("serial", SERIAL, rb"\1<serial>"),
    ("mac", MAC, b"<mac>"),
    ("addr", QUAD, b"<addr>"),
    ("uart", UART, b"<uart>"),
    ("home", HOME, b"<home>"),
]


def files(paths):
    for p in paths:
        if os.path.islink(p):
            continue
        if os.path.isdir(p):
            for root, dirs, names in os.walk(p):
                dirs[:] = sorted(d for d in dirs if not os.path.islink(os.path.join(root, d)))
                for n in sorted(names):
                    f = os.path.join(root, n)
                    if not os.path.islink(f):
                        yield f
        elif os.path.isfile(p):
            yield p
        else:
            print(f"imx95-redact-check: no such path: {p}", file=sys.stderr)
            sys.exit(2)


def rewrite(data):
    data = data.replace(b"\0", b"")
    for _, rx, rep in RULES:
        data = rx.sub(rep, data)
    return data


def check(paths):
    hits = 0
    for path in files(paths):
        with open(path, "rb") as f:
            data = f.read().replace(b"\0", b"")
        for no, line in enumerate(data.split(b"\n"), 1):
            for kind, rx, _ in RULES:
                if rx.search(line):
                    print(f"{path}:{no}:{kind}")
                    hits += 1
    return 1 if hits else 0


def main(argv):
    do_check = False
    if argv and argv[0] == "--check":
        do_check, argv = True, argv[1:]
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    if do_check:
        return check(argv)
    for path in files(argv):
        with open(path, "rb") as f:
            old = f.read()
        new = rewrite(old)
        if new != old:
            with open(path, "wb") as f:
                f.write(new)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
