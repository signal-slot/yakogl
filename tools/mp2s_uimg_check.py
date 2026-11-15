#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check a staged legacy U-Boot script image (boot.scr.uimg) before any reboot.

    tools/mp2s_uimg_check.py IMAGE [--expect FILE.cmd] [--sha256 FILE]

A corrupt script on bootfs is a stranding risk (docs/notes/
stm32mp257f-dk-standalone.md, section 8). The product script is built with
`mkimage -A arm64 -O linux -T script -C none -d mp2-standalone.cmd`
(section 3, "What loads it"); this reads the result without mkimage.

Legacy image_header_t, 64 bytes, big-endian: magic 0x27051956, header CRC32
(computed with the hcrc field zeroed), time, data size, load, entry point,
data CRC32, os, arch, type, comp (one byte each), name (32 bytes). Checks:

  magic, hcrc, size (the file holds the header and the data), dcrc,
  os (linux, 5), arch (arm64, 22), type (script, 6), comp (none, 0),
  payload: a list of 4-byte big-endian lengths ended by 0, then the bodies
  back to back. U-Boot's `source` runs the first body; mkimage writes
  exactly one and no padding, and trailing zero bytes after the last body
  are accepted,
  expect (with --expect): the first body is byte-identical to FILE,
  sha256 (with --sha256 FILE): FILE holds a line '<hex>  <name>' and <hex>
  is the sha256 of IMAGE.

Output: one line per check, "mp2s-uimg: ok <check>" or
"mp2s-uimg: FAIL <check>: <why>". A check that cannot run because an earlier
one failed prints FAIL too. Exit 0 when every check passes, 1 when any
fails, 2 on a usage error (an unreadable file included).
"""
import argparse
import hashlib
import struct
import sys
import zlib

MAGIC = 0x27051956
HDR = 64
OS_LINUX, ARCH_ARM64, TYPE_SCRIPT, COMP_NONE = 5, 22, 6, 0

results = []


def ok(check):
    results.append(True)
    print("mp2s-uimg: ok %s" % check)


def fail(check, why):
    results.append(False)
    print("mp2s-uimg: FAIL %s: %s" % (check, why))


def read(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError as e:
        print("mp2s-uimg: usage: %s" % e, file=sys.stderr)
        sys.exit(2)


def crc(b):
    return zlib.crc32(b) & 0xFFFFFFFF


def parse_payload(data):
    """Return (body, None) or (None, why)."""
    lens, off = [], 0
    while True:
        if off + 4 > len(data):
            return None, "length list is not ended by 0"
        (n,) = struct.unpack(">I", data[off:off + 4])
        off += 4
        if n == 0:
            break
        lens.append(n)
    if not lens:
        return None, "length list is empty"
    if off + sum(lens) > len(data):
        return None, "bodies need %d bytes, data has %d after the list" % (
            sum(lens), len(data) - off)
    if data[off + sum(lens):].strip(b"\0"):
        return None, "non-zero bytes after the last body"
    return data[off:off + lens[0]], None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("image")
    ap.add_argument("--expect", metavar="FILE.cmd")
    ap.add_argument("--sha256", metavar="FILE")
    a = ap.parse_args()  # exits 2 on a usage error

    img = read(a.image)
    expect = read(a.expect) if a.expect else None
    shaline = read(a.sha256) if a.sha256 else None

    data = None
    if len(img) < HDR:
        why = "file is %d bytes, below the %d-byte header" % (len(img), HDR)
        fail("size", why)
        for c in ("magic", "hcrc", "dcrc", "os", "arch", "type", "comp"):
            fail(c, "no header")
    else:
        (magic, hcrc, _time, dsize, _load, _ep, dcrc,
         os_, arch, typ, comp) = struct.unpack(">IIIIIIIBBBB", img[:32])
        if magic == MAGIC:
            ok("magic")
        else:
            fail("magic", "0x%08x, want 0x%08x" % (magic, MAGIC))
        got = crc(img[:4] + b"\0\0\0\0" + img[8:HDR])
        if got == hcrc:
            ok("hcrc")
        else:
            fail("hcrc", "computed 0x%08x, header says 0x%08x" % (got, hcrc))
        if len(img) >= HDR + dsize:
            ok("size")
            data = img[HDR:HDR + dsize]
            got = crc(data)
            if got == dcrc:
                ok("dcrc")
            else:
                fail("dcrc", "computed 0x%08x, header says 0x%08x" % (got, dcrc))
        else:
            fail("size", "header says %d data bytes, file has %d"
                 % (dsize, len(img) - HDR))
            fail("dcrc", "data is truncated")
        for name, val, want, label in (("os", os_, OS_LINUX, "linux"),
                                       ("arch", arch, ARCH_ARM64, "arm64"),
                                       ("type", typ, TYPE_SCRIPT, "script"),
                                       ("comp", comp, COMP_NONE, "none")):
            if val == want:
                ok(name)
            else:
                fail(name, "%d, want %d (%s)" % (val, want, label))

    body = None
    if data is None:
        fail("payload", "no data")
    else:
        body, why = parse_payload(data)
        if why:
            fail("payload", why)
        else:
            ok("payload")

    if expect is not None:
        if body is None:
            fail("expect", "no script body (payload failed)")
        elif body == expect:
            ok("expect")
        else:
            n = min(len(body), len(expect))
            at = next((i for i in range(n) if body[i] != expect[i]), n)
            fail("expect", "body is %d bytes, file %d, first difference at byte %d"
                 % (len(body), len(expect), at))

    if shaline is not None:
        words = shaline.decode("utf-8", "replace").split()
        want = words[0].lower() if words else ""
        if len(want) != 64 or any(c not in "0123456789abcdef" for c in want):
            fail("sha256", "no '<hex>  <name>' line in %s" % a.sha256)
        else:
            got = hashlib.sha256(img).hexdigest()
            if got == want:
                ok("sha256")
            else:
                fail("sha256", "image is %s, recorded %s" % (got, want))

    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
