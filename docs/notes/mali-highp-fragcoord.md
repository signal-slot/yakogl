<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall fragment stage: highp fp32 arithmetic (S2) and gl_FragCoord (S4)

Date 2026-10-20, tip `fc2f48f3a`. Plan #152 item `notes-highp-fragcoord`, for
issue #1384 (`gh` was not readable from the worker; the scope is
`driver/docs/nanopc-t6-mali.md`, "The general lowering (approved 2026-10-19)",
and the gap list of `docs/notes/mali-lowering-map.md`, "Gaps this note found in
the design", item 1). It closes the two holes that map left: it has sections
for S3-S6 but none for S2, and it names `gl_FragCoord` as a gap without the
design. Host research only: no file under `src/` or `tools/` changed, no
board run, nothing compiled.

Citations are Mesa 26.2.1 (`MESA_PAN_TREE`, here
`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`). `B/` is
`src/panfrost/compiler/bifrost/`, `N/` is `src/compiler/nir/`, `G/` is
`src/compiler/glsl/`. Target: the G610, arch 10 (`bifrost_nir_options_v9`,
`pan_compiler.c:93-97`), so every `arch < 11` is read as true. Our files are
under `src/shaderc/online/mali/` on `fc2f48f3a`, **before S1c**; the line
numbers below are that commit's. S1c has since landed: `frag.rs` (at
`da18ff68`) was deleted in `2db7bfbd` (`b75d518c` on `work/mali-lower-s1c`),
and the whole fragment lowering is in `nir_fs.rs` (`walk`, layout) and
`select.rs`. Re-grep the names before relying on a line.

