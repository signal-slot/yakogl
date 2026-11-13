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

    print("uboot_env_mp2 selftest: %d ok, %d failed" % (ok, failed))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
