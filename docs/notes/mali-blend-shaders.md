<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali Valhall: Mesa's blend shaders (#33; gl_test cases 211 and 190, #31 S6, #32)

Sections 1-5 are the study of gl_test cases 211 and 190 (item `study-blend`
of plan 153); section 6 is how the blend shaders are built now, for every
equation (dEQP-GLES2 `fragment_ops.blend`). Mesa is 26.2.1 (`MESA_PAN_TREE`);
`G/` is `src/gallium/drivers/panfrost/`, `L/` `src/panfrost/lib/`, `C/`
`src/panfrost/compiler/`, `N/` `src/compiler/nir/`.

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
two shaders below (both built, section 6).

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
  `Evaluate per-sample: true`, `Multisample Enable: true`. Written by
  `src/mali/frame.rs` now (`evaluate_per_sample`, DCD0 bit 11; pinned by
  `frame_blend_tests.rs`).
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

## 6. How yakogl builds them: Mesa's NIR passes, then Mesa's selection

Sections 2 and 4 read the shim's words as shapes ("the factor made once", "a
product fused into an FMA"); there are hundreds of such shapes over the GLES 2
equations, and which one Mesa makes depends on the order its NIR passes meet
the instructions in. `add src_color dst_color` alone ends as
`ffma(s, s, fmul(d, d))` for one alpha equation and `ffma(d, d, fmul(s, s))`
for another (`is_only_used_by_fadd` sees a vectorised product). So the blend
shader is built as Mesa builds it:

1. `src/shaderc/online/mali/blend_nir.rs`: the NIR
   `pan_blend_create_shader` makes for the key (`build`, Mesa's builder calls
   in Mesa's order, arguments right to left as Mesa's compiler evaluates
   them), then the NIR passes that change a blend shader, each a
   transcription of Mesa's on one block: `nir_lower_alu_width`,
   `nir_opt_vectorize`, `nir_opt_copy_prop`, `nir_opt_dce`, `nir_opt_cse`,
   `nir_opt_constant_folding`, `nir_opt_move`, `nir_opt_shrink_vectors`,
   `nir_lower_load_const_to_scalar` and the three algebraic passes, in the
   order of `bifrost_preprocess_nir`, `bifrost_postprocess_nir` and
   `bi_optimize_late` (`optimize`). Each source records its place in its
   value's use list, as NIR's lists keep it; the passes that walk uses walk
   them in that order.
2. The algebraic passes run Mesa's transforms (`blend_nir_rules.rs`,
   generated from `nir_opt_algebraic.py` and `bifrost_nir_algebraic.py` by
   `tools/mali/gen_blend_nir_rules.py`): `nir_search.c`'s matching (the
   commutative source orders in bit order, a variable bound once), its
   automaton (an instruction's state is the set of items it matches; a use
   is visited again only when its state changes), its worklist (the last
   instruction first) and its replacement (before the root, after a negated
   value). A transform whose replacement builds an opcode this module does
   not, or whose condition it does not evaluate, refuses the key
   (`BlendVariantNotLowered`) when it would apply: never other bytes than
   Mesa's. The fp class conditions are `nir_range_analysis.c`'s.
3. `src/shaderc/online/mali/blend.rs`: the NIR selected in its order as
   `bifrost_compile.c` does (`bi_alu_src_index`'s halves, `bi_make_vec_to`
   for the colour, `bi_f32_to_f16_to` for one blend constant word, the
   preloaded registers through MOVs put first), with the two early back-end
   rewrites a blend shader meets: a swizzled `FCLAMP` source moved out to
   its readers (`bi_lower_swizzle`) and constant halves folded into the
   COLLECT. Then the existing passes (`finish`).

Not modelled, measured to leave the 16-bit code alone: the UBO load of the
constants is kept whole (Mesa splits it, `nir_lower_mem_access_bit_sizes`,
and joins it, `nir_opt_load_store_vectorize`; the back end reads the words as
FAU words either way).

In the gate (README-pan_shc.md "Blend shaders"): the words equal
`pan_shc --blend`'s for all 4103 keys of `blend_deqp.words` and
`blend_random.sha` (1403 dEQP keys; 2100 random fp16 keys, 1500 at 1x and
600 at 4x; 600 random fp32 keys), and the references, `pan_shc --blend`
and `blend_nir_rules.rs` are made again from `MESA_PAN_TREE` by the pan_shc
row; one key's final NIR is pinned (`blend_nir.rs` tests). Measured once,
2026-11-12, with a harness outside the tree: the final NIR equals the NIR
Mesa's passes print for the 1403 dEQP keys and the 600 4x ones (and a
Python model of the passes for all 3503 fp16 keys, state by state);
`pan_shc --blend` equals the shim driver on its 13 recorded shaders.