What the references say is quoted from `tools/mali/testdata/*.expect.json`
(Mesa 26.2.1's GL driver on drm-shim, the bytes the plan wants), not from
reasoning. Where a claim is read from a reference and not derived from Mesa's
source, it says so.

## Part 1. S2, highp fp32 fragment arithmetic

### 1.1 How Mesa decides fp16 or fp32 for a fragment value

Mesa takes the width from GLSL precision, in the GLSL front end, before NIR
exists as a shader. Four steps:

1. **The cap.** Panfrost sets `fp16` and `glsl_16bit_consts` unless
   `PAN_DBG_NOFP16`, and `fp16_derivatives` and `fp16_const_buffers` on arch
   >= 6 (`gallium/drivers/panfrost/pan_screen.c:546`, `:585-588`). It does
   **not** set `int16` (`:591`, "Blocked on mesa issue 6075"). `st_extensions.c:318-322`
   turns the fp16 cap into `MediumFloat`/`LowFloat` ranges.
2. **The GLSL pass.** `lower_precision` runs only for an ES shader whose stage
   has the fp16 or int16 cap (`G/glsl_parser_extras.cpp:2487-2491`). Its
   per-rvalue rule is `handle_precision` (`G/lower_precision.cpp:295-312`):
   `GLSL_PRECISION_HIGH` is `CANT_LOWER` (`:304-305`), `MEDIUM` and `LOW` are
   `SHOULD_LOWER` (`:306-308`), "none" is decided by the operands (`:302-303`).
   A section of the expression tree is lowered only when everything in it can
   be (`find_lowerable_rvalues`, `:1355-1362`). So **a highp operation stays
   32-bit, and it is the only thing that does**: there is no "fp32 mode" to
   switch on, fp32 is what `lower_precision` leaves alone.
3. **Variables.** `lower_variables_visitor::visit(ir_variable)`
   (`G/lower_precision.cpp:1023-1060`) retypes only temporaries, autos and
   non-UBO float uniforms (`:1025-1031`) whose precision is medium or low
   (`:1033-1034`). It does not touch shader inputs and outputs. A mediump
   uniform becomes an fp16 uniform (the "fp16 block" of our `mod.rs`
   `MaliError::FsUniform`).
4. **NIR.** The fragment varyings and outputs are narrowed by
   `nir_lower_mediump_io(shader_in | shader_out, ~bi_fp32_varying_mask, false)`
   (`B/bifrost_nir.c:914-916`). `bi_fp32_varying_mask` (`:486-501`) keeps a
   varying at fp32 when it is flat-shaded or read as a texture coordinate
   (`bi_gather_texcoords :449-483`, "fp16 texture coordinates are not
   supported by the hardware", `:439-442`). Then `nir_lower_bit_size`,
   `nir_lower_alu_width` and `nir_lower_phis_to_scalar` run with
   `bi_vectorize_filter` (`:1087-1090`): fp32 ALU is width 1 (one lane per
   instruction), 16-bit ALU is width 2 (`:176-179`, `max(1, 32 / bit_size)` of
   the larger of destination and first source), `f2f16`/`u2f16`/`i2f16` width 2
   on arch < 11 (`:154-160`), `frcp`/`frsq` stay scalar (`:146-153`).
   `nir_lower_flrp` runs for 16, 32 and 64 bit (`:1083`).

**Two builtins are mediump in GLSL ES 1.00 and highp only from ES 3.00**:
`gl_FragCoord` (`G/builtin_variables.cpp:1333-1335`, `frag_coord_precision`)
and `gl_FragColor` (`:1371-1374`, `GLSL_PRECISION_MEDIUM`). That is why
`precision highp float;` in a femtovg shader still gives an fp16 **colour**:
`pan_nir_lower_fs_outputs` handles a `float16` colour variable
(`src/panfrost/compiler/pan_nir_lower_fs_outputs.c:109-117`, `:139-141`),
and the highp value is
converted at the store. `tools/mali/testdata/cases.txt:8-11` and the `# gl_FragCoord is mediump`
remarks on `fvg_fill_color_nodiscard` and `fvgtest_cover` are the pan_shc
SPIR-V path disagreeing with this, not a different Mesa behaviour.

**What the references show for a highp program** (all three read from the
`fs.fragment` region of the reference, `"text"`):

| Reference | FS words | Pins |
|---|---|---|
| `fvgtest_green` (`vec4(0,1,0,1)`) | `NOP`, `ATEST @r60, r60^, 0x3F800000`, `MOV.i32 r0, 0x3C000000`, `MOV.i32 r1, 0x3C000000`, `BLEND.slot0.v4.f16.end` | the colour is **fp16 even for a constant**: 0x3C00 is 1.0 as half, both halves in one word; `preload` `0x3000000000000000` (r60, r61), `work_reg_count` 32, `fau` 4 zero words |
| `fvgtest_ftcoord` (`vec4(ftcoord.x, ftcoord.y, 0, 1)`) | `LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store @r0:r1, r61^, index:0x0`, `V2F32_TO_V2F16 r0, r0^, r1^`, `ATEST ... 0x3F800000`, `MOV.i32 r1, 0x3C000000`, `BLEND.slot1.v4.f16.end` | a highp varying is loaded as **fp32 lanes** (`.f32` register format over an `src_f32` varying) and the only fp16 is the colour store: a pair of loaded lanes converts with `V2F32_TO_V2F16`; the blue/alpha constants are one `MOV.i32` |
| `fvg_fill_gradient_nodiscard_noscissor` | fp32 `FADD`, `FMA`, `FMIN`, `FRSQ`, `FRCP`, `FROUND.f32.rtn`; colour lanes leave through `FADD.f32 r1.h0, r2^, 0x0.neg` (an `f2f16` of one lane into a low half), `MKVEC.v2i16 r1, r1^.h0, r2^.h0`, then `BLEND ... v4.f16`; `ATEST @r60, r60^, r0.h0` on the converted alpha | all the arithmetic is fp32, **one lane per register**; the colour is converted lane by lane and packed with `MKVEC.v2i16` (these are the `F2f16 16 1` and `Vec2 16 2` rows `emit.rs` already has) |

`work_reg_count` is 32 in every `fvg_*` and `fvgtest_*` reference, including
the unrolled `fvg_filter_image` (checked in all of them). The two forms of the
colour conversion (a `V2F32_TO_V2F16` of a loaded pair against `FADD.f32 .h0`
plus `MKVEC.v2i16`) are both Mesa's; which one a given shape gets depends on
what `nir_opt_vectorize` found (open point 4).

### 1.2 Where the back end assumes fp16 today

Everything in the fragment stage that is not a vertex-stage reuse is written
for the mediump case, and **nothing in it knows a value's precision**: the IR
has none (design, "Precision per value is a later shared item"), and the only
precision read is each *uniform's* (`mod.rs` `compile_module_on`,
`prec(u.name)`, `:911`, `:963`, passed as `mediump` to the fragment plan).
A highp fragment shader is therefore not refused for its precision, it is
compiled **as fp16** if its shape is one the plan takes. `fvgtest_green` and
`fvgtest_ftcoord` are "ok" in `tools/mali/corpus-verdicts.txt:191-192` only
because a constant and a pure copy are fp16 colours in Mesa as well; a highp
`a + b` of two varyings would be accepted and wrong. (Read from the code; not
run.) By file and function:

| File | Function / item | The assumption |
|---|---|---|
| `mod.rs` | `compile_module_on`, `FsPath::Graph` branch (`:908-935`) and the default `FsPath::Plan` branch (`:937-980`) | `mediump` is the *uniform's* precision only; `sc.info.colour_f16 = true` is a constant (`:933`, `:978`); `MaliError::TexHighp` refuses a highp sampler (`:918-920`, `:958-960`, text `:321-323`) |
| `mod.rs` | `MaliError::FsUniform` (text `:286-288`), `Fp16Inexact` (`:355`), `ColourZeroHalf` (`:352`) | the fragment uniform is "one mediump vec4 read in its own lane pairs"; every constant must be exact in fp16 |
| `nir_fs.rs` | file header (`:9-12`), `Fs`, `walk` (`:254`) | "the arithmetic as 16-bit nodes (the mediump lowering)"; the `Alu` arm (`:343-373`) applies the algebraic rules to 16-bit lane **pairs** (`ident`, `:348-362`); any other op is `MaliError::Arithmetic` (`:373`); `LoadInput { .. }` other than a varying or `gl_PointCoord` is `SpecialInput` (`:316`) |
| `nir_fs.rs` | `vectorise` (`:636`), `Pairs`/`P`/`Leaf` (`:546-632`) | `nir_opt_vectorize` at width 2: `f16_exact` of every constant (`:637`, `:691`, `:699`), converted halves `Leaf::Half`, `Leaf::Mk` (a lane and a constant half) |
| `nir_fs.rs` | `fs_uniform` (`:719`), `late_rules` (`:731`, `F16X2_TWO`, `:42`) | an fp16 uniform block, two lanes a word; `fmul(a, 2.0)` as an fp16 pair |
| `nir_fs.rs` | `select` (`:768`), `convert` (`:915`), `operand` (`:980`), `op` (`:994`), `tex` (`:956`) | every operation is `Row { bits: 16, width: 2 }`; every varying load is followed by `f2f16` rows (`convert`); the look-up result is read as fp16 pairs (`TexSingle { f16: true }`) |
| `frag.rs` (historical: at `da18ff68`, deleted in `2db7bfbd`) | `Lanes::of`, `plan_fs` (`:838`), `vectorise` (`:761`), `fs_uniform` (`:746`), `late_rules` (`:1016`), `fs` (`:1089`), `convert` (`:1223`), `tex` (`:1258`), `FsPlan`, `REFERENCED` (`:553`), `F16X2_*` (`:70-73`) | the old plan, the fragment path before S1c; its work now lives in `nir_fs.rs` (`vectorise`, `fs_uniform`, `late_rules`, `select`, `select_highp`, `convert`, `tex`, `F16X2_TWO`) |
| `select.rs` | `f16_exact` (`:43`) | the exactness test every constant goes through |
| `emit.rs` | `ROWS` (`:32-49`) | the fp32 rows are the vertex stage's (`Fadd Fmul Ffma Frcp Fmax Fmin Fneg`, width 1); every fragment row is 16-bit width 2 except `F2f16 16 1` (`FADD.f32 .h0`) |
| `nir.rs` | `Alu` (`:70`) has no bit size; `Val::Pair` (`:63`) is the two-lane fp16 value; `Alu::F2f16`, `Alu::Vec2` (`:79-87`) | an fp32 value is `Val::Lane`, an fp16 value is `Val::Pair`: the bit size is in the `Val`, so a highp fragment value is the vertex stage's form and needs no new `Alu` |
| `fau.rs` | `FauWord::UniformHalves` (`:34-38`) | the only fragment-uniform form; fp32 uniform words (`FauWord::Uniform { dword }`) exist but are used by the vertex stage (S3's work) |
| `bir.rs` | `FaddV2f16`, `FaddImmV2f16`, `FmaV2f16`, `V2f32ToV2f16`, `MkvecV2i16`, `LdVarBufImmF32` | the fp16 ops exist and stay; the fp32 ops (`FaddF32`, `FmaF32`, `FaddImmF32`, ...) exist for the vertex stage |
| `opt.rs` | `mod_prop_forward` (`:107`), `lower_opt_instructions` (`:133`), `fuse_add_imm` (`:150`) | already handle both widths (`fuse_add_imm` makes `FaddImmF32` or `FaddImmV2f16`, `:168`) |
| `tests.rs` | every fragment population (`FS_*_POPULATION`, `FRAGMENT_VARYING_POPULATION`, `TRI_FS`...) | all 25 sources say `precision mediump float` and none `precision highp` (`grep -c`); `:859-880` pin the refusals on mediump sources; no test draws a highp fragment program |

Mesa's own side of this table is `bi_vectorize_filter` (`B/bifrost_nir.c:106-181`):
the mediump path is `nir_opt_vectorize` pairing 16-bit lanes; the highp path
is the same scalar form the vertex stage already has.

### 1.3 Register allocation and FAU effects of fp32

- **One lane, one register.** An fp32 lane takes a 32-bit register; an fp16
  pair takes one. The same colour that is 2 registers of pairs is 4 lanes of
  fp32 plus the conversions. Mesa's allocator tries the split file first,
  r0-r15 and r48-r63 (`B/bi_ra.c:435-438`), and sets `work_reg_count = 32`
  when that succeeds (`:1264-1280`); else the full 64-register file
  (`:1285-1292`), with spilling past `BI_MAX_REGS = 64` (`B/compiler.h:1114`,
  `bi_ra.c:1236-1262`). Our `ra.rs::allocate` (`:328`) does the split file
  only and returns `MaliError::Spill` otherwise (`ra.rs:433`, text
  `mod.rs:359-361`). Every highp reference in `testdata` is 32, even the
  unrolled filter, so S2-S6 do not need the full file; the first fp32 program
  that does is refused with `Spill`, which is the right verdict.
- **Pre-RA scheduling matters more.** `bi_pressure_schedule` runs before RA
  (`B/bifrost_compile.c:4575`) and our `sched.rs::pressure_schedule` (`:166`)
  is transcribed; fp32 lanes raise the pressure it fights.
- **Constants.** An fp32 constant takes the immediate table when the value is
  there as fp32 or as a half (`fau.rs` `lut32`, `lut16`, `:328-340`; the
  references show `0x38000000.h1`, an fp32 0.5 in the table's half form, in
  `FMA.f32` sources) and otherwise a promoted FAU word or an `FADD_IMM`
  (`fuse_add_imm`). The fp16 pairs' `F16X2_*` constants go away for highp.
  `lower_constants` is shared and already runs on both widths.
- **Uniforms.** A highp uniform is a plain 32-bit UBO word, pushed as
  `FauWord::Uniform { dword }`; only a mediump one is the fp16 `UniformHalves`
  form (`G/lower_precision.cpp:1025-1031`). That fragment-side push is S3, not
  S2: S2 keeps `MaliError::FsUniform`.
- **Varyings.** No change in the load: the load is `LD_VAR_BUF_IMM.f32` in
  both cases (`convert`, `nir_fs.rs:915-930`, already loads fp32 and
  converts); for highp the conversion step is dropped and each lane is
  `split` from the load.

### 1.4 What pins the words

| Reference | In `tools/mali/testdata` | Status |
|---|---|---|
| Case 28 (`fvgvs`, `driver/tools/pvrlab/gl_test.c:58`): `main.vert` + `fvgtest_green.frag` | `fvgtest_green.expect.json` (`fs.fragment` above) | `cases.txt` `equal` (pan_shc against the shim); our compiler's FS bytes against it are **not** a test today: no `tests.rs` string names `fvgtest_green` |
| Case 29 (`fvgft`, `gl_test.c:58-59`): `main.vert` + `fvgtest_ftcoord.frag` | `fvgtest_ftcoord.expect.json` (`cases.txt:82`, `fs.fragment=equal vs.position=equal vs.varying=equal`) | the same |
| Other highp constant programs | `fvg_stencil`, `fvg_stencil_nodiscard`, `fvgtest_discard2`'s non-discard path | `equal` against pan_shc too; same gap |
| The mediump regression | the populations of `tests.rs` (`:1570`, `:2076`, `:2105`, `:2134`, `:2242`, `:2451`) | byte-equal to `tools/mali/testdata/*.expect.json` today; must stay |

Also `tools/mali/corpus-verdicts.txt:191-192` (`fvgtest_ftcoord`, `fvgtest_green`:
`ok`; `tests.rs:727` pins the verdicts, "may only move forward").

Because S2's acceptance cases are copies and constants, **S2 adds no
arithmetic row**: `emit.rs` already has the fp32 rows a copy and a conversion
need. Arithmetic on fp32 fragment values is first drawn by S4's references.
S2 is: carry the width, take the highp path for a highp program, drop the pair
machinery for it, and prove it byte-equal on the two references above (new
tests, in the style of `the_texture_population_is_byte_equal_to_mesa_26_2_1`,
`tests.rs:2451`, reading the two `expect.json` files).

### 1.5 Steps for `s2-highp`

0. **After S1c (landed).** The fragment stage is on the shared graph;
   `frag.rs` (and its `FsPlan`, `REFERENCED`) was deleted in `2db7bfbd`. The
   names of 1.2 are re-grepped on that tree: `nir_fs.rs` `walk`, `select`,
   `select_highp`, `convert`, `tex`; `select.rs` `f16_exact`.
1. **Decide the precision rule (open point 1) and expose it.** The front end
   keeps only a bool (`sema.rs:839`, `default_float_precision`) and per
   uniform a `Precision` (`mod.rs:814`). S2 needs the fragment shader's
   default float precision (and any highp/mediump qualifier on a varying)
   as data on the module: a small `FsPrecision { Medium, High }` passed to
   `compile_module_on` next to `mediump`. Recommended rule: **a fragment
   shader whose default float precision is highp takes the highp path, one
   whose is mediump the existing path, a mixture is refused** with a new
   `MaliError` (name: the S2 author's; message in the style of the others).
   This also fixes the silent-fp16 hazard of 1.2, which S2 must at least
   refuse if it does nothing else.
2. **The highp walk.** In `nir_fs.rs::walk`, the `Alu` arm for a highp program
   builds `Val::Lane` nodes with the 32-bit rows (`emit::Row` bits 32, width
   1) exactly as `nir::build` does for the vertex stage, instead of the
   `Pairs`/`vectorise` machinery. The algebraic rules that apply at 32 bit
   are the vertex stage's (`late_rules`, `fuse` in `nir.rs`); reuse them, do
   not copy. Keep `Fp16Inexact` for the mediump path only.
3. **The load.** `convert` for a highp varying: `LdVarBufImmF32` then `split`;
   no `F2f16` rows. A texture coordinate lane stays fp32 as today.
4. **The colour store.** Per colour lane `F2f16 16 1` (`FADD.f32 .h0, x, -0.0`)
   or, for an adjacent pair that both come out of one load, `F2f16 16 2`
   (`V2F32_TO_V2F16`); constant lanes as one `MOV.i32` of the half pair
   (`0x3C000000` for (0, 1.0)); then `MkvecV2i16` of two `.h0` halves where the
   lanes are separate; ATEST on the converted alpha's `.h0` or on the constant
   `0x3F800000`. The form follows the source of the two lanes (open point
   4): **a pair takes `F2f16 16 2` exactly when both lanes are components of
   one SSA value inside the same aligned pair (`.xy` or `.zw`), otherwise each
   lane is `F2f16 16 1` and the pair is `MkvecV2i16`.** The graph therefore has
   to keep "lane i of value V" (the load, a pushed uniform vec4), not only the
   lane's number, for the colour lanes.
5. **Keep every refusal that is not S2's.** A highp uniform stays `FsUniform`
   (S3), a highp sampler `TexHighp` (Tex), an operation with no row
   `Arithmetic` (S4), `gl_FragCoord` `SpecialInput` (S4).
6. **Tests.** (a) Add `fvgtest_green` and `fvgtest_ftcoord` (and `fvg_stencil`,
   `fvg_stencil_nodiscard`) to a byte-equal test against their `expect.json`
   `fs.fragment` words, `work_reg_count`, `preload` and `fau`; (b) a highp
   program that needs an unavailable row is refused, not compiled as fp16
   (one assertion in the `tests.rs:859-880` list); (c) every mediump population
   stays byte-equal (nothing to add, nothing may move); (d) `corpus-verdicts.txt`
   unchanged or forward only; (e) `MALI_TESTS` in `tools/mali/gate.env` moves by
   the number of tests added (that file is under `tools/mali/**`: coordinate
   with the running `frame-msaa-zs`, rebase on it).
7. **Board.** Sweep cases 28 and 29 PASS and cases already PASS stay PASS
   (the plan's acceptance, run by the item that has the board).

## Part 2. gl_FragCoord

### 2.1 How Mesa gets the fragment position on v10

There is no varying and no `LD_VAR` for the position on Valhall: **x and y are
a preloaded register, z and w are separate**.

1. **GLSL to NIR.** `gl_FragCoord` is `load_frag_coord` (a vec4 system value;
   `N/nir_intrinsics.py:984`).
2. **Origin and centre, in the state tracker** (before NIR optimisation;
   `st_nir_lower_wpos_ytransform`, `mesa/state_tracker/st_glsl_to_nir.cpp:395-435`,
   options `:407-418`). Panfrost's caps are **upper-left origin and
   integer pixel centres** (`pan_screen.c:810-811`, `:814-816`, comment
   "pixel coord is in integer sysval on bifrost", `:809`), GLSL's defaults are
   lower-left and half-integer, so `nir_lower_wpos_ytransform` rewrites every
   read (`N/nir_lower_wpos_ytransform.c:159-204`, `lower_fragcoord`):
   `invert = true` (`:169-178`) and `adjX = adjY = 0.5` (`:194-200`). The
   rewrite (`emit_wpos_adjustment`, `:60-118`) is
   `x' = x + 0.5` (`:84-85`) and `y' = ffma_weak(y + 0.5, T.x, T.y)`
   (`:99-101`, `:103-111`, `base = invert ? 0 : 2`), where `T` is the new
   `gl_FbWposYTransform` vec4 state uniform, `STATE_FB_WPOS_Y_TRANSFORM`
   (`get_transform`, `:40-57`). **That is the "+0.5 centre", and it is a
   uniform-dependent y: the y flip is not free.**
3. **The value of `T`** (`mesa/program/prog_statevars.c:736-751`): for a
   framebuffer not flipped (`!FlipY`): `(1, 0, -1, height)`; for a flipped one
   (a window-system buffer): `(-1, height, 1, 0)`. With `invert` the shader
   reads `.xy`. (The reference's pair is `(1.0, 0.0)`, FAU words 18-19 of
   `fvg_fill_gradient_nodiscard_noscissor.expect.json`, read as `u9.w0`, `u9.w1`
   by `FMA.f32 r6, r6^, u9.w0, u9.w1`.) The value is the draw's, not the
   shader's: PVR does the same pair, `fragcoord_y_pair`
   (`src/pvr/backend.rs:1939-1946`, `(-1, height)` on a top-down target, else
   `(1, 0)`).
4. **NIR to the register.** In `bifrost_postprocess_nir`
   (`B/bifrost_nir.c:904-910`) fragment shaders run
   `nir_lower_frag_coord_to_pixel_coord`
   (`N/nir_lower_frag_coord_to_pixel_coord.c:10-29`): `load_frag_coord` becomes
   `vec4(u2f32(load_pixel_coord), load_frag_coord_z, load_frag_coord_w)`
   (`:20-24`). The comment (`:16-18`) says the centre shift is **already
   applied by the caller** ("`nir_lower_wpos_center` for VK, or
   `pixel_center_integer` for GL"), which is why step 2 comes first and why
   `u2f32` of an integer is correct. `load_pixel_coord` is a **16-bit** vec2
   (`N/nir_intrinsics.py:987-988`). It is **`u2f32`** only: there is no `i2f32`
   in it (correcting `mali-lowering-map.md`, "Gaps", item 1, which names
   "`U2f32`/`I2f32`").
5. **The preload.** `load_pixel_coord` is one `MOV.i32` of
   `BI_PRELOAD_POSITION_XY` (`B/bifrost_compile.c:1945-1947`, "Vectorized load
   of the preloaded i16vec2"); `BI_PRELOAD_POSITION_XY` is register **59**
   (`B/compiler.h:1138`, `bi_preload_reg` `:1207-1208`): x in the low half,
   y in the high half (read as `r59.h0`, `r59.h1` in the references). Which
   registers a program needs preloaded is "live before the first block"
   (`B/bifrost_compile.c:4685-4687`), plus r60 and r61 always on Valhall
   (`:4689-4701`). The driver writes the mask into the descriptor
   (`gallium/drivers/panfrost/pan_cmdstream.c:4515`, `:4534`,
   `preload.r48_r63 = info.preload >> 48`); `reads_frag_coord`
   (`pan_compiler.c:278-280`) feeds the driver's `PAN_VARY_FRAGCOORD` bit
   (`pan_cmdstream.c:2614`); the compiled Valhall programs load x and y from
   no varying (the references have no `LD_VAR` for the position), so that bit
   is not needed for them (not checked for z and w).
6. **The instructions.** The `u2f32` of the 16-bit vec2 is width 1 after
   `nir_lower_alu_width` (`bi_vectorize_filter`, `B/bifrost_nir.c:176-179`:
   `max(32, 16)`), so two scalar `u2f32` with a 16-bit source, which
   `bi_emit_alu` selects as `U16_TO_F32` (`B/bifrost_compile.c:3167-3176`; the
   32-bit-source vector form at `:2814-2837` is not this one; v11 removed
   `U16_TO_F32`, `:3168`, not our arch). Then `FADD x, 0.5` (made an
   `FADD_IMM` by `va_fuse_add_imm`, `B/valhall/va_optimize.c:67-110`) and the
   `FMA`. **Read from the reference** (`fvg_fill_gradient_nodiscard_noscissor`,
   and the same four lines in `fvg_fill_gradient_conic_nodiscard_noscissor`):

   ```
   U16_TO_F32 r5, r59.h0
   U16_TO_F32 r6, r59^.h1
   FADD_IMM.f32 r5, r5^, #0x3F000000
   FADD_IMM.f32 r6, r6^, #0x3F000000
   FMA.f32 r6, r6^, u9.w0, u9.w1          ; y * T.x + T.y
   ```

   `preload` is `0x3800000000000000` (r59, r60, r61) in every reference that
   reads `gl_FragCoord`, `0x3000000000000000` in every one that does not.
7. **Precision.** `gl_FragCoord` is mediump in ES 1.00 (1.1 above), but the
   reference computes the dither entirely in fp32 from it: there is no
   `V2F32_TO_V2F16` and no half arithmetic on the position, because the only
   consumer, `ditherNoise(gl_FragCoord.xy)`, is a highp function parameter
   (femtovg declares `precision highp float`) and a shader input is not
   retyped by `lower_precision` (1.1, step 3). This is read from the
   reference, not proved from the source: a program that did mediump arithmetic
   on `gl_FragCoord` is not in the corpus and would need its own reference.

### 2.2 The preload entry and the rows it needs

**The preload entry.** `preload.rs` is *not* it: that file builds the
framebuffer colour/ZS preload **programs** (`pan_fb_preload.c`), nothing to do
with register preload. The register preload is made in three places, none of
which names r59 today:

- `select.rs::preloads` (`:411-433`): reads of `Idx::Reg(r)` in a program become
  one `MOV.i32` per register at the start (`bi_preload`); the vertex stage
  already uses `Idx::Reg(59)` (`select.rs:157`, the `LEA`'s base) and
  `Idx::Reg(60)`, `Idx::Reg(61)`; the fragment varying loads use
  `Idx::Reg(61)`.
- `ra.rs` (`:417-433`): the coalescing of `MOV.i32` from a preloaded register
  (`bi_ra.c:490-522`) is generic over `reg`, and `default_affinity`
  (`:340`) includes r48-r63, so r59 needs no pin.
- `flow.rs::preload_mask` (`:399-408`): live-in registers, plus bits 60 and 61
  for a fragment shader. Bit 59 comes out of it when the program reads r59.
  `sc.info.preload` goes to the RSD through `src/mali` as `preload >> 48`
  (the same as the colour preload, `src/mali/backend.rs:476`).

So the **entry is a new constant in `ra.rs` beside `CUMULATIVE_COVERAGE`
(`:27`): `POSITION_XY: u8 = 59`**, and the fragment walk reads `Src::of(Idx::Reg(POSITION_XY))`.
No new mechanism.

**The rows.** The design's "(op, bit size) to BIR" table needs, for
`gl_FragCoord.xy`:

| Row | Mesa case | BIR | Ours |
|---|---|---|---|
| `U2f32` with a **16-bit source**, width 1 | `nir_op_u2f32`, `src_sz == 16` (`B/bifrost_compile.c:3167-3176`) | `U16_TO_F32 dst, src.h0/.h1` | missing: no `Op::U16ToF32` in `bir.rs` (the generated ISA has it, `isa/v10.rs:864-865`, `U16_TO_F32` value 337, `:10768`), no `pack.rs` arm; `Row` has no source size, so the row needs one (a source-size field or a distinct `Alu::U16ToF32`) |
| `Fadd` 32 with an fp32 constant 0.5 | `nir_op_fadd` | `FADD.f32` then `FADD_IMM` | present; `opt.rs::fuse_add_imm` does the fusion |
| `Ffma` 32 | `nir_op_ffma` | `FMA.f32` | present |

The consumers of the position (`fdot2`, `ffract`, the `mix`) are S4's other
rows; `gl_FragCoord` itself adds only the first row. `i2f32` is not needed here
(it is S4's only if a program converts a signed int).

**The y transform is a FAU input the design does not have.** `T.x`, `T.y` are
two pushed uniform words (`FauWord::Uniform`-like) whose values the draw
decides: a new variant, for example `FauWord::FbWposYTransform(component)`,
mapped like `Sysval` in `src/mali/program.rs:68-92` (the kind table and the
push writer), with the value supplied at draw time from the target's flip
(`(1, 0)` or `(-1, height)`, 2.1 step 3). Its place in the FAU order is
`bi_pick_ubo`'s: it sits in the default uniform block after the declared
uniforms (`pan_shc.c:456-496` mirrors where Mesa puts the state slot;
`st_update_state_param_locations`), so after S3's pushed words, in increasing
word order (`B/bi_opt_push_ubo.c`, see `mali-lowering-map.md` section 3). The
reference puts it at words 18-19 behind the uniform words the program reads;
**dump the FAU table and compare** rather than reasoning about the offset.
`src/mali/**` is owned by the running `frame-msaa-zs` item: this part of
S4 waits for it to land.

### 2.3 Where it lands in our code

| File | Change |
|---|---|
| `nir_fs.rs` `walk` (`:316`) | the `LoadInput { .. } => SpecialInput` arm gets a `IoSlot::FragCoord` case (the front end delivers it as `LoadInput`, `lower.rs:3690`, 4 lanes; `reflect.rs:1021` counts it as a varying for the packing budget only). Lanes 0 and 1 build the graph of 2.1 steps 2-6; lanes 2 and 3 (`frag_coord_z`/`w`: a separate interpolation) keep `SpecialInput`. femtovg reads `.xy` only. After S1c this is the shared graph's walk. |
| `nir.rs` | a `Kind` for the pixel-coordinate lane (the register read) and the `Alu` for the 16-bit-source conversion |
| `emit.rs` | the `U2f32` 16-bit-source row (above), added to `ROWS` when the reference draws it |
| `bir.rs`, `pack.rs` | `Op::U16ToF32`, with its info row and packing; the half select of the source (`.h0`, `.h1`: check what `Swz` means for a one-source conversion, `bir.rs:105-110`, which is the fp16-pair swizzle) |
| `select.rs` (`preloads`) | none: it makes the `MOV.i32` from `Idx::Reg(59)` already |
| `ra.rs` | `POSITION_XY = 59` beside `CUMULATIVE_COVERAGE` |
| `flow.rs` (`preload_mask`) | none |
| `fau.rs`, `src/mali/program.rs`, the draw that writes the push block | the `FbWposYTransform` word and its draw-time value |
| `mod.rs` | `MaliError::SpecialInput`'s text keeps naming `gl_FragCoord` until S4 lands, then loses it (`gl_FrontFacing`, `gl_PointCoord`, `gl_VertexID`, `gl_InstanceID` stay) |
| `link.rs` | check that `gl_FragCoord` takes no varying slot in the Mali link (the packing budget counts it, `reflect.rs:1005-1021`, Mesa's does not allocate one on Valhall) |
| `tests.rs` | `:885` pins `gl_FragCoord` as `SpecialInput` on a mediump source: it becomes the test of lane 2/3 or of mediump arithmetic on it |

### 2.4 Steps for `s4-fragcoord`

Order: after S2 (a highp fragment graph) and S3 (the pushed uniform words the
transform sits behind), inside S4 after the rows it shares (`fadd`, `ffma`).

1. **A shim reference for `gl_FragCoord` alone.** None exists: the
   `s4_*` references (`tools/mali/testdata/s4_*.frag`) have no `gl_FragCoord`,
   and the only references that read it are the gradient and conic programs,
   whose other rows (`fdot2`, `ffract`, `fsqrt`, `fsat`, ...) are S4's.
   Add one minimal highp program (for example
   `gl_FragColor = vec4(gl_FragCoord.xy * 0.01, 0.0, 1.0);` under
   `precision highp float`), with the same `cases.txt` row and `.expect.json`
   as the others (`tools/mali/drm_shim_ref.sh`; drawn twice, byte-equal). It is
   under `tools/mali/**`: coordinate with `frame-msaa-zs`. Read the five
   instructions of 2.1 step 6 and the FAU words off it.
2. **The row and the op.** `Op::U16ToF32` (with `pack.rs`), the `emit.rs` row
   with a source size, admitted by that reference.
3. **The walk.** `FragCoord` lanes 0 and 1 in the fragment walk: `u2f32` of the
   register's half, `fadd 0.5`, and for y `ffma(y', T.x, T.y)`, in Mesa's
   order (the transform wraps the intrinsic first, then the pixel-coordinate
   lowering, 2.1 steps 2 and 4). Refuse lanes 2 and 3 with `SpecialInput`.
4. **The preload.** `ra::POSITION_XY`; confirm the `preload` word is
   `0x3800000000000000` and `work_reg_count` 32 on the reference.
5. **The transform word.** `FauWord::FbWposYTransform`, the `program.rs`
   mapping, the draw-time value; compare the FAU table to the reference. The
   path already has the value as data (open point 3): `emit_stage_uniforms`
   has `flip` (`d.flip_y.then_some(target_height)`, `frame.rs:2203`) in the
   scope that writes the push words (`frame.rs:2244`), so the new
   `PushWord` arm computes `(1.0, 0.0)` for `None` and `(-1.0, height)` for
   `Some(height)` there; no new field on `DrawCall`, `GlesDraw` or the GL
   layer. The shim's value is `(1.0, 0.0)`; the board's window-system target
   is flipped, so the board picture is the proof of the sign, not the shim.
6. **The two gradient programs.** `fvg_fill_gradient_nodiscard_noscissor` and
   `fvg_fill_gradient_conic_nodiscard_noscissor` go byte-equal once the rest of
   S4's rows are in (both have no live `if`; `cases.txt` pins them
   `differs:unanalysed` at 13 and 4 words against pan_shc); the image variants wait for the
   Tex slice. The same four-line `gl_FragCoord` block is in all 20 programs
   of the map's gap 1.
7. **Verdicts.** `tools/mali/corpus-verdicts.txt` moves forward for those
   programs; the board picture of a gradient (dither) is compared with Mesa's.

## Open points

1. **The precision rule (S2, step 1): decided, whole-shader, carried by the
   interface, not by `sema.rs`.** (Plan #152 item `notes-highp-settle`; re-read
   and every citation below re-grepped on tip `cb33efd5`, i.e. after S1c's
   switch-over and `s1c-delete2`: `mod.rs` has one fragment branch,
   `compile_stage`'s `else` at `:890-915` over `nir_fs::walk`; the `FsPath`
   enum and the plan branch are gone.) What S2 uses, and why:
   - **The representation.** One value for the whole fragment shader: the
     float precision in force at the end of the translation unit's top-level
     `precision` statements, reduced to `FsPrecision { High, Medium }` (`Low`
     is `Medium`: Mesa lowers both, `G/lower_precision.cpp:304-308`, 1.1
     step 2). It is **not** a widened `default_float_precision` bool and
     **not** a precision per IR value. A module has no per-value data to
     widen: `ir::Module` carries `instrs`, `comps` (the width of each value),
     `nodes`, `locals`, `inputs`, `outputs`, `uniforms` and nothing about
     precision (`ir.rs:668-700`; `Module`, `:668`), and `lower.rs` never reads a `Precision`
     outside comments (`grep -n -i prec` finds `:2633`, `:3371`, `:5451`,
     `:5730`, all doc comments, and then only test sources from `:8631`): the front end discards the qualifier after
     `sema`. Per-value would mean putting a table in `Module` and teaching
     `lower.rs` ES's "result precision is the highest of the operands" rule
     (`G/lower_precision.cpp`'s `find_lowerable_rvalues`, 1.1 step 2) for
     five back ends; the design already defers that ("Precision per value is
     a later shared item", quoted in 1.2).
   - **Why not the sema bool.** `sema.rs:839` sets `default_float_precision =
     true` on any `precision <p> float;` and its only reader is
     `check_float_precision` (`sema.rs:875-886`, the ES 1.00 4.5.3 error for a
     fragment shader with no default). It runs before the IR exists, is private
     to `Sema`, and widening it to a value would make the checker the owner of
     a back-end input. The value is already computed, per stage and in
     declaration order, by `reflect::uniform_decls` (`reflect.rs:917-974`:
     `let mut float = ...` at `:919`, updated by `Decl::Precision` at
     `:928`; `resolved`, `:824`, turns a `Default` into it), and
     `Interface { reflection, uniforms, ast }` (`reflect.rs:978-982`, built at
     `:1057`) is
     what reaches the back end: the fragment side is `mali/mod.rs:1034`
     (`compile_module(&l.module, decls.uniforms, scratch)`), its vertex twin
     `:1055`; `:749-788` take `precision: &[UniformDecl]`.
     **S2 adds the final `float` of `uniform_decls` (and a `mixed` flag, next
     bullet) to that return and passes it where `precision`/`mediump` pass
     today** (`mod.rs:892`, `:905`): `compile_stage` branches on
     `FsPrecision` before `nir_fs::walk` (`mod.rs:893`; `walk` is `nir_fs.rs:463`); no change to
     `sema.rs`, `ast.rs` or `ir.rs`.
   - **What it handles.** `FsPrecision::High` for femtovg and every
     `fvgtest_*` (all `precision highp float`; `tools/shaders/fvgtest_*.frag`,
     `femtovg/*.frag`), `Medium` for every existing population (the 25
     `FS_*_POPULATION`/`TRI_FS`... sources of `tests.rs` all say `precision
     mediump float`, 1.2): the Medium branch is today's code, untouched, so
     those tests and `tests.rs`' references are the proof nothing moved.
   - **The mixture.** A shader that writes a float precision other than the
     default on a declaration other than a uniform (a `highp` varying or local
     under `mediump`, a `mediump` one under `highp`; `Low` and `Medium` count as
     the same) has `mixed = true`. `mixed` alone changes nothing: a *copy* of
     such a value is exact either way (`es100/light_amount.frag`,
     `varying highp vec3` under mediump, is `ok` today and its words are the
     `sel_whole_vec3_loc1` pattern (the `tests.rs:539-543` entry of `POPULATION`, `:489-549`), an fp32 load narrowed at the colour
     store, which is also what Mesa does). `mixed` **and** an arithmetic
     `Alu` in the module is refused with a new `MaliError` of the "is not
     lowered yet" class (name for the S2 author; `every_gap_has_its_own_message_of_the_not_lowered_class`
     in `tests.rs` pins the class). This is the only place the two precisions
     meet, and no corpus row has both (open point 2), so **no verdict line of
     `corpus-verdicts.txt` moves** because of the rule. A uniform's own
     precision keeps its present meaning (`mod.rs:892`: a mediump uniform
     block is the fp16 pair form; under `High` S3 pushes fp32 words and
     `MaliError::FsUniform` stays for the cases S3 does not take).
   - **Hand-over to S2.** The decision is the one step 1 of 1.5 recommended,
     with "a mixture is refused" narrowed to "a mixture *with arithmetic*", so
     as not to refuse `light_amount`.
2. **The silent hazard: measured. Real for a shape no corpus row has; zero
   affected corpus rows; fix within S2.** Commands: a throw-away `#[test]`
   appended to `src/shaderc/online/mali/tests.rs` (removed, not committed) that
   called `compile(SHC_STAGE_FRAGMENT, src)` (the host compiler, the build of
   `tools/mali/check.sh`: `cargo test --no-default-features --features
   host-test,gpu-mali,shaderc-online,no-shader-table --lib`) and printed
   `isa::decode` of every word of the result, for (i) every fragment source of
   `tools/mali/corpus-verdicts.txt` whose verdict is `ok` and which contains
   `highp`, (ii) every `tools/mali/testdata/*.frag` that declares `precision
   highp`, (iii) five synthetic sources.
   - **(i) Corpus rows.** The corpus has 31 `.frag` rows with verdict `ok`
     (`grep -F '.frag: ok' tools/mali/corpus-verdicts.txt`); 8 mention `highp`,
     every one a copy or a constant:

     | Row | highp as | Code (fragment) | Reference |
     |---|---|---|---|
     | `fvgtest_green.frag` | `precision highp float`, constant colour | `NOP, ATEST, MOV.i32 x2, BLEND` | `fvgtest_green.expect.json`, `cases.txt:83` `equal` |
     | `fvgtest_ftcoord.frag` | default, varying copy | `LD_VAR_BUF_IMM.f32, V2F32_TO_V2F16, ATEST, MOV.i32, BLEND` | `fvgtest_ftcoord.expect.json`, `cases.txt:82` `equal` |
     | `es300/cubelodv.frag` | default, vec4 varying copy | `LD_VAR_BUF_IMM.f32 (v3), V2F32_TO_V2F16 x2, ATEST, BLEND` | none; same words as `sel_whole_vec4_same` pattern |
     | `gltest/es3trunc_vary_fs.frag` | default, vec4 varying copy | as `cubelodv` | none; same |
     | `qt/smoothcolor.frag` (Qt; left the tree, #1509) | default, vec4 varying copy | as `cubelodv` | none; same |
     | `qt3d/debugobject.frag` (Qt; left the tree, #1509) | default, `vec4(vec3 varying, 1.0)` | `LD_VAR_BUF_IMM.f32 (v2), V2F32_TO_V2F16, FADD.f32 .h0, ATEST, MKVEC.v2i16, BLEND` | none; the `sel_whole_vec3_loc1` pattern |
     | `qt3d/clearshadowmap_300.frag` (Qt; left the tree, #1509) | default, constant `vec4(1.0)` | `NOP, ATEST, MOV.i32 x2, BLEND` | none; the `fvgtest_green` pattern |
     | `es100/light_amount.frag` | `varying highp vec3` under mediump | as `debugobject` | none; same pattern |

     In all 8 the only 16-bit operations are the colour-store conversions
     (`V2F32_TO_V2F16`, `FADD.f32 ... .h0`, `MKVEC.v2i16`) and 32-bit-lane
     loads, which are what Mesa emits for a highp copy (1.1 table, 1.4); **no
     `FADD.v2f16`/`FMA.v2f16`/other 16-bit arithmetic appears in any of
     them, and none uses fp16 where a reference uses fp32.** The two with a
     reference have, in the compile measurement above, word sequences equal to
     the `fs.fragment` text of their `expect.json` (this is our compiler against
     the shim; `cases.txt:82-83` pins pan_shc against it, a different claim); the others match a
     reference's pattern, not a reference (none exists for their sources; I did
     not run drm-shim). Affected rows: **0**.
   - **(ii) The `testdata` highp programs.** 61 `*.frag` declare `precision
     highp` (`s4_*`, `fvg_*`, `fvgtest_*`, `ctl_tex_highp`): 2 are accepted
     (`fvgtest_green`, `fvgtest_ftcoord`, above); the other 59 are refused
     today, by `Arithmetic` ("arithmetic other than +, -, * ..."), by "a
     `gl_FragColor` lane other than the varying's own" (`s4_ffma`, `s4_fneg`:
     a constant lane beside the arithmetic), by `FsUniform` (`fvgtest_inner`)
     or by `SpecialInput` (`ctl_tex_highp`). So **the whole reference
     population is safe by accident of the other refusals**.
   - **(iii) The hazard exists, on shapes no reference covers** (synthetic,
     compiled by the same call):

     | Source | Result |
     |---|---|
     | `precision highp float; varying vec4 a, b; gl_FragColor = a + b;` | **accepted**: `LD_VAR_BUF_IMM.f32`, `V2F32_TO_V2F16` x2, again for `b`, then `FADD.v2f16` x2, `ATEST`, `BLEND`: the sum is an fp16 add. Mesa keeps `a`, `b` as fp32 (highp is `CANT_LOWER`, 1.1 step 2) and adds in fp32 (4 `FADD.f32`, as `fvg_fill_gradient_nodiscard_noscissor.expect.json`'s arithmetic), narrowing once at the store |
     | same, `precision mediump float;` with `varying highp vec4 a, b;` | **accepted, same fp16 words** (the mixture of open point 1; Mesa fp32) |
     | same, `precision mediump float;` and mediump varyings | accepted, same words; **correct** (the mediump population) |
     | `highp`, `gl_FragColor = a * 0.3;` | refused, `Fp16Inexact` (0.3 is not exact in fp16): a non-fp16 constant is the one thing that stops it |
     | `highp`, `vec4(0.1, 0.2, 0.3, 1.0)` | refused, `Fp16Inexact` |

     So the hazard is `+`, `-`, `*`, negate and `abs` (the `Alu` arm,
     `nir_fs.rs:338-388` helpers, called from the `walk` arms `:554-583`) over all four colour lanes, constants exactly
     representable in fp16, in a highp shader; the damage is rounding to 11
     significant bits where Mesa keeps 24 (a coordinate above 2048, a sum of
     a large and a small term). Not run on the board: the source is known and
     the instruction stream shows it.
   - **Recommendation: fix within S2; no separate refuse-first commit.** The
     measured affected set is empty (0 corpus rows, 0 testdata references), so
     a refusal changes no verdict line and protects only shapes no committed
     file contains; S2 is the next item of the plan and replaces the
     refused shape with the right code, so a refusal first would be written
     and deleted in one stretch of work. What S2 must do is not let the
     window open in between: the *first* commit of S2's pull request is the
     branch on `FsPrecision` with `High` + any arithmetic `Alu` still refused
     (the gap class above), and the fp32 walk replaces that refusal in the
     later commits. That is one PR, bisectable, and the refusal is the same
     code that the mixture needs. If S2 slips past the other items, the
     refusal alone (`FsPrecision::High` and an `Alu`) is a four-line change
     to land at once; I did not make it (the contract of this item is the
     note).
   - **(c) The references and cases that pin S2's words.** Exist: *shim
     references* `fvgtest_green.expect.json` and `fvgtest_ftcoord.expect.json`
     (+ their `.vert`/`.frag`, `cases.txt:82-83`, `fs.fragment=equal` against
     pan_shc); for the fp32 arithmetic rows S2 shares with S4, `s4_ffma`,
     `s4_fneg`, `s4_fsat`, `s4_fceil`, `s4_ffloor` and the rest of
     `cases.txt:25-39,84-87` (all `precision highp`; the `s4_fneg` row is
     `fs.fragment=differs:unanalysed:1`: pan_shc and the shim differ in one word); *gl_test cases* 28
     (`fvgvs`, `fvgtest_green`, `driver/tools/pvrlab/gl_test.c:5068`) and 29 (`fvgft`,
     `fvgtest_ftcoord`, `:5134`), dispatch `:21622-21623`; both
     `PASS` on the board at the integration tip (`gl_test: case 28 fvgvs:
     PASS`, `case 29 fvgft: PASS` in all 6 `--sweep` logs of
     `0dce654a8:tools/mali/board-ref/2026-10-20-query/`: `baseline/` 2, `ours/` 2,
     `tip/ours/` 2), so they are
     regression guards, not goals. Case 30 (`fvgin`, `fvgtest_inner`) is S3's
     (`REFUSED` in every log, with the `FsUniform` message). **Missing:** (1) a Rust test of our
     `fs.fragment` words against `fvgtest_green.expect.json` and
     `fvgtest_ftcoord.expect.json`: `tests.rs` has no `fvgtest` reference
     (`grep -n fvgtest src/shaderc/online/mali/tests.rs` is empty; the
     tables at `:489-549` and `:1596-` list the `sel_*` programs only), so the
     two are pinned today only by the corpus verdict `ok`
     (`corpus-verdicts.txt:191-192`), which says "accepted", not "equal to
     Mesa"; the step-1 test of 1.4 (the `POPULATION` loop, `tests.rs:553-561`, is the style) is where they
     belong; (2) a shim reference for highp arithmetic over four varying
     lanes with no constant lane (`gl_FragColor = a + b;`, S2's own
     population: `s4_ffma` and `s4_fneg` carry constant lanes and are refused
     for that before the arithmetic); (3) a reference for the mixture, a
     mediump default with `varying highp` read by arithmetic, to pin the
     refusal against Mesa's fp32; (4) a gl_test case for highp arithmetic on
     the board: only the femtovg cases (28-33, 195, ...) and
     `gltest/kc_*`, `gltest/varyfma_fs.frag` (case 191) are highp and all are
     refused today for the other rows. (2) and (3) are `tools/mali/**`
     (`drm_shim_ref.sh`, `cases.txt`), the S2 item's to add.
3. **The y-transform value on the board: answered, the path has the flip; it
   lacks only the consumer.** Mesa's `T.xy` is `(-1, height)` for a flipped
   target and `(1, 0)` otherwise (`prog_statevars.c:736-751`, 2.1 step 3). The
   Mali frame path carries exactly that choice as data at draw time:
   - the source is `DrawCall::flip_y` (`src/mali/frame.rs:1215-1222`: "stored
     top-down (a window surface): Mesa's Y_0_TOP framebuffer", the same
     predicate Mesa's `FlipY` is), filled from `GlesDraw::flip_y`
     (`src/mali/backend.rs:1384`), which the GL layer sets from `fb_flip_y`
     (`src/gl/framebuffer.rs:1122-1124`: the window surface is flipped, an FBO
     and a pbuffer, `default_y_up`, are not; `framebuffer.rs:25`);
   - the target height is `t.height`, passed to `emit_stage_uniforms` for both
     stages (`frame.rs:2449-2451`); that function already derives
     `flip = d.flip_y.then_some(target_height)` (`frame.rs:2203`) and `sysval()`
     already uses it to mirror the viewport (`frame.rs:2167-2183`, Mesa's
     `st_atom_viewport.c:63-68`), so `flip` is the pair's whole input:
     `None` -> `(1.0, 0.0)`, `Some(h)` -> `(-1.0, h as f32)`;
   - `src/mali/frame_flip_tests.rs` is the proof that flipped and unflipped
     draws are separate cases on the board (it compares the viewport sysval
     words and DCD0's front-face bit of a pbuffer-style draw with Mesa's;
     tests of other modules set `flip_y: true` for a window target, e.g.
     `frame_query_tests.rs:130`).
   What is missing is only the consumer: `PushWord`
   (`frame.rs`, matched at `:2244`) and `FauWord` (`program.rs:68-72`, the
   `kind` table) have no word for the transform, and the fragment `sysvals`
   list is fixed at `[BlendConstants, ViewportScale, ViewportOffset]`
   (`program.rs:35`). Recommended: a push word whose value is computed from
   `flip` at `frame.rs:2244` (a `PushWord::FbWposYTransform { component }`),
   not a fourth sysval slot: the sysval UBO is a separate table
   (`frame.rs:2217-2230`) and Mesa's transform is a default-block uniform word
   (2.2), so the FAU bytes are the same either way and a slot would only add a
   UBO entry Mesa does not have. Nothing needs to be added to `DrawCall`,
   `GlesDraw` or the GL layer. Remaining, not read: the board check that the
   pbuffer (unflipped) and window (flipped) pictures of a gradient agree with
   Mesa's (2.4 step 5), which is the proof of the sign.
4. **Which colour-conversion form, when: answered.** From the source, the
   references and a NIR dump of one program of each form (`pan_shc` built at
   `$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1/build-pan/src/panfrost/pan_shc/pan_shc`;
   `BIFROST_MESA_DEBUG=shaders PAN_SHC_PRINT_NIR=1 python3 -I
   tools/mali/glsl2pan.py tools/mali/testdata/<n>.vert <n>.frag`, stderr; the
   compiler's own dump, after its passes, is the second NIR in the output).
   Pair, `fvgtest_ftcoord`:
   ```
   32x2  %2 = @load_var_buf_pan (%1 (0x0), %0) ...
   16x2  %3 = f2f16 %2
   16x4  %9 = vec4 %3.x, %3.y, %8 (0x0), %5 (0x3c00)
   ```
   Separate lanes, `fvgtest_cover` (two different defs, one a phi):
   ```
   16  %77 = f2f16 %12
   16  %78 = f2f16 %76
   16x4  %84 = vec4 %77, %78, %83 (0x0), %80 (0x3c00)
   ```
   **The rule.** The colour conversion of each fp32 lane is a scalar `f2f16`
   in NIR. `bi_vectorize_filter` returns width 2 for `f2f16`/`u2f16`/`i2f16`
   on arch < 11 (`B/bifrost_nir.c:154-160`, "On v10 and earlier we can take 2
   32-bit floats as srcs"), and `nir_opt_vectorize` (`bifrost_nir.c:229`) merges
   two instructions only when `instrs_equal` holds (`N/nir_opt_vectorize.c:226-273`):
   same op and bit size, and per source `alu_srcs_equal`
   (`:167-175`): the swizzle components fall in the same aligned group of
   `max_vec` (`swizzle[0] & ~(max_vec - 1)`, so with width 2 `.x/.y` together
   and `.z/.w` together, never `.y/.z`), **and `srcs_equal` (`:159-165`) the
   source is the same SSA def** (or both sources are constants). Therefore:
   - **`V2F32_TO_V2F16 d, a, b`** when the two lanes are components 0/1 or
     2/3 of the same vector def: a varying load (`LD_VAR_BUF_IMM ... .v2/.v3`,
     where `.xy` of a vec3 pairs and `.z` stays alone), or a pushed uniform
     vec4 (`.xy` and `.zw`). Emission `bifrost_compile.c:2771-2777`.
   - **`FADD.f32 .h0 d, a, -0.0` per lane plus `MKVEC.v2i16 d, a.h0, b.h0`**
     when the lanes are different defs: any fp32 arithmetic result (fp32 ALU
     ops have width 1 through the same filter, `bifrost_nir.c:207-211`,
     `32 / 32`), or a lone scalar with a constant partner. Scalar emission
     `bi_f32_to_f16_to`, `bifrost_compile.c:257-273` (arch >= 9, ours: `FADD`
     with `-0.0` and the f16 rounding; `arch < 9` would emit
     `V2F32_TO_V2F16 d, s, s` instead); the 16-bit vec2 is `bi_make_vec_to`'s `bi_mkvec_v2i16`
     (`:837-847`).
   - A constant lane is folded before this (the half pair `MOV.i32
     0x3C000000`, 1.5 step 4), so a pair `(var, const)` is the second form
     with the constant as `u?.w0.h0` (below).
   **References, one per form** (all `fs.fragment` text of
   `tools/mali/testdata/*.expect.json`, found with a script over every file;
   the programs that have either form are listed by it):
   - pair: `fvgtest_ftcoord` (`LD_VAR_BUF_IMM.f32...v2 @r0:r1` then
     `V2F32_TO_V2F16 r0, r0^, r1^`, `MOV.i32 r1, 0x3C000000`);
     `fvgtest_inner` (uniform `innerCol`, two `V2F32_TO_V2F16` of `u2.w0,u2.w1`
     and `u3.w0,u3.w1`); `fvgtest_discard2`, `sel_whole_vec4_same`,
     `sel_f_abs_a` and the other `sel_f_*` (a `vec4` varying: two pairs).
   - separate lanes: `fvgtest_cover` (two arithmetic results:
     `FADD.f32 r0.h0, r0^, 0x0.neg`, `FADD.f32 r1.h0, r1^, 0x0.neg`,
     `MKVEC.v2i16 r0, r0^.h0, r1^.h0`); every `fvg_fill_*` and `fvg_filter_*`
     program (two `MKVEC.v2i16`, four `FADD ... .h0`, see
     `fvg_fill_color_nodiscard_noscissor.expect.json`: the colour is
     `fmul` results, lane by lane).
   - mixed in one program: `sel_whole_vec3_loc1` (`vec4(v, 1.0)`, `v` a vec3
     varying: `V2F32_TO_V2F16 r0, r0^, r1^` for `v.xy`, then
     `FADD.f32 r1.h0, r2^, 0x0.neg` + `MKVEC.v2i16 r1, r1^.h0, u2.w0.h0` for
     `(v.z, 1.0)`, whose partner is a constant); `sel_xy01_float`
     (`vec4(v, 0.5, 0.25, 1.0)`, `v` a float: `FADD ... .h0` + `MKVEC.v2i16 r0,
     r0^.h0, u2.w0.h0`).
   **The consequence for S2** is in 1.5 step 4. Commands: `python3 -I` over
   `tools/mali/testdata/*.expect.json` counting `V2F32_TO_V2F16` and
   `MKVEC.v2i16` in `regions["fs.fragment"]["text"]`; `sed -n` on
   `nir_opt_vectorize.c`, `bifrost_nir.c`, `bifrost_compile.c`. Remaining: the
   rule is derived from the source and agrees with every reference read (the
   ones above); a program whose two lanes are different defs that NIR later
   proves equal (CSE/copy-prop before `nir_opt_vectorize` makes them one def)
   is the same rule seen after those passes, which the two dumps above show to hold for
   these two (the ftcoord pair is one `load_var_buf_pan` def; the cover
   lanes are an ALU result and a phi).
5. **`gl_FragCoord.z` and `.w`.** Not read by any femtovg shader; Mesa keeps
   them as separate `load_frag_coord_z/w` (`N/nir_lower_frag_coord_to_pixel_coord.c:22-24`,
   `pan_nir_lower_var_special.c:10-21`: `nir_load_var_special_pan`). Left as
   `SpecialInput`.
6. **Mediump arithmetic on `gl_FragCoord`** (ES 1.00 default precision
   mediump) is not in any reference; the pan_shc/GL disagreement that
   `cases.txt` labels "gl_FragCoord is mediump" is a SPIR-V-path artefact, not
   a second Mesa behaviour, but no reference of the GL path exercises it.
7. **FAU position of the transform.** Read from one reference (words 18-19);
   `bi_pick_ubo`'s order for the state slot is argued, not measured (2.2).
8. **`mali-lowering-map.md`** still says "`U2f32`/`I2f32` row" and "a fragment
   preload entry in `preload.rs`"; both are corrected above. This note did not
   edit it (the item's contract).
