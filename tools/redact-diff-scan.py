#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Scan the added lines of a git revision range for values a commit must not hold.

    tools/redact-diff-scan.py [RANGE]      default origin/main..HEAD

Board addresses, host names, device paths and credentials: classes, on added
lines only (a line a range removes leaves the repository, so it is not a
finding):

  tailnet-ipv4   an address in the tailnet's CGNAT range (100.64/10)
  ipv4           any other IPv4 address, except the RFC 5737 documentation
                 ranges, 0.0.0.0 and 127/8; a quad glued into a longer word
                 or a longer dotted run, and a dEQP release version, are
                 not addresses
  tailnet-dns    a tailnet DNS name (the .ts.net suffix)
  som-serial     a host name built from a board's SoM serial, as written in
                 the lab: verdin-(am62p|imx8mp)-<8 hex>; the product's
                 zeroed default is a look-alike, as in the allow file of
                 tools/release/redaction-scan.py
  root-at-host   root@ followed by a host named outright (root@<name> or a
                 root@$variable reference is not)
  bridge-port    a serial-bridge console port (3593x)
  uart-path      a USB adapter's device node: ttyUSB, ttyACM or ttyS with
                 its number after /dev/, or a stable path under
                 /dev/serial/by- (it carries the adapter's serial)
  usb-serial     a value assigned to iSerial, SerialNumber or Serial Number
                 (the kernel's and udev's names for a USB serial number)
  mac            a MAC address
  api-token      a provider key by its shape (a GitHub token, an AWS key id,
                 a Slack token, a Google API key, an OpenAI-style key)
  credential     password, passwd, token, secret or api_key assigned a value
                 (a `==` comparison is not)
  private-key    an ssh or PEM private-key header

Each hit prints one line `path:line: class: text`. Exit status:
  0  the scan ran and found nothing
  1  the scan ran and found at least one hit
  2  the scan could not run (a bad range, extra arguments, git failed)

A line that holds such a pattern on purpose -- a redactor's own rule, a
self-test fixture, a class table -- carries the marker `redact-scan: ok` in
a comment and is skipped. The marker is a review aid, not a hiding place:
the reviewer reads it in the diff.

Private values (lab host names, logins) are never in this file, so the
classes here are public shapes only; tools/release/redaction-scan.py scans
whole trees, reads those values from the environment, and is the gate for
them. This tool covers the one direction the tree scan cannot see: what a
range adds. Like redaction-scan.py, no line of this file matches a class of
this file: ranges of this file scan clean, and the self-test pins that by
committing copies of this file and of itself (its checks named "own").
"""

import re
import subprocess
import sys

MARK = "redact-scan: ok"
DEFAULT_RANGE = "origin/main..HEAD"

# IPv4 with each octet its own group, as tools/release/redaction-scan.py
# reads it: the octets decide the class, so no network literal is a match.
IPV4 = re.compile(r"(?<![\d.])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?!\d|\.\d)")
DOC3 = frozenset({(192, 0, 2), (198, 51, 100), (203, 0, 113)})  # RFC 5737

# A dEQP release in a console banner is a dotted quad too: the line names
# the release family before the version (or right into it), and a git
# describe tail follows. The same reading as redaction-scan.py's; the shape
# itself is pinned in the self-test's fixtures, never written here.
DEQP_BEFORE = re.compile(r"vulkan-cts-$", re.I)
DEQP_AFTER = re.compile(r"-\d+-g[0-9a-f]{7,40}(?![0-9A-Za-z])")
DEQP_LINE = re.compile(r"dEQP|vulkan-cts", re.I)

TOKEN = (
    r"ghp_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{22,}|gho_[A-Za-z0-9]{36}"
    r"|AKIA[0-9A-Z]{16}|xox[abprs]-[A-Za-z0-9-]{10,}|AIza[0-9A-Za-z_-]{35}"
    r"|(?<![A-Za-z0-9_-])sk-[A-Za-z0-9_-]{20,}")

# (class, regex, extra check or None); the extra check keeps a match that is
# a look-alike, exactly as redaction-scan.py's class table reads.
RULES = [
    ("tailnet-dns", re.compile(r"[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.ts\.net"
                               r"(?![A-Za-z0-9-])"), None),
    ("som-serial", re.compile(r"verdin-(?:am62p|imx8mp)-([0-9a-fA-F]{8})"
                              r"(?![0-9A-Za-z])"),
     lambda m: m.group(1) != "0" * 8),
    ("root-at-host", re.compile(r"(?<![A-Za-z0-9._%+-])root@"
                                r"[A-Za-z0-9][A-Za-z0-9._-]*"), None),
    ("bridge-port", re.compile(r"(?<!\d)3593\d(?!\d)"), None),
    ("uart-path", re.compile(r"/dev/(?:tty(?:USB|ACM|S)\d+"
                             r"|serial/by-[a-z]+/\S+)"), None),
    ("usb-serial", re.compile(r"(?i)\b(?:iSerial|SerialNumber|Serial Number)"
                              r"\s*[:=]\s*\S+"), None),
    ("mac", re.compile(r"(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}[:-]){5}"
                       r"[0-9A-Fa-f]{2}(?![0-9A-Fa-f:-])"), None),
    ("api-token", re.compile(TOKEN), None),
    ("credential", re.compile(r"(?i)\b(?:password|passwd|token|secret|api_key)"
                              r"\b\s*[:=]\s*(?!=)['\"]?[^\s'\"$<{(]{3,}"), None),
    ("private-key", re.compile(r"-----BEGIN (?:OPENSSH |RSA |EC |DSA |ENCRYPTED "
                               r"|PGP )?PRIVATE KEY-----"), None),
]


def _ip_class(m):
    octets = [int(m.group(i)) for i in (1, 2, 3, 4)]
    if max(octets) > 255:
        return None
    if DEQP_BEFORE.search(m.string, 0, m.start()) or (
            DEQP_LINE.search(m.string) and DEQP_AFTER.match(m.string, m.end())):
        return None
    a, b, c, d = octets
    if (a, b, c) in DOC3 or a == 127 or (a | b | c | d) == 0:
        return None
    return "tailnet-ipv4" if a == 100 and 64 <= b <= 127 else "ipv4"


def classify(line):
    hits = []
    for m in IPV4.finditer(line):
        cls = _ip_class(m)
        if cls:
            hits.append(cls)
    for name, rx, check in RULES:
        for m in rx.finditer(line):
            if check and not check(m):
                continue
            hits.append(name)
    return hits


def git_diff(rng):
    """The range's diff text; None when git failed (git's own error on stderr)."""
    p = subprocess.run(
        ["git", "-c", "core.quotepath=false", "diff", "--no-color",
         "--no-ext-diff", "--no-renames", "-U0", rng],
        capture_output=True, text=True, errors="replace")
    if p.returncode:
        sys.stderr.write(p.stderr)
        return None
    return p.stdout


def added_lines(diff):
    """(path, line number in the new file, text) of each line the diff adds.

    Hunk headers alone anchor a line: a binary patch carries data without a
    hunk, and its base85 lines are not added lines.
    """
    path, ln, hunk = None, 0, False
    for raw in diff.splitlines():
        if raw.startswith("diff --git"):
            path, ln, hunk = None, 0, False
        elif raw.startswith("+++ "):
            name = raw[4:]
            if name == "/dev/null":
                path = None
            else:
                path = name[2:] if name.startswith("b/") else name
        elif raw.startswith("@@ "):
            m = re.match(r"@@ -\S+ \+(\d+)", raw)
            ln, hunk = (int(m.group(1)), True) if m else (0, False)
        elif hunk and path and raw.startswith("+"):
            yield path, ln, raw[1:]
            ln += 1


def main(argv):
    args = argv[1:]
    if args[:1] in (["-h"], ["--help"]):
        print(__doc__)
        return 0
    if len(args) > 1:
        print("usage: tools/redact-diff-scan.py [RANGE]", file=sys.stderr)
        return 2
    rng = args[0] if args else DEFAULT_RANGE
    diff = git_diff(rng)
    if diff is None:
        return 2
    bad = 0
    for path, ln, text in added_lines(diff):
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
