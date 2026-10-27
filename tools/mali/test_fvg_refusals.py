# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fvg_refusals as f  # noqa: E402

P = "tools/shaders/femtovg/"
TEX = "Valhall: a second texture look-up in one fragment shader is not lowered yet"
FMA = ("Valhall: a fused multiply-add other than a matrix product's "
       "(a built-in function's) is not lowered yet")


class FvgRefusals(unittest.TestCase):
    def test_all_ok(self):
        self.assertEqual(f.classify([P + "a.frag: ok\n", "tools/shaders/fvgtest/b.frag: ok\n"]),
                         (2, 0, []))

    def test_texture_only(self):
        r = f.classify([P + "a.frag: ok", P + "b.frag: " + TEX,
                        P + "c.frag: Valhall: a texture coordinate other than a vec2 "
                        "varying read whole or two constants is not lowered yet"])
        self.assertEqual(r, (1, 2, []))

    def test_fma_is_other(self):
        line = P + "g.frag: " + FMA
        self.assertEqual(f.classify([P + "a.frag: ok", line]), (1, 0, [line]))

    def test_other_directories_ignored(self):
        self.assertEqual(f.classify(["tools/shaders/misc/x.frag: " + FMA]), (0, 0, []))


if __name__ == "__main__":
    unittest.main()
