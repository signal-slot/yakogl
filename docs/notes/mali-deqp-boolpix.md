<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the wrong pixels of bool/bvec shaders on the NanoPC-T6 (issue #11306)

**Status: investigation** (plan #387, item `fclane-lower-2`; issues #11306,
#7). No compiler change. The item added the Mesa 26.2.1 references
`tools/mali/testdata/deqp_boolpix_*` (29 shapes) and their pin test
`tests/mali_deqp_boolpix.rs`. It also added the five `lk_*` names to
`tests/mali_ref/mod.rs` `STRUCT_VALUES`: they are drawn with struct-member
values. Sites are `file:line` at the tip `f70946ff`, read only.

## 0. Summary

The NanoPC-T6 whole-tree run at main `8ff01055` has 299 cases that the
online compiler accepts but that fail with `Image comparison failed`. The
299 split into three causes. Only two of them are in the shader words:

| Cause | Cases | In the words? | What is wrong | Likely site |
|---|--:|---|---|---|
| **BW**: the bool uniform's word | 277 | no | our GL stores a true bool uniform as `1`; Mesa's words (and ours, which are the same) read it as the mask `~0` (`ICMP_OR.v2u16.eq.m1 r, mask.h00, u.w.h00`) | `src/gl/program.rs:2895`, `src/mali/frame.rs:3117`, record: `src/shaderc/online/mali/select.rs:603-611`, `:757` |
| **LK**: a mediump struct member pushed as fp16 halves | 12 | the record, not the instructions | the words read the member as an fp32 FAU word (`V2F32_TO_V2F16 r0, u2.w1, u2.w0`); our record makes that word `UniformHalves` (`00004c00` where Mesa pushes `41800000`) | `src/shaderc/online/mali/mod.rs:1623`, `:1632` |
| **D1**: `b2i32` of a 16-bit mask reads the low half for an odd lane | 10 | yes | `ivecN(bvecN)` widens `.h00` where Mesa widens `.h11` for lanes y and w | `src/shaderc/online/mali/nir_fs.rs:13867-13873` |

263 of the 277 BW cases have fragment words **byte-equal** to Mesa's: 98
whole pairs equal, 165 differing only in the vertex shader's varying
program, by value-equal forms. The other 14 differ in fragment forms that
give the same values. So the words cannot be what draws them wrong. What is
left is the value the words read: the uniform `ref_out0` (a `bool`/`bvecN`)
that dEQP's `isOk` compares against.

Fix order by cases (section 6):

1. BW, 277 cases. The driver writes Mesa's `~0`: step 2 (D1) of
   `docs/notes/mali-unif2-lowering.md` section 5.
2. LK, 12 cases. The mediump fp16 block keeps a struct member fp32 in the
   `bool_colour` and `int_colour16` routes, as every other route does.
3. D1, 10 cases. The odd lane's half.

Host forecast: 299 of 299 pass once all three land, under the caveats of
section 7.

## 1. Inputs and method

- Cases. The four slices
  `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` give 306 `Fail`
  rows whose detail is `Image comparison failed` and whose case has no row
  in that slice's `refusals.tsv`. Two groups among them are not bool
  shaders and have their own issues:
  - `fbo.render.recreate_colorbuffer.rebind_*`, 4 cases: issue #11329.
  - `texture.vertex.cube.*`, 3 cases: issue #7489.

  That leaves **299**, the number in docs/es2-cts-failures.md section M.1,
  row 16. Its split is the same: `swizzles` 162 (vertex 108, fragment 54),
  `conversions` 111 (to bool or bvec 101, to ivec 10), `functions` 14,
  `linkage.uniform_struct*` 12.
- dEQP sources. VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (the
  commit `DEQP_WORK` pins, `tools/cts/deps.toml`), fetched as a sparse,
  blobless checkout into `$CONNECTED_DEPS_DIR/VK-GL-CTS-gles2`. It holds
  `data/gles2/shaders/`,
  `modules/glshared/glsShaderLibrary{,Case}.cpp` and
  `framework/opengl/gluShaderLibrary.{cpp,hpp}`. It is not named
  `VK-GL-CTS`: a partial tree there would break `DEQP_WORK`, the reason
  `deps.toml` gives for `VK-GL-CTS-gles3`.
- Pairs. All 299 cases are ShaderLibrary cases, so none needs a dEQP run.
  A scratch generator parsed each case's `.test` row: its `values` block
  and its `both` / `vertex` / `fragment` sources, the last two with
  `removeExtraIndentation`. It then applied that commit's template:
  - `genVertexShader` and `specializeFragmentShader` for a `_fragment`
    case;
  - `specializeVertexShader` and `genFragmentShader` for a `_vertex` case;
  - `generate{Vertex,Fragment}Specialization` for the `linkage` programs;
  - `genCompareFunctions` and `genCompareOp` for the `isOk` checks.

  So the text is dEQP's verbatim, as the board ran it, which
  `tools/mali/deqp-host-measure.sh` would also have extracted from a dEQP
  log.
- Mesa references. `tools/mali/deqp_ref.sh <vert> <frag> <prefix>
  --instrument shim` for each of the 299 pairs. That is Mesa 26.2.1's
  gallium panfrost on drm-shim, G610, `gl_ref.py --uniform-values`. The shim
  was built the documented way into `$CONNECTED_DEPS_DIR/mesa-shim`
  (`drm_shim_ref.sh build`, run by `deqp_ref.sh` when absent). After the build, redrawing the
  committed `deqp_audit_v1_swz_mediump_bvec2_grg_vs` gave the same
  `expect.json` regions (every field but the trace's file name).
- Ours. A scratch test (not committed) ran `mali_ref::compile_pair` and
  `mali_ref::differences` on each pair, region by region. The 29
  representatives of section 5 are the committed references. Their
  `.vert`/`.frag` are the generated pairs, and `.license` names the source.

## 2. The 299 cases by group, stage and cause

"Words" is how our pair compares with Mesa's reference:
- *equal*: the whole pair is byte-equal;
- *vs only*: only the vertex varying program differs;
- *fs form*: the fragment shader differs by a form with the same values
  (section 4);
- *fs value*: the fragment shader computes another value;
- *FAU*: the instructions are equal and the FAU record is not.

| Group | Stage | Cause | Words | Cases |
|---|---|---|---|--:|
| `swizzles.vector_swizzles.mediump_bvec2_*` | vertex | BW | vs only | 21 |
| | fragment | BW | equal | 12 |
| `swizzles.vector_swizzles.mediump_bvec3_*` | vertex | BW | vs only | 39 |
| | fragment | BW | equal | 24 |
| `swizzles.vector_swizzles.mediump_bvec4_*` | vertex | BW | vs only | 48 |
| | fragment | BW | equal | 18 |
| `conversions.scalar_to_scalar` (to `bool`) | vertex | BW | equal 1, vs only 2 | 3 |
| | fragment | BW | equal | 2 |
| `conversions.scalar_to_vector` (to `bvecN`) | vertex | BW | equal 3, vs only 6 | 9 |
| | fragment | BW | equal | 6 |
| `conversions.vector_to_scalar` (to `bool`) | vertex | BW | equal 3, vs only 6 | 9 |
| | fragment | BW | equal | 3 |
| `conversions.vector_to_vector` (to `bvecN`) | vertex | BW | equal 6, vs only 12 | 18 |
| | fragment | BW | equal | 11 |
| `conversions.vector_to_vector` (`bvecN` to `ivecN`) | fragment | D1 | fs value | 5 |
| `conversions.vector_combine` (to `bvecN`) | vertex | BW | equal 4, vs only 22 | 26 |
| | fragment | BW | equal 5, fs form 9 | 14 |
| `conversions.vector_combine` (a `bvecN` to `ivecN`) | fragment | D1 | fs value | 5 |
| `functions.{array_arguments,datatypes,overloading,qualifiers}` (`bool`/`bvecN` arguments) | vertex | BW | vs only 9, fs form 1 | 10 |
| | fragment | BW | fs form | 4 |
| `linkage.uniform_struct*` (mediump struct uniforms, `float` output) | program | LK | FAU 6, FAU + fs form 6 | 12 |
| **Total** | | BW 277, LK 12, D1 10 | | **299** |

Shape dimensions, by section 4 of docs/notes/mali-deqp-families.md:
- precision: every case is mediump. The swizzle cases are `mediump_*`; the
  conversions, functions and linkage sources say `precision mediump float`.
- uniforms: the BW cases read a `bool`/`bvecN` reference uniform
  `ref_out0`; the D1 cases an `ivecN` one; the LK cases a `float` one and
  `struct` members.
- operators: `==` in the `isOk` of every BW case, plus `!` (`qualifiers`)
  and `==` chains (`datatypes`); swizzles in 162 cases. There are no
  `shaders.operator` cases.
- conversions: the `bool()`/`bvecN()` constructor of a float in every
  case, `ivecN()` of a `bvecN` in the D1 cases.

## 3. The three causes

### 3.1 BW: the bool uniform's word (277 cases, not in the words)

- **What.** Each of these programs compares a computed bool or bvec with
  the uniform `ref_out0`:
  - `isOk(out0, ref_out0)` with `bvecN == bvecN` in a fragment case;
  - `greaterThan(v_out0, vecN(0.5)) == ref_out0` in the fragment shader
    of a vertex case.

  Mesa's lowering gives the compare as
  `ICMP_OR.v2u16.eq.m1 r1, r1^.h00, u3.w0.h00, 0x0`: a 16-bit mask
  (`0`/`0xffff`) against the low half of the uniform's word. Mesa uploads
  a true bool as `~0`. Every reference's FAU shows `ffffffff` for the
  lanes `gl_ref.py` sets true. Our compiler emits the same words. Our GL
  stores the value as `0`/`1`:
  - `src/gl/program.rs:2895`, `values.truth(..) as u32`;
  - `copy_block_member` at `:876-893` for a block;
  - the push word copies it raw: `src/mali/frame.rs:3117`
    (`PushWord::Uniform { dword } => uniforms[dword]`), mapped at
    `src/mali/program.rs:108`.

  A true lane then compares `0xffff` with `0x0001` and `isOk` is false
  wherever the expected value has a true component. That holds for every
  case of the 277.
- **Evidence.**
  - Of the 277, 98 pairs are byte-equal to Mesa (instructions, FAU,
    preload, register count) and 165 differ only in the vertex varying
    program. The other 14 differ in fragment forms that compute the same
    values (section 4), so the drawn values could only be wrong through
    an input.
  - Not one `swizzles.vector_swizzles.mediump_bvec*` or `conversions.*_to_{bool,bvecN}`
    case passes on the board at `8ff01055`: 294 rows, all `Fail`.
  - The three `conversions.vector_combine.bool_bool*_to_ivecN_fragment`
    cases do pass. Their shaders are just as boolean, with no bool
    uniform (`ref_out0` is an `ivecN`) and no `bvec` odd lane.
- **Why no test saw it.** `tests/mali_ref` checks a FAU word of a bool
  uniform against Mesa's `~0` (`uniform_value`, `tests/mali_ref/mod.rs:519-525`),
  but what the driver writes is not checked there. The unif2 design
  (`docs/notes/mali-unif2-lowering.md` section 5) found the gap for the
  `.not` forms only ("With 1, `x` and `LSHIFT_AND` forms and `CSEL`/`MUX`
  tests (against 0) are right"). The equality compare against the
  uniform's half shown here is wrong with 1 too. So step 2 (D1) of that
  design is needed for this family as it stands, not only for the `.not`
  forms. A 32-bit form, `ICMP_OR.u32.eq.m1 r0, r0^, u2.w0`
  (`bw_fn_out_bool_fs`), is wrong with 1 in the same way.

### 3.2 LK: a mediump struct member pushed as fp16 halves, read as fp32 (12 cases)

- **What.** In `linkage.uniform_struct` the struct is
  `struct Struct {mediump float a; mediump float b;}`, and the fragment
  shader is `out0 = val.b + val.a; ... isOk(out0, ref_out0, 0.05)`.
  - Mesa keeps a struct member fp32 (lower_precision.cpp:1029-1031). Its
    fragment shader starts `V2F32_TO_V2F16.discard r0, u2.w1, u2.w0`, and
    its FAU is `41800000, 42000000` (16.0, 32.0 as fp32).
  - Our instructions are the same, but our record says `u2.w0` is
    `UniformHalves { lo: val.a }` and `u2.w1` is `UniformHalves { lo: val.b }`.
    The driver fills `00004c00` and `00005000` (the fp16 values in the
    low half), which the fp32 read takes as denormals near 0.
  - The six `*_vec4*` cases show the same in a vec4 member:
    `UniformHalves { lo: 0, hi: Some(1) }` read by `V2F32_TO_V2F16`. They
    also have the form difference of section 4 (O).
- **Site.** `src/shaderc/online/mali/mod.rs:1540-1541` defines
  `fp32(name)`: declared highp **or a struct member** (`name.contains('.')`).
  - The fragment walk's `fp32_uniforms` (`:1543-1548`) uses it, so the
    instructions read the member at 32 bits.
  - So do the `mask_colour`, `loop16`, `guarded16`/`kept16`/`frag32`,
    `multi16` and default routes (`:1613`, `:1643`, `:1655`, `:1664`,
    `:1671`).
  - The `bool_colour` route (`:1623`) and the `int_colour16` route
    (`:1632`) instead pass
    `highp = |name| matches!(prec(name), Some(Precision::High))` to
    `select::UboLayout::fs16`. `fs16` (`select.rs:635`) then keeps
    `halves` for the member, so its row is fp16 halves.
  - These programs take one of those two routes. Their uniform rows are
    halves, which only those two closures allow for a member. Which of
    the two each takes was not traced; dEQP's `isOk` colour of a bool
    points to `bool_colour`.
- **Why no test saw it.** The audit's `deqp_audit_o_linkage_uniform_struct_vec4`
  reference was drawn before `gl_ref.py` set struct members, so its FAU is
  all zeros there and in our record's values. The new `lk_*` references
  are drawn with member values and listed in `STRUCT_VALUES`, so
  `differences` reports the FAU words.

### 3.3 D1: the odd lane of a 16-bit mask widened from the low half (10 cases)

- **What.** `ivec2(bvec2(v_in0))` (`conversions.vector_to_vector.bvec2_to_ivec2_fragment`).
  - Mesa widens lane x with `IADD.v2u16 r0, r0^.h00, 0x0` and lane y with
    `IADD.v2u16 r1, r0.h11, 0x0`.
  - Ours widens `.h00` for both (`IADD.v2u16 r0, r0^.h00, 0x0`), and the
    row CSE merges them, so `out0.y` is `in0.x`.
- **Site.** `src/shaderc/online/mali/nir_fs.rs:13867-13873`: the `b2i32` of
  a 16-bit mask matches `Val::Lane(c, _)`, drops the lane and builds
  `Vec2(s[0].swz(Swz::H00), …)`. This is the audit's D1
  (`docs/notes/mali-deqp-compiled-audit.md` section 4.1, then
  `nir_fs.rs:12205-12210` at `7b540fee`), unchanged.
- **Cases.** 10:
  - `vector_to_vector.bvec{2,3,4}_to_ivec2`, `bvec3_to_ivec3`,
    `bvec4_to_ivec4`;
  - `vector_combine.bvec2_bvec2_to_ivec4`, `bvec2_float_to_ivec3`,
    `bvec2_int_to_ivec3`, `bvec3_float_to_ivec4`, `vec2_bvec2_to_ivec4`.

  All are `_fragment`. The audit listed 13 at `7b540fee`. The other three,
  `bool_bool{,_bool,_bool_bool}_to_ivecN_fragment`, pass on the board at
  `8ff01055`.

## 4. The value-equal forms (why the 179 non-equal BW pairs still read right)

These are the differences of the BW pairs. None changes a value for dEQP's
inputs. Each is still a "not Mesa's words" item for its owner. The tags
reuse the audit's where it has one.

| Tag | Where | What (ours / Mesa) | BW cases | Audit cause |
|---|---|---|--:|---|
| V1 | vertex varying | `bool(a)` / `bvecN(a)` as `FCMP_OR.f32.ne.f1` (or `.m1` + `CSEL`); Mesa `FADD.f32 rX.h0, a, -0` or `V2F32_TO_V2F16`, `FCMP_OR.v2f16.ne`, `S16_TO_S32`, `CSEL` | 101 | V1 (section 4.2) |
| VL | vertex varying | the attribute loaded only as wide as the lanes read (`LD_ATTR_IMM.f32` of a `vec2`); Mesa loads the whole vector | 42 | none (new; the vertex side of L1) |
| WT | vertex varying | the same load and operations, another `.wait`/slot | 17 | S1 (order) |
| A1 | vertex varying | attribute index 2 loaded before index 1 | 5 | A1 (section 4.5) |
| FP | fragment | lanes of separate scalar varyings narrowed and compared one by one (`FADD.f32.discard r0.h0` ×2, two one-lane `FCMP_OR`), or paired in another order; Mesa `V2F32_TO_V2F16` pairs | 9 | F1-like (pairing), not F1's fp32 compare |
| S1 | fragment | the same instructions, two swapped (`functions.datatypes.bool_bvec{3,4}`) | 2 | S1 (section 4.4) |
| NF | fragment | `in0 == false` as `FCMP_OR.v2f16.ne` + `ICMP_OR.u32.eq ... 0x0` + a 32-bit compare against the whole uniform word; Mesa folds the `not` into `FCMP_OR.v2f16.eq` and compares the half (`functions.qualifiers.{out,inout}_bool_fragment`) | 2 | M1-like (a fold) |
| CF | both | `f != !f` (always true) folded by Mesa: no varying, the fragment shader reads `ref_out0` directly (`MOV.i32.discard r0, u2.w0`, then `MUX`); ours computes it in the vertex shader (`functions.qualifiers.in_bool_vertex`) | 1 | M1-like |
| O | fragment | the linkage `vec4` member read as a `V2F32_TO_V2F16` pair where Mesa reads the one used component (`FADD.f32.discard r0.h0, u2.w0`); the uniform-only varying Mesa moves (LK cases) | 6 (LK) | O (`o_linkage_uniform_struct_vec4`) |

V1 is the audit's vertex fp16 cause. Its owner, intval step 3, does not
name `F2b` (`bool()` of a mediump float, `nir.rs:5741`), which every V1
case here needs (audit section 4.2, "Owner"). The vertex walk's catch-all
`IrOp::Alu` arm widens every operand (`nir.rs:5396-5419`, `vs_widen`
`:3205`).

## 5. The references (`tools/mali/testdata/deqp_boolpix_*`)

Each row gives the first differing instruction word of the first region
that differs, as `[index] ours → Mesa`. "Group" says how many of the 299
cases the shape stands for: the shape's cell in section 2.

| Reference | dEQP case (`dEQP-GLES2.functional.shaders.`) | Cause | List | First differing word (ours → Mesa) |
|---|---|---|---|---|
| `bw_swz_bvec3_zyx_fs` | `swizzles.vector_swizzles.mediump_bvec3_zyx_fragment` | BW | LOWERED | none: byte-equal |
| `bw_swz_bvec4_wzyx_fs` | `swizzles.vector_swizzles.mediump_bvec4_wzyx_fragment` | BW | LOWERED | none |
| `bw_conv_float_to_bool_fs` | `conversions.scalar_to_scalar.float_to_bool_fragment` | BW | LOWERED | none |
| `bw_conv_int_to_bvec2_vs` | `conversions.scalar_to_vector.int_to_bvec2_vertex` | BW | LOWERED | none |
| `bw_conv_vec4_to_bool_fs` | `conversions.vector_to_scalar.vec4_to_bool_fragment` | BW | LOWERED | none |
| `bw_conv_bvec3_to_bvec3_fs` | `conversions.vector_to_vector.bvec3_to_bvec3_fragment` | BW | LOWERED | none |
| `bw_conv_ivec4_to_bvec4_vs` | `conversions.vector_to_vector.ivec4_to_bvec4_vertex` | BW | LOWERED | none |
| `bw_swz_bvec2_grg_vs` | `swizzles.vector_swizzles.mediump_bvec2_grg_vertex` | BW+V1 | DIFFERS | vs.varying `[1] 00f4c10340c0c041 FCMP_OR.f32.ne.f1 r1, r1^, 0x0, 0x0` → `00a5c00000044140 V2F32_TO_V2F16 r0, r0^, r1^` |
| `bw_swz_bvec4_wzyx_vs` | `swizzles.vector_swizzles.mediump_bvec4_wzyx_vertex` | BW+V1 | DIFFERS | vs.varying `[1] 00f4c30340c0c043 FCMP_OR.f32.ne.f1 r3, r3^, 0x0, 0x0` → `00a5c00000044140 V2F32_TO_V2F16 r0, r0^, r1^` |
| `bw_swz_bvec2_r_vs` | `swizzles.vector_swizzles.mediump_bvec2_r_vertex` | BW+VL | DIFFERS | vs.varying `[0] 0866800202117d7c LD_ATTR_IMM.f32 @r0 … index:0x1` → `0866800412117d7c LD_ATTR_IMM.v2.f32 @r0:r1 … index:0x1` |
| `bw_conv_bool_to_bool_vs` | `conversions.scalar_to_scalar.bool_to_bool_vertex` | BW+V1 | DIFFERS | vs.varying `[1] 00f4c00340c0c040 FCMP_OR.f32.ne.f1 r0, r0^, 0x0, 0x0` → `00a440100000c040 FADD.f32 r0.h0, r0^, 0x0.neg` |
| `bw_conv_float_to_bvec4_vs` | `conversions.scalar_to_vector.float_to_bvec4_vertex` | BW+V1 | DIFFERS | vs.varying `[1] 00f4c00340c0c040 FCMP_OR.f32.ne.f1` → `00a440100000c040 FADD.f32 r0.h0, r0^, 0x0.neg` |
| `bw_conv_bvec4_to_bool_vs` | `conversions.vector_to_scalar.bvec4_to_bool_vertex` | BW+VL | DIFFERS | vs.varying `[0] 0866800202117d7c LD_ATTR_IMM.f32 @r0` → `0866800832117d7c LD_ATTR_IMM.v4.f32 @r0:r1:r2:r3` |
| `bw_conv_vec2_bvec2_to_bvec4_vs` | `conversions.vector_combine.vec2_bvec2_to_bvec4_vertex` | BW+A1 | DIFFERS | vs.varying `[0] 0066800412213d3c LD_ATTR_IMM.v2.f32 … index:0x2` → `0866800412113d3c LD_ATTR_IMM.v2.f32.wait0 … index:0x1` |
| `bw_conv_bool_bool_to_bvec2_fs` | `conversions.vector_combine.bool_bool_to_bvec2_fragment` | BW+FP | DIFFERS | fs `[1] 68a440100000c040 FADD.f32.discard r0.h0, r0^, 0x0.neg` → `68a5c00000044140 V2F32_TO_V2F16.discard r0, r0^, r1^` |
| `bw_conv_bvec3_float_to_bvec4_fs` | `conversions.vector_combine.bvec3_float_to_bvec4_fragment` | BW+FP | DIFFERS | fs `[1] 68a5c10000044241 V2F32_TO_V2F16.discard r1, r1^, r2^` → `68a5c00000044041 V2F32_TO_V2F16.discard r0, r1^, r0^` |
| `bw_fn_bool_bvec3_fs` | `functions.datatypes.bool_bvec3_fragment` | BW+S1 | DIFFERS | fs `[4] 00f1c0008cc04040 ICMP_OR.v2u16.eq.m1 r0, r0^.h00, r0^.h11` → `00f5c10388c0c041 FCMP_OR.v2f16.ne.m1 r1, r1^.h00, 0x0` |
| `bw_fn_out_bool_fs` | `functions.qualifiers.out_bool_fragment` | BW+NF | DIFFERS | fs `[2] 00f5c00388c0c040 FCMP_OR.v2f16.ne.m1 r0, r0^.h00, 0x0` → `00f5c00088c0c040 FCMP_OR.v2f16.eq.m1 r0, r0^.h00, 0x0` |
| `bw_fn_in_bool_vs` | `functions.qualifiers.in_bool_vertex` | BW+CF | DIFFERS | fs `[0] 085c80020200007d LD_VAR_BUF_IMM.f32 @r0, index:0x0` → `6891c00000000084 MOV.i32.discard r0, u2.w0` (and Mesa has no varying program) |
| `bw_fn_global_in_bool_vs` | `functions.array_arguments.global_in_bool_vertex` | BW+V1 | DIFFERS | vs.varying `[1] 00f4c00380c0c040 FCMP_OR.f32.ne.m1 r0, r0^, 0x0, 0x0` → `00a5c00000044140 V2F32_TO_V2F16 r0, r0^, r1^` |
| `bw_fn_user_func_arg_bool_types_vs` | `functions.overloading.user_func_arg_bool_types_vertex` | BW+V1 | DIFFERS | vs.varying `[1] 00f4c00380c0c040 FCMP_OR.f32.ne.m1 r0, r0^, 0x0, 0x0` → `00a440100000c040 FADD.f32 r0.h0, r0^, 0x0.neg` |
| `d1_conv_bvec2_to_ivec2_fs` | `conversions.vector_to_vector.bvec2_to_ivec2_fragment` | D1 | DIFFERS | fs `[3] 00a1c0000800c040 IADD.v2u16 r0, r0^.h00, 0x0` → `00a1c1300800c000 IADD.v2u16 r1, r0.h11, 0x0` |
| `d1_conv_bvec4_to_ivec4_fs` | `conversions.vector_to_vector.bvec4_to_ivec4_fragment` | D1 | DIFFERS | fs `[4] 00a1c1000800c041 IADD.v2u16 r1, r1^.h00, 0x0` → `00a1c2300800c001 IADD.v2u16 r2, r1.h11, 0x0` |
| `d1_conv_bvec3_float_to_ivec4_fs` | `conversions.vector_combine.bvec3_float_to_ivec4_fragment` | D1+FP | DIFFERS | fs `[1] 68a5c10000044241 V2F32_TO_V2F16.discard r1, r1^, r2^` → `68a441100000c041 FADD.f32.discard r1.h0, r1^, 0x0.neg`; D1's own word follows: Mesa's `00a1c2300800c001 IADD.v2u16 r2, r1.h11, 0x0` has no counterpart in ours |
| `lk_uniform_struct` | `linkage.uniform_struct` | LK | DIFFERS | instructions equal; fs FAU word 4: ours `00004c00` (`UniformHalves` of `val.a`) → Mesa `41800000` (fp32 16.0); word 5 `00005000` → `42000000` |
| `lk_uniform_struct_partial` | `linkage.uniform_struct_partial` | LK | DIFFERS | instructions equal; fs FAU word 4: `00005000` → `42000000` (`val.b`) |
| `lk_uniform_struct_use_case_rip` | `linkage.uniform_struct_use_case_rip` | LK | DIFFERS | instructions equal; fs FAU word 4: `00005600` → `42c00000` (`val.quadraticAttenuation`) |
| `lk_uniform_struct_vec4` | `linkage.uniform_struct_vec4` | LK+O | DIFFERS | fs `[0] 68a5c00000048485 V2F32_TO_V2F16.discard r0, u2.w1, u2.w0` → `68a440100000c084 FADD.f32.discard r0.h0, u2.w0, 0x0.neg`; FAU `4c404c00` → `41800000` |
| `lk_uniform_struct_fragment_only_vec4` | `linkage.uniform_struct_fragment_only_vec4` | LK+O | DIFFERS | fs `[0] 68a5c00000048584 V2F32_TO_V2F16.discard r0, u2.w0, u2.w1` → `68a440100000c085 FADD.f32.discard r0.h0, u2.w1, 0x0.neg`; FAU halves → fp32 |

The 7 LOWERED references are the evidence for BW: the words are Mesa's, yet
their cases fail on the board. A fix of BW moves none of them. A fix of a
form (V1, VL, ...) moves its DIFFERS rows to LOWERED.

## 6. Fix order by cases

| Step | Cause | Cases | Site (tip `f70946ff`) | Change | Moves in `tests/mali_deqp_boolpix.rs` |
|--:|---|--:|---|---|---|
| 1 | BW | 277 | `src/gl/program.rs:2895`; `src/mali/program.rs:108`; `src/mali/frame.rs:3117`; record: `src/shaderc/online/mali/fau.rs` (`FauWord`), `select.rs:603-611`, `:757` | `docs/notes/mali-unif2-lowering.md` section 5 / step 2 (D1): `FauWord::UniformBool { dword }` pushed for a `GL_BOOL*` dword, `PushWord::UniformBool` written as `if w != 0 { !0 } else { 0 }` (a `UboRow` `bool` flag for a block row). Needs a host test from `glUniform1i(b, 1)` to the push word (`src/mali/frame_tests.rs`): `tests/mali_ref` cannot see it. | none (BW is not in the words); board: 277 |
| 2 | LK | 12 | `src/shaderc/online/mali/mod.rs:1623`, `:1632` | pass `fp32(name)` (the struct-member rule) as the other routes do, so `fs16` keeps a member fp32 (`select.rs:635-641`) | `lk_uniform_struct`, `_partial`, `_use_case_rip` to LOWERED if nothing else differs (their instructions are equal today); `_vec4` ones still need O |
| 3 | D1 | 10 | `src/shaderc/online/mali/nir_fs.rs:13867-13873` | take the lane's own half (`H11` for an odd lane), as the audit's D1 fix candidate | `d1_conv_bvec2_to_ivec2_fs`, `d1_conv_bvec4_to_ivec4_fs` (and those of the audit's `deqp_audit_d1_*` with a `bvec` source); `d1_conv_bvec3_float_to_ivec4_fs` also needs FP |
| later | V1, VL, A1, WT, FP, S1, NF, CF, O | 0 more board cases here | audit sections 4.2-4.8 | byte-equality only: the values are right already | the rest of DIFFERS |

Steps 1 and 2 are independent of each other and of the open compiler
steps. Step 1 alone covers 277 of the 299 cases of #11306.

## 7. Caveats

- **Forecast, not a board result.** "299 pass" assumes the three fixes and
  no other run-time difference. The vertex forms of section 4 compute the
  same values for dEQP's inputs: every input is a normal float or 0, so
  fp32 and fp16 `!= 0` agree. The host cannot draw.
- **BW is inferred from the words plus the GL code.** The host draws
  nothing. The inference holds because 98 whole pairs are byte-equal,
  every BW case reads a bool uniform, every bool-uniform case of these
  groups fails on the board, and the bool cases without one pass. A board
  check after step 1 confirms it.
- **The earlier references' `isOk` is not dEQP's.** For vertex cases the
  audit's `deqp_audit_v1_swz_mediump_bvec2_grg_vs` (and the
  `deqp_intval*_reslib_*` pairs it copies) writes
  `bool isOk (vec3 a, bvec3 b) { return (bvec3(a) == b); }`, while
  `genCompareFunctions` at `93bca01` writes
  `return (greaterThan(a, vec3(0.5)) == b);`. Their fragment words
  therefore are not those of the board's shader. The `deqp_boolpix_*`
  pairs use dEQP's text.
- `STRUCT_VALUES` gained the five `deqp_boolpix_lk_*` names, as that list's
  rule asks of a reference drawn with member values.
  `tests/mali_ref_selftest.rs` still passes.

## 8. To redo

1. Make the case list: `results.tsv` `Fail` with `Image comparison failed`
   minus each slice's `refusals.tsv` cases, minus `fbo.*` and `texture.*`.
2. Fetch VK-GL-CTS `93bca01` sparse into `$CONNECTED_DEPS_DIR/VK-GL-CTS-gles2`:

   ```sh
   git clone --filter=blob:none --no-checkout https://github.com/KhronosGroup/VK-GL-CTS.git
   git sparse-checkout set --no-cone /data/gles2/shaders/ /modules/glshared/ /framework/opengl/gluShaderLibrary.*
   git checkout 93bca01861b0e3ef3c387027a9791e6d065f900c
   ```

3. Build each pair from its `.test` row through the template functions
   named in section 1.
4. Draw each with `tools/mali/deqp_ref.sh ... --instrument shim`.
5. Compare each with `mali_ref::compile_pair` + `mali_ref::differences`.

The full 299-pair sweep took under a minute of drawing once the shim was
built.
