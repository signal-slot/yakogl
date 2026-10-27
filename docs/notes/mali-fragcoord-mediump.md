<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall fragment stage: arithmetic on gl_FragCoord that the back end calls mediump

Date 2026-10-25, tip `653fe45d`. Plan #153 item `precision2`, for issue #1384.
Host research only: no file under `src/` changed, no board run. Mesa 26.2.1
(`MESA_PAN_TREE`), cited as `G/` = `src/compiler/glsl/`. Words are quoted from
`tools/mali/testdata/fbload_<name>.expect.json` (`name` = `fbload`,
`fbload_int`, `mipgen`, `isamp`, `predif_fs_coord`, `predlive_fs_coord`), drawn
by `tools/mali/drm_shim_ref.sh run` from the `.vert`/`.frag` beside them
(the `.frag` are byte copies of `tools/shaders/internal/es/{fbload,fbload_int,
mipgen}.frag` and `tools/shaders/gltest/{isamp,predif_fs_coord,predlive_fs_coord}.frag`).
Vertex shaders: the three `#version 300 es` internal shaders and `isamp` need an
ES 3.00 vertex shader (`internal/es/quad.vert` is `#version 100` and cannot
link with them), so `fbload_{fbload,fbload_int,mipgen,isamp}.vert` is a
`layout(location=0) in vec4 a_pos` pass-through; the two ES 1.00 gltest shaders
use `gltest/position_only.vert`. The references' `gl_ref.py` default state:
64x64 target, a 2x2 RGBA8 texture bound to a `sampler2D` (none for
`usampler2D`/`isampler2D`: the unit is unbound, so the descriptor words are not
a statement about those formats).

## 1. What the refusal actually covers

`MaliError::FragCoordMediump` (`src/shaderc/online/mali/nir_fs.rs`, the two
sites at the `IoSlot::FragCoord` load and the `Alu` arm) fires on an arithmetic
node whose operand is the `gl_FragCoord` read itself, and it never looks at the
shader's `#version`. Mesa decides the precision of the built-in by version:
`G/builtin_variables.cpp:1333-1335`, `frag_coord_precision` is
`GLSL_PRECISION_HIGH` for ES 3.00 and later, `MEDIUM` for ES 1.00. So the six
shaders split into two groups:

| Group | Shaders | `gl_FragCoord` precision in Mesa | Reference arithmetic on it |
|---|---|---|---|
| ES 3.00 | `fbload`, `fbload_int`, `mipgen`, `isamp` | **highp** | fp32 (see 2) |
| ES 1.00 | `predif_fs_coord`, `predlive_fs_coord` | **mediump** | fp16 compare (see 3) |

