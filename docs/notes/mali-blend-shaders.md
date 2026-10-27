<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: Mesa's blend shaders for gl_test cases 211 and 190 (#1384 S6, #1385)

A study, not an implementation (item `study-blend` of plan 153; the item
`blendshaders` builds it). Mesa is 26.2.1 (`MESA_PAN_TREE`), the byte
reference the drm-shim; `G/` is `src/gallium/drivers/panfrost/`, `L/`
`src/panfrost/lib/`, `C/` `src/panfrost/compiler/`, `N/` `src/compiler/nir/`.
What the frame does today for the constant colour's blend shader at 1x is in
`docs/notes/mali-frame-refusals.md` section 3 and
`src/shaderc/online/mali/blend.rs`; this note is what is new.

## 1. When Mesa picks a blend shader

`panfrost_get_blend` (`G/pan_context.c:163-229`), per render target, in
this order:

1. fixed function (returns 0, no shader) if the equation is fixed function
   (`info.fixed_function`, set at CSO creation, `G/pan_cmdstream.c:4398-4401`:
   `pan_blend_can_fixed_function`, `L/pan_blend.c:284-298`, and the constant
   rule below), the format is blendable, there is no alpha-to-one **and** the
   constants it reads are one value (`pan_blend_is_homogenous_constant`,
   `L/pan_blend.c:244-258`);
2. none if the target is not written (`!info.enabled`);
3. none if the equation is opaque on arch >= 6;
4. otherwise a blend shader.

`can_fixed_function_equation` (`L/pan_blend.c:65-106`): only ADD, SUBTRACT,
REVERSE_SUBTRACT (no MIN/MAX); for a non-float target the 2src form
(`dst*src + src*dst`), else both factors supported (`factor_is_supported`:
no SRC_ALPHA_SATURATE) and, up to an invert, **equal, or one of them ONE/ZERO**
(`src == dest || src == ONE || dest == ONE`, :102; arch >= 6 allows
DST_ALPHA). The colour and alpha equations must each pass.
`pan_blend_supports_constant` (`L/pan_blend.h:94`) is false for arch 6 and for
arch 7 RT > 0 only; the G610 is arch 10, so a constant is never a reason
by itself. `src/mali/blend.rs` (`can_fixed_function`, `needs_shader`) already
mirrors all of this.

