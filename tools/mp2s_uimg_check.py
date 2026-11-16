#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the staged boot.scr.uimg of the STM32MP257F-DK standalone default.

    tools/mp2s_uimg_check.py IMAGE [--source PATH]

The DK's standalone route stages our product script as
/boot/yakogl/standalone/boot.scr.uimg on the microSD's bootfs, where ST's
distro loop sources it before the boot has committed to anything
(docs/notes/stm32mp257f-dk-standalone.md, "What loads it", and the staging
table of section 7). This tool reads that staged file - a copy of it, so
host only and no board - and answers one question before any reboot: is
this a loadable script image, and does it carry the audited script? A
staged file it refuses must not be rebooted into; the file is a copy, so
refusing costs nothing and rebooting it is what this prevents.

    ssh "${MP2_SUDO-sudo -n }cat /boot/yakogl/standalone/boot.scr.uimg" > "$LOG/staged.uimg"
    python3 -I tools/mp2s_uimg_check.py "$LOG/staged.uimg"

The image is the legacy U-Boot image the table's build line writes
(`mkimage -A arm64 -O linux -T script -C none -d mp2-standalone.cmd`); the
header wrapper is read with tools/uboot_script.py's parser, the one place
this format is implemented here, so no mkimage is needed at check time and
no mkimage is run. The checks are the ones U-Boot's `source` itself makes
before it runs a byte (magic, header CRC, data CRC unless `verify` is off:
docs/notes/stm32mp257f-dk-standalone.md :194-196, ST/boot/image-board.c:1014,
:1023-1037; the data CRC is checked here always), plus the pins of the route:

  1. the 64-byte legacy header: magic 0x27051956, header CRC over the
     header, ih_size bytes of data present, data CRC over them;
  2. the file is the image and nothing else: no bytes after ih_size.
     `source` would ignore them, but the install record hashes whole files
     (docs/notes/stm32mp257f-dk-standalone.md :478-480), so a trailing byte
     is a staging fault;
  3. type script (6), compression none (0), os linux (5), arch arm64 (22):
     the header the build line writes;
  4. the script data holds exactly one file (a length table of one non-zero
     length; an image of an empty script has no length table and is already
     refused at 1);
  5. the script equals PATH, byte for byte: by default the committed
     driver/tools/mp2-standalone/mp2-standalone.cmd - the script its
     selftest.sh lints and its qemu/ harness runs - so a staged image that
     carries an older, edited or foreign script is refused although its
     header is perfect. --source PATH compares against another audited
     script.

ih_time and ih_name are not pinned: the build's time is reproducible
(SOURCE_DATE_EPOCH-based, driver/tools/mp2-standalone/build.sh) but re-
staging the same script legitimately carries another time, and the name is
free text (build.sh passes -n mp2-standalone; the table's line passes none).

What this is not: install's staging read-back hashes the file it wrote
against the build's output ("did my bytes reach the card", :112-114), and
tools/mp2s_hush_lint.py lints the script's text against ST's hush. This
checks the staged artefact from the file alone: a valid image whose script
is, byte for byte, the audited source.

Output: five "ok: ..." lines on stdout in the order above when accepted,
stderr empty. A refusal prints exactly one "refused: <why>" line on stderr
and stops there, so earlier "ok:" lines may already be on stdout. Exit 0
accepted, exit 2 refused or wrong command line (usage prints no "refused:"
line). Exit 3 is not used.
"""
import argparse
import hashlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import uboot_script as us  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_SOURCE = os.path.normpath(os.path.join(HERE, "..", "driver", "tools",
                                               "mp2-standalone",
                                               "mp2-standalone.cmd"))
IH_OS_LINUX = 5
IH_ARCH_ARM64 = 22


def refuse(why):
    print(f"refused: {why}", file=sys.stderr)
    sys.exit(2)


def read_bytes(what, path):
    try:
        with open(path, "rb") as fh:
            return fh.read()
    except OSError as err:
        refuse("%s %s is not a readable file (%s)"
               % (what, path, err.strerror or err))


def first_difference(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def main(argv):
    ap = argparse.ArgumentParser(
        prog="mp2s_uimg_check.py",
        description="Check the staged boot.scr.uimg of the STM32MP257F-DK "
                    "standalone default (no mkimage, no board).")
    ap.add_argument("image", help="the staged script image, or a copy of it")
    ap.add_argument("--source", default=DEFAULT_SOURCE,
                    help="the audited script the payload must equal "
                         "(default: driver/tools/mp2-standalone/"
                         "mp2-standalone.cmd)")
    args = ap.parse_args(argv)
    if not args.source:
        ap.error("--source: empty path")

    image = read_bytes("image", args.image)
    try:
        hdr, files = us.parse(image)
    except us.Refused as err:
        refuse(str(err))
    print("ok: legacy header: magic 0x%08x, header CRC 0x%08x ok, name %r"
          % (us.IH_MAGIC, hdr["hcrc"], hdr["name"]))

    extra = len(image) - us.HEADER.size - hdr["size"]
    if extra != 0:
        refuse("%d bytes after the end of the image (ih_size %d)"
               % (extra, hdr["size"]))
    print("ok: data: CRC 0x%08x ok over ih_size %d bytes, "
          "the file ends with the image" % (hdr["dcrc"], hdr["size"]))

    if hdr["os"] != IH_OS_LINUX:
        refuse("os %d, expected %d (linux)" % (hdr["os"], IH_OS_LINUX))
    if hdr["arch"] != IH_ARCH_ARM64:
        refuse("arch %d, expected %d (arm64)" % (hdr["arch"], IH_ARCH_ARM64))
    print("ok: header: type script (%d), os linux (%d), arch arm64 (%d), "
          "comp none (%d)" % (us.IH_TYPE_SCRIPT, IH_OS_LINUX,
                              IH_ARCH_ARM64, us.IH_COMP_NONE))

    if len(files) != 1:
        refuse("script data holds %d files, expected 1" % len(files))
    script = files[0]
    print("ok: script data: one file, %d bytes" % len(script))

    source = read_bytes("script source", args.source)
    if script != source:
        refuse("the staged script differs from the audited source %s "
               "(%d bytes in the image, %d in the source, first difference "
               "at byte %d)" % (args.source, len(script), len(source),
                                first_difference(script, source)))
    print("ok: script equals %s (%d bytes, sha256 %s)"
          % (args.source, len(script), hashlib.sha256(script).hexdigest()))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
