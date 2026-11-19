#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Host self-test of tools/uboot_env.py on the STM32MP257F-DK's environment layout.

ST's U-Boot 2023.10 keeps a redundant environment of 0x2000 bytes per copy in
the u-boot-env partition: copy 1 at 0x7e000, copy 2 at 0x7c000 (below it).
uboot_env.py reads it with
    --size 0x2000 --redundant --offset 0x7e000 --offset2 0x7c000
This builds synthetic partition images in a temporary directory (CRC-32 LE
over the data area, a flag byte, NUL-separated name=value strings, a double
NUL, 0x00 padding) and drives uboot_env.py through its command line.

    python3 -I tools/uboot_env_mp2_selftest.py

No root, no network, no board. Last line: "uboot_env_mp2 selftest: N ok, M failed".
"""
import os
import subprocess
import sys
import tempfile
import zlib

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "uboot_env.py")
SIZE, OFF1, OFF2 = 0x2000, 0x7e000, 0x7c000
PART = 0x80000
LAYOUT = ["--size", "0x2000", "--redundant", "--offset", "0x7e000", "--offset2", "0x7c000"]

ok = failed = 0


def copy(strs, flags, crc_ok=True):
    area = b"".join(s + b"\0" for s in strs) + b"\0"
    area = area.ljust(SIZE - 5, b"\0")
    crc = zlib.crc32(area)
    if not crc_ok:
        crc ^= 0xdeadbeef
    return crc.to_bytes(4, "little") + bytes([flags]) + area


def image(c1, c2):
    b = bytearray(b"\xff" * PART)
    b[OFF1:OFF1 + SIZE] = c1
    b[OFF2:OFF2 + SIZE] = c2
    return bytes(b)


def run(*args):
    p = subprocess.run([sys.executable, "-I", TOOL, *args], capture_output=True)
    return p.returncode, p.stdout.decode(), p.stderr.decode()


def case(name, cond, detail=""):
    global ok, failed
    if cond:
        ok += 1
        print("ok   %s" % name)
    else:
        failed += 1
        print("FAIL %s %s" % (name, detail))


def v(n):
    return [b"bootcmd=run bootcmd_mmc0", b"ver=" + n]


def main():
    with tempfile.TemporaryDirectory() as d:
        def put(name, data):
            p = os.path.join(d, name)
            with open(p, "wb") as f:
                f.write(data)
            return p

        # both valid, the newer flag wins (either copy)
        for name, f1, f2, want in (("copy 1 newer", 5, 4, "c1"), ("copy 2 newer", 4, 5, "c2")):
            img = put("a.img", image(copy(v(b"c1"), f1), copy(v(b"c2"), f2)))
            rc, out, err = run("print", img, "-n", "ver", *LAYOUT)
            case("both valid, %s wins" % name, rc == 0 and out.strip() == want,
                 repr((rc, out, err)))
        img = put("a.img", image(copy(v(b"c1"), 7), copy(v(b"c2"), 7)))
        rc, out, _ = run("print", img, "-n", "ver", *LAYOUT)
        case("equal flags: copy 1", rc == 0 and out.strip() == "c1", repr(out))
        rc, out, _ = run("check", img, *LAYOUT)
        case("check: both ok, active copy 1", rc == 0 and "active: copy 1" in out, out)

        # flag wrap-around: 0xff against 0x00 is the 0x00 one
        img = put("w.img", image(copy(v(b"c1"), 0xff), copy(v(b"c2"), 0x00)))
        rc, out, _ = run("print", img, "-n", "ver", *LAYOUT)
        case("flags 0xff vs 0x00: the wrapped 0x00 copy wins", rc == 0 and out.strip() == "c2", out)
        img = put("w2.img", image(copy(v(b"c1"), 0x00), copy(v(b"c2"), 0xff)))
        rc, out, _ = run("print", img, "-n", "ver", *LAYOUT)
        case("flags 0x00 vs 0xff (copy 1 wrapped): copy 1 wins", rc == 0 and out.strip() == "c1", out)

        # bad CRC: uboot_env.py refuses a pair with a bad copy (its docstring
        # says so); check names the bad copy and the good one
        img = put("b.img", image(copy(v(b"c1"), 9, crc_ok=False), copy(v(b"c2"), 8)))
        rc, out, err = run("print", img, *LAYOUT)
        case("copy 1 bad CRC: print of the pair is refused (exit 2)",
             rc == 2 and "refused" in err and out == "", repr((rc, out, err)))
        rc, out, err = run("check", img, *LAYOUT)
        case("copy 1 bad CRC: check says BAD copy 1 and ok copy 2",
             rc == 2 and "BAD copy 1" in out and out.count("ok  ") == 1, repr((rc, out, err)))
        img2 = put("b2.img", image(copy(v(b"c1"), 9), copy(v(b"c2"), 8, crc_ok=False)))
        rc, out, err = run("check", img2, *LAYOUT)
        case("copy 2 bad CRC: check says BAD copy 2", rc == 2 and "BAD copy 2" in out, out)

        # copy 2 sits below copy 1: only --offset2 0x7c000 finds it
        img = put("o.img", image(copy(v(b"c1"), 5), copy(v(b"c2"), 4)))
        rc, out, err = run("print", img, "-n", "ver", "--size", "0x2000", "--redundant",
                           "--offset", "0x7e000")
        case("default offset2 (0x80000) runs past the partition: refused",
             rc == 2 and "past the end" in err, repr((rc, out, err)))
        rc, out, err = run("check", img, "--size", "0x2000", "--redundant", "--offset", "0x7a000")
        case("default offset2 reads the wrong bytes (copies at 0x7a000/0x7c000: BAD copy 1)",
             rc == 2 and "BAD copy 1" in out, repr((rc, out, err)))
        rc, out, err = run("check", img, *LAYOUT)
        case("with --offset2 0x7c000 both copies are ok", rc == 0 and out.count("ok  ") == 2, out)

        # read-only dump, then compare two identical images
        before = put("before.img", image(copy(v(b"c1"), 5), copy(v(b"c2"), 4)))
        os.chmod(before, 0o444)
        rc, dump1, _ = run("print", before, *LAYOUT)
        after = put("after.img", open(before, "rb").read())
        rc2, dump2, _ = run("print", after, *LAYOUT)
        case("print is read-only and repeatable",
             rc == 0 and rc2 == 0 and dump1 == dump2 and "bootcmd=run bootcmd_mmc0" in dump1
             and open(before, "rb").read() == open(after, "rb").read())
        rc, out, err = run("diff", before, after, *LAYOUT)
        case("diff of two identical images: no difference, exit 0",
             rc == 0 and out == "", repr((rc, out, err)))
        changed = put("changed.img", image(copy(v(b"c1x"), 5), copy(v(b"c2"), 4)))
        rc, out, _ = run("diff", before, changed, *LAYOUT)
        case("diff of images differing in a variable: reported, exit 1",
             rc == 1 and "changed ver: c1 -> c1x" in out, repr((rc, out)))
        case("no subcommand above wrote the image",
             open(before, "rb").read() == image(copy(v(b"c1"), 5), copy(v(b"c2"), 4)))

        # the DK's stock partition: copy 1 valid (flags 0x02), copy 2 all zero
        BLANK = ["--allow-blank-copy"]
        stock = image(copy(v(b"c1"), 2), bytes(SIZE))
        img = put("blank.img", stock)
        rc, out, err = run("check", img, *LAYOUT)
        case("blank copy 2 without the opt-in: refused (BAD copy 2, exit 2), as before",
             rc == 2 and "BAD copy 2" in out, repr((rc, out, err)))
        rc, out, err = run("set", img, os.path.join(d, "x.img"), "yako_once=t", *LAYOUT)
        case("blank copy 2 without the opt-in: set refused, no output file",
             rc == 2 and not os.path.exists(os.path.join(d, "x.img")), repr((rc, out, err)))
        rc, out, err = run("check", img, *LAYOUT, *BLANK)
        case("blank copy 2 with --allow-blank-copy: check exit 0, BLANK copy 2, active copy 1",
             rc == 0 and "BLANK copy 2" in out and "ok  copy 1" in out and "active: copy 1" in out,
             repr((rc, out, err)))
        rc, out, _ = run("print", img, "-n", "ver", *LAYOUT, *BLANK)
        case("blank copy 2 with the opt-in: print reads copy 1", rc == 0 and out.strip() == "c1", out)
        rc, out, err = run("--allow-blank-copy", "check", img, "--size", "0x2000", "--offset", "0x7e000")
        case("--allow-blank-copy without a redundant layout: a usage error (exit 2)",
             rc == 2 and "needs a redundant layout" in err, repr((rc, out, err)))

        # only all-zero is blank: one non-zero byte, or a 0xff copy, is a bad copy
        nz = bytearray(SIZE)
        nz[100] = 1
        img = put("nz.img", image(copy(v(b"c1"), 2), bytes(nz)))
        rc, out, err = run("check", img, *LAYOUT, *BLANK)
        case("copy 2 zero but for one byte (bad CRC): refused even with the opt-in",
             rc == 2 and "BAD copy 2" in out, repr((rc, out, err)))
        img = put("ff.img", image(copy(v(b"c1"), 2), b"\xff" * SIZE))
        rc, out, err = run("set", img, os.path.join(d, "x.img"), "a=b", *LAYOUT, *BLANK)
        case("copy 2 all 0xff (bad CRC): set refused even with the opt-in",
             rc == 2 and not os.path.exists(os.path.join(d, "x.img")), repr((rc, out, err)))
        img = put("bb.img", image(bytes(SIZE), bytes(SIZE)))
        rc, out, err = run("check", img, *LAYOUT, *BLANK)
        case("both copies blank: refused with the opt-in (no valid copy)", rc == 2, repr((rc, out, err)))
        img = put("bad1.img", image(copy(v(b"c1"), 2, crc_ok=False), bytes(SIZE)))
        rc, out, err = run("check", img, *LAYOUT, *BLANK)
        case("copy 1 bad CRC next to a blank copy 2: refused with the opt-in", rc == 2, repr((rc, out, err)))
        img = put("blank1.img", image(bytes(SIZE), copy(v(b"c2"), 3)))
        rc, out, _ = run("print", img, "-n", "ver", *LAYOUT, *BLANK)
        case("blank copy 1 next to a valid copy 2: copy 2 read (either copy may be the blank one)",
             rc == 0 and out.strip() == "c2", out)

        # the edit lands in copy 1, in place, flags 0x02 kept; copy 2 stays zero
        img = put("blank.img", stock)
        armed = os.path.join(d, "armed.img")
        rc, out, err = run("set", img, armed, "yako_once=t1", *LAYOUT, *BLANK)
        new = open(armed, "rb").read() if rc == 0 else b""
        case("set with a blank copy 2: copy 1 rewritten in place, flags 0x02 kept",
             rc == 0 and "wrote copy 1" in out and new[OFF1 + 4] == 2
             and zlib.crc32(new[OFF1 + 5:OFF1 + SIZE]) == int.from_bytes(new[OFF1:OFF1 + 4], "little"),
             repr((rc, out, err)))
        case("set with a blank copy 2: copy 2 still all zero, no byte outside copy 1 changed",
             rc == 0 and new[OFF2:OFF2 + SIZE] == bytes(SIZE)
             and new[:OFF1] == stock[:OFF1] and len(new) == len(stock), "")
        rc, out, _ = run("print", armed, "-n", "yako_once", *LAYOUT, *BLANK)
        case("set with a blank copy 2: the edit reads back from copy 1", rc == 0 and out.strip() == "t1", out)
        undone = os.path.join(d, "undone.img")
        rc, out, _ = run("unset", armed, undone, "yako_once", *LAYOUT, *BLANK)
        case("set then unset with a blank copy 2: the stock image byte for byte",
             rc == 0 and open(undone, "rb").read() == stock, out)

        # U-Boot's first saveenv (the hook's): copy 2 written with flags 0x03;
        # both valid now, the flags pick copy 2, with or without the opt-in
        saved = put("saved.img", image(copy(v(b"c1") + [b"yako_once=t1"], 2), copy(v(b"c2"), 3)))
        for extra, how in (([], "without"), (BLANK, "with")):
            rc, out, _ = run("check", saved, *LAYOUT, *extra)
            case("after U-Boot's saveenv (flags 0x02/0x03) %s the opt-in: both ok, active copy 2" % how,
                 rc == 0 and out.count("ok  ") == 2 and "active: copy 2" in out
                 and "BLANK" not in out, out)
        nxt = os.path.join(d, "next.img")
        rc, out, _ = run("set", saved, nxt, "yako_once=t2", *LAYOUT, *BLANK)
        n2 = open(nxt, "rb").read() if rc == 0 else b""
        case("the next arm after U-Boot's saveenv writes copy 1 with flags 0x04 (the inactive one)",
             rc == 0 and "wrote copy 1" in out and n2[OFF1 + 4] == 4
             and n2[OFF2:OFF2 + SIZE] == open(saved, "rb").read()[OFF2:OFF2 + SIZE], out)

    print("uboot_env_mp2 selftest: %d ok, %d failed" % (ok, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
