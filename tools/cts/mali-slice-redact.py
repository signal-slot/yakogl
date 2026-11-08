#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check NanoPC-T6 dEQP-GLES2 slices for addresses, host names and serials.

  python3 -I tools/cts/mali-slice-redact.py SLICE...
  python3 -I tools/cts/mali-slice-redact.py --self-test

A slice is tools/cts/log/<date>-t6-gles2-<session>-<sha8>/ (layout in the
docstring of mali-deqp-tally.py). Every file under each SLICE (any depth,
README.md included) is scanned line by line; board addresses, host names, UART
paths and USB serial numbers never go in a file or a commit. A hit prints

  REDACT <file>:<line>: <kind>

(once per kind and line; the matched text is never echoed, so the report does
not repeat what it found). Exit 0: clean. Exit 1: at least one hit. Exit 2: a
path cannot be read (every unreadable file is named; the rest is still scanned).

Kinds:
  ipv4        a dotted quad of four 0-255 octets, wherever it stands (also
              after "scheme://" or in "addr:port"). Not reported: 0.0.0.0,
              127.x.x.x, and a quad glued to a word, '.' or '-' on its left
              ("mesa-24.0.8.1", "v1.2.3.4") or to a word/'-'/".digit" on its
              right (a version or a longer dotted number)
  tailnet     a *.ts.net name, or an address in 100.64.0.0/10 (Tailscale)
  ipv6        an address with "::" or all eight groups ("fe80::1", "2001:db8::2")
  mac         six hex pairs joined by ':' or '-'
  userhost    user@host (also an e-mail address, and unit instances such as
              serial-getty@ttyS2.service: failing safe, rename the unit)
  serial-path a /dev/serial/ path (by-id carries the USB serial)
  uart        ttyUSB<n> or ttyACM<n>
  usb-serial  a "SerialNumber: <value>" line as the kernel logs a USB device

