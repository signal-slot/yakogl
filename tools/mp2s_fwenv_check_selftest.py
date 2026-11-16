#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_fwenv_check_selftest.py - tools/mp2s_fwenv_check.py on fake roots
(plan #362, item fwenv-check). No board; fake roots under a temp directory.

    python3 -I tools/mp2s_fwenv_check_selftest.py

The fake DK: SD card mmcblk1 (type SD; u-boot-env p3 of 512 KiB, bootfs p4,
rootfs p5) and eMMC mmcblk0 (type MMC; the same labels), /boot mounted from
mmcblk1p4. Cases: good SD config (by path, by by-partlabel, with comments
and optional fields); by-partlabel resolving to the eMMC; the eMMC by path;
one copy on each; an eMMC claiming SD; overlapping copies; offsets past the
partition end; wrong env size; a partition that is not u-boot-env; a whole
disk; /boot on the eMMC; nothing on /boot; a missing config; a single copy.
This is a .py so that the scope map of tools/check.sh keeps it in `tools`.
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CHECK = os.path.join(HERE, "mp2s_fwenv_check.py")


def w(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def mkdisk(r, disk, typ):
    w(f"{r}/sys/block/{disk}/device/type", typ + "\n")


def mkpart(r, disk, part, num, name, sectors):
    d = f"{r}/sys/block/{disk}/{part}"
    w(f"{d}/partition", f"{num}\n")
    w(f"{d}/size", f"{sectors}\n")
    w(f"{d}/uevent", f"MAJOR=179\nPARTNAME={name}\n")
    os.makedirs(f"{r}/sys/class/block", exist_ok=True)
    os.symlink(f"../../block/{disk}/{part}", f"{r}/sys/class/block/{part}")


def cfg2(r, a, b):
    w(f"{r}/etc/fw_env.config", "%s %s %s\n%s %s %s\n" % (*a, *b))


def mkroot(r):
    mkdisk(r, "mmcblk1", "SD")
    mkdisk(r, "mmcblk0", "MMC")
    for disk, parts in (("mmcblk1", ((3, "u-boot-env", 1024), (4, "bootfs", 65536), (5, "rootfs", 65536))),
                        ("mmcblk0", ((3, "u-boot-env", 1024), (4, "bootfs", 65536)))):
        for n, name, sec in parts:
            mkpart(r, disk, f"{disk}p{n}", n, name, sec)
    os.makedirs(f"{r}/dev/disk/by-partlabel")
    os.symlink("../../mmcblk1p3", f"{r}/dev/disk/by-partlabel/u-boot-env")
    w(f"{r}/proc/self/mountinfo",
      "28 1 179:4 / /boot rw,relatime shared:3 - vfat /dev/mmcblk1p4 rw\n")
    cfg2(r, ("/dev/mmcblk1p3", "0x7e000", "0x2000"), ("/dev/mmcblk1p3", "0x7c000", "0x2000"))


PL = "/dev/disk/by-partlabel/u-boot-env"


def s_good(r): pass
def s_partlabel(r): cfg2(r, (PL, "0x7e000", "0x2000"), (PL, "0x7c000", "0x2000"))
def s_comments(r): w(f"{r}/etc/fw_env.config", "# env\n\n/dev/mmcblk1p3 0x7e000 0x2000 0x200 16 # c\n/dev/mmcblk1p3 507904 8192\n")
def s_partlabel_emmc(r):
    os.remove(f"{r}/dev/disk/by-partlabel/u-boot-env")
    os.symlink("../../mmcblk0p3", f"{r}/dev/disk/by-partlabel/u-boot-env")
    s_partlabel(r)
def s_emmc_direct(r): cfg2(r, ("/dev/mmcblk0p3", "0x7e000", "0x2000"), ("/dev/mmcblk0p3", "0x7c000", "0x2000"))
def s_one_each(r): cfg2(r, ("/dev/mmcblk1p3", "0x7e000", "0x2000"), ("/dev/mmcblk0p3", "0x7c000", "0x2000"))
def s_emmc_says_sd(r):
    w(f"{r}/sys/block/mmcblk0/device/type", "SD\n")
    s_emmc_direct(r)
def s_overlap(r): cfg2(r, ("/dev/mmcblk1p3", "0x7e000", "0x2000"), ("/dev/mmcblk1p3", "0x7d000", "0x2000"))
def s_past_end(r): cfg2(r, ("/dev/mmcblk1p3", "0x7e000", "0x2000"), ("/dev/mmcblk1p3", "0x80000", "0x2000"))
def s_wrong_size(r): cfg2(r, ("/dev/mmcblk1p3", "0x7e000", "0x4000"), ("/dev/mmcblk1p3", "0x7a000", "0x4000"))
def s_not_env(r): cfg2(r, ("/dev/mmcblk1p5", "0", "0x2000"), ("/dev/mmcblk1p5", "0x2000", "0x2000"))
def s_whole_disk(r): cfg2(r, ("/dev/mmcblk1", "0", "0x2000"), ("/dev/mmcblk1", "0x2000", "0x2000"))
def s_boot_emmc(r): w(f"{r}/proc/self/mountinfo", "28 1 179:4 / /boot rw - vfat /dev/mmcblk0p4 rw\n")
def s_no_boot(r): w(f"{r}/proc/self/mountinfo", "")
def s_no_config(r): os.remove(f"{r}/etc/fw_env.config")
def s_single(r): w(f"{r}/etc/fw_env.config", "/dev/mmcblk1p3 0x7e000 0x2000\n")

CASES = [
    ("good", 0, "fwenv: ok SD u-boot-env copies at 0x7e000 and 0x7c000", s_good),
    ("good_partlabel", 0, "fwenv: ok", s_partlabel),
    ("good_comments", 0, "fwenv: ok", s_comments),
    ("partlabel_emmc", 2, "fwenv: REFUSE", s_partlabel_emmc),
    ("emmc_direct", 2, "type 'MMC'", s_emmc_direct),
    ("one_each", 2, "type 'MMC'", s_one_each),
    ("emmc_says_sd", 2, "mmcblk0", s_emmc_says_sd),
    ("overlap", 2, "overlap", s_overlap),
    ("past_end", 2, "runs past the end", s_past_end),
    ("wrong_size", 2, "env size", s_wrong_size),
    ("not_env", 2, "not u-boot-env", s_not_env),
    ("whole_disk", 2, "not a partition", s_whole_disk),
    ("boot_emmc", 2, "not on the disk holding /boot", s_boot_emmc),
    ("no_boot", 2, "nothing is mounted on /boot", s_no_boot),
    ("no_config", 2, "no etc/fw_env.config", s_no_config),
    ("single_copy", 2, "want 2", s_single),
]


def main():
    fail = 0
    with tempfile.TemporaryDirectory(prefix="mp2s_fwenv.") as t:
        for name, want, sub, setup in CASES:
            r = os.path.join(t, name)
            mkroot(r)
            setup(r)
            p = subprocess.run([sys.executable, "-I", CHECK, "--root", r],
                               capture_output=True, text=True)
            out = (p.stdout + p.stderr).strip()
            if p.returncode == want and sub in out:
                print(f"ok   {name}: {out}")
            else:
                print(f"FAIL {name}: exit {p.returncode} (want {want}), '{out}' (want '*{sub}*')")
                fail = 1
    if not fail:
        print("mp2s_fwenv_check_selftest: all ok")
    return fail


if __name__ == "__main__":
    sys.exit(main())
