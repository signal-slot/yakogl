#!/usr/bin/env python3
"""Refuse unless /boot is the microSD's bootfs, never the eMMC.

Pure function over captured text (docs/notes/stm32mp257f-dk-standalone.md
sections 1, 7 and 8). Never runs anything on a board, never opens a device.

Inputs, as three files or one JSON bundle on stdin:
  --findmnt FILE    `findmnt -J /boot`
  --lsblk FILE      `lsblk -J -o NAME,KNAME,PKNAME,PARTLABEL,FSTYPE,MOUNTPOINT,TYPE,TRAN,SUBSYSTEMS`
  --disk-type FILE  /sys/block/<disk>/device/type of the parent disk
                    ("SD" or "MMC"); lines "<disk> <type>" are also read
  bundle: {"findmnt": ..., "lsblk": ..., "disk_type": ...}, each value
          the captured text, or parsed JSON; disk_type may be a
          {disk: type} object.

Both disks may carry the same partition labels, so the label alone never
decides: the mounted partition is found by kernel name in lsblk, and its
parent disk is what must read SD.

Exit 0 ok, 1 refuse, 2 unusable input.
"""
import argparse
import json
import sys

# section 1: the card's ST partitions (bootfs included)
EXPECTED = ("fsbla1", "fsbla2", "metadata1", "metadata2",
            "fip-a", "fip-b", "u-boot-env", "bootfs")

COLLECT = """\
findmnt -J /boot
lsblk -J -o NAME,KNAME,PKNAME,PARTLABEL,FSTYPE,MOUNTPOINT,TYPE,TRAN,SUBSYSTEMS
for d in /sys/block/mmcblk*; do echo "$(basename $d) $(cat $d/device/type)"; done
"""


class Unusable(Exception):
    pass


def as_json(v, what):
    if isinstance(v, (dict, list)):
        return v
    if not isinstance(v, str) or not v.strip():
        raise Unusable("%s is empty" % what)
    try:
        return json.loads(v)
    except ValueError as e:
        raise Unusable("%s is not JSON: %s" % (what, e))


def parse_types(v):
    """-> ({disk: type}, bare type or None)"""
    if isinstance(v, dict):
        return {str(k): str(t).strip() for k, t in v.items()}, None
    if not isinstance(v, str) or not v.strip():
        raise Unusable("disk type is empty")
    named = {}
    bare = None
    for line in v.splitlines():
        w = line.split()
        if len(w) == 1:
            bare = w[0]
        elif len(w) == 2:
            named[w[0]] = w[1]
        elif w:
            raise Unusable("disk type line not understood: %r" % line)
    if bare is not None and named:
        raise Unusable("disk type mixes bare and named lines")
    if bare is not None and bare not in ("SD", "MMC"):
        raise Unusable("disk type %r is neither SD nor MMC" % bare)
    return named, bare


def walk(nodes, parent=None):
    for n in nodes:
        yield n, parent
        yield from walk(n.get("children") or [], n)


def decide(findmnt, lsblk, disk_type):
    """-> (ok, reason). Raises Unusable."""
    fm = as_json(findmnt, "findmnt")
    lb = as_json(lsblk, "lsblk")
    types, bare = parse_types(disk_type)
    if not isinstance(fm, dict) or not isinstance(lb, dict) \
            or not isinstance(lb.get("blockdevices"), list):
        raise Unusable("findmnt/lsblk JSON has the wrong shape")
    fss = fm.get("filesystems")
    if not isinstance(fss, list):
        raise Unusable("findmnt JSON has no filesystems list")
    if not fss:
        return False, "/boot is not mounted"
    fs = fss[0]
    if not isinstance(fs, dict):
        raise Unusable("findmnt entry has the wrong shape")
    if fs.get("target") != "/boot":
        return False, "findmnt target is %r, not /boot" % fs.get("target")
    if fs.get("fstype") != "ext4":
        return False, "/boot is %r, not ext4" % fs.get("fstype")
    src = fs.get("source") or ""
    if not src.startswith("/dev/") or "[" in src:
        return False, "/boot source %r is not a plain block device" % src
    kname = src[len("/dev/"):]

    nodes = list(walk(lb["blockdevices"]))
    hits = [(n, p) for n, p in nodes if kname in (n.get("kname"), n.get("name"))]
    if len(hits) != 1:
        return False, "%s found %d times in lsblk" % (src, len(hits))
    part, disk = hits[0]
    if part.get("type") != "part" or disk is None:
        return False, "%s is not a partition" % src
    if part.get("partlabel") != "bootfs":
        return False, "%s is labelled %r, not bootfs" % (src, part.get("partlabel"))
    if part.get("fstype") != "ext4":
        return False, "%s fstype in lsblk is %r, not ext4" % (src, part.get("fstype"))
    dname = disk.get("kname") or disk.get("name")
    if disk.get("type") != "disk" or not str(dname).startswith("mmcblk"):
        return False, "parent %r of %s is not an mmcblk disk" % (dname, src)

    if bare is not None:
        dt = bare
    elif dname in types:
        dt = types[dname]
    else:
        raise Unusable("no device type captured for %s" % dname)
    if dt != "SD":
        return False, "parent disk %s is device type %r, not SD" % (dname, dt)

    have = {c.get("partlabel") for c in disk.get("children") or []}
    missing = [e for e in EXPECTED if e not in have]
    if missing:
        return False, "disk %s lacks ST partitions: %s" % (dname, " ".join(missing))
    return True, "/boot is %s (bootfs, ext4) on SD disk %s with all ST partitions" % (src, dname)


def read(path, what):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError as e:
        raise Unusable("cannot read %s: %s" % (what, e.strerror or e))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--findmnt")
    ap.add_argument("--lsblk")
    ap.add_argument("--disk-type")
    ap.add_argument("--bundle", action="store_true",
                    help="read one JSON bundle from stdin")
    ap.add_argument("--collect", action="store_true",
                    help="print the read-only commands to run over ssh; run nothing")
    a = ap.parse_args(argv)
    if a.collect:
        sys.stdout.write(COLLECT)
        return 0
    try:
        if a.bundle or not (a.findmnt or a.lsblk or a.disk_type):
            b = as_json(sys.stdin.read(), "bundle")
            if not isinstance(b, dict) or not all(
                    k in b for k in ("findmnt", "lsblk", "disk_type")):
                raise Unusable("bundle needs findmnt, lsblk and disk_type")
            args = (b["findmnt"], b["lsblk"], b["disk_type"])
        else:
            if not (a.findmnt and a.lsblk and a.disk_type):
                raise Unusable("need --findmnt, --lsblk and --disk-type, or a bundle on stdin")
            args = (read(a.findmnt, "findmnt"), read(a.lsblk, "lsblk"),
                    read(a.disk_type, "disk type"))
        ok, why = decide(*args)
    except Unusable as e:
        print("unusable: %s" % e)
        return 2
    print("%s: %s" % ("ok" if ok else "refuse", why))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