**Case 211 is not "4x falls back to a shader".** The sample count is not an
input of the choice. `glBlendColor(0.25, 0.5, 0.75, 1)` with
`CONSTANT_COLOR`/`ONE_MINUS_CONSTANT_COLOR` reads three different constants
(`blend_factor_constant_mask`: RGB for both), so rule 1 fails
(`panfrost_get_blend`'s homogeneous check) at 1x too: it is case 8's fifth frame
(already built, `blend::Equation`), and at 4x the same draw takes a *different
shader* (the sample count is in the cache key, section 2). In the shim's
trace the blended draw's descriptor is `Mode: Shader`.

**Case 190 (`blendonce`) draws four quads in one frame**
(`driver/tools/pvrlab/gl_test.c:16510`), each under one state, over a cleared
(0.25, 0.5, 0.75, 0.5) pixel; the source (0.75, 0.5, 0.25, 0.25):

| column | state | Mesa |
|---|---|---|
| 1 | ADD, `SRC_ALPHA, 1-SRC_ALPHA` colour and alpha | fixed function (factors match up to an invert) |
| 2 | the same, red masked (`colormask 0b1110`) | fixed function (mask in the equation word, `Color Mask: 14`) |
| 3 | colour `MIN`, alpha ADD `SRC_ALPHA, 1-SRC_ALPHA` | **blend shader** (MIN) |
| 4 | colour ADD `1-DST_ALPHA, 1-SRC_ALPHA`, alpha ADD `ONE, ZERO` | **blend shader** (`DST_ALPHA` vs `SRC_ALPHA`: unequal, neither ONE/ZERO) |

The trace confirms it (`Mode: Fixed-Function` twice, `Mode: Shader` twice,
descriptors in section 3). Columns 1 and 2 need nothing new; 3 and 4 are the
two shaders below. The existing `shader_equation` returns None for both
(`b.rgb != b.alpha`, and a MIN/`DST_ALPHA` factor).

## 2. How the shader is built

`GENX(pan_blend_get_shader_locked)` (`G/pan_blend_cso.c:59-130`) caches on
(format, src0/src1 types, rt, logicop, **nr_samples**, the **equation without
the constants**, alpha_to_one); a new key calls `pan_blend_create_shader`
(`L/pan_blend.c:798-943`), then `pan_blend_cso.c`'s two steps and the
compile. The cache key has no colour mask except through `equation.color_mask`
(part of the equation): two draws of one equation and different masks are two
shaders. A draw with `nr_samples` is `surf->nr_samples ?: texture->nr_samples`
(`pan_context.c:203`): **4 for the backend's 4x view of a 1x image**.

The NIR (`pan_blend_create_shader`), for an RGBA8 target:

1. `dest_type` fp16 (`pan_unpacked_type_for_format`, 8-bit narrowed to 16 on
   arch >= 6, :825-828). The source colour is `load_blend_input` (4 x
   fp16: Valhall delivers it in r0-r1); the second source is the same load
   with `dual_source_blend_index 1` (dead without dual-source blending).
2. The destination: `load_tile_pan` with `pan_nir_tile_rt_sample(rt, sample)`
   (`C/pan_nir.h:16-23`: a 32-bit word of bytes `(sample, rt, 0, 255)`, y = 255
   "the current pixel"), the coverage `pan_nir_tile_default_coverage`
   (`cumulative_coverage & 0x1f`, :46-49) and the high word of the opaque
   internal descriptor as the conversion operand. **The sample is
   `nr_samples > 1 ? load_sample_id : 0`** (`pan_blend.c:880-883`).
3. `nir_color_blend` (`N/nir_lower_blend.c:911-990`), per channel: the
   source and the constants are `fsat`ed for a UNORM target (:943-947; **the
   destination is not**; the inputs, not the factors, are clamped); a missing
   channel reads 0 (alpha 1); `nir_blend_factored(func)` is true for
   ADD/SUBTRACT/REVERSE_SUBTRACT only: then `psrc = src * f_src`, `pdst = dst *
   f_dst` (`nir_blend_factor`, :179-196: the factor's value, `1 - f` for an
   inverted one, ONE folded); then `nir_blend_func` (:51-70): `fadd`, `fsub`,
   `fsub` reversed, **MIN/MAX: `fmin`/`fmax(src, dst)` with no factor at all
   (case 190 column 3's colour)**. A factor that is the same value in
   several channels (`1 - As`, `1 - Ad`) is the same NIR value, so CSE builds it
   once: this is what "blendonce" tests (the shim's column 3 computes `1 - As`
   once for alpha; column 4 computes `1 - Ad` once for red, green, blue).
4. `nir_color_mask(color, dest, rt.color_mask)` then the format's channels
   (:918-925): with all four channels written no select is emitted.
5. Only if `color != dest`: `blend_pan(cumulative_coverage, opaque
   descriptor (imm64), color)`, then `blend_return_pan` (`BRANCHZI` on the link
   register r48, arch >= 9).
6. The constants: `load_blend_const_color_rgba` becomes a load of the sysval
   UBO's first vec4 (`pan_lower_blend_constants`, `pan_blend_cso.c:41-57`),
   which every **fragment** shader pushes as its first four FAU words
   (`bi_pick_blend_constants`, `C/bifrost/bi_opt_push_ubo.c:92-114`; see
   `docs/notes/mali-frame-refusals.md` section 3): the blend shader reads
   them as FAU `u0`..`u1` (`V2F32_TO_V2F16 r, u0.w0, u0.w1`), **never an
   inlined constant, and the shader does not depend on the glBlendColor
   value**. The compile has `is_blend` and `fau.pushable_ubos` for the sysval
   UBO only, so the shader has no FAU words of its own (`fau.count == 0`, the
   check in `src/shaderc/online/mali/blend.rs finish`).

What changes between the cases, from the shim's words (section 5):

- **Case 211 (4x, constants)**: the equation is case 8's (`CONSTANT_COLOR,
  ONE_MINUS_CONSTANT_COLOR`, ADD both) and the body after the destination load
  is the 1x body with other register numbers (same FMA/FADD/`V2F32_TO_V2F16`
  sequence, same final `BLEND ... [r2:r3], r60, target:0x0` and `BRANCHZI`).
  Only the prologue differs: 1x builds the pixel word as the constant
  `0xFF000000` (`IADD_IMM r4, #0xFF000000`) and the destination is `LD_TILE @r4:r5`.
  At 4x:

  ```
  MOV.i32 r2, r61                           ; the preloaded sample word
  IADD_IMM.i32 r3, 0x0, #0x1F
  RSHIFT_AND.i32 r2, r2, 0x13121110.b00, r3 ; (r61 >> 16) & 0x1f: the sample id
                                            ;   (bifrost_compile.c:887-896)
  MKVEC.v2i8 r4, 0x0.b0, 0xFFFFFFFF.b0, 0x0 ; the bytes (0, 255) of the word
  MKVEC.v2i8 r2, r2^.b0, 0x0.b0, r4^        ; (sample, rt 0, 0, 255): pan_nir_tile_rt_sample
  LSHIFT_AND.i32 r3, r60, 0x0.b00, r3^      ; coverage & 0x1f (r3 held the 0x1f)
  IADD_IMM.i32 r4, 0x0, #0xED000            ; the conversion word, as at 1x
  LD_TILE.v4.f16.slot0 @r2:r3, r2^, r3^, r4 ; sample's destination
  ```

  then the blend, and the result is written by the same `BLEND` as at 1x, with
  r60 the coverage of the sample this thread runs (per-sample shading, below).
- **Case 190 column 3** (MIN colour): sources `fsat`ed (`FADD.clamp_0_1 r, r,
  0`), `1 - As` as `FADD.v2f16 r2, r1.neg.h11, 1.0.h11`, alpha
  `As*As + Ad*(1-As)` as two `FMA.v2f16`, red/green/blue `FMIN.v2f16` of the
  saturated source against the unsaturated destination, `MKVEC.v2i16` for the
  vec4, the `BLEND`, `BRANCHZI`. 16 words.
- **Case 190 column 4**: `1 - Ad` (one `FADD` of the destination pair's alpha
  half, `.h11`), `1 - As`, three channel products fused by `FMA.v2f16`
  (`src * (1-Ad) + dst * (1-As)`), alpha ONE/ZERO is the source alpha itself
  (no instruction), `MKVEC.v2i16`, the `BLEND`, `BRANCHZI`. 17 words.

## 3. Descriptor fields that change

`panfrost_emit_blend` (`G/pan_cmdstream.c:286-427`), per target, with a
shader (`blend_shaders[i] != 0`): the shim's trace of the 211 and 190 draws:

| field | fixed function | blend shader |
|---|---|---|
| word 0 `Load Destination` | `info.load_dest` | the same (true for 211, and for 190 columns 3 and 4) |
| `Enable`, `sRGB`, `Round to FB precision` | | unchanged (`Round to FB precision: false`) |
| `Blend Constant` | the packed homogeneous constant | **not written (0)**, :336-337 |
| word 1 `Equation` | `pan_pack_blend`, with the colour mask | **not written (0)**: the trace's `Color Mask: 0` with the mask 15 or any; the colour mask lives in the shader (`nir_color_mask`) |
| words 2-3 `Internal.mode` | `Fixed-Function` / `Opaque` | **`Shader`** |
| `Internal.shader.pc` | | the shader's address, **low 32 bits** (211: `4294959488` = 0xFFFFE180; 190: `4294959488`, `4294959744` = 0xFFFFE280, 16-byte aligned, 256 bytes apart in the shim); no `return_value` on v10 (arch <= 7 only, :378-383) |
| `Fixed-Function.*` (rt, num_comps, conversion) | set | **not set** (the trace prints `Num Comps: 1`, `Raw: true`, an unnamed memory format: zero). The target's format/conversion is in the shader instead: the `LD_TILE`'s conversion word and the `BLEND`'s opaque internal descriptor (`pan_blend_get_internal_desc`, `L/pan_blend.c:975-1001`: mode OPAQUE (1), `num_comps` 4, rt 0, memory format `pan_dithered_format_from_pipe_format(fmt, false)` = 0xED000 for RGBA8; low word `0x19`) |

Already built in `src/mali/frame.rs` (the `Shader`-mode branch of the blend
descriptor, the `INVALID_BLEND_SHADER` alignment/window checks).

Outside the blend descriptor, **at 4x with a blend shader**:

- `DCD0.Evaluate per-sample` = true (`valhall_has_blend_shader`,
  `G/pan_csf.c:1568-1570`, `G/pan_jm.c:570-572`: `rast->multisample &&
  (min_samples > 1 || valhall_has_blend_shader)`, set per draw by
  `panfrost_emit_blend_valhall`, `pan_cmdstream.c:921-932`). At 1x it is
  not set (the multisample term). The trace of the 211 blended draw has
  `Evaluate per-sample: true`, `Multisample Enable: true`. **Not yet written
  by `src/mali/frame.rs`** (no `evaluate_per_sample` outside the genxml).
- The fragment shader's preload needs r60 (coverage) and **r61 (the sample
  word)**: Valhall fragment programs preload both always
  (`C/bifrost/bifrost_compile.c:4690-4701`; ours, `flow.rs:564`, `tests.rs:2818`
  `preload >> 48 == 0x3000`). Nothing to add; the shim's trace has
  `R60: true, R61: true` for both draws of 211.
- `Sample Count: 4`, tile size, etc., are the 4x frame we build already
  (case 208/207); the blend shader neither changes the FBD nor the tiler.

## 4. The words against the existing disassembly (2026-10-21-mesa-refs3)

`0dce654a8:tools/mali/board-ref/2026-10-21-mesa-refs3/blend-shaders.disasm.txt` is the
**board's Mesa 25.0.7** (case 8's 1x shader, 19 words, and case 211's 4x
one, 19 words). It is not the shim's Mesa 26.2.1, and the two differ; the
byte reference of this project is the shim's (the 1x
`tools/mali/testdata/blend_*.expect.json` are 26.2.1's, and
`src/shaderc/online/mali/blend.rs` equals them):

