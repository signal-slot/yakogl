#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Do the counted default's environment edits fit the DK's u-boot-env?

    tools/mp2s_env_capacity.py [--size N] [--margin N] [--out FILE]
                               (--image FILE [--offset N] [--offset2 N] | --text FILE)
                               NAME=VALUE ...

  --image FILE   a redundant environment image (two copies of --size bytes,
                 copy 1 at --offset (default 0), copy 2 at --offset2
                 (default --offset + --size)), such as a dd of the top 0x4000
                 bytes of the u-boot-env partition
  --text FILE    a text environment, one NAME=VALUE per line (fw_printenv
                 output); it is laid out as a redundant pair of --size bytes
  --size N       CONFIG_ENV_SIZE, the size of one copy, default 0x2000: the
                 DK's ST U-Boot has ENV_SIZE 0x2000 (docs/notes/
                 stm32mp257f-dk-uboot-boot.md:73-74 and the table at :88,
                 ST/arch/arm/mach-stm32mp/Kconfig:33-34); copy 0 is at 0x7e000
                 and the redundant copy at 0x7c000 of the 512 KiB partition
                 (0x7e000: pre-study-stm32mp257f-dk.md:265)
  --margin N     free bytes that must be left after the edit (default 256)
  --out FILE     write the edited image (both copies saved once each) here

The data area of a redundant copy is --size minus 5 bytes (CRC32 and flags).
"Used" is U-Boot's: every "name=value" and its NUL, plus the final NUL. The
edit is made on a copy with tools/uboot_env.py (the active copy is left alone,
the other one gets the new strings, the CRC32 and flags + 1, as env_save
does), and then once more so that the other copy is written too; both copies
are re-parsed and their CRCs checked. Exit 0: fits with the margin; 1: it
does not; 2: usage or an unreadable environment.
"""
import argparse
import os
import sys
import tempfile
import types
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import uboot_env as ue  # noqa: E402

DEFAULT_SIZE = 0x2000   # CONFIG_ENV_SIZE, an ST Kconfig value, see the docstring


class Usage(Exception):
    pass


def used(entries):
    return sum(len(e) + 1 for e in entries) + 1


def number(s):
    try:
        v = int(s, 0)
    except ValueError:
        raise argparse.ArgumentTypeError("%r is not a number" % s)
    if v < 0:
        raise argparse.ArgumentTypeError("%r is negative" % s)
    return v


def from_text(path, size):
    try:
        with open(path, "rb") as fh:
            lines = [ln for ln in fh.read().split(b"\n") if ln.strip()]
    except OSError as e:
        raise Usage("cannot read %s: %s" % (path, e.strerror))
    for ln in lines:
        if b"=" not in ln:
            raise Usage("%s: line %r is not NAME=VALUE" % (path, ln.decode("latin-1")))
    area = size - 5
    data = b"".join(ln + b"\0" for ln in lines) + b"\0"
    if len(data) > area:
        raise Usage("%s: the text needs %d bytes, a copy holds %d" % (path, len(data), area))
    data += b"\xff" * (area - len(data))
    # both copies equal, flags 0: a tie, copy 1 is active
    return (zlib.crc32(data).to_bytes(4, "little") + b"\0" + data) * 2


def emulate(path, a, assigns, out_path):
    """Two saves of the edit, so that both copies hold it; returns the file."""
    cur = path
    for _ in range(2):
        img = ue.Image(cur, a)
        _, files = ue.edit(img, lambda e: ue.apply_set(e, assigns), {cur: out_path})
        for p, data in files.items():
            ue.write_out(p, data)
        cur = out_path
    return cur


def run(args):
    if args.size < 6:
        raise Usage("--size %d is too small" % args.size)
    assigns = [os.fsencode(x) for x in args.assign]
    for x in assigns:
        if b"=" not in x:
            raise Usage("%s is not NAME=VALUE" % os.fsdecode(x))
    with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as tmp:
        work = os.path.join(tmp, "env.bin")
        a = types.SimpleNamespace(redundant=True, offset=args.offset, offset2=args.offset2,
                                  size=args.size, second=None)
        if args.text:
            with open(work, "wb") as fh:
                fh.write(from_text(args.text, args.size))
            a.offset, a.offset2 = 0, None
        else:
            try:
                with open(args.image, "rb") as fh:
                    raw = fh.read()
            except OSError as e:
                raise Usage("cannot read %s: %s" % (args.image, e.strerror))
            with open(work, "wb") as fh:
                fh.write(raw)
        try:
            before = ue.Image(work, a)
            bad = before.problems()
            if bad:
                raise Usage("environment refused: " + bad[0])
            src = before.copies[before.active()]
            area = len(src.area)
            new_entries = ue.apply_set(src.entries, assigns)
        except ue.Refused as e:
            raise Usage("environment refused: %s" % e)
        ub, ua = used(src.entries), used(new_entries)
        free = area - ua
        print("env: %s, copy size 0x%x, data area %d bytes, active %s" % (
            args.text or args.image, args.size, area, src.label))
        print("used before: %d bytes (%d variables), free %d" % (ub, len(src.entries), area - ub))
        print("used after:  %d bytes (%d variables), free %d, margin %d"
              % (ua, len(new_entries), free, args.margin))
        if free < 0:
            print("DOES NOT FIT: needs %d more bytes" % -free)
            return 1
        out = os.path.join(tmp, "out.bin")
        try:
            res = emulate(work, a, assigns, out)
            check = ue.Image(res, a)
            if check.problems():
                raise ue.Refused(check.problems()[0])
            want = sorted(new_entries)
            for c in check.copies:
                if sorted(c.entries) != want or zlib.crc32(c.area) != c.crc:
                    raise ue.Refused("%s does not re-parse to the edited environment" % c.label)
            print("both copies re-parsed: CRC32 ok, flags 0x%02x and 0x%02x, active %s"
                  % (check.copies[0].flags, check.copies[1].flags,
                     check.copies[check.active()].label))
        except ue.Refused as e:
            raise Usage("edit refused: %s" % e)
        if args.out:
            with open(res, "rb") as fh:
                ue.write_out(args.out, fh.read())
            print("wrote %s" % args.out)
        if free >= args.margin:
            print("FITS with the margin")
            return 0
        print("TOO TIGHT: fits, but only %d bytes free, margin is %d" % (free, args.margin))
        return 1


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--image")
    g.add_argument("--text")
    ap.add_argument("--size", type=number, default=DEFAULT_SIZE)
    ap.add_argument("--margin", type=number, default=256)
    ap.add_argument("--offset", type=number, default=0)
    ap.add_argument("--offset2", type=number, default=None)
    ap.add_argument("--out")
    ap.add_argument("assign", nargs="+", metavar="NAME=VALUE")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    try:
        return run(args)
    except Usage as e:
        print("mp2s_env_capacity.py: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
