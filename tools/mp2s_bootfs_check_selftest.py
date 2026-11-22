#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""tools/mp2s_bootfs_check.py without a board (plan #362, item bootfs-check).

    python3 -I tools/mp2s_bootfs_check_selftest.py

Synthetic findmnt/lsblk/blkid/sysfs captures of a DK with a microSD
(mmcblk1) and an eMMC (mmcblk2) that carry the same GPT labels. The good
case is accepted; every refusal is one "refused:" line on stderr, exit 2,
and nothing on stdout.
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "mp2s_bootfs_check.py")
LABELS = ["fsbla1", "fsbla2", "metadata1", "metadata2", "fip-a", "fip-b",
          "u-boot-env", "bootfs", "vendorfs", "rootfs", "userfs"]
EXT4 = {"bootfs", "vendorfs", "rootfs", "userfs"}


def lsblk_disk(d, skip=None):
    out = ['NAME="%s" PKNAME="" TYPE="disk" FSTYPE="" LABEL="" PARTLABEL=""' % d]
    for n, lab in enumerate(LABELS, 1):
        if lab != skip:
            fs = "ext4" if lab in EXT4 else ""
            out.append('NAME="%sp%d" PKNAME="%s" TYPE="part" FSTYPE="%s" '
                       'LABEL="" PARTLABEL="%s"' % (d, n, d, fs, lab))
    return "\n".join(out) + "\n"


def blkid_disk(d):
    return '/dev/%sp8: LABEL="bootfs" UUID="1" TYPE="ext4" PARTLABEL="bootfs"\n' % d


def good():
    return {"findmnt": "/dev/mmcblk1p8\n",
            "lsblk": lsblk_disk("mmcblk1") + lsblk_disk("mmcblk2"),
            "blkid": blkid_disk("mmcblk1") + blkid_disk("mmcblk2"),
            "type": "SD\n"}


def run(files, tmp):
    args = [sys.executable, "-I", TOOL]
    for k, opt in (("findmnt", "--findmnt"), ("lsblk", "--lsblk"),
                   ("blkid", "--blkid"), ("type", "--disk-type")):
        p = os.path.join(tmp, k)
        with open(p, "w") as f:
            f.write(files[k])
        args += [opt, p]
    r = subprocess.run(args, capture_output=True, text=True)
    return r.returncode, r.stdout, r.stderr


def mut(**kw):
    f = good()
    f.update(kw)
    return f


NOT_EXT4_OLD = 'NAME="mmcblk1p8" PKNAME="mmcblk1" TYPE="part" FSTYPE="ext4"'
NOT_EXT4_NEW = 'NAME="mmcblk1p8" PKNAME="mmcblk1" TYPE="part" FSTYPE="vfat"'

CASES = [
    ("not 'SD'", mut(findmnt="/dev/mmcblk2p8\n", type="MMC\n")),
    ("'MMC'", mut(type="MMC\n")),
    ("not 'bootfs'", mut(findmnt="/dev/mmcblk1p10\n",
                         blkid='/dev/mmcblk1p10: LABEL="rootfs" TYPE="ext4"\n')),
    ("exactly one", mut(findmnt="/dev/mmcblk1p8\n/dev/mmcblk2p8\n")),
    ("not a /dev partition", mut(findmnt="tmpfs\n")),
    ("not a partition", mut(findmnt="/dev/mmcblk1\n")),
    ("not in the lsblk", mut(findmnt="/dev/sda1\n")),
    ("blkid LABEL", mut(blkid=good()["blkid"].replace(
        '/dev/mmcblk1p8: LABEL="bootfs"', '/dev/mmcblk1p8: LABEL="other"'))),
    ("not in the blkid", mut(blkid=blkid_disk("mmcblk2"))),
    ("not ext4", mut(lsblk=good()["lsblk"].replace(NOT_EXT4_OLD, NOT_EXT4_NEW))),
    ("lacks ST's partitions: u-boot-env",
     mut(lsblk=lsblk_disk("mmcblk1", "u-boot-env") + lsblk_disk("mmcblk2"))),
    ("not 'bootfs'", mut(lsblk=good()["lsblk"].replace(
        'PARTLABEL="bootfs"', 'PARTLABEL=""', 1))),
]


def main():
    ok = fail = 0
    with tempfile.TemporaryDirectory(prefix="mp2s-bootfs-") as tmp:
        rc, out, err = run(good(), tmp)
        if rc == 0 and out.startswith("ok: /boot is /dev/mmcblk1p8") and not err:
            ok += 1
        else:
            print("FAIL accepted case", rc, err)
            fail += 1
        # the DK as it boots: the card is mmcblk0 on some boots (type SD);
        # the number never decides
        rc, out, err = run(mut(findmnt="/dev/mmcblk0p8\n",
                               lsblk=lsblk_disk("mmcblk0"),
                               blkid=blkid_disk("mmcblk0")), tmp)
        if rc == 0 and out.startswith("ok: /boot is /dev/mmcblk0p8") and not err:
            ok += 1
        else:
            print("FAIL accepted case (the card as mmcblk0)", rc, err)
            fail += 1
        for needle, files in CASES:
            rc, out, err = run(files, tmp)
            if (rc == 2 and err.count("refused: ") == 1 and needle in err
                    and not out):
                ok += 1
            else:
                print("FAIL want refusal %r: exit %d: %s" % (needle, rc, err))
                fail += 1
        none = os.path.join(tmp, "none")
        args = [sys.executable, "-I", TOOL]
        for o in ("--findmnt", "--lsblk", "--blkid", "--disk-type"):
            args += [o, none]
        r = subprocess.run(args, capture_output=True, text=True)
        if r.returncode == 2 and "refused: cannot read" in r.stderr and not r.stdout:
            ok += 1
        else:
            print("FAIL unreadable input")
            fail += 1
        r = subprocess.run([sys.executable, "-I", TOOL],
                           capture_output=True, text=True)
        if r.returncode == 2 and not r.stdout:
            ok += 1
        else:
            print("FAIL usage")
            fail += 1
    print("%d ok, %d failed" % (ok, fail))
    return 0 if fail == 0 and ok == 16 else 1


if __name__ == "__main__":
    sys.exit(main())
