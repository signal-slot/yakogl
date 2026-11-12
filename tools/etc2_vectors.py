#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Regenerate src/gl/etc2_vectors.rs: ETC2/EAC blocks and what Mesa's own
decoder makes of them.

    python3 -I tools/etc2_vectors.py <mesa-src-dir> > src/gl/etc2_vectors.rs

<mesa-src-dir> is the `src` directory of a Mesa tree (the vectors in the tree
are from Mesa 26.2.1, read only, $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/src).
The decoder is src/mesa/main/texcompress_etc.c itself, compiled with stub
headers up to `_mesa_unpack_etc2_format` and called on one 4x4 image per
block, so what the tests compare with is Mesa's code, not a reading of it.

The blocks are pseudo-random (a fixed LCG) but stratified: the colour formats
get blocks of every mode (individual, differential, T, H, planar) with the
flag bit both ways, the EAC formats a zero multiplier and the signed base
codeword -128 as well. Needs a C compiler (cc).
"""
import os
import subprocess
import sys
import tempfile

SHIM = r"""
#include <stdint.h>
#include <stdbool.h>
#include <assert.h>
typedef unsigned int GLuint;
typedef int GLint;
typedef unsigned char GLubyte;
typedef signed char GLbyte;
typedef unsigned short GLushort;
typedef short GLshort;
typedef unsigned char GLboolean;
#define GL_FALSE 0
#define GL_TRUE 1
#define CLAMP(x, lo, hi) ((x) < (lo) ? (lo) : ((x) > (hi) ? (hi) : (x)))
#define MIN2(a, b) ((a) < (b) ? (a) : (b))
#define UNUSED __attribute__((unused))
#define UNUSED_TEXSTORE_PARAMS int unused_
#define UNREACHABLE(x) __builtin_unreachable()
typedef enum {
   MESA_FORMAT_ETC1_RGB8, MESA_FORMAT_ETC2_RGB8, MESA_FORMAT_ETC2_SRGB8,
   MESA_FORMAT_ETC2_RGBA8_EAC, MESA_FORMAT_ETC2_SRGB8_ALPHA8_EAC,
   MESA_FORMAT_ETC2_R11_EAC, MESA_FORMAT_ETC2_RG11_EAC,
   MESA_FORMAT_ETC2_SIGNED_R11_EAC, MESA_FORMAT_ETC2_SIGNED_RG11_EAC,
   MESA_FORMAT_ETC2_RGB8_PUNCHTHROUGH_ALPHA1,
   MESA_FORMAT_ETC2_SRGB8_PUNCHTHROUGH_ALPHA1,
} mesa_format;
"""

# (name, mesa_format, block bytes, output bytes of one 4x4 image)
FORMATS = [
    ("RGB8", "MESA_FORMAT_ETC2_RGB8", 8, 64),
    ("SRGB8", "MESA_FORMAT_ETC2_SRGB8", 8, 64),
    ("RGBA8_EAC", "MESA_FORMAT_ETC2_RGBA8_EAC", 16, 64),
    ("SRGB8_ALPHA8_EAC", "MESA_FORMAT_ETC2_SRGB8_ALPHA8_EAC", 16, 64),
    ("R11", "MESA_FORMAT_ETC2_R11_EAC", 8, 32),
    ("SIGNED_R11", "MESA_FORMAT_ETC2_SIGNED_R11_EAC", 8, 32),
    ("RG11", "MESA_FORMAT_ETC2_RG11_EAC", 16, 64),
    ("SIGNED_RG11", "MESA_FORMAT_ETC2_SIGNED_RG11_EAC", 16, 64),
    ("RGB8_PT", "MESA_FORMAT_ETC2_RGB8_PUNCHTHROUGH_ALPHA1", 8, 64),
    ("SRGB8_PT", "MESA_FORMAT_ETC2_SRGB8_PUNCHTHROUGH_ALPHA1", 8, 64),
]


class Lcg:
    def __init__(self, seed):
        self.s = seed

    def byte(self):
        self.s = (self.s * 6364136223846793005 + 1442695040888963407) & (2**64 - 1)
        return (self.s >> 33) & 0xFF

    def bytes(self, n):
        return bytearray(self.byte() for _ in range(n))


def diff_sum(v):
    return (v >> 3) + [0, 1, 2, 3, -4, -3, -2, -1][v & 7]


def mode_of(b, pt):
    """The mode ETC2's rules select (ES 3.0 / the Data Format Spec)."""
    flag = b[3] & 2
    if not flag and not pt:
        return "individual"
    if not 0 <= diff_sum(b[0]) <= 31:
        return "T"
    if not 0 <= diff_sum(b[1]) <= 31:
        return "H"
    if not 0 <= diff_sum(b[2]) <= 31:
        return "planar"
    return "differential"


def colour_blocks(rng, pt, per_mode):
    want = {m: [] for m in ("individual", "differential", "T", "H", "planar")}
    # the punch-through format has no individual mode: bit 1 is its opaque flag
    if pt:
        del want["individual"]
    while any(len(v) < per_mode for v in want.values()):
        b = rng.bytes(8)
        m = mode_of(b, pt)
        # random blocks reach T, H and planar only when a channel overflows, so
        # most draws are discarded; the punch-through ones carry both flag values
        if len(want[m]) < per_mode:
            want[m].append(bytes(b))
    # a corner-case block each: all zero, all one
    out = [blk for v in want.values() for blk in v]
    out += [bytes(8), bytes([255] * 8)]
    return out


