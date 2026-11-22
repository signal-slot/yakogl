#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Is /boot the STM32MP257F-DK's microSD bootfs, and not the eMMC's?

    tools/mp2s_bootfs_check.py --findmnt FILE --lsblk FILE --blkid FILE
                               --disk-type FILE

Guard of the runner's `install` (tools/board-stm32mp2-standalone.sh,
docs/notes/stm32mp257f-dk-standalone.md section 7 "Runner verbs", section 1).
Host only: it reads four captured files, one per command run on the board,
and never touches a device.

  --findmnt  `findmnt -n -o SOURCE /boot`        e.g. /dev/mmcblk1p8
  --lsblk    `lsblk -P -o NAME,PKNAME,TYPE,FSTYPE,LABEL,PARTLABEL`
  --blkid    `blkid`   (lines `/dev/x: LABEL="bootfs" TYPE="ext4" ...`)
  --disk-type  `cat /sys/block/<disk>/device/type` of the disk of the source:
             "SD" for the card, "MMC" for the eMMC

The eMMC carries ST's partition layout with the same labels as the card
(section 1), so a label never decides alone. The source must be:

  - exactly one device, a partition (lsblk TYPE "part") of a disk;
  - on a disk whose sysfs type reads "SD" (no mmcblk index stands for the
    eMMC: on the DK the card was mmcblk2 on one boot and mmcblk0 on the
    next, board 2026-11-22);
  - labelled bootfs, as lsblk's PARTLABEL and as blkid's filesystem LABEL
    and PARTLABEL where blkid reports them, with no disagreement;
  - ext4;
  - on a disk that carries ST's whole layout beside it (fsbla1 ... userfs,
    u-boot-env), so that it is a card and not a lone partition.

Output: "ok: <why>" on stdout; one "refused: <why>" line on stderr and
exit 2 for a failed guard, an unreadable input or a wrong command line.
"""
import argparse
import re
import sys

LAYOUT = {"fsbla1", "fsbla2", "metadata1", "metadata2", "fip-a", "fip-b",
          "u-boot-env", "bootfs", "vendorfs", "rootfs", "userfs"}


class Refused(Exception):
    pass


def read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError as e:
        raise Refused(f"cannot read {path}: {e.strerror}")


def pairs(line):
    return dict(re.findall(r'(\w+)="([^"]*)"', line))


def check(findmnt, lsblk, blkid, disk_type):
    srcs = [ln.strip() for ln in findmnt.splitlines() if ln.strip()]
    if len(srcs) != 1:
        raise Refused(f"/boot has {len(srcs)} sources, want exactly one")
    src = srcs[0]
    m = re.fullmatch(r"/dev/([A-Za-z0-9_-]+)", src)
    if not m:
        raise Refused(f"/boot source {src!r} is not a /dev partition")
    name = m.group(1)

    rows = [pairs(ln) for ln in lsblk.splitlines() if "NAME=" in ln]
    row = next((r for r in rows if r.get("NAME") == name), None)
    if row is None:
        raise Refused(f"{src} is not in the lsblk capture")
    if row.get("TYPE") != "part":
        raise Refused(f"{src} is a {row.get('TYPE')!r}, not a partition")
    disk = row.get("PKNAME", "")
    if not disk:
        raise Refused(f"{src} names no parent disk")
    if disk_type.strip() != "SD":
        raise Refused(f"disk {disk} says {disk_type.strip()!r}, not 'SD' "
                      "(an eMMC says 'MMC' whatever its number)")
    if row.get("FSTYPE") != "ext4":
        raise Refused(f"{src} is {row.get('FSTYPE')!r}, not ext4")
    if row.get("PARTLABEL") != "bootfs":
        raise Refused(f"{src} partition label is {row.get('PARTLABEL')!r}, "
                      "not 'bootfs'")

    for ln in blkid.splitlines():
        dev, _, rest = ln.partition(":")
        if dev.strip() != src:
            continue
        kv = pairs(rest)
        for key in ("LABEL", "PARTLABEL"):
            if key in kv and kv[key] != "bootfs":
                raise Refused(f"blkid {key} of {src} is {kv[key]!r}, "
                              "not 'bootfs'")
        if "TYPE" in kv and kv["TYPE"] != "ext4":
            raise Refused(f"blkid TYPE of {src} is {kv['TYPE']!r}, not ext4")
        break
    else:
        raise Refused(f"{src} is not in the blkid capture")

    have = {r.get("PARTLABEL") for r in rows if r.get("PKNAME") == disk}
    missing = sorted(LAYOUT - have)
    if missing:
        raise Refused(f"disk {disk} lacks ST's partitions: "
                      + " ".join(missing))
    return src, disk


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    for a in ("findmnt", "lsblk", "blkid", "disk-type"):
        ap.add_argument("--" + a, required=True, metavar="FILE")
    try:
        args = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    try:
        src, disk = check(read(args.findmnt), read(args.lsblk),
                          read(args.blkid), read(args.disk_type))
    except Refused as e:
        print(f"refused: {e}", file=sys.stderr)
        return 2
    print(f"ok: /boot is {src}, bootfs (ext4) of the SD card {disk}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