| | board 25.0.7 (`blend-shaders.disasm.txt`) | shim 26.2.1 (`blend_case211.*`) |
|---|---|---|
| constants | immediates in the shader (`0x3A003A00`, `0x34000000.h11`): the key holds them | pushed FAU `u0.w0..u1.w1` through `V2F32_TO_V2F16`: one shader serves every glBlendColor |
| 4x destination | `LD_TILE @r4:r5, r2, r60, r3`, r60 whole (no 0x1f mask) | masked coverage `& 0x1f`, the sample word built by `MKVEC.v2i8` |
| 4x write | **`ST_TILE.v4.f16.slot1 @r0:r1, r2, r60, r3`** at the same pixel word | **`BLEND.slot1.v4.f16 @r0:r1, [r2:r3], r60`** with the internal descriptor, as at 1x |
| length | 19 words | 22 words |

Both read the sample id the same way (`MOV r61`, `RSHIFT_AND` by 16, mask
0x1f) and both read the destination with `LD_TILE` per sample. The record
(`README.md` there) is right for the board's Mesa: `ST_TILE`. 26.2.1 builds
the `BLEND` form (`pan_blend_create_shader` emits `blend_pan` at `:926` for every sample count
and arch >= 6; the `ST_TILE` path of 25.0.7 is not in 26.2.1's blend shader
builder, `bi_emit_st_tile` serves other users). **Open, for the board session after `blendshaders`: that the shim's
`BLEND` at 4x with a per-sample thread's r60 writes only the thread's sample.**
It is what Mesa 26.2.1 would run on the T6; the pixels are the proof. If it
fails there, the fallback is the board's `ST_TILE` words
(`blend-shaders.disasm.txt:26-44`), with the operands in the shim's
registers; neither is guessed here. The case-211 diagonal check
(`0x80008060`, two samples of each destination) is the one that tells.

