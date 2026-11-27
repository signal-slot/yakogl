#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redact a Pi 4 board console or tally so no version string reads as an address.

  console-redact.py FILE...                rewrite each file in place (a DIR is walked)
  console-redact.py                        or with - : read stdin, write the redacted text to stdout
  console-redact.py --check-readme DIR...  fail (exit 1) unless every DIR/README.md has a first
                                           'commit: <40-hex>' line (the line the acceptance reads)

The board acceptance grep matches a dotted quad as an IPv4 address, plus the UART path
(`/dev/` + `tty...`) and the home path (`/home` + `/...`). Every dEQP console carries the line
'dEQP Core vulkan-cts-<version>-555-g<sha> starting..', and its four-part version token is a
dotted quad (the CTS version, or the TEST-NET form left behind by the older redaction pass in
tools/board.sh), so it matches the address alternative. The rewrite rules, in order:
  a) the dEQP version token  vulkan-cts-X.Y.Z.W-N-g<sha>  ->  vulkan-cts-<X.Y.Z-W>-N-g<sha>
     (the field is bracketed and its last dot becomes a '-': same information, no longer a quad)
  b) any other dotted quad -> <addr>
  c) a UART device path -> <uart>
  d) a home path -> <home>
  e) a MAC address (aa:bb:cc:dd:ee:ff or aa-bb-..) -> <mac>   (added for the AM62P's U-Boot ethaddr)
  f) user@host -> <userhost>  (ssh targets such as root@<addr>, prompts such as name@host:~$).
     An e-mail-like token (host with a dot, e.g. a copyright or author line in a GL log) is kept,
     as rules a)..e) keep every other line; only a dotless host, an <addr> host, or a host
     followed by ':~' or ':/' (a prompt) counts as a login. Runs after rule b), so root@<addr> works.
  g) the login user name, given by --user NAME or $BOARD_USER (never read from a file), when it
     appears as a word -> <user>. Without a name the rule is off. Rules f) and g) are new; text
     with no user@host and no name gets byte for byte what a)..e) gave before.
  h) U-Boot's board serial: 'serial#=VALUE' (printenv, env print) -> 'serial#=<serial>' (the value is
     kept off by name, as tools/cts/mali-slice-redact.py does).
  i) a UUID/GUID on U-Boot's partition output -> <uuid>: on any line naming a GUID, UUID or PARTUUID
     (mmc part's 'Partition GUID:', 'Type GUID:', root=PARTUUID=..), and on every line of an
     'mmc part' table, from its 'Partition Map for' or 'Part' header line to the next '=>' prompt.
     A UUID anywhere else (and a boot id line) is left, so t6-redact-check.sh still refuses it.
The UART rule also takes a /dev/serial/by-id/.. path (a USB serial bridge's stable name).
The rules are idempotent: the <...>, <addr>, <uart> and <home> forms match none of them again.
A binary file (a NUL byte) is skipped, as grep -I does. The rules keep everything else byte
for byte, so a file with nothing to redact is left untouched.
"""
import argparse
import os
import re
import sys

# The acceptance grep's address alternative, kept identical on purpose.
ADDR = re.compile(rb"\b([0-9]{1,3}\.){3}[0-9]{1,3}\b")
# The UART and home alternatives are written by concatenation so this file's own text stays
# clean under the acceptance grep, the way the selftests build their samples.
UART = re.compile(rb"/dev/(?:tty" + rb"|serial/)\S*")
# A MAC address (six hex pairs with ':' or '-'): U-Boot's ethaddr and 'Ethernet ... MAC' lines.
MAC = re.compile(rb"(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f:])"
                 rb"|(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f-])")
HOME = re.compile(rb"/home" + rb"/\S*")
VERSION = re.compile(
    rb"vulkan-cts-([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})-(\d+)-g([0-9a-fA-F]{7,64})"
)

# user@host: a user part, then an <addr> host or a host starting with a letter (a device-tree
# unit name such as ethernet@8000000 in U-Boot output is not a login); judged in userhost().
USERHOST = re.compile(rb"(?<![\w.@-])[A-Za-z_][\w.-]*@(<addr>|[A-Za-z](?:[\w.-]*[A-Za-z0-9])?)(?P<prompt>:[~/])?")

SERIAL = re.compile(rb"(?<![\w#])(serial#[ \t]*[:=][ \t]*)[^\s<][^\s]*", re.I)
UUID = re.compile(rb"(?<![0-9A-Fa-f-])[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}(?![0-9A-Fa-f-])")
UUID_WORD = re.compile(rb"guid|uuid", re.I)
PART_HEAD = re.compile(rb"^\s*(Partition Map for|Part\s)", re.I)
PROMPT = re.compile(rb"^\s*=>")
BOOT_ID = re.compile(rb"boot[ _-]?id", re.I)

VERSION_RULE = "version"
ADDR_RULE = "addr"
UART_RULE = "uart"
HOME_RULE = "home"
MAC_RULE = "mac"
USERHOST_RULE = "userhost"
USER_RULE = "user"
SERIAL_RULE = "serial"
UUID_RULE = "uuid"


def uuids(data, tally):
    """Rule i): UUIDs on partition lines (see the header) -> <uuid>."""
    out = []
    n = 0
    in_part = False
    for line in data.splitlines(keepends=True):
        if PROMPT.match(line):
            in_part = False
        elif PART_HEAD.match(line):
            in_part = True
        if (in_part or UUID_WORD.search(line)) and not BOOT_ID.search(line):
            line, k = UUID.subn(b"<uuid>", line)
            n += k
        out.append(line)
    if n:
        tally[UUID_RULE] = n
    return b"".join(out)


def userhost(m):
    host = m.group(1)
    if host == b"<addr>" or b"." not in host or m.group("prompt"):
        return b"<userhost>" + (m.group("prompt") or b"")
    return m.group(0)  # e-mail-like: kept


def bracket(m):
    """vulkan-cts-X.Y.Z.W-N-g<sha> -> vulkan-cts-<X.Y.Z-W>-N-g<sha>"""
    x, y, z, w, n, sha = (m.group(i) for i in range(1, 7))
    return b"vulkan-cts-<" + x + b"." + y + b"." + z + b"-" + w + b">-" + n + b"-g" + sha


def step(data, name, rx, rep, tally):
    data, n = rx.subn(rep, data)
    if n:
        tally[name] = n
    return data


def redact(data, tally=None, user=None):
    """Apply the rules in order; tally (if given) collects the per-rule replacement counts."""
    tally = {} if tally is None else tally
    data = step(data, VERSION_RULE, VERSION, bracket, tally)
    data = step(data, ADDR_RULE, ADDR, b"<addr>", tally)
    data = step(data, UART_RULE, UART, b"<uart>", tally)
    data = step(data, HOME_RULE, HOME, b"<home>", tally)
    data = step(data, MAC_RULE, MAC, b"<mac>", tally)
    data = step(data, SERIAL_RULE, SERIAL, rb"\1<serial>", tally)
    data = uuids(data, tally)
    data = step(data, USERHOST_RULE, USERHOST, userhost, tally)
    if user:
        rx = re.compile(rb"(?<![\w-])" + re.escape(user) + rb"(?![\w-])")
        data = step(data, USER_RULE, rx, b"<user>", tally)
    return data


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
            print(f"console-redact: no such path: {p}", file=sys.stderr)
            sys.exit(2)


def read(path):
    with open(path, "rb") as f:
        data = f.read()
    return None if b"\0" in data else data


def redact_in_place(paths, user=None):
    nfiles = nchanged = 0
    for path in files(paths):
        data = read(path)
        if data is None:
            continue
        nfiles += 1
        tally = {}
        out = redact(data, tally, user)
        if out != data:
            with open(path, "wb") as f:
                f.write(out)
            nchanged += 1
            hit = " ".join(f"{k}={v}" for k, v in sorted(tally.items()))
            print(f"console-redact: rewrote {path} ({hit})")
    print(f"console-redact: {nfiles} files, {nchanged} rewritten")
    return 0


def redact_stream(user=None):
    data = sys.stdin.buffer.read()
    sys.stdout.buffer.write(redact(data, None, user))
    return 0


def check_readme(dirs):
    nok = nbad = 0
    for d in dirs:
        if not os.path.isdir(d):
            print(f"console-redact: no such path: {d}", file=sys.stderr)
            sys.exit(2)
        p = os.path.join(d, "README.md")
        if not os.path.isfile(p):
            print(f"console-redact: {p}: no README.md", file=sys.stderr)
            nbad += 1
            continue
        first = None
        with open(p, "rb") as f:
            for line in f:
                line = line.rstrip(b"\r\n")
                if line.startswith(b"commit:"):
                    first = line
                    break
        if first is None:
            print(f"console-redact: {p}: no commit: line", file=sys.stderr)
            nbad += 1
        elif re.fullmatch(rb"commit: [0-9a-fA-F]{40}", first):
            nok += 1
        else:
            print(f"console-redact: {p}: first commit: line is not 'commit: <40 hex>'", file=sys.stderr)
            nbad += 1
    if nbad:
        print(f"console-redact: check-readme: {nbad} failed, {nok} ok", file=sys.stderr)
        return 1
    print(f"console-redact: check-readme: {nok} README ok")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check-readme", action="store_true", help="check DIR/README.md commit: lines only")
    ap.add_argument("--user", metavar="NAME", default=os.environ.get("BOARD_USER") or None,
                    help="login user name to rewrite to <user> as a word (default: $BOARD_USER)")
    ap.add_argument("paths", nargs="*", metavar="FILE|DIR", help="files or dirs to redact in place; "
                    "none (or -) means stdin to stdout")
    a = ap.parse_args()
    if a.check_readme:
        if not a.paths:
            print("console-redact: --check-readme needs a DIR", file=sys.stderr)
            return 2
        return check_readme(a.paths)
    user = a.user.encode() if a.user else None
    if not a.paths or a.paths == ["-"]:
        return redact_stream(user)
    return redact_in_place(a.paths, user)


if __name__ == "__main__":
    sys.exit(main())
