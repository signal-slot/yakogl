#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Redact or check identifiers in Verdin iMX95 U-Boot console captures.

  python3 -I tools/cts/imx95-redact-check.py PATH...           rewrite in place
  python3 -I tools/cts/imx95-redact-check.py --check PATH...   report only

PATH is a file or a directory (every regular file below it, any depth). Rewrite
mode replaces, in place:

  U-Boot env value of ethaddr, eth<n>addr, serial#, serverip, ipaddr,
    gatewayip, netmask             -> <redacted>   ("ethaddr=<redacted>")
  six hex pairs joined by ':'/'-'  -> <mac>
  Toradex banner "Serial#: <v>"    -> <serial>
  dotted quad (four 0-255 octets)  -> <addr>
  /dev/tty...                      -> <uart>
  /home/...                        -> <home>

A dotted quad glued to a word, '.' or '-' on its left ("mesa-24.0.8.1", "v1.2.3.4")
or to a word, '-' or ".digit" on its right is a version, not an address: kept
(the dEQP/driver version tokens). NUL bytes (UART line noise) are removed in
rewrite mode and ignored when checking, so they cannot hide a value; files are
otherwise handled as bytes (latin-1 round trip). Symlinks are skipped in both
modes (a rewrite cannot leave the directory).

--check prints "<file>:<line>:<kind>" per kind and line (never the value) and
exits 1 on any finding, 0 when clean, 2 when a path cannot be read.
Stdlib only.
"""
import os
import re
import sys

OCT = r'(?:25[0-5]|2[0-4]\d|1?\d?\d)'
IPV4 = re.compile(r'(?<![\w.-])' + OCT + r'(?:\.' + OCT + r'){3}(?![\w-]|\.\d)')
MAC = re.compile(r'(?<![\w:-])(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}(?![\w:-])', re.I)
ENV = re.compile(
    r'(?<![\w#])((?:ethaddr|eth\d+addr|serial#|serverip|ipaddr|gatewayip|netmask)=)[^\s]*',
    re.I)
SERIAL = re.compile(r'(?<![\w#])(Serial#[ \t]*:[ \t]*)[^\s]+', re.I)
UART = re.compile(r'/dev/tty[^\s\'"]*')
HOME = re.compile(r'/home/[^\s\'"]*')

# order matters: env values first, so "ethaddr=<mac>" cannot appear.
RULES = [
    ('env', ENV, r'\1<redacted>'),
    ('serial', SERIAL, r'\1<serial>'),
    ('mac', MAC, '<mac>'),
    ('ipv4', IPV4, '<addr>'),
    ('uart', UART, '<uart>'),
    ('home', HOME, '<home>'),
]


def clean(data):
    return data.replace(b'\0', b'').decode('latin-1')


def redact(text):
    for _, rx, rep in RULES:
        text = rx.sub(rep, text)
    return text


def kinds(line):
    return sorted(k for k, rx, rep in RULES if rx.sub(rep, line) != line)


def files(paths, err):
    for p in paths:
        if os.path.islink(p):
            continue
        if os.path.isdir(p):
            for root, dirs, names in os.walk(p):
                dirs.sort()
                for n in sorted(names):
                    f = os.path.join(root, n)
                    if os.path.isfile(f) and not os.path.islink(f):
                        yield f
        elif os.path.isfile(p):
            yield p
        else:
            err.append(p)
            print('imx95-redact-check: cannot read %s' % p, file=sys.stderr)


def main(argv):
    check = False
    if argv and argv[0] == '--check':
        check, argv = True, argv[1:]
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    err = []
    hits = 0
    for f in files(argv, err):
        try:
            with open(f, 'rb') as fh:
                orig = fh.read()
            text = clean(orig)
            if check:
                for i, line in enumerate(text.split('\n'), 1):
                    for k in kinds(line):
                        print('%s:%d:%s' % (f, i, k))
                        hits += 1
            else:
                new = redact(text).encode('latin-1')
                if new != orig:
                    with open(f, 'wb') as fh:
                        fh.write(new)
        except OSError as e:
            err.append(f)
            print('imx95-redact-check: %s: %s' % (f, e.strerror), file=sys.stderr)
    if err:
        return 2
    return 1 if hits else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
