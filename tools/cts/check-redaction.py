#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redaction check of a plan's diff (plan #388, c7).

    tools/cts/check-redaction.py [RANGE]          (default origin/main...HEAD)
    tools/cts/check-redaction.py --files F...     (every line of the files)

It is tools/redact-diff-scan.py (imported, not copied: IPv4 outside the
documentation ranges, tailnet 100.64/10 and *.ts.net, /dev/tty* and
/dev/serial paths, MAC addresses, password/token literals, private-key
headers, the `redact-scan: ok` marker, the RANGE rules, the exit codes)
plus the classes that scanner lacks:

    ipv6         an IPv6 address (three or more colons, or a link-local prefix)
                 other than ::, ::1 and 2001:db8::/32
    usb-serial   a USB serial string: ID_SERIAL / ID_SERIAL_SHORT /
                 ATTRS{serial} / iSerial / SerialNumber with a literal value,
                 or a usb-<vendor>_<product>_<serial>-ifNN link name
    ssh-host     ssh/scp/sftp/rsync user@host where host is a literal name
                 or address. Placeholders pass: $VAR, ${VAR}, $(cmd), <name>,
                 example.invalid, localhost, the documentation ranges, the
                 words board and host, and github.com (the public remote).
                 Board variables in the repo: BOARD_HOST, BOARD_UART,
                 PI4_HOST, PI4_UART, PI5_HOST, PI5_UART, T6_UART.

Output `path:line: class: masked`, exit 0 clean, 1 hits, 2 unreadable.
tools/cts/check-redaction-selftest.sh checks it.
"""
import importlib.util
import ipaddress
import os
import re
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    "redact_diff_scan", os.path.join(_HERE, "..", "redact-diff-scan.py"))
base = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(base)

RE_V6 = re.compile(r"(?<![\w:.])(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}(?![\w:.])")
RE_USB = re.compile(
    r"(?:ID_SERIAL(?:_SHORT)?\s*=|ATTRS\{serial\}\s*==?|iSerial\s+\d+|"
    r"SerialNumber\s*[=:])\s*[\"']?([^\s\"'$<{*][^\s\"']{5,})", re.I)
RE_USBLINK = re.compile(r"\busb-[A-Za-z0-9]+_[A-Za-z0-9_.-]+-if\d{2}\b")
RE_SSHCMD = re.compile(r"\b(?:ssh|scp|sftp|rsync)\b")
RE_USERHOST = re.compile(r"(?<![\w.$-])([A-Za-z_][\w-]*)@([^\s'\"`;|&)>]+)")
PLACEHOLDER_HOSTS = {"board", "host", "localhost", "github.com"}


def v6_ok(text):
    if text.count(":") < 3 and not text.lower().startswith("fe80::"):  # redact-scan: ok
        return False
    try:
        a = ipaddress.IPv6Address(text)
    except ValueError:
        return False
    return not (a.is_unspecified or a.is_loopback
                or a in ipaddress.ip_network("2001:db8::/32"))


def host_is_placeholder(h):
    h = h.rstrip(".,:")
    if h[:1] in "$<{[%" or h.endswith(".invalid") or h in PLACEHOLDER_HOSTS:
        return True
    return base.ipv4_class(h) is None and re.fullmatch(r"[\d.]+", h) is not None


def extra_hits(line):
    hits = []
    for m in RE_V6.finditer(line):
        if v6_ok(m.group(0)):
            hits.append((m.start(), "ipv6", m.group(0)))
    for m in RE_USB.finditer(line):
        hits.append((m.start(1), "usb-serial", m.group(1)))
    for m in RE_USBLINK.finditer(line):
        hits.append((m.start(), "usb-serial", m.group(0)))
    c = RE_SSHCMD.search(line)
    if c:
        for m in RE_USERHOST.finditer(line, c.end()):
            if not host_is_placeholder(m.group(2)):
                hits.append((m.start(2), "ssh-host", m.group(2)))
    return hits


_base_scan = base.scan_line


def scan_line(line):
    if base.MARKER in line:
        return []
    return sorted(_base_scan(line) + extra_hits(line))


base.scan_line = scan_line  # the imported main() then reports the new classes


def scan_files(paths):
    count = 0
    for p in paths:
        try:
            with open(p, encoding="utf-8", errors="replace") as f:
                for n, line in enumerate(f, 1):
                    for _c, cls, text in scan_line(line.rstrip("\n")):
                        print("%s:%d: %s: %s" % (p, n, cls, base.mask(text)))
                        count += 1
        except OSError as e:
            sys.stderr.write("check-redaction: %s\n" % e)
            return 2
    if count:
        sys.stderr.write("check-redaction: %d hit(s)\n" % count)
        return 1
    return 0


def main(argv):
    if len(argv) >= 2 and argv[1] in ("-h", "--help"):
        print(__doc__.strip())
        return 0
    if len(argv) >= 2 and argv[1] == "--files":
        return scan_files(argv[2:])
    if len(argv) > 2:
        print(__doc__.strip())
        return 2
    rng = argv[1] if len(argv) == 2 else "origin/main...HEAD"
    return base.main([argv[0], rng])


if __name__ == "__main__":
    sys.exit(main(sys.argv))
