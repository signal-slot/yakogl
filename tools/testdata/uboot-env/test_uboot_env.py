# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""tools/uboot_env.py through its command line, on synthetic images.

Run by tools/uboot_env_selftest.sh. Every image is built here from the
format (a little-endian zlib CRC-32 over the data area, a flag byte for a
redundant copy, NUL-terminated strings, an extra NUL, padding), not with the
tool, and no byte comes from a board.
"""
import os
import subprocess
import sys
import tempfile
import unittest
import zlib

TOOL = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "uboot_env.py")

VARS = [b"bootdelay=1", b"bootcmd=run bootcmd_stm32mp", b"boot_targets=mmc0 mmc1",
        b"fdtfile=st/board.dtb"]


def copy(entries, size, pad=0x00, flags=None):
    """One environment copy of size bytes."""
    hdr = 4 if flags is None else 5
    data = b"".join(e + b"\0" for e in entries) + b"\0"
    area = data + bytes([pad]) * (size - hdr - len(data))
    assert len(area) == size - hdr
    h = zlib.crc32(area).to_bytes(4, "little")
    return h + (b"" if flags is None else bytes([flags])) + area


class Env(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def path(self, name, data=None):
        p = os.path.join(self.d, name)
        if data is not None:
            with open(p, "wb") as fh:
                fh.write(data)
        return p

    def read(self, name):
        with open(self.path(name), "rb") as fh:
            return fh.read()

    def run_tool(self, *args):
        return subprocess.run([sys.executable, "-I", TOOL] + list(args), capture_output=True)

    def ok(self, *args):
        r = self.run_tool(*args)
        self.assertEqual(r.returncode, 0, r.stderr.decode())
        return r

    def refused(self, *args, out=None):
        r = self.run_tool(*args)
        self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
        self.assertIn(b"refused", r.stderr)
        if out:
            self.assertFalse(os.path.exists(self.path(out)), "output written on a refusal")
            self.assertFalse(os.path.exists(self.path(out) + ".tmp"))
        return r


class Single(Env):
    def test_print(self):
        img = self.path("env", copy(VARS, 0x400))
        r = self.ok("print", img)
        self.assertEqual(r.stdout, b"".join(v + b"\n" for v in VARS))
        r = self.ok("print", img, "bootcmd")
        self.assertEqual(r.stdout, b"bootcmd=run bootcmd_stm32mp\n")
        r = self.ok("print", "-n", img, "boot_targets")
        self.assertEqual(r.stdout, b"mmc0 mmc1\n")
        r = self.run_tool("print", img, "missing")
        self.assertEqual(r.returncode, 1)
        self.assertIn(b"not defined", r.stderr)

    def test_check(self):
        img = self.path("env", copy(VARS, 0x400))
        r = self.ok("check", img)
        self.assertIn(b"ok  copy 1", r.stdout)
        self.assertIn(b"4 variables", r.stdout)

    def roundtrip(self, pad):
        orig = copy(VARS, 0x400, pad)
        img = self.path("in", orig)
        r = self.ok("set", img, self.path("set"), "yako_once=run yako_boot")
        self.assertIn(b"copy 1", r.stdout)
        s = self.read("set")
        self.assertEqual(len(s), len(orig))
        data = b"".join(e + b"\0" for e in VARS)
        self.assertEqual(s[4:4 + len(data)], data, "existing variables keep order and bytes")
        new = b"yako_once=run yako_boot\0\0"
        self.assertEqual(s[4 + len(data):4 + len(data) + len(new)], new, "new one appended")
        self.assertEqual(s[4 + len(data) + len(new):], orig[4 + len(data) + len(new):],
                         "padding kept")
        self.assertEqual(int.from_bytes(s[:4], "little"), zlib.crc32(s[4:]))
        self.assertEqual(s, copy(VARS + [b"yako_once=run yako_boot"], 0x400, pad))
        self.ok("check", self.path("set"))
        self.ok("unset", self.path("set"), self.path("back"), "yako_once")
        self.assertEqual(self.read("back"), orig, "set then unset is the input byte for byte")

    def test_roundtrip_pad00(self):
        self.roundtrip(0x00)

    def test_roundtrip_padff(self):
        self.roundtrip(0xff)

    def test_change_in_place(self):
        img = self.path("in", copy(VARS, 0x400, 0xff))
        self.ok("set", img, self.path("out"), "bootdelay=0")
        self.assertEqual(self.read("out"),
                         copy([b"bootdelay=0"] + VARS[1:], 0x400, 0xff))
        self.ok("set", img, self.path("out2"), "bootcmd=x", "new=1")
        self.assertEqual(self.read("out2"),
                         copy([VARS[0], b"bootcmd=x"] + VARS[2:] + [b"new=1"], 0x400, 0xff),
                         "a shorter value: the freed bytes take the padding")

    def test_unset_absent_and_value_with_equals(self):
        orig = copy(VARS, 0x400)
        img = self.path("in", orig)
        self.ok("unset", img, self.path("out"), "nothere")
        self.assertEqual(self.read("out"), orig)
        self.ok("set", img, self.path("out2"), "args=a=b c")
        r = self.ok("print", "-n", self.path("out2"), "args")
        self.assertEqual(r.stdout, b"a=b c\n")

    def test_bytes_outside_untouched(self):
        # The env is the last 0x200 bytes of a 0x1000 "partition", like
        # U-Boot's env at the top of an mmc partition.
        head = bytes((i * 7) & 0xff for i in range(0xe00))
        orig = head + copy(VARS, 0x200)
        img = self.path("part", orig)
        self.refused("check", img)
        self.ok("--offset", "0xe00", "--size", "0x200", "check", img)
        self.ok("set", img, self.path("out"), "--offset", "0xe00", "--size", "0x200", "x=1")
        out = self.read("out")
        self.assertEqual(out[:0xe00], head)
        self.assertEqual(out[0xe00:], copy(VARS + [b"x=1"], 0x200))

    def test_size_smaller_than_file(self):
        orig = copy(VARS, 0x100) + b"\xa5" * 0x40
        img = self.path("in", orig)
        self.ok("set", img, self.path("out"), "--size", "256", "x=1")
        out = self.read("out")
        self.assertEqual(out[0x100:], b"\xa5" * 0x40)
        self.assertEqual(out[:0x100], copy(VARS + [b"x=1"], 0x100))

    def test_bad_crc_refused(self):
        bad = bytearray(copy(VARS, 0x400))
        bad[20] ^= 0x01
        img = self.path("bad", bytes(bad))
        r = self.refused("check", img)
        self.assertIn(b"BAD copy 1", r.stdout)
        self.refused("print", img)
        self.refused("set", img, self.path("out"), "x=1", out="out")
        self.refused("unset", img, self.path("out"), "bootdelay", out="out")
        bad2 = bytearray(copy(VARS, 0x400, 0xff))
        bad2[0x3ff] = 0x00
        self.refused("check", self.path("bad2", bytes(bad2)))

    def test_overflow_refused(self):
        full = copy(VARS, 4 + sum(len(v) + 1 for v in VARS) + 1)
        img = self.path("full", full)
        self.ok("check", img)
        self.refused("set", img, self.path("out"), "x=1", out="out")
        self.refused("set", img, self.path("out"), "bootdelay=10", out="out")
        # Same length fits exactly.
        self.ok("set", img, self.path("out"), "bootdelay=2")
        # No terminating empty string inside the area: the data does not fit.
        area = b"a=1\0b=2"
        nofit = zlib.crc32(area).to_bytes(4, "little") + area
        r = self.refused("check", self.path("nofit", nofit))
        self.assertIn(b"do not end", r.stdout)
        self.refused("set", self.path("nofit"), self.path("out3"), "x=1", out="out3")
        # --size larger than the file.
        self.refused("--size", "0x800", "check", img)

    def test_bad_names_and_values_refused(self):
        img = self.path("in", copy(VARS, 0x400))
        for arg in ("=1", "noequals", "bootdelay="):
            self.refused("set", img, self.path("out"), arg, out="out")
        for name in ("a=b", ""):
            self.refused("unset", img, self.path("out"), name, out="out")
        # A refusal later in the list writes nothing either.
        self.refused("set", img, self.path("out"), "good=1", "=bad", out="out")

    def test_nul_refused(self):
        # argv cannot carry NUL; call the functions the CLI uses.
        sys.path.insert(0, os.path.dirname(TOOL))
        sys.dont_write_bytecode = True
        try:
            import uboot_env
        finally:
            sys.path.pop(0)
        with self.assertRaises(uboot_env.Refused):
            uboot_env.apply_set([], [b"a\0b=1"])
        with self.assertRaises(uboot_env.Refused):
            uboot_env.apply_set([], [b"a=1\0"])
        with self.assertRaises(uboot_env.Refused):
            uboot_env.apply_unset([], [b"a\0"])

    def test_diff(self):
        a = self.path("a", copy(VARS, 0x400))
        b = self.path("b", copy([VARS[0], b"bootcmd=other"] + VARS[3:] + [b"new=1"], 0x400))
        self.assertEqual(self.ok("diff", a, a).stdout, b"")
        r = self.run_tool("diff", a, b)
        self.assertEqual(r.returncode, 1)
        self.assertEqual(r.stdout.splitlines(), [
            b"changed bootcmd: run bootcmd_stm32mp -> other",
            b"removed boot_targets=mmc0 mmc1",
            b"added   new=1"])


class Redundant(Env):
    SIZE = 0x200

    def pair(self, f1, f2, e1=VARS, e2=VARS, pad=0x00):
        return copy(e1, self.SIZE, pad, f1) + copy(e2, self.SIZE, pad, f2)

    def active(self, img, *extra):
        r = self.ok("--redundant", "check", img, *extra)
        return r.stdout.splitlines()[-1]

    def test_flag_selection(self):
        old, new = VARS, VARS + [b"which=new"]
        for f1, f2, want in ((1, 2, b"copy 2"), (2, 1, b"copy 1"), (5, 5, b"copy 1"),
                             (0xff, 0x00, b"copy 2"), (0x00, 0xff, b"copy 1"),
                             (0xfe, 0xff, b"copy 2")):
            e1, e2 = (new, old) if want == b"copy 1" else (old, new)
            img = self.path("p", self.pair(f1, f2, e1, e2))
            self.assertEqual(self.active(img), b"active: " + want, (f1, f2))
            r = self.ok("--redundant", "print", "-n", img, "which")
            self.assertEqual(r.stdout, b"new\n", (f1, f2))

    def test_set_writes_the_inactive_copy(self):
        orig = self.pair(3, 4, VARS, VARS + [b"x=1"])
        img = self.path("p", orig)
        r = self.ok("set", "--redundant", img, self.path("out"), "y=2")
        self.assertIn(b"copy 1 (bytes 0x0..0x1ff", r.stdout)
        self.assertIn(b"flags 0x05", r.stdout)
        out = self.read("out")
        self.assertEqual(out[self.SIZE:], orig[self.SIZE:], "the active copy is left alone")
        self.assertEqual(out[:self.SIZE], copy(VARS + [b"x=1", b"y=2"], self.SIZE, 0, 5))
        self.assertEqual(self.active(self.path("out")), b"active: copy 1")
        # And again: now copy 2 is written with 6.
        r = self.ok("unset", "--redundant", self.path("out"), self.path("out2"), "y")
        self.assertIn(b"copy 2 (bytes 0x200..0x3ff", r.stdout)
        out2 = self.read("out2")
        self.assertEqual(out2[:self.SIZE], out[:self.SIZE])
        self.assertEqual(out2[self.SIZE:], copy(VARS + [b"x=1"], self.SIZE, 0, 6))

    def test_wraparound(self):
        img = self.path("p", self.pair(0xfe, 0xff))
        self.ok("set", "--redundant", img, self.path("out"), "a=1")
        out = self.read("out")
        self.assertEqual(out[4], 0x00, "0xff + 1 wraps to 0x00 in copy 1")
        self.assertEqual(self.active(self.path("out")), b"active: copy 1")
        self.ok("set", "--redundant", self.path("out"), self.path("out2"), "b=2")
        out2 = self.read("out2")
        self.assertEqual(out2[self.SIZE + 4], 0x01)
        self.assertEqual(self.active(self.path("out2")), b"active: copy 2")
        r = self.ok("--redundant", "print", self.path("out2"))
        self.assertEqual(r.stdout.splitlines()[-2:], [b"a=1", b"b=2"])

    def test_paddings(self):
        for pad in (0x00, 0xff):
            orig = self.pair(1, 2, pad=pad)
            img = self.path("p", orig)
            self.ok("set", "--redundant", img, self.path("out"), "a=1")
            self.ok("unset", "--redundant", self.path("out"), self.path("back"), "a")
            back = self.read("back")
            self.assertEqual(back[:self.SIZE], copy(VARS + [b"a=1"], self.SIZE, pad, 3), pad)
            self.assertEqual(back[self.SIZE:], copy(VARS, self.SIZE, pad, 4), pad)

    def test_bad_copy_refused(self):
        for which in (0, 1):
            p = bytearray(self.pair(1, 2))
            p[which * self.SIZE + 30] ^= 0x10
            img = self.path("p", bytes(p))
            r = self.refused("--redundant", "check", img)
            self.assertIn(b"BAD copy %d" % (which + 1), r.stdout)
            self.assertIn(b"ok  copy %d" % (2 - which), r.stdout)
            self.refused("set", "--redundant", img, self.path("out"), "a=1", out="out")
            self.refused("--redundant", "print", img)

    def test_overflow_refused(self):
        size = 5 + sum(len(v) + 1 for v in VARS) + 1
        p = copy(VARS, size, 0, 1) + copy(VARS, size, 0, 2)
        img = self.path("p", p)
        self.refused("set", "--redundant", "--size", str(size), img, self.path("out"), "a=1",
                     out="out")

    def test_offsets_inside_a_partition(self):
        # Copy 1 above copy 2 at the top of a partition, as env/mmc.c puts
        # them; the rest of the partition is not touched.
        filler = b"\x5a" * 0x100
        orig = filler + copy(VARS, self.SIZE, 0xff, 8) + copy(VARS, self.SIZE, 0xff, 9)
        img = self.path("part", orig)
        lay = ["--size", "0x200", "--offset", "0x300", "--offset2", "0x100"]
        r = self.ok("check", img, *lay)
        self.assertIn(b"active: copy 1", r.stdout)
        r = self.ok("set", img, self.path("out"), "a=1", *lay)
        self.assertIn(b"copy 2 (bytes 0x100..0x2ff", r.stdout)
        out = self.read("out")
        self.assertEqual(out[:0x100], filler)
        self.assertEqual(out[0x300:], orig[0x300:])
        self.assertEqual(out[0x100:0x300], copy(VARS + [b"a=1"], self.SIZE, 0xff, 10))
        self.refused("check", img, "--size", "0x200", "--offset", "0x100",
                     "--offset2", "0x200")

    def test_two_files(self):
        c1 = copy(VARS, self.SIZE, 0, 7)
        c2 = copy(VARS + [b"z=9"], self.SIZE, 0, 8)
        f1, f2 = self.path("c1", c1), self.path("c2", c2)
        r = self.ok("--second", f2, "check", f1)
        self.assertIn(b"active: copy 2", r.stdout)
        r = self.ok("set", f1, self.path("o1"), "--second", f2, "--second-out",
                    self.path("o2"), "q=1")
        self.assertIn(b"copy 1", r.stdout)
        self.assertEqual(self.read("o2"), c2)
        self.assertEqual(self.read("o1"), copy(VARS + [b"z=9", b"q=1"], self.SIZE, 0, 9))
        r = self.run_tool("diff", f1, self.path("o1"), "--second", f2,
                          "--second-b", self.path("o2"))
        self.assertEqual((r.returncode, r.stdout), (1, b"added   q=1\n"))


if __name__ == "__main__":
    unittest.main(verbosity=1)
