#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check or scrub committed i.MX8MP logs for board addresses, logins and serial device paths.

  imx8mp-gles3-redact.py --check DIR...   report file:line and the pattern name of every hit (never the
                                          matched text); exit 1 on any hit, else print 'redact: <n> files clean'
  imx8mp-gles3-redact.py --fix DIR...     rewrite the hits in place to <tty>, <addr> and <login>

The first three patterns are those of the plan's acceptance grep (grep -rEIn on tools/cts/log/*-imx8mp-gles3-* and
*-imx8mp-es2-*). Dotted version strings such as 1.2.3.4 also match the address pattern: --check reports them
and --fix replaces them too, so the writer of a summary rewords them.
Armadillo-X2 patterns (EXTRA) also cover /dev/ttymxc paths, container names, host-name prompts, user@host without a dot, *.ts.net, IPv6, MAC addresses, USB serial
numbers, /dev/serial/ paths and rfc2217:// (socket, telnet, ssh, tcp) URLs. Binary files (a NUL byte) are skipped,
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
# Armadillo-X2 additions (the X2 runs Base OS on a tailnet, logs in as root, has a USB serial console and is read
# through /dev/mem). These go beyond the acceptance grep; the three above stay byte-identical to it.
# /tmp/yakogl-kexec, hex record words and image sha256 (64 hex digits, no colons) are not matched.
HEX = r"[0-9A-Fa-f]"
CONT = r"\bpodman\s+(start|stop|restart|kill|rm|logs|exec|inspect)\s+((-[A-Za-z-]+(\s+[0-9]+)?)\s+)*(?![0-9]+\b)[A-Za-z0-9_][A-Za-z0-9_.-]*"
PROMPT = r"^\[?[A-Za-z0-9_][A-Za-z0-9_.-]*:[~/][^\s]*[#$] "
LOGINP = r"^(?!Last\b)[A-Za-z0-9][A-Za-z0-9_.-]* login:"
EXTRA = [
    # a login at a host name without a dot (root at the board's short name)
    ("login", r"(?<![A-Za-z0-9._-])[A-Za-z_][A-Za-z0-9._-]*@[A-Za-z0-9_][A-Za-z0-9_-]*", "<login>"),
    ("tailnet", r"[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*\.ts\.net\b|\bts\.net\b", "<host>"),
    ("ipv6", r"(?<![A-Za-z0-9_:.])(" + HEX + r"{1,4}:){7}" + HEX + r"{1,4}(?![A-Za-z0-9_:])"
             r"|(?<![A-Za-z0-9_:.])(" + HEX + r"{1,4}:){1,6}:(" + HEX + r"{1,4}(:" + HEX + r"{1,4}){0,5})?(?![A-Za-z0-9_:])"
             r"|(?<![A-Za-z0-9_:.])::" + HEX + r"{1,4}(:" + HEX + r"{1,4}){0,5}(?![A-Za-z0-9_:])"
             r"|(?<![A-Za-z0-9_])fe80:[0-9A-Fa-f:]*(%[A-Za-z0-9._-]+)?", "<addr6>"),
    ("mac", r"(?<![0-9A-Za-z:-])(" + HEX + r"{2}[:-]){5}" + HEX + r"{2}(?![0-9A-Za-z:-])", "<mac>"),
    ("tty", r"/dev/serial/|/dev/tty(USB|ACM)|/dev/cu\.(usb|wch)", "<tty>"),
    ("usbserial", r"\busb-[A-Za-z0-9_.-]+-if[0-9]+(-port[0-9]+)?\b"
                  r"|\biSerial\s+[0-9]+\s+[^\s<]"
                  r"|\b(i?SerialNumber|ID_SERIAL(_SHORT)?|serial_?number)\b\s*[:=]?\s*[0-9A-Za-z]", "<serial>"),
    ("url", r"\b(rfc2217|socket|telnet|ssh|tcp)://[^\s\"'<>]+", "<url>"),
    # X2 runs: the i.MX UART device, a container name (undo / UNDO lines, containers_stopped=, app_back) and a
    # host name in a shell or getty prompt. Placeholders such as "podman start <name>" are not matched.
    ("ttymxc", r"/dev/ttymxc[0-9]*", "<tty>"),
    ("container", CONT, "<container>"),
    ("container", r"\bcontainers_stopped=[A-Za-z0-9_][^\s]*|\bapp_back[= ]+(?!not\b)[A-Za-z0-9_][A-Za-z0-9_.-]*", "<container>"),
    ("prompt", PROMPT, "<host>"),
    ("prompt", LOGINP, "<host>"),
]
COMPILED = [(n, re.compile(p.encode()), r.encode()) for n, p, r in PATTERNS + EXTRA]
# --fix extends the hit to the whole token (device number, full host name) so no tail is left behind.
FIX = [
    (re.compile(rb"(rfc2217|socket|telnet|ssh|tcp)://[^\s\"'<>]+"), b"<url>"),
    (re.compile(rb"/dev/serial/[^\s\"']*"), b"<tty>"),
    (re.compile(rb"/dev/tty(USB|ACM)[0-9]*"), b"<tty>"),
    (re.compile(rb"[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*\.ts\.net\b"), b"<host>"),
    (COMPILED[1][1], b"<addr>"),
    (re.compile(rb"[A-Za-z0-9._-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*"), b"<login>"),
    (re.compile(rb"/dev/ttymxc[0-9]*"), b"<tty>"),
    (re.compile(rb"(\bpodman\s+(?:start|stop|restart|kill|rm|logs|exec|inspect)\s+(?:-[A-Za-z-]+(?:\s+[0-9]+)?\s+)*)"
                rb"[A-Za-z0-9_][A-Za-z0-9_.-]*"), rb"\1<container>"),
    (re.compile(rb"(\bcontainers_stopped=)[A-Za-z0-9_][^\s]*"), rb"\1<container>"),
    (re.compile(rb"(\bapp_back[= ]+)(?!not\b)[A-Za-z0-9_][A-Za-z0-9_.-]*"), rb"\1<container>"),
    (re.compile(rb"^(\[?)[A-Za-z0-9_][A-Za-z0-9_.-]*(:[~/][^\s]*[#$] )", re.M), rb"\1<host>\2"),
    (re.compile(rb"^(?!Last\b)[A-Za-z0-9][A-Za-z0-9_.-]* login:", re.M), b"<host> login:"),
    # USB serials: keep the keyword, replace the whole value token (a one-character match left the tail behind)
    (re.compile(rb"\busb-[A-Za-z0-9_.-]+-if[0-9]+(-port[0-9]+)?\b"), b"<serial>"),
    (re.compile(rb"(\biSerial\s+[0-9]+\s+)[^\s<][^\s]*"), rb"\1<serial>"),
    (re.compile(rb"(\b(?:i?SerialNumber|ID_SERIAL(?:_SHORT)?|serial_?number)\b\s*[:=]?\s*)[0-9A-Za-z][^\s]*"), rb"\1<serial>"),
] + [(rx, rep) for n, rx, rep in COMPILED[3:] if n in ("tailnet", "ipv6", "mac")]


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
