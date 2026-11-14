#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The blend shaders of dEQP-GLES2's functional.fragment_ops.blend, Mesa's
bytes (tools/mali/README-pan_shc.md, "Blend shaders").

The blend states are es2fBlendTests.cpp's (VK-GL-CTS
modules/gles2/functional/es2fBlendTests.cpp, BlendTests::init): the three
groups, every parameter set, the blend colour (0.2, 0.4, 0.6, 0.8), an RGBA8
target written whole at 1x. Of those, the ones Mesa 26.2.1 draws with a
blend shader (panfrost_get_blend, src/gallium/drivers/panfrost/
pan_context.c:170-200: not fixed function by pan_blend_can_fixed_function,
src/panfrost/lib/pan_blend.c:64-106 and :284-298, or a constant read in
channels that differ, pan_blend_is_homogenous_constant :244-258) are listed
once each, in the order the tests first draw them.

  blend_deqp_ref.py list                 the equations, pan_shc --blend's input
  blend_deqp_ref.py words <pan_shc out>  the reference file from pan_shc's
                                         stdout (tools/mali/testdata/
                                         blend_deqp.words)
  blend_deqp_ref.py random               2700 more keys: every function,
                                         factor and colour mask drawn at
                                         random (seeded), 1500 at 1x and
                                         600 at 4x with case 211's (an
                                         fp16 colour), 600 with an fp32
                                         colour (1x and 4x)
  blend_deqp_ref.py digest <pan_shc out> their reference
                                         (tools/mali/testdata/blend_random.sha)
  blend_deqp_ref.py u32                  100 keys with a uint32 colour (a
                                         highp `out uvec4`): the first 100
                                         fp32 keys of `random`, `u32` for
                                         `f32`
  blend_deqp_ref.py u32words <pan_shc out>
                                         their reference (tools/mali/
                                         testdata/blend_u32.words)
  blend_deqp_ref.py min_f32              the key of blend_min_f32.words
  blend_deqp_ref.py lines <pan_shc out>  its words, one a line (tools/mali/
                                         testdata/blend_min_f32.words)

The reference file has one line an equation: the equation as pan_shc reads
it, ':', then the shader's words (hex, little-endian u64, the padding
dropped). The digest file has, instead of the words, the first 16 hex
digits of the SHA-256 of their bytes (each word little-endian).
"""
import json
import sys

FUNCS = ["add", "subtract", "reverse_subtract"]
# GL's order in es2fBlendTests.cpp's blendFunctions table, gallium's names
FACTORS = [
    "zero", "one", "src_color", "inv_src_color", "dst_color", "inv_dst_color",
    "src_alpha", "inv_src_alpha", "dst_alpha", "inv_dst_alpha",
    "const_color", "inv_const_color", "const_alpha", "inv_const_alpha",
    "src_alpha_saturate",
]
COLOUR = (0.2, 0.4, 0.6, 0.8)


def base(f):
    """util_blendfactor_without_invert: ZERO is ONE inverted."""
    if f == "zero":
        return "one"
    return f[4:] if f.startswith("inv_") else f


def channel_fixed_function(func, src, dst, is_alpha):
    """can_fixed_function_equation (pan_blend.c:64-106), UNORM, v10 (2src)."""
    if func not in ("add", "subtract", "reverse_subtract"):
        return False
    srcs = ["dst_color"] + (["dst_alpha"] if is_alpha else [])
    dsts = ["src_color"] + (["src_alpha"] if is_alpha else [])
    if func == "add" and src in srcs and dst in dsts:
        return True
    if "src_alpha_saturate" in (src, dst):
        return False
    return base(src) == base(dst) or base(src) == "one" or base(dst) == "one"


def constant_mask(eq):
    m = 0
    for f in (eq[1], eq[2], eq[4], eq[5]):
        b = base(f)
        m |= 0b0111 if b == "const_color" else 0b1000 if b == "const_alpha" else 0
    return m


def needs_shader(eq):
    """panfrost_get_blend for a written, not opaque target."""
    fixed = channel_fixed_function(eq[0], eq[1], eq[2], False) and \
        channel_fixed_function(eq[3], eq[4], eq[5], True)
    m = constant_mask(eq)
    read = [COLOUR[i] for i in range(4) if m >> i & 1]
    return not (fixed and all(c == read[0] for c in read))


def states():
    """Every blend state the three groups draw, in test order."""
    for eq in FUNCS:  # equation_src_func_dst_func
        for s in FACTORS:
            for d in FACTORS:
                if d != "src_alpha_saturate":
                    yield (eq, s, d, eq, s, d)
    for is_dst in (False, True):  # rgb_func_alpha_func.{src,dst}
        for rgb in FACTORS:
            for alpha in FACTORS:
                if is_dst and "src_alpha_saturate" in (rgb, alpha):
                    continue
                for other in ("one", "zero", "src_color", "dst_color"):
                    if is_dst:
                        yield ("add", other, rgb, "add", other, alpha)
                    else:
                        yield ("add", rgb, other, "add", alpha, other)
    for rgb in FUNCS:  # rgb_equation_alpha_equation
        for alpha in FUNCS:
            yield (rgb, "one", "one", alpha, "one", "one")


def equations():
    seen = []
    for eq in states():
        if needs_shader(eq) and eq not in seen:
            seen.append(eq)
    return [" ".join(eq) + " 15 1" for eq in seen]


def random_keys():
    """The keys of blend_random.sha: three seeded draws over every function,
    factor (SRC_ALPHA_SATURATE a source factor only, a MIN or MAX channel's
    factors ONE, as st/mesa hands them) and colour mask; the third with an
    fp32 colour."""
    import random
    funcs = FUNCS + ["min", "max"]
    out = []
    for seed, n, samples, first in ((20261112, 1500, 1, []),
                                    (4, 600, 4, ["add const_color inv_const_color add const_color inv_const_color 15 4"]),
                                    (32, 600, None, ["min one one min one one 15 1 f32"])):
        rnd = random.Random(seed)
        seen = set(first)
        while len(seen) < n:
            rf, af = rnd.choice(funcs), rnd.choice(funcs)
            rs, rd = rnd.choice(FACTORS_RANDOM), rnd.choice(FACTORS_RANDOM[:-1])
            as_, ad = rnd.choice(FACTORS_RANDOM), rnd.choice(FACTORS_RANDOM[:-1])
            if rf in ("min", "max"):
                rs = rd = "one"
            if af in ("min", "max"):
                as_ = ad = "one"
            mask = rnd.choice([15] * 6 + list(range(1, 15)))
            if samples is None:
                seen.add(f"{rf} {rs} {rd} {af} {as_} {ad} {mask} {rnd.choice([1, 4])} f32")
            else:
                seen.add(f"{rf} {rs} {rd} {af} {as_} {ad} {mask} {samples}")
        out += sorted(seen)
    return out


def u32_keys():
    """The keys of blend_u32.words: Mesa keys a uint32 colour apart
    (src0_type nir_type_uint32), but converts it from float as the target's
    base type (pan_blend.c:837-862), so its words are the fp32 key's."""
    f32 = [k for k in random_keys() if k.endswith(" f32")]
    return [k[:-len("f32")] + "u32" for k in f32[:100]]