Case 190 has no board record yet (Mesa on the board for 190 is a `mesarefs4`
item); the 1x blend shaders of 25.0.7 and 26.2.1 differ in the constants only
(no constant is read by 190), so the words below are expected to match the
board's Mesa up to register naming, not verified.

## 5. The reference files and how to reproduce them

```
tools/mali/testdata/blend_case211.{words,txt}        the 4x constant blend's shader, 22 words
tools/mali/testdata/blend_case190_col3.{words,txt}   MIN colour / SRC_ALPHA alpha, 16 words
tools/mali/testdata/blend_case190_col4.{words,txt}   1-Ad,1-As colour / ONE,ZERO alpha, 17 words
tools/mali/testdata/blend_case190_211_ref.py         the GL calls of the two cases (ES 3 context)
tools/mali/testdata/blend_case190_211_ref.sh         the run on the shim (drm_shim_ref.sh's environment)
tools/mali/testdata/blend_case190_211_words.py       the extractor
```

`*.words` is one 64-bit word per line, the 8 bytes little-endian as the
`words` of the `*.expect.json` files; `*.txt` is pandecode's bytes and
disassembly of the `Blend shader` block. The shader is the whole block (it
ends in its `BRANCHZI.absolute.end`; the padding of the allocation is not
printed). The two cases differ from gl_test in the pass-through program and
the viewport only: the blend shader depends on the state, not the program
(the fragment shader's `BLEND` and `BRANCHZI` are the program's own and are in
the existing `blend_*` references). 4x is `glFramebufferTexture2DMultisampleEXT`
with 4 samples, as the backend's 4x and gl_test's case 208/211 board
references.