A file that is not UTF-8 is scanned as latin-1. Stdlib only.
"""
import os
import re
import sys

OCT = r'(?:25[0-5]|2[0-4]\d|1?\d?\d)'
IPV4 = re.compile(r'(?<![\w.-])(' + OCT + r'(?:\.' + OCT + r'){3})(?![\w-]|\.\d)')
TSNET = re.compile(r'(?<![\w.-])(?:[\w-]+\.)+ts\.net(?![\w-])', re.I)
H = r'[0-9a-f]{1,4}'
IPV6 = re.compile(
    r'(?<![\w:.])(?:(?:' + H + r':){7}' + H +
    r'|(?:' + H + r':){0,7}' + r'[0-9a-f]{0,4}::(?:' + H + r':){0,6}[0-9a-f]{0,4})'
    r'(?![\w:])', re.I)
MAC = re.compile(r'(?<![\w:-])(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}(?![\w:-])', re.I)
USERHOST = re.compile(r'(?<![\w.@-])[\w][\w.+-]*@[\w][\w-]*(?:\.[\w-]+)*')
SERIAL_PATH = re.compile(r'/dev/serial/')
UART = re.compile(r'tty(?:USB|ACM)\d', re.I)
USB_SERIAL = re.compile(r'SerialNumber[:=]\s*\S', re.I)


def kinds(line):
    """The sorted set of kinds found in one line."""
    found = set()
    for m in IPV4.finditer(line):
        a = [int(x) for x in m.group(1).split('.')]
        if a == [0, 0, 0, 0] or a[0] == 127:
            continue
        found.add('tailnet' if a[0] == 100 and 64 <= a[1] <= 127 else 'ipv4')
    if TSNET.search(line):
        found.add('tailnet')
    for m in IPV6.finditer(line):
        if re.search(r'[0-9a-f]', m.group(0), re.I):
            found.add('ipv6')
    if MAC.search(line):
        found.add('mac')
    if USERHOST.search(line):
        found.add('userhost')
    if SERIAL_PATH.search(line):
        found.add('serial-path')
    if UART.search(line):
        found.add('uart')
    if USB_SERIAL.search(line):
        found.add('usb-serial')
    return sorted(found)


def scan_file(path, out):
    """Print the hits of one file; return the number of hits."""
    with open(path, 'rb') as f:
        data = f.read()
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        text = data.decode('latin-1')
    n = 0
    for no, line in enumerate(text.splitlines(), 1):
        for k in kinds(line):
            out.write('REDACT %s:%d: %s\n' % (path, no, k))
            n += 1
    return n


def scan_paths(paths, out=sys.stdout, err=sys.stderr):
    """Scan the slices; return the exit code."""
    hits = 0
    unreadable = 0

    def fail(path, e):
        nonlocal unreadable
        unreadable += 1
        err.write('mali-slice-redact: cannot read %s: %s\n' % (path, e.strerror))

    for top in paths:
        if os.path.isdir(top):
            files = []
            for d, dirs, names in os.walk(top, onerror=lambda e: fail(e.filename, e)):
                dirs.sort()
                files += [os.path.join(d, n) for n in sorted(names)]
        else:
            files = [top]
        for p in files:
            try:
                hits += scan_file(p, out)
            except OSError as e:
                fail(p, e)
    if unreadable:
        return 2
    return 1 if hits else 0


# All addresses are from documentation ranges (RFC 5737, 3849, MAC 00:00:5e:00:53:xx).
CASES = [
    ('plain kernel line: panthor fff00000.gpu: GPU reset', []),
    ('mesa-24.0.8.1 and v1.2.3.4 and 1.2.3.4.5', []),
    ('loopback 127.0.0.1 and 0.0.0.0', []),
    ('not octets 300.1.2.3 and 1.2.3', []),
    ('timestamps 12:34:56 and std::string', []),
    ('link up at 192.0.2.7', ['ipv4']),
    ('fetch http://192.0.2.7:8000/x', ['ipv4']),
    ('ssh://192.0.2.7', ['ipv4']),
    ('tftp://192.0.2.7/boot, then done.', ['ipv4']),
    ('addr=192.0.2.7.', ['ipv4']),
    ('peer 100.64.0.9', ['tailnet']),
    ('host board-1.example.ts.net up', ['tailnet']),
    ('inet6 2001:db8::2/64', ['ipv6']),
    ('addr fe80::1%eth0', ['ipv6']),
    ('addr 2001:db8:0:0:0:0:0:2', ['ipv6']),
    ('eth0: 00:00:5e:00:53:01', ['mac']),
    ('eth0: 00-00-5e-00-53-01', ['mac']),
    ('prompt root@board:~#', ['userhost']),
    ('ls /dev/serial/by-id/usb-X_Y-if00-port0', ['serial-path']),
    ('FTDI converter now attached to ttyUSB0', ['uart']),
    ('console /dev/ttyACM1', ['uart']),
    ('usb 1-1: SerialNumber: A12345', ['usb-serial']),
    ('ssh root@192.0.2.7 via 100.64.0.9', ['ipv4', 'tailnet', 'userhost']),
]


def self_test():
    import io
    import tempfile
    bad = 0
    for line, want in CASES:
        got = kinds(line)
        if got != want:
            bad += 1
            print('FAIL kinds(%r) = %s, want %s' % (line, got, want))
    with tempfile.TemporaryDirectory() as d:
        os.mkdir(os.path.join(d, 'kmsg'))
        for name, body in (('meta.txt', 'rows=1-2\n'), ('kmsg/a.txt', 'ok\nlink 192.0.2.7\n')):
            with open(os.path.join(d, name), 'w') as f:
                f.write(body)
        out, err = io.StringIO(), io.StringIO()
        rc = scan_paths([d], out, err)
        if rc != 1 or out.getvalue() != 'REDACT %s:2: ipv4\n' % os.path.join(d, 'kmsg', 'a.txt'):
            bad += 1
            print('FAIL dirty slice: rc=%d out=%r' % (rc, out.getvalue()))
        if '192.0.2.7' in out.getvalue():
            bad += 1
            print('FAIL a match was echoed')
        if scan_paths([os.path.join(d, 'meta.txt')], io.StringIO(), io.StringIO()) != 0:
            bad += 1
            print('FAIL clean file reported')
        if scan_paths([os.path.join(d, 'missing')], io.StringIO(), io.StringIO()) != 2:
            bad += 1
            print('FAIL a missing path is not exit 2')
    print('mali-slice-redact self-test: %s' % ('ok' if not bad else '%d failed' % bad))
    return 1 if bad else 0


def main(argv):
    if argv == ['--self-test']:
        return self_test()
    if not argv or argv[0].startswith('-'):
        sys.stderr.write(__doc__.split('\n\n')[0] + '\n')
        return 2
    return scan_paths(argv)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