# the key of blend_min_f32.words
MIN_F32 = "min one one min one one 15 1 f32"


# the factor order the random draw was made with
FACTORS_RANDOM = [
    "zero", "one", "src_color", "inv_src_color", "dst_color", "inv_dst_color",
    "src_alpha", "inv_src_alpha", "dst_alpha", "inv_dst_alpha",
    "const_color", "inv_const_color", "const_alpha", "inv_const_alpha",
    "src_alpha_saturate",
]


def read_words(path):
    got = {}
    for line in open(path):
        if line.startswith("{"):
            o = json.loads(line)
            w = list(o["words"])
            while w and w[-1] == "0" * 16:
                w.pop()
            got[o["equation"]] = w
    return got


def main():
    if sys.argv[1:] == ["random"]:
        print("\n".join(random_keys()))
        return 0
    if len(sys.argv) == 3 and sys.argv[1] == "digest":
        import hashlib
        import struct
        want = random_keys()
        got = read_words(sys.argv[2])
        missing = [e for e in want if e not in got]
        if missing:
            print(f"blend_deqp_ref: {len(missing)} equations missing, first '{missing[0]}'", file=sys.stderr)
            return 1
        for e in want:
            data = b"".join(struct.pack("<Q", int(w, 16)) for w in got[e])
            print(e + " : " + hashlib.sha256(data).hexdigest()[:16])
        return 0
    if sys.argv[1:] == ["u32"]:
        print("\n".join(u32_keys()))
        return 0
    if sys.argv[1:] == ["min_f32"]:
        print(MIN_F32)
        return 0
    if len(sys.argv) == 3 and sys.argv[1] == "lines":
        got = read_words(sys.argv[2])
        if list(got) != [MIN_F32]:
            print(f"blend_deqp_ref: not the one key '{MIN_F32}'", file=sys.stderr)
            return 1
        print("\n".join(got[MIN_F32]))
        return 0
    if sys.argv[1:] == ["list"]:
        print("\n".join(equations()))
        return 0
    if len(sys.argv) == 3 and sys.argv[1] in ("words", "u32words"):
        want = equations() if sys.argv[1] == "words" else u32_keys()
        got = read_words(sys.argv[2])
        missing = [e for e in want if e not in got]
        if missing:
            print(f"blend_deqp_ref: {len(missing)} equations missing, first '{missing[0]}'", file=sys.stderr)
            return 1
        for e in want:
            print(e + " : " + " ".join(got[e]))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