```sh
# the shim (built once, as the other references): tools/mali/drm_shim_ref.sh build <mesa-26.2.1.tar.xz> <work>
W=<work>                      # holds inst/lib64
T=tools/mali/testdata
for c in 190 211; do bash $T/blend_case190_211_ref.sh $W $c /tmp/blend$c; done   # /tmp/blend$c.trace
python3 -I $T/blend_case190_211_words.py /tmp/blend190.trace $T/blend_case190_col3 $T/blend_case190_col4
python3 -I $T/blend_case190_211_words.py /tmp/blend211.trace $T/blend_case211
```

It was run twice and the six files are identical. Shim:
`$CONNECTED_DEPS_DIR/mesa-shim` (a `drm_shim_ref.sh build` of 26.2.1).

## 6. What yakogl changes (for `blendshaders`; nothing here is implemented)

`src/shaderc/online/mali/blend.rs` (the compiler):

- `Equation` (`{src, dst}`, ADD both channels, constant-colour factors) grows
  to the whole `nir_color_blend` input: per channel (`rgb`, `alpha`) a `Func`
  (ADD, SUBTRACT, REVERSE_SUBTRACT, MIN, MAX) and src/dst `Factor` (ZERO, ONE,
  SRC_ALPHA, DST_ALPHA, the inverts, the constant's), the colour mask, and the
  target's sample count (1 or 4). It is also the cache key (Mesa's key minus
  the constants).
  `Factor::SrcAlpha`/`InvSrcAlpha`/`InvDstAlpha` and `Func::Min` are what 190
  needs; SRC_COLOR/DST_COLOR/SATURATE and the rest of the table can stay
  `BlendVariantNotLowered` until a reference asks.
