#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Scan the added lines of a git revision range for private values.

    tools/redact-diff-scan.py [RANGE]      default origin/main..HEAD

Flags, on added lines only: IPv4 addresses outside the documentation ranges
(RFC 5737), 0.0.0.0 and 127/8; tailnet 100.64.0.0/10 addresses; *.ts.net
names; /dev/tty(USB|ACM|S)N and /dev/serial/by-id paths; MAC addresses;
password=/token= assignments; ssh private-key headers.
Prints file:line: class: text for each hit and exits 1 on any hit.
A line holding such a pattern on purpose carries the marker
`redact-scan: ok` (in a comment); it is skipped.
Exit 2: git failed.  tools/release/redaction-scan.py scans a whole tree;
this one scans a diff.
"""
import ipaddress
import re
import subprocess
import sys

MARK = "redact-scan: ok"
DOC = [ipaddress.ip_network(n) for n in
       ("192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24", "0.0.0.0/32", "127.0.0.0/8")]
TAILNET = ipaddress.ip_network("100.64.0.0/10")
IPV4 = re.compile(r"(?<![\w.])((?:25[0-5]|2[0-4]\d|1?\d?\d)(?:\.(?:25[0-5]|2[0-4]\d|1?\d?\d)){3})(?!\w|\.\d)")  # redact-scan: ok
RULES = [  # redact-scan: ok
    ("tailnet-name", re.compile(r"[A-Za-z0-9-]+\.ts\.net\b")),  # redact-scan: ok
    ("uart-path", re.compile(r"/dev/(?:tty(?:USB|ACM|S)\d+|serial/by-id/\S+)")),  # redact-scan: ok
    ("mac", re.compile(r"(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f:-])")),  # redact-scan: ok
    ("credential", re.compile(r"(?i)\b(?:password|passwd|token|secret|api_key)\s*[:=]\s*(?!=)['\"]?[^\s'\"$<{(]{3,}")),  # redact-scan: ok
    ("private-key", re.compile(r"-----BEGIN (?:OPENSSH |RSA |EC |DSA )?PRIVATE KEY-----")),  # redact-scan: ok
]


def classify(line):
    hits = []
    for m in IPV4.finditer(line):
        try:
            ip = ipaddress.ip_address(m.group(1))
        except ValueError:
            continue
        if any(ip in n for n in DOC):
            continue
        hits.append("tailnet-ipv4" if ip in TAILNET else "ipv4")
    for name, rx in RULES:
        if rx.search(line):
            hits.append(name)
    return hits


def added_lines(rng):
    out = subprocess.run(["git", "diff", "--no-color", "--no-ext-diff", "-U0", "--no-renames", rng],
                         capture_output=True, text=True, errors="replace")
    if out.returncode:
        sys.stderr.write(out.stderr)
        sys.exit(2)
    path, ln = None, 0
    for raw in out.stdout.splitlines():
        if raw.startswith("+++ "):
            path = raw[4:]
            path = None if path == "/dev/null" else path[2:] if path.startswith("b/") else path
        elif raw.startswith("@@"):
            m = re.match(r"@@ -\S+ \+(\d+)", raw)
            ln = int(m.group(1))
        elif raw.startswith("+") and path:
            yield path, ln, raw[1:]
            ln += 1


def main(argv):
    rng = argv[1] if len(argv) > 1 else "origin/main..HEAD"
    if rng in ("-h", "--help"):
        print(__doc__)
        return 0
    bad = 0
    for path, ln, text in added_lines(rng):
        if MARK in text:
            continue
        for cls in classify(text):
            bad += 1
            print(f"{path}:{ln}: {cls}: {text.strip()[:120]}")
    if bad:
        print(f"redact-diff-scan: {bad} hit(s) in {rng}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