The colour's type is in the key (`colour_f32`, Mesa's `src0_type`): a highp
ES 3.00 `out vec4` is fp32 in r0-r3 and converted in the blend shader
(`V2F32_TO_V2F16`, the clamp fused). A `highp uvec4` colour into an RGBA8
UNORM target is not a GL error, but the result is undefined (ES 3.0,
shader outputs: values that do not match the buffer's format). It takes the
fp32 key, and that is Mesa's shader for it too: `pan_blend_create_shader`
loads 32 bits for a `nir_type_uint32` `src0_type` and converts them from
float, the target's base type, not from the source type (pan_blend.c:837-862,
"We assume the destination type here"), and `bi_emit_load_blend_input`
reads only the type's size (bifrost_compile.c:924-946). In the gate:
`blend_u32.words`, 100 uint32 keys made by `pan_shc --blend` with `u32`,
equal our words of the same keys at fp32
(`a_uint32_colour_s_blend_shaders_are_mesa_s`).
Every other integer colour output (an `ivec4`, an `int`, `uint` or narrower
vector, a `mediump` one, a `uvec4` highp only through `precision highp int`)
has no reference and is refused at the fragment compile
(`MaliError::IntColour`, `FsPrecision::of`), so it never reaches a blend
shader. The record's `colour_f16`, which `shader_key` reads, comes from one
rule, `FsPrecision::colour_f16`.

The frame side: `src/mali/blend.rs` `shader_key` maps a draw's equation,
colour mask, sample count and colour type (the fragment record's
`colour_f16`) to the key for an RGBA8 target at 1x or 4x, a MIN or MAX
channel's factors as ONE, ONE (not read, so one shader);
`src/mali/backend.rs` compiles it once per key (the 8 most recent kept) and
the frame writes the descriptor in Shader mode (section 3), with
`Evaluate per-sample` at 4x (DCD0 bit 11). The frame requires the address
exactly when `needs_shader` holds for a draw that writes a channel
(`check_draw`, `NOT_BUILT_BLEND_SHADER` without it, `INVALID_BLEND_SHADER`
for an address given where none is needed). Any other target (RGB565,
XRGB8888, RGBA16F) or a 2x one has no key, so the backend gives the frame no
address and the draw stays `NOT_BUILT_BLEND_SHADER` / `NOT_BUILT_FLOAT_BLEND`.

The Mesa 26.2.1 sources it follows (`MESA_PAN_TREE`): `pan_blend.c`
(`pan_blend_can_fixed_function`, `can_fixed_function_equation`,
`pan_blend_is_homogenous_constant`, `pan_blend_create_shader`), the choice in
`pan_context.c` `panfrost_get_blend`, `bifrost_compile.c` (the selection of
the blend NIR: `load_blend_input`, `load_tile_pan`, `blend_pan`,
`blend_return_pan`, the preloaded r60/r61) and `nir_opt_algebraic.py` with
`bifrost_nir_algebraic.py` (the transforms `blend_nir_rules.rs` is generated
from).

## 6a. The tests it rests on

| Test | What it pins |
|---|---|
| `src/mali/blend_choice_tests.rs` | `needs_shader` / `can_fixed_function` against a transcription of Mesa's rule over every GLES2 state (all fifteen factors, five equations, separate colour and alpha, homogeneous or not constants); RGBA8 and RGB565 the same path, RGBA16F the float one |
| `src/mali/frame_blend_tests.rs` | the frame: fixed-function descriptors byte for byte against the board (ten modes) and the drm-shim (case 91); the `Shader`-mode descriptor of case 8 and case 190 against the board; every shader-needing equation builds into an RGBA8 target at 1x and 4x given an address (descriptor `01 02 00 00`, no equation, address in word 3; DCD0 per-sample at 4x only); every fixed-function equation keeps the fixed-function descriptor and refuses a shader; no channel written needs none; what stays refused by name (an unbuilt target or sample count: no key, no address, `NOT_BUILT_BLEND_SHADER`; RGBA16F outside the float set: `NOT_BUILT_FLOAT_BLEND`) |
| `tests/mali_blend_states.rs` | every `fragment_ops.blend` state (the three families) through the GL core on the Mali mock: none refused |
| `src/shaderc/online/mali/blend_tests.rs`, `tests.rs` | the words: every dEQP state of the drm-shim reference (`pan_blend_ref.py`, RGBA8, 1x, fp16 colour) equals the builder's; `blend_random.sha` and `blend_u32.words` cover 4x and fp32 colours |

## 7. Facts and open points, one line each

