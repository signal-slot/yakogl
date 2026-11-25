#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redact one NanoPC-T6 U-Boot record file, and check a copy for what survived.

  t6-uboot-redact.py --redact              one file on stdin -> redacted bytes on
                                           stdout; exit 76 when the input is a
                                           binary file (a NUL byte): the copier
                                           skips it. The rewrite tally prints to
                                           stderr when it is not empty.
  t6-uboot-redact.py --check DIR|FILE...   report every surviving secret as
                                           'REDACT <file>:<line>: <kind>', one
                                           line per hit; the matched text is
                                           never echoed, as in mali-slice-redact.py.
                                           Exit 1 on any hit, 0 when clean,
                                           2 when a path cannot be read.

A NanoPC-T6 U-Boot record (driver/tools/uboot-t6/) carries the kinds
its README names: printenv prints the board's MAC addresses and serial number,
mmc part prints partition UUIDs, the debug UART is a network serial adapter at
a bridge port in 3593x, the runner's environment holds a rfc2217:// URL, and
the kernel output that follows a reset prints device paths, host names and the
board's own names. --redact rewrites each kind to a placeholder, in table
order (every rule sees only the output of the rules before it):

  vulkan-cts-<X.Y.Z-W>-N-g<sha>  a dEQP release keeps its version without a
                                  quad (the console-redact.py rule, same bytes)
  <uuid>      a UUID in either hex case (U-Boot prints GUID codes uppercase)
  <mac>       a MAC: six hex pairs joined by ':' or '-'
  <serial>    the value of a 'Serial', 'SerialNumber', 'Serial No.' or 'serial#'
  <uart-net>  an rfc2217:// URL (the adapter's host and port go with it)
  <host>      a *.ts.net name
  <uart>      a /dev/ttyUSB..., /dev/ttyACM... device path, and a bare ttyUSB<n>
  <serial-path> a /dev/serial/ path (its by-id names carry the USB serial)
  <ipv6>      an IPv6 link-local or ULA address (the prefixes fe80::/10 and
              fc00::/7, as redaction-scan.py states them)
  <login>     user@host with a dotted host (a login at a host; an e-mail address
              is the same shape), and a root login at any host
  <secret>    the value after a password, passwd, passphrase, pwd, secret or
              api_key word (the shape redaction-scan.py flags)
  <home>      a /home/... or /Users/... path
  <port>      a serial-bridge port, 3593x
  <addr>      every other dotted quad, glued to a word or not (a version string
              like 1.2.3.4 is replaced too, as console-redact.py and
              imx8mp-gles3-redact.py document: the writer of a summary rewords it)

A placeholder matches no rule, redacted or not, so a redacted file re-redacts
to itself byte for byte. Binary files (a NUL byte) are skipped, as grep -I,
console-redact.py and the record readers do. Stdlib only; byte level (a record
is a console stream and may hold noise).