For the first group the refusal is wrong in kind, not just incomplete: the
shader is highp throughout and the fp32 path of `mali-highp-fragcoord.md` 2.1
(`U16_TO_F32`, `FADD_IMM 0.5`, `FMA` with the y pair) is what Mesa emits. The
back end treats the read as mediump because its front end does not carry the
ES version into the precision of the built-in. For the second group the
refusal is the open case 2.1 step 7 of that note named ("a program that did
mediump arithmetic on `gl_FragCoord` is not in the corpus").

## 2. ES 3.00: the position stays fp32

Common prefix, in every one of the four (`preload` `0x3800000000000000`, r59
holds the pixel x in `.h0` and y in `.h1`):

```
U16_TO_F32 r0, r59.h1          ; (.discard in the first instruction when x is read later)
FADD_IMM.f32 r0, r0^, #0x3F000000
FMA.f32 r0, r0^, uA.w0, uA.w1  ; y * T.x + T.y, the (1.0, 0.0) pair on an FBO
```

The pair `(1.0, 0.0)` is a FAU constant the driver supplies
(`gl_FbWposYTransform`, `mali-highp-fragcoord.md` 2.1 step 3). The shaders then
differ in what they do with the fp32 values:

* **`fbload`**: `y = gl_FragCoord.y * u_flip.x + u_flip.y` is a second `FMA`
  with the uniform pair (`u2`), `int(y)` is `F32_TO_S32.rtz`, `int(x)` is
  `U16_TO_U32 r1, r59.h0`: the integer x skips the +0.5 and the float path
  altogether (Mesa folds `f2i(u2f(x)+0.5)` to the integer x). The fetch is
  `TEX_FETCH.slot0.reserved.32.2d @r0:r1:r2:r3, @r2:r3, [r0:r1]` (coordinate
  pair in r2:r3, texture descriptor index from `u4.w0`), then `ATEST` with the
  alpha and `BLEND.v4.f32`.
* **`fbload_int`**: the same words, `TEX_FETCH` on a `usampler2D`, and
  `BLEND.v4.u32` (an integer colour: `ATEST` takes the constant `0x3F800000`
  as alpha, not a fetched lane; the pixel output is `highp uvec4`).
* **`mipgen`**: both coordinates go through `U16_TO_F32` and `+0.5`, y through
  the pair `FMA`, then `x * u_scale.x`, `y * u_scale.y` as
  `FMA r, r^, u3.wN, 0x0.neg` (the `-0.0` addend that keeps `-0.0`
  correct), and the look-up is `TEX_SINGLE ... 2d.computed` (a `texture`, with
  the level computed from the derivatives), colour `BLEND.v4.f32`.
* **`isamp`**: `ivec2(gl_FragCoord.xy) / 32` is `F32_TO_S32.rtz` on y and
  `U16_TO_U32` on x, then Mesa's signed divide by a power of two: two `CSEL.s32`
  selecting the sign and the bias, `IABS.s32`, `RSHIFT_OR.i32` by 5
  (`0x7060504.b11`), `IMUL.i32` by the sign, i.e. the usual `idiv` by 32 for a
  value of either sign. Then `TEX_FETCH`, `IADD_IMM 0x80` on four lanes,
  `S32_TO_F32` on four lanes, `FMA` by `1/255` (`0x3B808081`) and the blend.

## 3. ES 1.00: the position is converted to fp16 at the read

`predif_fs_coord` (`if (gl_FragCoord.x < 32.0) ...`) and `predlive_fs_coord`
(the same condition around live values): the programs are the first with the
mediump read in the corpus. Mesa's words, from the references:

```
U16_TO_F32.discard r0, r59^.h0      ; x as fp32, the same as the highp path
FADD_IMM.f32 r0, r0^, #0x3F000000   ; + 0.5 (the pixel centre, in fp32)
FADD.f32 r0.h0, r0^, 0x0.neg        ; fp32 -> fp16: an FADD.f32 with an h0 destination
FCMP_OR.v2f16.lt.m1 r0, r0^.h00, u3.w0, 0x0    ; x16 < 32.0 in fp16 (u3.w0 = 0x50005000 = 32.0, 32.0)
S16_TO_S32 r3, r0.h0                ; the mask lane for the CSEL below
```

So the conversion is Mesa's `f2f16` of the `load_frag_coord` result after
`nir_lower_frag_coord_to_pixel_coord`: the fp32 lane is built exactly as in
the highp case (preload, `U16_TO_F32`, `FADD_IMM`), and the fp32 -> fp16
narrowing is `FADD.f32 rN.h0, rN, -0.0` (the conversion idiom `bi_emit_alu`
makes for `f2f16` with an fp32 source; `-0.0` keeps a signed zero). It is the
same `FADD.f32 rN.h0 ... 0x0.neg` the colour store uses when a fp32 value is
written to a `gl_FragColor` of fp16 type (`predif`: `FADD.f32 r1.h0, r1^,
0x0.neg`). Notes on what is read from the words:

* The y pair is not computed because only `.x` is read; the fragment preload
  and r59 are unchanged.
* The comparison is a packed `FCMP_OR.v2f16.lt.m1` against the pair
  `0x5000 5000`; the branch of the `if` is `CSEL.u32.eq` on the sign-extended
  lane (`S16_TO_S32`), both arms computed (the program has no branch).
* Arithmetic on the mediump read stays fp16 until `f2f32`: the colour lanes
  use `MKVEC.v2i16` and `BLEND.v4.f16`; the highp uniform `u_a` appears in
  fp32 `FMA`s whose results are narrowed with `FADD.f32 .h0`.
* The shaders declare `precision highp float`, but `gl_FragCoord` (mediump
  built-in in ES 1.00) is not a variable `lower_precision` retypes
  (`G/lower_precision.cpp:1023-1060`); the comparison operand precision is the
  operands' (`:295-312`), and a literal `32.0` with no precision takes the
  built-in's: hence the fp16 compare.

## 4. What our back end lacks, per shader

Common to the four ES 3.00 shaders: the back end must read the shader's
`#version` (or the built-in's precision) so that `gl_FragCoord` takes the highp
path (`Fs::frag_coord`, already built for a highp read) when the shader is ES
3.00. After that the remaining items are the ones named by the next refusal:

| Shader | After the precision fix, still needed |
|---|---|
| `fbload` | `texelFetch` (`TEX_FETCH`, the `.32.2d` form; the colour is fp32 `BLEND.v4.f32`), `F32_TO_S32.rtz`, `U16_TO_U32`, a `highp sampler2D` (`MaliError::TexHighp`), a `vec2` uniform pair read as two fp32 words of the FAU (`FsUniform` today refuses a highp uniform) |
| `fbload_int` | the above plus `usampler2D`, an unsigned texel as `BLEND.v4.u32`, a `highp uvec4` colour output (`MaliError::IntValue`) |
| `mipgen` | a `texture()` at a computed coordinate (`TEX_SINGLE ... computed`; the `texcoord` item), fp32 `FMA` with the `-0.0` addend for the scale, a highp sampler |
| `isamp` | `isampler2D` `texelFetch`, signed integer divide by a constant power of two (`CSEL`/`IABS`/`RSHIFT_OR`/`IMUL`), integer add, `S32_TO_F32`, an `ivec2` constructor from the fp32 position (`MaliError::IntValue`, `Arithmetic`) |
| `predif_fs_coord` | the ES 1.00 mediump read: build the highp lane as in step 2.1 of `mali-highp-fragcoord.md`, then narrow it with `FADD.f32 .h0, -0.0`; `FCMP_OR.v2f16.lt.m1` on a packed pair; a branch as `CSEL.u32.eq` on `S16_TO_S32`; mixed fp16/fp32 values (`MixedPrecision` today refuses the mixture of the highp uniforms with the fp16 position) |
| `predlive_fs_coord` | the same, plus the mixed-precision colour assembly (`MKVEC.v2i16`, several narrowings) |

`predif`/`predlive` also depend on the mixed-precision work of
`mali-mixed-precision.md` (item `study-mixprec`), because the highp uniforms
and the mediump position meet in one colour. The four ES 3.00 shaders do not
need any fp16 lowering at all: they are the cheapest of the six once the
version is honoured, and two of them (`fbload`, `fbload_int`) and `mipgen` are
the driver's own paths (PowerVR frame load and `glGenerateMipmap`), so they
are the shaders worth lowering first.

## 5. What the back end does now (plan #153 item `frame-cull`)

Every program of section 4 is lowered and pinned byte-equal to its reference
(`src/shaderc/online/mali/tests.rs`, `FRAG_COORD_POPULATION`), with the rules
the code cites:

* ES 3.00 (`fbload`, `fbload_int`, `mipgen`, `isamp`): the fp32 path. The
  signed divide is `imul(isign(a), ushr(iabs(a), log2 b))`
  (`nir_opt_algebraic.py:208`), `isign` as `imin(imax(a, -1), 1)`
  (`:2812`, `lower_isign`), built one vector operation at a time.
* ES 1.00 highp (`predif_fs_coord`, `predlive_fs_coord`, prec_82 and
  prec_119): an operation whose operands are the read, fp16 results of such
  operations or constants is fp16 (`lower_precision.cpp:295-312`); the read
  is narrowed there. One with a highp operand is fp32 and reads the
  position at fp32. An fp16 value read by a highp operation or stored to a
  highp local is widened (`F16_TO_F32`). A 32-bit `bcsel` reads a 16-bit
  mask through `i2i32` (`S16_TO_S32`, `pan_nir_lower_bool_to_bitsize`).
  `floor` and `fract` of x and y are one `vec2` of halves
  (`nir_lower_alu_width` builds it, `nir_opt_vectorize` pairs the scalar
  operations on it); the late `f2f32(op(f2fmp(a)))` rule does not see
  through that `vec2`, so they stay fp16.
* The colour's narrowing of a select of two constants is folded into a
  16-bit select (`nir_try_constant_fold_alu`,
  `nir_opt_constant_folding.c:58-150`); a select with another side is
  narrowed after it (s4_b2f). Panfrost does not set `support_16bit_alu`,
  so `nir_opt_algebraic.py:3149-3150` never fires.
* predlive's `a * 2.0` of a local is NIR's `fmul(#a, fmul(b, #c))` rule
  (`:2406`), and its `s` the late `bcsel` of two `fadd`s (`:4139-4143`).
* A mediump tree's constant is rounded to nearest even
  (`_mesa_float_to_half`); mixprec_hello.expect.json pins it.

Two corpus programs follow the same rules but have no reference yet (the
drm-shim run of each was not recorded): es100/tri_fragcoord (`1/1280` rounded
to `0x1266`) and es100/fragz (gl_FragCoord.z of a highp shader is
`LD_VAR_SPECIAL` of `frag_z`, `pan_nir_lower_var_special.c:17-38`; the ISA
reads its update as `.none`). Record them with `tools/mali/drm_shim_ref.sh
run` (`es100/fragz.frag` with `prec_84.vert`, `tri_fragcoord` with its own
`.vert`) and pin them like the others.

## 6. Caveats

* `gl_ref.py` binds no texture for an integer sampler, so the `isamp` and
  `fbload_int` descriptors in the references are the shim's defaults; the
  program words (what these references pin) do not depend on them.
* The references use the draw state of `gl_ref.py` (a 64x64 RGBA8 target,
  `GL_FLOAT` quad); the y pair is therefore `(1.0, 0.0)` in each.
* Nothing here was checked on the board.
