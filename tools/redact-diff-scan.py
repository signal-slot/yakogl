#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redaction scan of the lines a git revision range adds (plan #388, c7).

    tools/redact-diff-scan.py [RANGE]     (default origin/main..HEAD)

RANGE is A..B or A...B (both read as A...B: what B adds since its merge
base with A, as `git log A..B` counts commits, so a branch behind main is
not charged with main's newer lines), or one revision REV, read as
REV...HEAD. Only the added lines of that diff are read (removed and context
lines are not), so a branch is judged by what it adds, not by what main
already holds. Binary files are skipped.

Classes:

    ipv4         an IPv4 address other than 0.0.0.0, 127.0.0.0/8 and the
                 RFC 5737 documentation ranges 192.0.2.0/24, 198.51.100.0/24
                 and 203.0.113.0/24 (version strings such as 1.4.6.2.555
                 and section numbers are not addresses, nor is a quad with
                 a one-digit first octet such as 4.6.2.14: in this tree
                 that is a spec section; no private range starts below 10)
    tailnet-ip   an address in the tailnet range 100.64/10 (CGNAT)
    tailnet-dns  a *.ts.net name
    tty          /dev/ttyUSB<N>, /dev/ttyACM<N>, /dev/ttyS<N> and the
                 by-id and by-path links under /dev/serial
    mac          a MAC address (six hex pairs joined by ':' or '-'), except
                 the all-zero and broadcast ones
    credential       a password, passwd or token assignment with a literal value
                 (KEY=value, or KEY = "value" / "KEY": "value"); a value that
                 starts with $, <, { or * is a placeholder and not flagged
    private-key  an ssh / PEM private-key header

Each hit is printed as `path:line: class: masked` (the match is shown as its
first character and "***", never in full). Exit 0: no hit; 1: hits; 2: the
range could not be read.

Allowlist: a line that holds such a pattern on purpose (the regexes below,
a test fixture) carries the marker `redact-scan: ok` anywhere on the same
line, usually in a comment; that line is skipped. The marker is for
patterns and made-up values only, never for a real address or name.

tools/redact-diff-scan-selftest.sh checks it on a throwaway repository.
"""
import ipaddress
import re
import subprocess
import sys

MARKER = "redact-scan: ok"
DEFAULT_RANGE = "origin/main..HEAD"

DOC_NETS = [ipaddress.ip_network(n) for n in (
    "192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24", "127.0.0.0/8")]
TAILNET = ipaddress.ip_network(((100 << 24) | (64 << 16), 10))  # 100.64/10

RE_V4 = re.compile(r"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?!\w|\.\d)")
RE_TSNET = re.compile(r"(?<![\w.-])(?:[A-Za-z0-9-]+\.)+ts\.net(?![\w-])", re.I)
RE_TTY = re.compile(r"/dev/(?:tty(?:USB|ACM|S)\d+"
                    r"|serial/by-(?:id|path)/[^\s'\"`,;)]*)")
RE_MAC = re.compile(r"(?<![0-9A-Za-z:-])(?:[0-9A-Fa-f]{2}([:-]))"
                    r"(?:[0-9A-Fa-f]{2}\1){4}[0-9A-Fa-f]{2}(?![0-9A-Za-z:-])")
# KEY=value without spaces (shell, env, URL query, kwargs) ...
RE_SECRET_BARE = re.compile(
    r"(?<![A-Za-z0-9])(?:password|passwd|token)=(?!=)"
    r"([^\s\"'$<{*(][^\s\"',;&)]*)", re.I)
# ... or a quoted literal: KEY = "value", "KEY": "value", KEY: 'value'
RE_SECRET_QUOTED = re.compile(
    r"(?<![A-Za-z0-9])(?:password|passwd|token)[\"']?\s*(?::|=(?!=))\s*"
    r"[\"']([^\"'$<{*][^\"']*)[\"']", re.I)
RE_PRIVKEY = re.compile(
    r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----")  # redact-scan: ok

RE_HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")


def ipv4_class(text):
    try:
        addr = ipaddress.ip_address(text)
    except ValueError:
        return None  # an octet above 255: not an address
    if addr in TAILNET:
        return "tailnet-ip"
    if str(addr) == "0.0.0.0" or any(addr in n for n in DOC_NETS):
        return None
    if len(text.split(".", 1)[0]) == 1:
        return None  # 1-9.x.y.z: a spec section or version (4.6.2.14)
    return "ipv4"


def scan_line(line):
    """[(column, class, match)] for one added line."""
    if MARKER in line:
        return []
    hits = []
    for m in RE_V4.finditer(line):
        cls = ipv4_class(m.group(0))
        if cls:
            hits.append((m.start(), cls, m.group(0)))
    for m in RE_TSNET.finditer(line):
        hits.append((m.start(), "tailnet-dns", m.group(0)))
    for m in RE_TTY.finditer(line):
        hits.append((m.start(), "tty", m.group(0)))
    for m in RE_MAC.finditer(line):
        v = m.group(0).lower().replace("-", ":")
        if v not in ("00:00:00:00:00:00", "ff:ff:ff:ff:ff:ff"):
            hits.append((m.start(), "mac", m.group(0)))
    seen = set()
    for rx in (RE_SECRET_BARE, RE_SECRET_QUOTED):
        for m in rx.finditer(line):
            if m.start(1) not in seen:
                seen.add(m.start(1))
                hits.append((m.start(1), "credential", m.group(1)))
    for m in RE_PRIVKEY.finditer(line):
        hits.append((m.start(), "private-key", m.group(0)))
    hits.sort()
    return hits


def mask(text):
    return text[:1] + "***"


def unquote_path(p):
    # git quotes a path with a quote, backslash or control character in it
    if len(p) >= 2 and p.startswith('"') and p.endswith('"'):
        return p[1:-1]
    return p


def diff_args(rng):
    """A..B, A...B and REV as the one argument A...B of `git diff`."""
    if "..." in rng:
        return [rng]
    if ".." in rng:
        a, b = rng.split("..", 1)
        return ["%s...%s" % (a or "HEAD", b or "HEAD")]
    return [rng + "...HEAD"]


def main(argv):
    if len(argv) > 2 or (len(argv) == 2 and argv[1] in ("-h", "--help")):
        print(__doc__.strip())
        return 0 if len(argv) == 2 else 2
    rng = argv[1] if len(argv) == 2 else DEFAULT_RANGE
    cmd = ["git", "-c", "core.quotepath=off", "diff", "--no-color",
           "--no-ext-diff", "--no-textconv", "--unified=0", "-M",
           *diff_args(rng), "--"]
    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        sys.stderr.write("redact-diff-scan: git diff %s failed: %s\n"
                         % (rng, proc.stderr.decode("utf-8", "replace").strip()))
        return 2
    path = None
    header = False  # between "diff --git" and the file's first hunk
    lineno = 0
    count = 0
    for raw in proc.stdout.decode("utf-8", "replace").split("\n"):
        if raw.startswith("diff --git "):
            path = None
            header = True
            continue
        if header and raw.startswith("+++ "):
            target = raw[4:]
            path = None if target == "/dev/null" else unquote_path(target)
            if path and path.startswith("b/"):
                path = path[2:]
            continue
        m = RE_HUNK.match(raw)
        if m:
            header = False
            lineno = int(m.group(1))
            continue
        if header or path is None:
            continue
        if raw.startswith("+"):
            for _col, cls, text in scan_line(raw[1:]):
                print("%s:%d: %s: %s" % (path, lineno, cls, mask(text)))
                count += 1
            lineno += 1
        elif raw.startswith(" "):
            lineno += 1
    if count:
        sys.stderr.write("redact-diff-scan: %d hit(s) in the lines %s adds\n"
                         % (count, rng))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