def eac_blocks(rng, signed, n):
    out = [bytes(rng.bytes(8)) for _ in range(n)]
    for i, b in enumerate(out):
        if i % 4 == 1:
            b = bytearray(b)
            b[1] &= 0x0F  # multiplier 0: the modifier is added unscaled
            out[i] = bytes(b)
        if signed and i % 4 == 2:
            b = bytearray(out[i])
            b[0] = 0x80  # -128 reads as -127
            out[i] = bytes(b)
    out += [bytes(8), bytes([255] * 8)]
    return out


def build_vectors():
    rng = Lcg(0x0E7C2)
    v = {}
    v["RGB8"] = colour_blocks(rng, False, 10)
    v["RGB8_PT"] = colour_blocks(rng, True, 10)
    alpha = eac_blocks(rng, False, 12)
    rgbs = colour_blocks(rng, False, 3)
    v["RGBA8_EAC"] = [a + c for a, c in zip(alpha, rgbs)]
    r = eac_blocks(rng, False, 24)
    sr = eac_blocks(rng, True, 24)
    v["R11"] = r
    v["SIGNED_R11"] = sr
    g = eac_blocks(rng, False, 24)
    sg = eac_blocks(rng, True, 24)
    v["RG11"] = [a + b for a, b in zip(r, g)]
    v["SIGNED_RG11"] = [a + b for a, b in zip(sr, sg)]
    v["SRGB8"] = v["RGB8"]
    v["SRGB8_ALPHA8_EAC"] = v["RGBA8_EAC"]
    v["SRGB8_PT"] = v["RGB8_PT"]
    return v


def run_mesa(mesa_src, vectors):
    etc = os.path.join(mesa_src, "mesa", "main", "texcompress_etc.c")
    with open(etc) as f:
        text = f.read()
    cut = text.index("fetch_etc1_rgb8(")
    cut = text.rindex("static void", 0, cut)
    with tempfile.TemporaryDirectory() as d:
        shim = os.path.join(d, "shim")
        os.makedirs(os.path.join(shim, "util"))
        for h in ("texcompress.h", "texcompress_etc.h", "texstore.h", "config.h", "macros.h",
                  "format_unpack.h", os.path.join("util", "format_srgb.h")):
            open(os.path.join(shim, h), "w").close()
        open(os.path.join(d, "pre.h"), "w").write(SHIM)
        open(os.path.join(d, "etc.c"), "w").write(text[:cut])
        main = ['#include "etc.c"', "#include <stdio.h>"]
        for name, fmt, bs, ob in FORMATS:
            blocks = vectors[name]
            main.append("static const uint8_t b_%s[][%d] = {" % (name, bs))
            for b in blocks:
                main.append("  {%s}," % ",".join(str(x) for x in b))
            main.append("};")
        main.append("int main(void) {")
        for name, fmt, bs, ob in FORMATS:
            main.append("""  for (unsigned i = 0; i < sizeof(b_%s) / sizeof(b_%s[0]); i++) {
    uint8_t out[64] = {0};
    _mesa_unpack_etc2_format(out, %d, b_%s[i], %d, 4, 4, %s, false);
    printf("%s");
    for (int k = 0; k < %d; k++) printf(" %%02x", out[k]);
    printf("\\n");
  }""" % (name, name, ob // 4, name, bs, fmt, name, ob))
        main.append("  return 0; }")
        open(os.path.join(d, "main.c"), "w").write("\n".join(main))
        exe = os.path.join(d, "gen")
        subprocess.run(["cc", "-O1", "-w", "-include", os.path.join(d, "pre.h"), "-I" + shim, "-I" + mesa_src,
                        "-I" + os.path.join(d), os.path.join(d, "main.c"), "-o", exe], check=True)
        res = subprocess.run([exe], check=True, capture_output=True, text=True).stdout
    out = {name: [] for name, *_ in FORMATS}
    for line in res.splitlines():
        name, *hexes = line.split()
        out[name].append(bytes(int(h, 16) for h in hexes))
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    vectors = build_vectors()
    got = run_mesa(sys.argv[1], vectors)
    # the sRGB twins make the same bytes in Mesa's decoder (the curve is the sampler's)
    for a, b in (("RGB8", "SRGB8"), ("RGBA8_EAC", "SRGB8_ALPHA8_EAC"), ("RGB8_PT", "SRGB8_PT")):
        assert got[a] == got[b], (a, b)
    print("// SPDX-License-Identifier: Apache-2.0")
    print("// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.")
    print("//! GENERATED by tools/etc2_vectors.py from Mesa 26.2.1's _mesa_unpack_etc2_format")
    print("//! (src/mesa/main/texcompress_etc.c): ETC2/EAC blocks and the 4x4 image Mesa")
    print("//! decodes each to, as hex. Do not edit; regenerate.")
    print()
    print("/// (block, Mesa's decoded 4x4 image), each as hex. RGB, RGBA and punch-through")
    print("/// images are 16 texels of R, G, B, A bytes; R11 images 16 u16 (little endian),")
    print("/// signed ones i16; RG11 images 16 texels of two u16/i16.")
    for name in ("RGB8", "RGBA8_EAC", "RGB8_PT", "R11", "SIGNED_R11", "RG11", "SIGNED_RG11"):
        print("pub(super) const %s: &[(&str, &str)] = &[" % name)
        for blk, out in zip(vectors[name], got[name]):
            print('    ("%s", "%s"),' % (blk.hex(), out.hex()))
        print("];")


if __name__ == "__main__":
    main()
