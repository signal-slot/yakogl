#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check or scrub committed i.MX8MP logs for board addresses, logins and serial device paths.

  imx8mp-gles3-redact.py --check DIR...   report file:line and the pattern name of every hit (never the
                                          matched text); exit 1 on any hit, else print 'redact: <n> files clean'
  imx8mp-gles3-redact.py --fix DIR...     rewrite the hits in place to <tty>, <addr> and <login>

The three patterns are those of the plan's acceptance grep (grep -rEIn on tools/cts/log/*-imx8mp-gles3-* and
*-imx8mp-es2-*). Dotted version strings such as 1.2.3.4 also match the address pattern: --check reports them
and --fix replaces them too, so the writer of a summary rewords them. Binary files (a NUL byte) are skipped,
as grep -I does. Each DIR may also be a single file.
"""
import argparse
import os
import re
import sys

# Keep these identical to the acceptance grep alternatives.
PATTERNS = [
    ("tty", r"/dev/tty(USB|ACM)", "<tty>"),
    ("addr", r"([0-9]{1,3}\.){3}[0-9]{1,3}(:[0-9]+)?", "<addr>"),
    ("login", r"[A-Za-z0-9._-]+@[A-Za-z0-9-]+\.[A-Za-z]", "<login>"),
]
COMPILED = [(n, re.compile(p.encode()), r.encode()) for n, p, r in PATTERNS]
# --fix extends the hit to the whole token (device number, full host name) so no tail is left behind.
FIX = [
    (re.compile(rb"/dev/tty(USB|ACM)[0-9]*"), b"<tty>"),
    (COMPILED[1][1], b"<addr>"),
    (re.compile(rb"[A-Za-z0-9._-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*"), b"<login>"),
]


def files(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, dirs, names in os.walk(p):
                dirs.sort()
                for n in sorted(names):
                    yield os.path.join(root, n)
        elif os.path.isfile(p):
            yield p
        else:
            print(f"redact: no such path: {p}", file=sys.stderr)
            sys.exit(2)


def read(path):
    with open(path, "rb") as f:
        data = f.read()
    return None if b"\0" in data else data


def check(paths):
    nfiles = nhits = 0
    for path in files(paths):
        data = read(path)
        if data is None:
            continue
        nfiles += 1
        for ln, line in enumerate(data.split(b"\n"), 1):
            for name, rx, _ in COMPILED:
                if rx.search(line):
                    print(f"{path}:{ln}: {name}")
                    nhits += 1
    if nhits:
        print(f"redact: {nhits} hit(s) in {nfiles} files", file=sys.stderr)
        return 1
    print(f"redact: {nfiles} files clean")
    return 0


def fix(paths):
    nfiles = nchanged = 0
    for path in files(paths):
        data = read(path)
        if data is None:
            continue
        nfiles += 1
        out = data
        for rx, rep in FIX:
            out = rx.sub(rep, out)
        if out != data:
            with open(path, "wb") as f:
                f.write(out)
            nchanged += 1
            print(f"redact: rewrote {path}")
    print(f"redact: {nfiles} files, {nchanged} rewritten")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--check", action="store_true")
    g.add_argument("--fix", action="store_true")
    ap.add_argument("dirs", nargs="+", metavar="DIR")
    a = ap.parse_args()
    return check(a.dirs) if a.check else fix(a.dirs)


if __name__ == "__main__":
    sys.exit(main())
