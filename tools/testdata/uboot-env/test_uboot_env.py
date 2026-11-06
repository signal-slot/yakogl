# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Host tests of tools/uboot_env.py; every image is built here."""
import os
import subprocess
import sys
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "..", "uboot_env.py")
SIZE = 0x200
VARS = [(b"bootcmd", b"run a; run b"), (b"baudrate", b"115200"), (b"x", b"")]


def image(vs, size=SIZE, pad=0, flag=None):
    hdr = 4 if flag is None else 5
    body = b"".join(k + b"=" + v + b"\0" for k, v in vs) + b"\0"
    data = body + bytes([pad]) * (size - hdr - len(body))
    out = zlib.crc32(data).to_bytes(4, "little")
    if flag is not None:
        out += bytes([flag])
    return out + data


class T(unittest.TestCase):
    def setUp(self):
        self.td = tempfile.TemporaryDirectory()
        self.addCleanup(self.td.cleanup)

    def p(self, name, data=None):
        path = os.path.join(self.td.name, name)
        if data is not None:
            with open(path, "wb") as f:
                f.write(data)
        return path

    def run_tool(self, *a):
        r = subprocess.run([sys.executable, "-I", TOOL, *a],
                           capture_output=True, text=True)
        return r.returncode, r.stdout, r.stderr

    def rd(self, path):
        with open(path, "rb") as f:
            return f.read()

    def test_single_print(self):
        f = self.p("e", image(VARS))
        rc, out, _ = self.run_tool("print", f)
        self.assertEqual((rc, out),
                         (0, "bootcmd=run a; run b\nbaudrate=115200\nx=\n"))
        self.assertEqual(self.run_tool("print", "-n", f, "baudrate")[1],
                         "115200\n")
        self.assertEqual(self.run_tool("print", f, "nope")[0], 1)
        self.assertEqual(self.run_tool("check", f)[0], 0)

    def test_single_set_unset_roundtrip_both_paddings(self):
        for pad in (0x00, 0xff):
            src = image(VARS, pad=pad)
            f, g, h, h2 = (self.p("e", src), self.p("g"), self.p("h"),
                           self.p("h2"))
            self.assertEqual(self.run_tool("set", f, g, "new=1", "x=y")[0], 0)
            self.assertEqual(self.run_tool("print", g)[1],
                             "bootcmd=run a; run b\nbaudrate=115200\nx=y\nnew=1\n")
            self.assertEqual(self.rd(g)[-1], pad)
            self.assertEqual(self.run_tool("check", g)[0], 0)
            self.assertEqual(self.run_tool("set", f, h2, "new=1")[0], 0)
            self.assertEqual(self.run_tool("unset", h2, h, "new")[0], 0)
            self.assertEqual(self.rd(h), src)

    def test_size_keeps_bytes_outside(self):
        f, g = self.p("e", image(VARS) + b"\xaa" * 100), self.p("g")
        self.assertEqual(
            self.run_tool("set", "--size", "0x200", f, g, "a=b")[0], 0)
        self.assertEqual(self.rd(g)[SIZE:], b"\xaa" * 100)

    def test_bad_crc_refused(self):
        bad = bytearray(image(VARS))
        bad[10] ^= 1
        f, g = self.p("e", bytes(bad)), self.p("g")
        self.assertEqual(self.run_tool("check", f)[0], 2)
        self.assertEqual(self.run_tool("set", f, g, "a=b")[0], 2)
        self.assertEqual(self.run_tool("unset", f, g, "x")[0], 2)
        self.assertEqual(self.run_tool("print", f)[0], 2)
        self.assertFalse(os.path.exists(g))

    def test_overflow_and_bad_names_refused(self):
        f, g = self.p("e", image(VARS)), self.p("g")
        self.assertEqual(self.run_tool("set", f, g, "big=" + "a" * SIZE)[0], 2)
        self.assertEqual(self.run_tool("set", f, g, "=v")[0], 2)
        self.assertEqual(self.run_tool("set", f, g, "noequals")[0], 2)
        self.assertEqual(self.run_tool("unset", f, g, "a=b")[0], 2)
        self.assertFalse(os.path.exists(g))
        # a size the data does not fit, and one larger than the file
        self.assertEqual(self.run_tool("check", "--size", "0x100", f)[0], 2)
        self.assertEqual(self.run_tool("check", "--size", "0x400", f)[0], 2)

    def test_redundant_flag_selection(self):
        c1, c2 = image(VARS, flag=3), image(VARS, flag=2)
        f, g, h = self.p("r", c1 + c2), self.p("g"), self.p("h")
        rc, out, _ = self.run_tool("set", "--redundant", f, g, "a=b")
        self.assertEqual(rc, 0)
        self.assertIn("copy 2", out)
        self.assertIn("flag 4", out)
        new = self.rd(g)
        self.assertEqual(new[:SIZE], c1)
        self.assertEqual(new[SIZE + 4], 4)
        self.assertEqual(
            self.run_tool("print", "-n", "--redundant", g, "a")[1], "b\n")
        # copy 2 is now newer: the next set writes copy 1 with flag 5
        rc, out, _ = self.run_tool("set", "--redundant", g, h, "c=d")
        self.assertIn("copy 1", out)
        self.assertEqual(self.rd(h)[4], 5)
        self.assertEqual(self.rd(h)[SIZE:], new[SIZE:])

    def test_redundant_wraparound(self):
        g = self.p("g")
        # flag 255 then 0: the copy with 0 is the newer one, in both orders
        f = self.p("r", image(VARS, flag=255) + image(VARS[:1], flag=0))
        self.assertEqual(self.run_tool("print", "--redundant", f)[1],
                         "bootcmd=run a; run b\n")
        rc, out, _ = self.run_tool("set", "--redundant", f, g, "a=b")
        self.assertIn("copy 1", out)
        self.assertEqual(self.rd(g)[4], 1)
        f2 = self.p("r2", image(VARS[:1], flag=0) + image(VARS, flag=255))
        self.assertEqual(self.run_tool("print", "--redundant", f2)[1],
                         "bootcmd=run a; run b\n")
        # the new flag wraps from 255 to 0
        f3 = self.p("r3", image(VARS, flag=255) + image(VARS, flag=254))
        self.run_tool("set", "--redundant", f3, g, "a=b")
        self.assertEqual(self.rd(g)[SIZE + 4], 0)
        f4 = self.p("r4", image(VARS, flag=254) + image(VARS, flag=255))
        self.run_tool("set", "--redundant", f4, g, "a=b")
        self.assertEqual(self.rd(g)[4], 0)

    def test_redundant_one_bad_and_two_files(self):
        bad = bytearray(image(VARS, flag=9))
        bad[8] ^= 1
        f = self.p("r", bytes(bad) + image(VARS[:2], flag=1))
        rc, out, _ = self.run_tool("check", "--redundant", f)
        self.assertEqual(rc, 2)
        self.assertIn("copy 1: BAD", out)
        self.assertEqual(
            self.run_tool("set", "--redundant", f, self.p("g"), "a=b")[0], 2)
        self.assertEqual(self.run_tool("print", "--redundant", f)[1],
                         "bootcmd=run a; run b\nbaudrate=115200\n")
        a, b = self.p("a", image(VARS, flag=7)), self.p("b", image(VARS, flag=6))
        o = self.p("o")
        rc, out, _ = self.run_tool("set", "--redundant", "--second", b, a, o,
                                   "k=v")
        self.assertEqual(rc, 0)
        self.assertIn("copy 2", out)
        self.assertEqual(self.rd(o)[4], 8)
        self.assertEqual(self.run_tool("print", "-n", "--redundant", "--second",
                                       o, a, "k")[1], "v\n")

    def test_redundant_padding_and_overflow(self):
        f = self.p("r", image(VARS, pad=0xff, flag=1) +
                   image(VARS, pad=0xff, flag=0))
        g = self.p("g")
        self.assertEqual(self.run_tool("set", "--redundant", f, g, "a=b")[0], 0)
        self.assertEqual(self.rd(g)[-1], 0xff)
        self.assertEqual(self.run_tool("set", "--redundant", f, g,
                                       "a=" + "z" * SIZE)[0], 2)

    def test_diff(self):
        a = self.p("a", image(VARS))
        b = self.p("b", image([(b"bootcmd", b"other"), (b"x", b""),
                               (b"n", b"1")]))
        rc, out, _ = self.run_tool("diff", a, b)
        self.assertEqual(rc, 1)
        self.assertEqual(out.splitlines(), ["~bootcmd: run a; run b -> other",
                                            "-baudrate=115200", "+n=1"])
        self.assertEqual(self.run_tool("diff", a, a)[0], 0)

    def test_second_needs_redundant(self):
        f = self.p("e", image(VARS))
        self.assertEqual(self.run_tool("print", "--second", f, f)[0], 2)


if __name__ == "__main__":
    unittest.main()
