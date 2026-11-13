#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Host-side image check for the STM32MP257F-DK standalone default.

    tools/mp2s_image_check.py IMAGE [--free-bytes N] [--require-marker TEXT]

The runner's `default <image>` (tools/board-stm32mp2-standalone.sh, plan
#362) must refuse an image that the product script would refuse at boot
(docs/notes/stm32mp257f-dk-standalone.md, section 3 step 7 :287-293 and
section 7 "Runner verbs" :533: image check fails: header, window, size,
marker). The product script runs the checks below on the board after
loading; they are the checks of driver/tools/uboot-mp2/mp2-mmc.cmd, run
here on the host before the runner stages anything:

  - minimum size: the file holds the 0x40-byte arm64 Image header
    (mp2-mmc.cmd:21-22, :44-47);
  - window: the file loads at 0xa0000000 (mp2-mmc.cmd:19, the DK DTS's
    zephyr,sram start) into the 0x10000000-byte window (mp2-mmc.cmd:20;
    docs/notes/stm32mp257f-dk-uboot-boot.md :188-191), so the file itself
    must not be larger than the window (mp2-mmc.cmd:48-51);
  - free space: bootfs has about 34 MiB free and the runner refuses an
    image larger than half of it
    (docs/notes/stm32mp257f-dk-standalone.md :71-72); with --free-bytes N
    the file must be at most N/2 bytes;
  - the ARM64 Image magic "ARM\\x64" (0x644d5241 little-endian) at byte 56
    (mp2-mmc.cmd:65-70);
  - the header's image_size: 64 little-endian bits at byte 16, the RAM the
    image takes from its start (.bss included; the file is only its first
    part), inside the window (mp2-mmc.cmd:71-84);
  - with --require-marker TEXT: the bytes TEXT (UTF-8) occur somewhere in
    the image. The watchdog-refresh marker
    (docs/notes/stm32mp257f-dk-standalone.md :446-459, checked for
    `forever` at :370-372) is defined by plan #362's iwdg-refresh item;
    this tool only takes it as a parameter.

One difference, in the refusing direction: mp2-mmc.cmd:81 compares the
low word of image_size with the window using `itest.l`, which is signed,
so on the board a footprint between 2 GiB and 4 GiB passes; here any
image_size above the window is refused. The image goes in at 0xa0000000
with `go`, not `booti` (mp2-mmc.cmd:87-90), so the footprint really does
have to fit the window.

Output: each check that passes prints one "ok: ..." line on stdout, in
the order of mp2-mmc.cmd (size before content); a check not made prints
one "skip: ..." line. A refusal prints exactly one "refused: <why>" line
on stderr and stops there, so earlier "ok:" lines may already be on
stdout. Exit 0 when every check passed or was skipped, exit 2 when the
image is refused or the command line is wrong (then no "refused:" line).
"""
import argparse
import os
import sys

LOAD_ADDR = 0xA0000000  # mp2-mmc.cmd:19 (mp2z_load)
WINDOW = 0x10000000  # mp2-mmc.cmd:20 (mp2z_window)
MIN_SIZE = 0x40  # mp2-mmc.cmd:21-22 (mp2z_min: the arm64 Image header)
MAGIC_OFF = 0x38  # mp2-mmc.cmd:65-66 (byte 56)
MAGIC = 0x644D5241  # mp2-mmc.cmd:67 ("ARM\\x64", little-endian)
IMGSZ_OFF = 0x10  # mp2-mmc.cmd:71-72, :79 (byte 16, 64 bits)


def num(text):
    try:
        value = int(text, 0)
    except ValueError:
        raise argparse.ArgumentTypeError("N is decimal or 0x hex")
    if value < 0:
        raise argparse.ArgumentTypeError("N is negative")
    return value


def refuse(why):
    print(f"refused: {why}", file=sys.stderr)
    raise SystemExit(2)


def main(argv):
    ap = argparse.ArgumentParser(
        prog="tools/mp2s_image_check.py",
        description="Host check of a DK standalone default image: the "
        "header, window, size and marker checks the product script "
        "would make at boot (see --help in the source's docstring).",
    )
    ap.add_argument("image", help="the arm64 Image file to check")
    ap.add_argument(
        "--free-bytes",
        type=num,
        metavar="N",
        help="free bytes of the bootfs partition; the image must be at "
        "most N/2 of them (omitted: the check is skipped)",
    )
    ap.add_argument(
        "--require-marker",
        metavar="TEXT",
        help="refuse unless the bytes TEXT occur in the image (omitted: "
        "the check is skipped)",
    )
    args = ap.parse_args(argv)
    if args.require_marker == "":
        ap.error("--require-marker TEXT is empty")
    path = args.image

    # Size first, before the file is opened, as the script sizes the file
    # before it loads it (mp2-mmc.cmd:35-51).
    try:
        size = os.path.getsize(path)
    except OSError as e:
        refuse(f"{path} is not a readable file ({e.strerror})")
    if size < MIN_SIZE:  # mp2-mmc.cmd:44-47
        refuse(f"file {size} bytes, below the {MIN_SIZE:#x}-byte Image header")
    print(f"ok: {size} bytes, at least the {MIN_SIZE:#x}-byte Image header")
    if size > WINDOW:  # mp2-mmc.cmd:48-51
        refuse(f"file {size} bytes, above the {WINDOW:#x}-byte window at {LOAD_ADDR:#x}")
    print(f"ok: {size} bytes, inside the {WINDOW:#x}-byte window at {LOAD_ADDR:#x}")
    if args.free_bytes is None:
        print("skip: free space (no --free-bytes given)")
    else:  # docs/notes/stm32mp257f-dk-standalone.md :71-72
        half = args.free_bytes // 2
        if size > half:
            refuse(f"file {size} bytes, more than half the {args.free_bytes} free bytes (limit {half})")
        print(f"ok: {size} bytes, at most half the {args.free_bytes} free bytes ({half})")

    try:
        with open(path, "rb") as f:
            data = f.read() if args.require_marker is not None else f.read(MIN_SIZE)
    except OSError as e:
        refuse(f"{path} is not a readable file ({e.strerror})")

    magic = int.from_bytes(data[MAGIC_OFF : MAGIC_OFF + 4], "little")
    if magic != MAGIC:  # mp2-mmc.cmd:65-70
        refuse("no ARM64 Image magic at byte 56 of the file")
    print("ok: ARM64 Image magic at byte 56")
    imgsz = int.from_bytes(data[IMGSZ_OFF : IMGSZ_OFF + 8], "little")
    if imgsz >= 1 << 32:  # mp2-mmc.cmd:74-78
        refuse("the header image_size is 4 GiB or more")
    if imgsz > WINDOW:  # mp2-mmc.cmd:79-84, unsigned (see the docstring)
        refuse(f"the image takes {imgsz} bytes of RAM (header image_size), above the {WINDOW:#x}-byte window")
    print(f"ok: header image_size {imgsz} bytes of RAM, inside the {WINDOW:#x}-byte window")

    if args.require_marker is None:
        print("skip: marker (no --require-marker given)")
    else:
        marker = args.require_marker.encode("utf-8", "surrogateescape")
        at = data.find(marker)
        if at < 0:
            refuse(f"the image does not contain the marker {args.require_marker!r}")
        print(f"ok: marker {args.require_marker!r} first at byte {at}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