- 211's blend shader choice is the non-homogeneous constant, not the 4x.
- 4x adds: the sample-id prologue (r61), `DCD0.Evaluate per-sample` (written
  by `frame.rs` for a 4x draw with a blend shader), the cache key's sample count. The descriptor words are the 1x shape (mode Shader, pc).
- 190 needs two shaders (columns 3 and 4); columns 1 and 2 are fixed function.
- No constant is inlined in 26.2.1; the board's 25.0.7 inlines them and, at 4x,
  ends in `ST_TILE` instead of `BLEND`: to be settled by the board run
  (section 4).

## 8. RGBA16F targets: fixed function, no blend shader (case 91 hdrblend)

Mesa 26.2.1 makes `R16G16B16A16_FLOAT` blendable on v9+ (`BFMT_FLOAT`,
`pan_format.c:66-76, 117-121`), so `panfrost_get_blend`
(`G/pan_context.c:170-200`) reads `fixed_function_float`, the equation with
`is_float` (`G/pan_cmdstream.c:4381-4407`): add, subtract or reverse subtract
with the factors ZERO, ONE, SRC_ALPHA and ONE_MINUS_SRC_ALPHA only, or ZERO
with ONE_MINUS_SRC_COLOR or DST_ALPHA on the destination of an add
(`can_fixed_function_equation`, `pan_blend.c:36-47, 77-86`). Case 91's two
equations (ONE, ONE_MINUS_SRC_ALPHA and SRC_ALPHA, ONE_MINUS_SRC_ALPHA) and
every colour mask (blending off is fixed function) are in that set; no blend
shader is needed. `to_mali_function` does not read `is_float`, so the
equation word is the UNORM one.

What `src/mali` builds (`blend::can_fixed_function_float`; `frame.rs`
`check_draw`, `OpenFrame::float_blend`, `end_frame`), pinned by
`src/mali/frame_blend_tests.rs` (`case_91_*` and the three RGBA16F tests
after them) against `tools/mali/testdata/fsout_91_hdrcolor.trace`, the
drm-shim's hdrcolor draw into **RGBA8** (`gl_ref.py` has no float target),
with the fragment program of `fsout_91_hdrcolor.expect.json` uploaded:

| Word | Mesa's (hdrcolor, RGBA8) | Ours, RGBA16F | Differs |
|---|---|---|---|
| Blend w0 | load destination, enable, constant 0, round false | the same | no |
| Blend w1 (equation) | `A Src, B Dest, C Src Alpha inverted` x2, mask 15 | the same | no |
| Blend w2-w3 | Fixed-Function, 4 comps, RT 0, `RGBA8 TB RGBA` (0xED000), raw false | `RGBA16F RGBA` (0xDF000, `MALI_BLEND_RGBA16F`) | bytes 13, 14 only: `d0 0e` -> `f0 0d` |
| DCD0 / DCD1 (r57 / r58) | 0x21AA / 0x1FFFF | the same | no |
| RT `Internal Format` | R8G8B8A8 (1) | RGBA16F (0x12) | yes, the format |
| RT writeback (bits 35..39) | 0x13 (`R8G8B8A8` / `RGBA16F` float) | 0x13 (`Writeback Float Format` RGBA16F) | no |
| RT clear | 8-bit pattern | raw halves (`pan_pack_raw`) | yes, the format |
| RT block format | AFBC, CRC | linear | yes, as every frame (`frame_mrt_tests.rs`) |
| Tile, allocation | 4 bytes a sample | 8 bytes a sample: 1024 px, 8192 | the format |

The right half's descriptor is the board's `over` (Mesa 25.0.7, RGBA8,
`tools/mali/board-dump/blend-2026-10-19.txt`) with the same conversion bytes
replaced. A frame whose RGBA16F draws are all opaque writes of every channel
keeps the board's 25.0.7 words (`Internal Format` RAW64, `Writeback Format`
RAW64: case 54, `frame_formats_tests.rs`); one blended or colour-masked draw
makes the frame's tile buffer RGBA16F (Mesa 26.2.1's words for any RGBA16F
frame). The memory the writeback produces is the same halves either way.

Still refused, `NOT_BUILT_FLOAT_BLEND`: an equation outside the float set
into an RGBA16F target (a colour or destination-colour factor, the 2src
form, a constant, min, max), which Mesa runs in a blend shader;
`shader_key` builds RGBA8888 keys only.

Open, for the board: no dump shows `Internal Format` RGBA16F on the T6 (the
board's Mesa 25.0.7 has no `BFMT_FLOAT` and keeps RAW64 with a blend shader
for a blended draw). A Mesa 26.2.1 board reference of case 91, or the case
itself passing x2, confirms that the hardware blends in an RGBA16F tile
buffer with these words; until then the RT words are 26.2.1's source and the
drm-shim's RGBA8 frame with the format named.
