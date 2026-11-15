#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compute the one environment edit of the DK's standalone default, on an image.

    tools/mp2s_env_plan.py default IMAGE OUT --count N|forever --record RECORDFILE [LAYOUT]
    tools/mp2s_env_plan.py nodefault IMAGE OUT --record RECORDFILE [LAYOUT]
    tools/mp2s_env_plan.py verify BEFORE AFTER --expect default|nodefault [LAYOUT]
  LAYOUT: as tools/uboot_env.py (--size, --redundant, --offset, --offset2,
  --second); default and nodefault then also take --second-out, verify takes
  --second-after for AFTER's second file.

Why: docs/notes/stm32mp257f-dk-standalone.md sections 3, 5 and 7. The runner
tools/board-stm32mp2-standalone.sh reads the `u-boot-env` partition with dd,
calls this tool on the host and writes the result back. This tool imports
tools/uboot_env.py as a library (set/unset semantics: the inactive copy is
written with flags+1) and never changes it.

default: in ONE edit writes mp2s_ver=1, mp2s_left=N, mp2s_prefixes=<the live
boot_prefixes, byte for byte> and boot_prefixes='/yakogl/standalone/ <live>'.
RECORDFILE gets the live boot_prefixes value, exactly those bytes (no name, no
newline); the undo lines of the note's section 7 are printed on stdout
before OUT is written. Refused unless both CRCs are valid, boot_prefixes is
set, mp2s_left, mp2s_prefixes and mp2s_ver are absent, no yako_* or mp2z_*
variable exists, boot_prefixes does not start with '/yakogl/standalone/', and N
is 1-9 or forever.

nodefault: when boot_prefixes starts with '/yakogl/standalone/' it must
have mp2s_prefixes set, which must equal RECORDFILE byte for byte; then
boot_prefixes is set back to it. mp2s_ver, mp2s_left and mp2s_prefixes are
deleted. With nothing to do the tool exits 0 and OUT is IMAGE's bytes.

verify: reads AFTER again (as from fw_printenv or a dd read-back) and checks
that the changes from BEFORE are those --expect names and nothing else.

Write guard (one function, write_guard, used by every write): a change to
any name outside the four (mp2s_ver, mp2s_left, mp2s_prefixes, boot_prefixes)
refuses. In particular bootcmd, bootcmd_stm32mp, altbootcmd, bootcount,
bootlimit, upgrade_available, boot_targets, preboot, boot_syslinux_conf,
boot_extlinux and boot_scripts keep their bytes and their place.