- `shader()`: after the existing prologue/`PIXEL_INDICES` code, take the
  sample count: **at 4x emit the prologue of section 2** (`MOV r61` through
  `bir::Op::MovI32` of `Idx::Reg(61)`, `RshiftAndI32`, two `MkvecV2i8` (an
  op `select`/`opt` may lack: `lower_mkvec_swz`), then `LD_TILE` with that
  word and the masked coverage), and keep the final `BLEND` + `BRANCHZI`.
  The per-channel arithmetic generalises the existing `match (eq.src, eq.dst,
  ..)`: `fsat` of the source (and of the constants when read), a factor
  value computed **once per distinct factor** (`1 - As`: one `FADD.v2f16` of
  the pair's `.h11` lanes feeds both channels: `Src`'s `.h11` selection),
  `FMIN.v2f16`/`FMAX` of source and destination for MIN/MAX (no factors, the
  destination unsaturated), a product fused into the sum as an `FMA.v2f16` with
  the other term as addend (the existing fusion; a ZERO/ONE factor drops its
  multiply). The words are the NIR vectorisation's (pairs of 16-bit lanes:
  rg and ba), so mixed per-channel equations (column 3, 4: colour and alpha
  differ) split into the pair that has the alpha channel and the pair that
  does not, as `blend_case190_col*.txt` shows (`MKVEC.v2i16 r1, rA.h0, rB.h1`
  joins the halves). Expect the ordering/register allocation
  (`sched::pressure_schedule`, `ra::allocate_blend`) to need a pass over
  each reference: the register numbers differ per equation.
  Byte-equal tests against the three `.words` files, in `tests.rs`' blend
  section (where the `blend_*.expect.json` run), loaded with `include_str!`
  (the words are the u64s of the `words` field).
- `ra.rs allocate_blend` and `flow.rs`: nothing known to change; the 4x
  prologue reads r61 (preloaded) and r60.

`src/mali/blend.rs`:

- `shader_equation(&Blending) -> Option<blend::Equation>`: stop requiring `rgb
  == alpha`, `Func::Add` and constant-only factors; map the rest of the
  table to the new `Equation`; None for a combination `shader()` does not
  build (the refusal stays by name). It also takes the colour mask and the
  sample count (a `Target` argument) since they are part of the key.
- `needs_shader` is already right for 190 (MIN and the unequal `DST_ALPHA`
  pair are not `can_fixed_function`) and 211 (`constant()` is None).

`src/mali/backend.rs`:

- `draw_blend_shader` (`:560-577`): `built = mask == 0xf && target.samples == 1
  && target.format == Rgba8888` is the gate: drop `target.samples == 1` (4x is
  built), keep the mask rule until a masked blend shader has a reference
  (`nir_color_mask` with a partial mask is not in the shim files here: a
  mask 14 column with a MIN would be the case), pass `target.samples` in the
  `Equation`.
- `blend_shader` (`:524`): the cache compares `eq`; `BLEND_SHADER_WORDS` is 32
  (the longest here is 22), `BLEND_SHADERS` 8 (190 needs 2). The key is the
  new `Equation`.
- the draw's `GlesDraw` already carries `color_mask`, `blend` and samples.

`src/mali/frame.rs`:

- the DCD0 (`:2927` area): **`evaluate_per_sample: t.samples > 1 &&
  d.blend_shader.is_some()`** (the one frame-side change 211 needs; also
  update the 4x test that pins DCD0's bytes);
- `NOT_BUILT_BLEND_SHADER`'s text (`:184`) and its comment (the 4x and
  separate colour/alpha now built).

Tests to move with it: `src/mali/frame_blend_tests.rs:225-235` (the refusal
row of MIN and of `const` stays only for what is still refused: its MIN is now
built), `src/mali/backend_tests.rs:895-900`, the `NOT_BUILT_BLEND_SHADER`
rows at `:1081`/`:1124`, and `MALI_TESTS` in `tools/mali/gate.env`. The gl_test
sweep: 211 and 190 turn from refused to PASS only on a T6 run; a Mesa
reference for 190 on the board is the `mesarefs4` item.

## 7. Facts and open points, one line each

- 211's blend shader choice is the non-homogeneous constant, not the 4x.
- 4x adds: the sample-id prologue (r61), `DCD0.Evaluate per-sample`, the cache
  key's sample count. The descriptor words are the 1x shape (mode Shader, pc).
- 190 needs two shaders (columns 3 and 4); columns 1 and 2 are fixed function.
- No constant is inlined in 26.2.1; the board's 25.0.7 inlines them and, at 4x,
  ends in `ST_TILE` instead of `BLEND`: to be settled by the board run
  (section 4).
