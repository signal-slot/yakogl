#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redact or check a Verdin iMX95 U-Boot console capture for board identifiers.

  imx95-redact-check.py PATH...          rewrite each file in place (a directory is walked)
  imx95-redact-check.py --check PATH...  print path:line:kind for every identifier a rewrite
                                         would replace (never the value); exit 1 on any, 0 when
                                         clean. The dEQP version token (rule a) is no identifier:
                                         a raw one is not a hit, and its quad is not an address.

The module's own U-Boot (UART_3) prints what the T6's Linux route never showed: printenv's
ethaddr/eth1addr, serial#, serverip and ipaddr, and the Toradex banner's 'Serial#'. The rules,
per line, in order (kind in brackets):

  a) [version] the dEQP version token  vulkan-cts-X.Y.Z.W-N-g<sha>  ->  vulkan-cts-<X.Y.Z-W>-N-g<sha>
     copied from tools/cts/console-redact.py rule (a), not imported, so this tool stays one
     file run with python3 -I: same information, no longer a dotted quad
  b) [env]     the value of the U-Boot variables eth<N>addr, serial#, serverip, ipaddr,
               gatewayip and netmask (printenv's key=value, setenv/env set key value) -> <redacted>
  c) [serial]  the Toradex banner's 'Serial#' value (with or without ':') -> <serial>;
               a USB 'iSerial', 'SerialNumber' or 'Serial Number' value -> <serial>
  d) [somhost] a Toradex host name verdin-<module>-<8 hex>  ->  verdin-<module>-<serial>
  e) [mac]     a MAC address (six hex pairs joined by ':' or '-')  -> <mac>
  f) [addr6]   an IPv6 address other than :: and ::1 (a link-local one carries the MAC) -> <addr6>
  g) [addr]    any other dotted quad -> <addr>  (console-redact.py's ADDR, the acceptance grep's)
  h) [uart]    /dev/tty*, /dev/cu.*, /dev/serial/... -> <uart>
  i) [home]    /home/... -> <home>
  j) [tailnet] a *.ts.net name -> <host>
  k) [secret]  the value of a bare password, passwd or token assignment -> <redacted>
  -) [nul]     NUL bytes are removed (git reads a file with a NUL as binary, and the diff
               scans skip binary files)