A refusal is one line on stderr, exit 2 and no output file (no OUT, no
RECORDFILE).
"""
import argparse
import importlib.util
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("uboot_env", os.path.join(HERE, "uboot_env.py"))
ue = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ue)

Refused = ue.Refused
ALLOWED = (b"mp2s_ver", b"mp2s_left", b"mp2s_prefixes", b"boot_prefixes")
OURS = (b"mp2s_ver", b"mp2s_left", b"mp2s_prefixes")
PREFIX = b"/yakogl/standalone/"
HEAD = PREFIX + b" "
PROG = "mp2s_env_plan.py"
COUNTS = [x.encode() for x in "123456789"] + [b"forever"]


def table(entries):
    """name -> value, the last entry of a name winning as U-Boot reads it."""
    return {ue.name_of(e): ue.value_of(e) for e in entries}


def write_guard(before, after):
    """Refuse unless after differs from before only in the ALLOWED names.

    Entries of every other name must be identical, in bytes and in order,
    duplicates included. Every write goes through here."""
    keep = lambda es: [e for e in es if ue.name_of(e) not in ALLOWED]
    if keep(before) != keep(after):
        b, a = table(before), table(after)
        bad = sorted(n for n in set(b) | set(a) if n not in ALLOWED and b.get(n) != a.get(n))
        raise Refused("write guard: the edit would change %s"
                      % (", ".join(ue.text(n) for n in bad) or "the order of other variables"))


def plan(img, out_paths, fn):
    """The guarded edit of img through uboot_env.edit; (message, files)."""
    def guarded(entries):
        new = fn(entries)
        write_guard(entries, new)
        return new
    return ue.edit(img, guarded, out_paths)


def undo_lines(count):
    lines = ["UNDO, from Linux: tools/board-stm32mp2-standalone.sh nodefault"]
    if count == "forever":
        lines += [
            "UNDO, without Linux: a person at the U-Boot prompt (a key at \"Hit any key to "
            "stop autoboot\"): setenv mp2s_left 0; saveenv; boot",
            "UNDO, without Linux, not relying on our script: a person at the U-Boot prompt: "
            "setenv boot_prefixes '<the value in the record file>'; setenv mp2s_left; "
            "setenv mp2s_prefixes; setenv mp2s_ver; saveenv; boot "
            "(no console: the card in a PC, delete yakogl/standalone/boot.scr.uimg on bootfs)",
        ]
    else:
        lines.append("UNDO, without Linux: none needed: after %s boots the script restores "
                     "boot_prefixes and boots Linux" % count)
    return lines


def load(path, a, second):
    img = ue.Image(path, a, second)
    img.active()  # refuses on a bad CRC in either copy
    return img


def live(img):
    return img.copies[img.active()].entries


def outputs(a):
    outs = {a.image: a.out}
    if a.second:
        outs[a.second] = a.second_out
    return outs


def cmd_default(a):
    if a.count.encode() not in COUNTS:
        raise Refused("--count must be 1-9 or forever, not %r" % a.count)
    img = load(a.image, a, a.second)
    t = table(live(img))
    if b"boot_prefixes" not in t:
        raise Refused("boot_prefixes is not set")
    have = [ue.text(n) for n in OURS if n in t]
    if have:
        raise Refused("a default is already set: %s present" % ", ".join(have))
    other = sorted(n for n in t if n.startswith((b"yako_", b"mp2z_")))
    if other:
        raise Refused("%s present (a one-shot is armed or left over)"
                      % ", ".join(ue.text(n) for n in other))
    cur = t[b"boot_prefixes"]
    if cur.startswith(PREFIX):
        raise Refused("boot_prefixes already starts with /yakogl/standalone/")
    assign = [b"mp2s_ver=1", b"mp2s_left=" + a.count.encode(), b"mp2s_prefixes=" + cur,
              b"boot_prefixes=" + HEAD + cur]
    outs = outputs(a)
    msg, files = plan(img, outs, lambda e: ue.apply_set(e, assign))
    with open(a.record, "wb") as fh:
        fh.write(cur)
    for line in undo_lines(a.count):
        print(line)
    sys.stdout.flush()
    for p, data in files.items():
        ue.write_out(p, data)
    print(msg)
    return 0


def cmd_nodefault(a):
    img = load(a.image, a, a.second)
    t = table(live(img))
    outs = outputs(a)
    extra = sorted(n for n in t if n.startswith(b"mp2s_") and n not in OURS)
    if extra:
        raise Refused("unexpected variable %s: needs a person" % ue.text(extra[0]))
    bp = t.get(b"boot_prefixes")
    assign, drop = [], [n for n in OURS if n in t]
    if bp is not None and bp.startswith(PREFIX):
        if not bp.startswith(HEAD) or b"mp2s_prefixes" not in t:
            raise Refused("boot_prefixes has our prefix but mp2s_prefixes is not set: "
                          "needs a person")
        try:
            with open(a.record, "rb") as fh:
                rec = fh.read()
        except OSError as e:
            raise Refused("cannot read the record: %s" % e.strerror)
        if t[b"mp2s_prefixes"] != rec:
            raise Refused("mp2s_prefixes differs from the record %s" % a.record)
        assign = [b"boot_prefixes=" + rec]
    if not assign and not drop:
        for p, q in outs.items():
            shutil.copyfile(p, q + ".tmp")
            os.replace(q + ".tmp", q)
        print("nothing to undo: no default is set")
        return 0

    def fn(entries):
        if assign:
            entries = ue.apply_set(entries, assign)
        return ue.apply_unset(entries, drop) if drop else entries
    msg, files = plan(img, outs, fn)
    print("UNDO: nothing to undo: nodefault only removes")
    sys.stdout.flush()
    for p, data in files.items():
        ue.write_out(p, data)
    print(msg)
    return 0


def cmd_verify(a):
    b = table(live(load(a.before, a, a.second)))
    t = table(live(load(a.after, a, a.second_after)))
    changed = {n for n in set(b) | set(t) if b.get(n) != t.get(n)}
    bad = sorted(n for n in changed if n not in ALLOWED)
    if bad:
        raise Refused("verify: changed outside the allowed names: %s"
                      % ", ".join(ue.text(n) for n in bad))
    if a.expect == "default":
        cur = b.get(b"boot_prefixes")
        if cur is None:
            raise Refused("verify: BEFORE has no boot_prefixes")
        want = {b"mp2s_ver": b"1", b"mp2s_prefixes": cur, b"boot_prefixes": HEAD + cur}
        for n, v in want.items():
            if t.get(n) != v:
                raise Refused("verify: %s is not as default sets it" % ue.text(n))
        if t.get(b"mp2s_left") not in COUNTS:
            raise Refused("verify: mp2s_left is not 1-9 or forever")
        if changed != set(ALLOWED):
            raise Refused("verify: the changed names are not exactly the four of default")
    else:
        left = [ue.text(n) for n in OURS if n in t]
        if left:
            raise Refused("verify: %s still set" % ", ".join(left))
        if b.get(b"boot_prefixes", b"").startswith(HEAD):
            if t.get(b"boot_prefixes") != b.get(b"mp2s_prefixes"):
                raise Refused("verify: boot_prefixes is not restored to mp2s_prefixes")
        elif t.get(b"boot_prefixes") != b.get(b"boot_prefixes"):
            raise Refused("verify: boot_prefixes changed though it had no prefix")
    print("verify %s: ok, changed: %s"
          % (a.expect, ", ".join(ue.text(n) for n in sorted(changed)) or "nothing"))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 epilog=__doc__.split("\n", 2)[2])
    sub = ap.add_subparsers(dest="cmd")
    d = sub.add_parser("default")
    n = sub.add_parser("nodefault")
    v = sub.add_parser("verify")
    for p in (d, n, v):
        ue.layout_args(p, False)
    for p in (d, n):
        p.add_argument("image")
        p.add_argument("out")
        p.add_argument("--record", required=True)
        p.add_argument("--second-out")
    d.add_argument("--count", required=True)
    v.add_argument("before")
    v.add_argument("after")
    v.add_argument("--expect", required=True, choices=("default", "nodefault"))
    v.add_argument("--second-after")
    try:
        a = ap.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    if a.cmd is None:
        ap.print_usage(sys.stderr)
        return 2
    try:
        if a.cmd == "verify":
            if bool(a.second) != bool(a.second_after):
                raise Refused("--second and --second-after go together")
        elif bool(a.second) != bool(a.second_out):
            raise Refused("--second and --second-out go together")
        return {"default": cmd_default, "nodefault": cmd_nodefault, "verify": cmd_verify}[a.cmd](a)
    except Refused as e:
        print("%s: refused: %s" % (PROG, e), file=sys.stderr)
        return 2
    except OSError as e:
        print("%s: refused: %s: %s" % (PROG, e.filename, e.strerror), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
