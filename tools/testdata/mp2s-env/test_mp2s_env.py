# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The DK u-boot-env fixtures of this directory, decoded with tools/uboot_env.py.

Run as `TMPDIR=<dir> python3 -I -B tools/testdata/mp2s-env/test_mp2s_env.py`.
It checks every image's CRCs, flags, active copy and variables against
README.md's table through uboot_env.py's command line, and runs
`regen.sh --check` (a rebuild into $TMPDIR, byte for byte).
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
TOOL = os.path.join(ROOT, "tools", "uboot_env.py")
TXT = os.path.join(ROOT, "driver", "tools", "mp2-standalone", "st-env",
                   "stm32mp25-dk-initial-env.txt")
LAYOUT = ["--redundant", "--size", "0x2000"]
SIZE = 0x2000

# The runner's "default" write (design note section 3) and the script's count.
DEFAULT = {"mp2s_ver": "1", "mp2s_prefixes": "/ /boot/",
           "boot_prefixes": "/yakogl/standalone/ / /boot/"}


def st_defaults():
    env = {}
    with open(TXT, encoding="latin-1") as fh:
        for ln in fh.read().split("\n"):
            if ln:
                name, value = ln.split("=", 1)
                env[name] = value   # a repeated name: the last value wins
    return env


def with_(base, **kw):
    env = dict(base)
    env.update(kw)
    return env


PRISTINE = st_defaults()
INSTALLED = with_(PRISTINE, **DEFAULT)
COUNTING2 = with_(INSTALLED, mp2s_left="2")
COUNTING1 = with_(INSTALLED, mp2s_left="1")
LEFT0 = with_(INSTALLED, mp2s_left="0")

# image: (copy 1 vars, flags), (copy 2 vars, flags), active copy or None
IMAGES = {
    "pristine.bin": ((PRISTINE, 0), (PRISTINE, 0), 1),
    "installed.bin": ((PRISTINE, 0), (INSTALLED, 1), 2),
    "counting2.bin": ((COUNTING2, 2), (INSTALLED, 1), 1),
    "counting1.bin": ((COUNTING2, 2), (COUNTING1, 3), 2),
    "left0.bin": ((LEFT0, 4), (COUNTING1, 3), 1),
}


def run(*args):
    return subprocess.run([sys.executable, "-I", TOOL] + list(args), capture_output=True,
                          text=True, encoding="latin-1")


def decode(path):
    r = run(*LAYOUT, "print", path)
    if r.returncode != 0:
        raise AssertionError("print %s: %s" % (path, r.stderr))
    env = {}
    for ln in r.stdout.split("\n"):
        if ln:
            name, value = ln.split("=", 1)
            assert name not in env, name
            env[name] = value
    return env


class Fixtures(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR"))

    def tearDown(self):
        self.tmp.cleanup()

    def image(self, name):
        with open(os.path.join(HERE, name), "rb") as fh:
            return fh.read()

    def copy_alone(self, name, n):
        """Copy n of an image paired with itself, so that uboot_env.py reads it."""
        c = self.image(name)[(n - 1) * SIZE:n * SIZE]
        p = os.path.join(self.tmp.name, "%s.copy%d" % (name, n))
        with open(p, "wb") as fh:
            fh.write(c * 2)
        return p

    def test_sizes(self):
        for name in list(IMAGES) + ["one-copy-bad.bin", "both-bad.bin"]:
            self.assertEqual(len(self.image(name)), 2 * SIZE, name)

    def test_st_defaults_hold_what_the_design_reads(self):
        self.assertEqual(PRISTINE["boot_prefixes"], "/ /boot/")
        self.assertEqual(PRISTINE["boot_net_usb_start"], "true")
        for name in ("bootcount", "bootlimit", "upgrade_available"):
            self.assertNotIn(name, PRISTINE)
        self.assertFalse([n for n in PRISTINE if n.startswith("mp2s_")])

    def test_valid_images(self):
        for name, (c1, c2, active) in IMAGES.items():
            with self.subTest(image=name):
                path = os.path.join(HERE, name)
                r = run(*LAYOUT, "check", path)
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertIn("flags 0x%02x" % c1[1], r.stdout.split("\n")[0])
                self.assertIn("flags 0x%02x" % c2[1], r.stdout.split("\n")[1])
                self.assertIn("active: copy %d" % active, r.stdout)
                self.assertEqual(decode(path), (c1, c2)[active - 1][0])
                self.assertEqual(decode(self.copy_alone(name, 1)), c1[0])
                self.assertEqual(decode(self.copy_alone(name, 2)), c2[0])

    def test_the_count_is_the_only_difference(self):
        r = run(*LAYOUT, "diff", os.path.join(HERE, "counting2.bin"),
                os.path.join(HERE, "counting1.bin"))
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("mp2s_left", r.stdout)
        self.assertNotIn("boot_prefixes", r.stdout)

    def test_one_copy_bad(self):
        path = os.path.join(HERE, "one-copy-bad.bin")
        r = run(*LAYOUT, "check", path)
        self.assertNotEqual(r.returncode, 0)
        lines = r.stdout.split("\n")
        self.assertTrue(lines[0].startswith("BAD copy 1"), r.stdout)
        self.assertTrue(lines[1].startswith("ok  copy 2"), r.stdout)
        self.assertIn("flags 0x03", lines[1])
        # uboot_env.py refuses a pair with a bad copy; U-Boot would use copy 2.
        self.assertNotEqual(run(*LAYOUT, "print", path).returncode, 0)
        self.assertEqual(decode(self.copy_alone("one-copy-bad.bin", 2)), COUNTING1)
        # Only copy 1's CRC byte 0 differs from counting1.bin.
        a, b = self.image("counting1.bin"), self.image("one-copy-bad.bin")
        self.assertEqual([i for i in range(len(a)) if a[i] != b[i]], [0])

    def test_both_bad(self):
        path = os.path.join(HERE, "both-bad.bin")
        r = run(*LAYOUT, "check", path)
        self.assertNotEqual(r.returncode, 0)
        lines = r.stdout.split("\n")
        self.assertTrue(lines[0].startswith("BAD copy 1"), r.stdout)
        self.assertTrue(lines[1].startswith("BAD copy 2"), r.stdout)
        self.assertNotEqual(run(*LAYOUT, "print", path).returncode, 0)
        a, b = self.image("counting1.bin"), self.image("both-bad.bin")
        self.assertEqual([i for i in range(len(a)) if a[i] != b[i]], [0, SIZE])

    def test_regen_check(self):
        r = subprocess.run(["bash", os.path.join(HERE, "regen.sh"), "--check"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
