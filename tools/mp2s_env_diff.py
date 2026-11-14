#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Before/after U-Boot environment verdict for the DK standalone default.

    tools/mp2s_env_diff.py BEFORE AFTER                   [--image LAYOUT]
    tools/mp2s_env_diff.py BEFORE MID --expect-default [N] [--image LAYOUT]

BEFORE and AFTER are `fw_printenv` text dumps (name=value per line, the value
may hold '='; the last line of a name wins), or with --image raw U-Boot
environment images read through tools/uboot_env.py (read only), with its
layout flags --size, --offset, --offset2 and --redundant.

It prints the added, removed and changed variables, then exactly one verdict
line, and exits 0 for the first form below and 1 for the second:

  mp2s-env: restored
  mp2s-env: NOT restored: <reasons>

Restored means: boot_prefixes is back byte for byte, no mp2s_* is left, and
bootcmd, bootcount, altbootcmd, bootlimit and upgrade_available are the same
as before (set or unset alike). Other differences are listed, not judged.

--expect-default checks a mid-run state instead (verdict lines
`mp2s-env: default set` / `mp2s-env: NOT default set: <reasons>`): mp2s_ver=1,
mp2s_left N (any of 1..9 without N), mp2s_prefixes equal to the before
boot_prefixes, and boot_prefixes equal to '/yakogl/standalone/ ' plus the
before value.

Values of ethaddr, serial#, ipaddr, serverip and other *addr* variables are
printed as <redacted>; load addresses (loadaddr, scriptaddr, *_addr_r,
fdt_addr, kernel_addr) are not. Reasons name variables, never values.
Exit 2 (one line on stderr) when an input cannot be read.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import uboot_env  # noqa: E402

KEEP = ("bootcmd", "bootcount", "altbootcmd", "bootlimit", "upgrade_available")
PREFIX = "/yakogl/standalone/ "
SECRET = re.compile(r"ethaddr|serial#|ipaddr|serverip|addr")
LOAD = re.compile(r"^(loadaddr|scriptaddr|fdt_addr|kernel_addr|ramdisk_addr|fdtaddr)$|_addr_r$")


def shown(name, value):
    if value is None:
        return "<unset>"
    if SECRET.search(name) and not LOAD.search(name):
        return "<redacted>"
    return value


def load_dump(path):
    env = {}
    with open(path, encoding="utf-8", errors="surrogateescape") as fh:
        for line in fh:
            line = line.rstrip("\r\n")
            if "=" not in line or line.startswith("#"):
                continue
            name, value = line.split("=", 1)
            if name:
                env[name] = value
    return env


def load_image(path, a):
    img = uboot_env.Image(path, a)
    copy = img.copies[img.active()]
    env = {}
    for e in copy.entries:
        n, v = uboot_env.name_of(e), uboot_env.value_of(e)
        env[n.decode("utf-8", "surrogateescape")] = v.decode("utf-8", "surrogateescape")
    return env


def report(before, after):
    for n in sorted(set(before) | set(after)):
        b, a = before.get(n), after.get(n)
        if b == a:
            continue
        if b is None:
            print("added   %s=%s" % (n, shown(n, a)))
        elif a is None:
            print("removed %s=%s" % (n, shown(n, b)))
        else:
            print("changed %s: %s -> %s" % (n, shown(n, b), shown(n, a)))


def restored(before, after):
    why = ["%s is still set" % n for n in sorted(after) if n.startswith("mp2s_")]
    if before.get("boot_prefixes") != after.get("boot_prefixes"):
        why.append("boot_prefixes differs")
    why += ["%s differs" % n for n in KEEP if before.get(n) != after.get(n)]
    return why


def default_set(before, mid, left):
    why = []
    if mid.get("mp2s_ver") != "1":
        why.append("mp2s_ver is not 1")
    got = mid.get("mp2s_left")
    ok = got == left if left else (got is not None and re.fullmatch(r"[1-9]", got))
    if not ok:
        why.append("mp2s_left is not %s" % (left or "1..9"))
    live = before.get("boot_prefixes")
    if live is None:
        why.append("before has no boot_prefixes")
    elif mid.get("mp2s_prefixes") != live:
        why.append("mp2s_prefixes differs from the before boot_prefixes")
    if live is not None and mid.get("boot_prefixes") != PREFIX + live:
        why.append("boot_prefixes is not the standalone prefix plus the before value")
    return why


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("--expect-default", nargs="?", const="", metavar="N")
    ap.add_argument("--image", action="store_true", help="BEFORE and AFTER are raw env images")
    ap.add_argument("--size", type=uboot_env.number)
    ap.add_argument("--offset", type=uboot_env.number, default=0)
    ap.add_argument("--offset2", type=uboot_env.number)
    ap.add_argument("--redundant", action="store_true")
    a = ap.parse_args(argv)
    a.second = None
    try:
        if a.image:
            before, after = load_image(a.before, a), load_image(a.after, a)
        else:
            before, after = load_dump(a.before), load_dump(a.after)
    except (OSError, uboot_env.Refused) as e:
        print("mp2s-env: cannot read: %s" % e, file=sys.stderr)
        return 2
    report(before, after)
    if a.expect_default is None:
        why, ok, bad = restored(before, after), "restored", "NOT restored"
    else:
        why = default_set(before, after, a.expect_default)
        ok, bad = "default set", "NOT default set"
    if why:
        print("mp2s-env: %s: %s" % (bad, "; ".join(why)))
        return 1
    print("mp2s-env: %s" % ok)
    return 0


if __name__ == "__main__":
    sys.exit(main())
