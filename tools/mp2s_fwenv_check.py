#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""mp2s_fwenv_check.py - does the DK's fw_env.config reach only the microSD's
u-boot-env? Read-only (plan #362, item fwenv-check).

    tools/mp2s_fwenv_check.py [--root /] [--config etc/fw_env.config]
                              [--env-size 0x2000]

The DK has an eMMC (sdmmc2) with ST's layout and the same partition labels as
the card, so a fw_env.config naming /dev/disk/by-partlabel/u-boot-env or an
mmcblk index could resolve to the eMMC. fw_printenv/fw_setenv write through
that file, so this check runs first. It reads only files under --root and
writes nothing.

fw_env.config (libubootenv / u-boot-tools): '#' starts a comment, blank lines
are skipped, a line is "<device> <offset> <env size> [<sector size>
[<sectors>]]", numbers decimal or 0x hex. CONFIG_SYS_REDUNDAND_ENVIRONMENT=y
means exactly two lines, one per copy.

Passes only if every device
  - resolves, through symlinks under root, to a partition (sys/class/block/
    <name>/partition) of a disk whose sys/block/<disk>/device/type is "SD"
    (an eMMC says "MMC" whatever its number; the number never decides: on
    the DK the card is mmcblk0 on some boots),
  - is on the disk holding the partition mounted on /boot,
  - is a partition named u-boot-env (uevent PARTNAME),
  - has an env size equal to --env-size (default 0x2000, the CONFIG_ENV_SIZE
    tools/board-stm32mp2.sh hands tools/uboot_env.py), and
  - holds its copy inside the partition (size from sysfs) without the other
    copy overlapping it.
Pass: one "fwenv: ok ..." line on stdout, exit 0. Otherwise "fwenv: REFUSE
<why>" on stdout and exit 2 (a usage error is argparse's exit 2 as well).
"""
import argparse
import os
import re
import sys


class Refuse(Exception):
    pass


def resolve(root, path):
    """Resolve PATH under ROOT following symlinks (absolute targets are
    re-rooted); returns an absolute path inside the root."""
    parts = [p for p in path.split("/") if p]
    out = []
    hops = 0
    while parts:
        p = parts.pop(0)
        if p == ".":
            continue
        if p == "..":
            if out:
                out.pop()
            continue
        cand = os.path.join(root, *out, p)
        if os.path.islink(cand):
            hops += 1
            if hops > 40:
                raise Refuse("too many symlinks resolving %s" % path)
            tgt = os.readlink(cand)
            if tgt.startswith("/"):
                out = []
            parts = [q for q in tgt.split("/") if q] + parts
        else:
            out.append(p)
    return "/" + "/".join(out)


def rd(root, rel):
    try:
        with open(os.path.join(root, rel.lstrip("/"))) as fh:
            return fh.read().strip()
    except OSError:
        return None


def num(s, what):
    try:
        return int(s, 0)
    except ValueError:
        raise Refuse("%s '%s' is not a number" % (what, s))


def parse_config(text):
    rows = []
    for ln in text.splitlines():
        ln = ln.split("#", 1)[0].strip()
        if not ln:
            continue
        f = ln.split()
        if not 3 <= len(f) <= 5:
            raise Refuse("fw_env.config line '%s': want device offset size "
                         "[sector size [sectors]]" % ln)
        rows.append((f[0], num(f[1], "offset"), num(f[2], "env size"),
                     [num(x, "sector field") for x in f[3:]]))
    return rows


def part_info(root, name):
    """(disk, partition size in bytes, PARTNAME) of a block partition NAME."""
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", name):
        raise Refuse("device name '%s' is not a block device name" % name)
    sysp = resolve(root, "/sys/class/block/" + name)
    if rd(root, sysp + "/partition") is None:
        raise Refuse("%s is not a partition (no sys/class/block/%s/partition)"
                     % (name, name))
    disk = os.path.basename(os.path.dirname(sysp))
    if not os.path.isdir(os.path.join(root, "sys/block", disk)):
        raise Refuse("%s: its disk '%s' is not in sys/block" % (name, disk))
    size = rd(root, sysp + "/size")
    if size is None or not size.isdigit():
        raise Refuse("%s: no readable sysfs size" % name)
    partname = None
    for ln in (rd(root, sysp + "/uevent") or "").splitlines():
        if ln.startswith("PARTNAME="):
            partname = ln[len("PARTNAME="):]
    return disk, int(size) * 512, partname


def boot_source(root):
    """The device mounted on /boot, from mountinfo, else mounts."""
    src = None
    mi = rd(root, "/proc/self/mountinfo")
    if mi is not None:
        for ln in mi.splitlines():
            f = ln.split()
            if len(f) > 9 and f[4] == "/boot" and "-" in f:
                src = f[f.index("-") + 2]
    else:
        m = rd(root, "/proc/mounts") or rd(root, "/proc/self/mounts")
        if m is None:
            raise Refuse("no proc/self/mountinfo or mounts: cannot find /boot")
        for ln in m.splitlines():
            f = ln.split()
            if len(f) > 1 and f[1] == "/boot":
                src = f[0]
    if src is None:
        raise Refuse("nothing is mounted on /boot")
    return src


def check(root, cfgrel, env_size):
    text = rd(root, cfgrel)
    if text is None:
        raise Refuse("no %s" % cfgrel.lstrip("/"))
    rows = parse_config(text)
    if len(rows) != 2:
        raise Refuse("%d line(s) in fw_env.config, want 2 (redundant "
                     "environment, CONFIG_SYS_REDUNDAND_ENVIRONMENT=y)"
                     % len(rows))
    bname = os.path.basename(resolve(root, boot_source(root)))
    bdisk = part_info(root, bname)[0]
    spans = []
    for dev, off, size, _extra in rows:
        name = os.path.basename(resolve(root, dev))
        disk, psize, partname = part_info(root, name)
        kind = rd(root, "/sys/block/%s/device/type" % disk)
        if kind != "SD":
            raise Refuse("%s is on a disk of type '%s', not SD (eMMC?)"
                         % (dev, kind if kind is not None else "unknown"))
        if disk != bdisk:
            raise Refuse("%s is not on the disk holding /boot" % dev)
        if partname != "u-boot-env":
            raise Refuse("%s is partition '%s', not u-boot-env"
                         % (dev, partname or "unnamed"))
        if size != env_size:
            raise Refuse("env size 0x%x, uboot_env.py assumes 0x%x"
                         % (size, env_size))
        if off + size > psize:
            raise Refuse("copy at 0x%x size 0x%x runs past the end of the "
                         "partition (0x%x bytes)" % (off, size, psize))
        spans.append((name, off, size))
    (n1, o1, s1), (n2, o2, s2) = spans
    if n1 != n2:
        raise Refuse("the two copies are on different partitions")
    if o1 < o2 + s2 and o2 < o1 + s1:
        raise Refuse("the copies at 0x%x and 0x%x overlap (size 0x%x)"
                     % (o1, o2, s1))
    return "fwenv: ok SD u-boot-env copies at 0x%x and 0x%x size 0x%x" % (o1, o2, s1)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default="/")
    ap.add_argument("--config", default="etc/fw_env.config")
    ap.add_argument("--env-size", default="0x2000")
    a = ap.parse_args()
    try:
        print(check(a.root, "/" + a.config.lstrip("/"), int(a.env_size, 0)))
    except Refuse as e:
        print("fwenv: REFUSE %s" % e)
        sys.exit(2)


if __name__ == "__main__":
    main()