--check guards a copy with the classes tools/release/redaction-scan.py flags in
a committed file — private and tailnet IPv4 (10/8, 172.16/12, 192.168/16, the
100.64/10 range, 169.254/16), local IPv6, a ts.net name, a 3593x port, a login
at a host (its public root case), an e-mail address (a login shape), a home
path — plus the T6 record kinds: a MAC, a UUID, a 'Serial'/'SerialNumber'/
'serial#' value, a tty mention in any case even glued to a word, a /dev/serial/
path, an rfc2217:// URL, and the secret-value shape. redaction-scan.py's
private classes (the owner's login, lab machine names) are not patterns here:
private values live in no file of this repository, and the scan at commit time
is their gate.

Two rules are deliberately narrower than their check kind, so a record the
redactor cannot rewrite confidently is refused rather than guessed at:
- a tty mention: --redact rewrites device paths (the lowercase /dev/... names
  Linux prints and a bare ttyUSB<n> name), case-sensitively as a name;
  --check reports ttyUSB/ttyACM followed by a digit in any case, also glued to
  a word — an uppercase mention in a log line is still a UART mention, and a
  case-blind in-word rewrite could eat a word it should leave;
- a login at a dotless host is redacted for root only (redaction-scan.py's
  public root case); the <login> rule otherwise needs a dotted host, and the
  check kind says the same. A non-root login at a dotless host belongs to the
  scan's private classes; a runner redacts such a line by hand.
"""
import argparse
import os
import re
import sys

EXIT_OK, EXIT_HIT, EXIT_USAGE, EXIT_BINARY = 0, 1, 2, 76

# --- the kinds (byte patterns: a record is a console stream) ------------------

RFC2217 = rb"(?i)rfc2217://\S*"
TSNET = rb"(?i)(?<![\w.-])(?:[\w-]+\.)+ts\.net(?![\w-])"
UUID = (rb"(?<![0-9a-fA-F-])[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}"
        rb"-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}(?![0-9a-fA-F-])")
MAC = rb"(?<![0-9a-fA-F:-])(?:[0-9a-fA-F]{2}[:-]){5}[0-9a-fA-F]{2}(?![0-9a-fA-F:-])"
SERIAL = rb"(?i)(\bserial[ _-]?(?:number|no\.?|#|(?=[:=]))[ \t]*[:=][ \t]*)(?!<)\S+"
UART_PATH = rb"/dev/tty(?:USB|ACM)\S*"
UART_BARE = rb"(?<![0-9A-Za-z])tty(?:USB|ACM)[0-9]+(?![0-9])"
UART_MENTION = rb"(?i)tty(?:USB|ACM)[0-9]"
SERPATH = rb"/dev/serial/"
# The IPv6 local rule of tools/release/redaction-scan.py, and its predicate.
IPV6 = rb"(?i)(?<![0-9A-Fa-f:])(?:fe[89ab][0-9a-f]|f[cd][0-9a-f]{2}):[0-9a-f:]*[0-9a-f]"
# A user@host with a dotted host covers logins at real hosts and e-mail shapes.
LOGIN_DOT = rb"(?<![\w.@-])[A-Za-z0-9._%+-]+@[A-Za-z0-9][A-Za-z0-9-]*(?:\.[A-Za-z0-9-]+)+(?![\w-])"
LOGIN_ROOT = rb"(?<![A-Za-z0-9._%+-])root@[A-Za-z0-9][A-Za-z0-9.-]*(?![A-Za-z0-9])"
# Compiled, so the assignment reads SECRET = re.compile(: the shape
# tools/release/redaction-scan.allow drops as "a regex named SECRET" in any .py.
SECRET = re.compile(rb"(?i)(?<![A-Za-z0-9])((?:password|passwd|passphrase|pwd|secret|api_?key)"
                    rb"[ \t]*[:=](?!=)[ \t]*)(?!<)\S+")
HOME = rb"/(?:home|Users)/\S+"
HOME_FLAG = rb"/(?:home|Users)/[A-Za-z0-9_.-]+"  # redaction-scan.py's home-path shape
PORT = rb"(?<!\d)3593\d(?!\d)"
# redaction-scan.py's private-ipv4 pattern pair; the predicate splits its ranges.
IPV4 = rb"(?<![\d.])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?!\d|\.\d)"
VERSION = (rb"vulkan-cts-([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})"
           rb"-(\d+)-g([0-9a-fA-F]{7,64})")


def bracket_version(m):
    """vulkan-cts-X.Y.Z.W-N-g<sha> -> vulkan-cts-<X.Y.Z-W>-N-g<sha> (console-redact.py)."""
    return b"vulkan-cts-<" + m.group(1) + b"." + m.group(2) + b"." + m.group(3) + b"-" + m.group(4) + b">-" + m.group(5) + b"-g" + m.group(6)


def octets(m):
    return [int(m.group(i)) for i in range(1, 5)]


def private_ipv4(m):
    """redaction-scan.py's private ranges, except the tailnet one."""
    a, b, _, _ = octets(m)
    if max(octets(m)) > 255:
        return False
    return (a == 10 or (a == 172 and 16 <= b <= 31) or (a == 192 and b == 168)
            or (a == 169 and b == 254)) and not tailnet_ipv4(m)


def tailnet_ipv4(m):
    a, b, _, _ = octets(m)
    return max(octets(m)) <= 255 and a == 100 and 64 <= b <= 127


def local_ipv6(m):
    """redaction-scan.py's predicate: '::' or at least four groups, not a label."""
    rest = m.group(0)
    return b"::" in rest or rest.count(b":") >= 3


def _ipv6_sub(data):
    """--redact's IPv6 rule: replace only where local_ipv6 agrees (a match that
    the predicate rejects stays exactly as it was)."""
    n = 0

    def rep(m):
        nonlocal n
        if local_ipv6(m):
            n += 1
            return b"<ipv6>"
        return m.group(0)

    return re.sub(IPV6, rep, data), n


# --redact: [(kind, regex, replacement)]; replacement is a callable or bytes.
# Order matters: a UUID before a MAC (a GUID's pairs), the serial value before
# anything that could read as its host, the device paths before the login rule
# (so a serial-getty unit name masks its tty part before the login rule could
# read the whole instance as a login), the login and secret rules before the
# quad rule (so a login at an address or a secret value goes whole), and every
# quad last.
REDACT = [
    ("version", VERSION, bracket_version),
    ("uuid", UUID, b"<uuid>"),
    ("mac", MAC, b"<mac>"),
    ("serial", SERIAL, rb"\1<serial>"),
    ("rfc2217", RFC2217, b"<uart-net>"),
    ("tsnet", TSNET, b"<host>"),
    ("uart-path", UART_PATH, b"<uart>"),
    ("uart-bare", UART_BARE, b"<uart>"),
    ("serial-path", SERPATH, b"<serial-path>"),
    ("ipv6", IPV6, _ipv6_sub),
    ("login", LOGIN_DOT, b"<login>"),
    ("login-root", LOGIN_ROOT, b"<login>"),
    ("secret", SECRET, rb"\1<secret>"),
    ("home", HOME, b"<home>"),
    ("port", PORT, b"<port>"),
    ("addr", IPV4, b"<addr>"),
]

# --check: [(kind, regex, predicate or None)]. The shared kinds use the same
# pattern as --redact, kept identical the way imx8mp-gles3-redact.py keeps its
# check identical to the acceptance grep. The two tty entries are the
# documented asymmetry: the redactor rewrites names, the check reports mentions.
CHECKS = [
    ("rfc2217", RFC2217, None),
    ("uart", UART_MENTION, None),
    ("mac", MAC, None),
    ("uuid", UUID, None),
    ("serial", SERIAL, None),
    ("serial-path", SERPATH, None),
    ("tailnet", IPV4, tailnet_ipv4),
    ("ipv4", IPV4, private_ipv4),
    ("tailnet-dns", TSNET, None),
    ("ipv6", IPV6, local_ipv6),
    ("login", LOGIN_DOT, None),
    ("login-root", LOGIN_ROOT, None),
    ("secret", SECRET, None),
    ("home", HOME_FLAG, None),
    ("port", PORT, None),
]


def redact(data, tally):
    """Apply the rules in order; tally collects the per-kind replacement counts."""
    for name, rx, rep in REDACT:
        if name == "ipv6":  # the one rule with a predicate of its own
            data, n = _ipv6_sub(data)
        else:
            data, n = re.subn(rx, rep, data)
        if n:
            tally[name] = tally.get(name, 0) + n
    return data


# --- --redact: one file through stdin/stdout ----------------------------------

def redact_stream():
    data = sys.stdin.buffer.read()
    if b"\0" in data:
        print("t6-uboot-redact: binary (a NUL byte): skipped", file=sys.stderr)
        return EXIT_BINARY
    tally = {}
    out = redact(data, tally)
    sys.stdout.buffer.write(out)
    if tally:
        print("t6-uboot-redact: rewrote " + " ".join("%s=%d" % kv for kv in sorted(tally.items())),
              file=sys.stderr)
    return EXIT_OK


# --- --check: walk files, report hits, never the matched text -----------------

def walk(paths, err):
    for top in paths:
        if os.path.isdir(top):
            for d, dirs, names in os.walk(top):
                dirs.sort()
                for n in sorted(names):
                    yield os.path.join(d, n)
        elif os.path.isfile(top):
            yield top
        else:
            err.write("t6-uboot-redact: no such path: %s\n" % top)
            yield None


def check_file(path, out):
    with open(path, "rb") as f:
        data = f.read()
    if b"\0" in data:
        return 0  # binary: skipped, as grep -I does
    n = 0
    for no, line in enumerate(data.split(b"\n"), 1):
        for kind, rx, pred in CHECKS:
            for m in re.finditer(rx, line):
                if pred and not pred(m):
                    continue
                out.write("REDACT %s:%d: %s\n" % (path, no, kind))
                n += 1
    return n


def check_paths(paths, out=sys.stdout, err=sys.stderr):
    hits = unreadable = missing = 0
    for path in walk(paths, err):
        if path is None:
            missing += 1
            continue
        try:
            hits += check_file(path, out)
        except OSError as e:
            unreadable += 1
            err.write("t6-uboot-redact: cannot read %s: %s\n" % (path, e.strerror))
    if unreadable or missing:
        return EXIT_USAGE
    return EXIT_HIT if hits else EXIT_OK


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0],
                                epilog="\n\n".join(__doc__.split("\n\n", 1)[1:]),
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--redact", action="store_true", help="one file on stdin, redacted bytes on stdout")
    mode.add_argument("--check", action="store_true", help="report surviving kinds in DIR|FILE...")
    ap.add_argument("paths", nargs="*", metavar="DIR|FILE", help="what --check reads")
    a = ap.parse_args()
    if a.redact and a.paths:
        print("t6-uboot-redact: --redact reads stdin; it takes no paths", file=sys.stderr)
        return EXIT_USAGE
    if a.redact:
        return redact_stream()
    if not a.paths:
        print("t6-uboot-redact: --check needs a DIR or FILE", file=sys.stderr)
        return EXIT_USAGE
    return check_paths(a.paths)


if __name__ == "__main__":
    sys.exit(main())