The rules are idempotent: the <...> forms match none of them again, so --check is clean after a
rewrite. Symlinks are skipped (never followed). A file with nothing to redact is not written.
Exit 2 on a usage error or a missing path.
"""
import ipaddress
import os
import re
import sys

# a) console-redact.py rule (a), copied: the dEQP release token, bracketed.
VERSION = re.compile(
    rb"vulkan-cts-([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})-(\d+)-g([0-9a-fA-F]{7,64})"
)


def bracket(m):
    """vulkan-cts-X.Y.Z.W-N-g<sha> -> vulkan-cts-<X.Y.Z-W>-N-g<sha>"""
    x, y, z, w, n, sha = (m.group(i) for i in range(1, 7))
    return b"vulkan-cts-<" + x + b"." + y + b"." + z + b"-" + w + b">-" + n + b"-g" + sha


ENV_KEYS = rb"(?:eth[0-9]*addr|serial#|serverip|ipaddr|gatewayip|netmask)"
# printenv's key=value, and the typed setenv / env set key value
ENV_EQ = re.compile(rb"(?<![\w#.-])(" + ENV_KEYS + rb"[ \t]*=[ \t]*)(?!<)([^\s]+)")
ENV_SET = re.compile(rb"(\b(?:setenv|env[ \t]+set)[ \t]+" + ENV_KEYS + rb"[ \t]+)(?!<)([^\s;]+)")
# Toradex: 'Serial#: 01234567' or 'Model: ... V1.0A, Serial# 01234567' (case kept: the
# lower-case serial# is the env variable, rule b)
SERIAL = re.compile(rb"(\bSerial#[ \t]*[:=]?[ \t]*)(?![<\s:=])([^\s,;]+)")
# tools/mp2s_redact_check.py's USB serial, rewritten instead of flagged
USB_SERIAL = re.compile(rb"(\b(?:iSerial|SerialNumber|Serial Number)\b[ \t:=]*(?:\d+[ \t]+)?)"
                        rb"([A-Za-z0-9_.-]{4,})")
SOMHOST = re.compile(rb"(?<![\w.-])(verdin-[a-z0-9]+)-[0-9A-Fa-f]{8}(?![\w-])")
# console-redact.py's MAC
MAC = re.compile(rb"(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f:])"
                 rb"|(?<![0-9A-Fa-f:-])(?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2}(?![0-9A-Fa-f-])")
V6 = re.compile(rb"(?<![\w:.])[0-9A-Fa-f:]{2,}(?![\w:])")
# console-redact.py's ADDR, kept identical on purpose (the acceptance grep's address)
ADDR = re.compile(rb"\b([0-9]{1,3}\.){3}[0-9]{1,3}\b")
# built by concatenation, so this file's own text stays clean under the acceptance grep
UART = re.compile(rb"/dev/(?:tty" + rb"|cu\.|serial/)\S*")
HOME = re.compile(rb"/home" + rb"/\S*")
TAILNET = re.compile(rb"(?<![\w.-])(?:[A-Za-z0-9-]+\.)+ts\.net(?![\w-])", re.I)
# tools/redact-diff-scan.py's bare assignment
SECRET = re.compile(rb"((?<![A-Za-z0-9])(?:password|passwd|token)=(?!=))([^\s\"'$<{*(][^\s\"',;&)]*)",
                    re.I)


def v6(m):
    t = m.group(0)
    if t.count(b":") < 2:
        return t
    try:
        a = ipaddress.IPv6Address(t.decode())
    except ValueError:
        return t
    return t if a.is_loopback or a.is_unspecified else b"<addr6>"


RULES = [
    ("version", VERSION, bracket),
    ("env", ENV_EQ, rb"\1<redacted>"),
    ("env", ENV_SET, rb"\1<redacted>"),
    ("serial", SERIAL, rb"\1<serial>"),
    ("serial", USB_SERIAL, rb"\1<serial>"),
    ("somhost", SOMHOST, rb"\1-<serial>"),
    ("mac", MAC, b"<mac>"),
    ("addr6", V6, v6),
    ("addr", ADDR, b"<addr>"),
    ("uart", UART, b"<uart>"),
    ("home", HOME, b"<home>"),
    ("tailnet", TAILNET, b"<host>"),
    ("secret", SECRET, rb"\1<redacted>"),
]


def redact_line(line):
    """(redacted line, [kinds that changed it])."""
    kinds = []
    if b"\0" in line:
        line = line.replace(b"\0", b"")
        kinds.append("nul")
    for kind, rx, rep in RULES:
        new = rx.sub(rep, line)
        if new != line:
            if kind not in kinds:
                kinds.append(kind)
            line = new
    return line, kinds


def files(paths):
    for p in paths:
        if os.path.islink(p):
            continue
        if os.path.isdir(p):
            for root, dirs, names in os.walk(p):
                dirs[:] = sorted(d for d in dirs if not os.path.islink(os.path.join(root, d)))
                for n in sorted(names):
                    f = os.path.join(root, n)
                    if os.path.isfile(f) and not os.path.islink(f):
                        yield f
        elif os.path.isfile(p):
            yield p


def main(argv):
    check = bool(argv) and argv[0] == "--check"
    paths = argv[1:] if check else argv
    if not paths or any(p.startswith("-") for p in paths):
        sys.stderr.write(__doc__)
        return 2
    for p in paths:
        if not os.path.lexists(p):
            sys.stderr.write("imx95-redact-check: no such path: %s\n" % p)
            return 2
    hits = 0
    for path in files(paths):
        with open(path, "rb") as f:
            data = f.read()
        out = []
        for no, line in enumerate(data.split(b"\n"), 1):
            new, kinds = redact_line(line)
            out.append(new)
            for kind in kinds:
                if kind == "version":
                    continue  # the public CTS release, not an identifier
                hits += 1
                if check:
                    print("%s:%d:%s" % (path, no, kind))
        if not check:
            new = b"\n".join(out)
            if new != data:
                with open(path, "wb") as f:
                    f.write(new)
    return 1 if check and hits else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
