<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): TexVertex, TexKind and FragCoordMediump residue at `349f989f`: steps and Mesa references

**HOST-ONLY.** Plan #387 item `texmiscres-refs` (issue #7, ROADMAP stage 5;
issue #32 for the texture look-up kinds). No change under `src/`. The item
measures the three texts again at the integration tip, puts every
first-refusal case into a step, and draws the Mesa 26.2.1 references for the
shapes that no `deqp_texmisc_*` / `deqp_fmaloc_*` reference covers
(`tools/mali/testdata/deqp_texmiscres_*`, pinned by
`tests/mali_deqp_texmiscres.rs`). It builds on the design
[`mali-texmisc-lowering.md`](mali-texmisc-lowering.md) (TV1, TK1-TK3, FC0-FC2)
and the reference note [`mali-deqp-texmisc.md`](mali-deqp-texmisc.md), whose
section 4 has the six steps the schedule calls `texmisc-lower-1..6`
(`mali-lowering-schedule.md` 2.10 and 4b item 10). Step numbers below are
those six. The G310 needs no separate work
([`mali-g310-vs-g610.md`](mali-g310-vs-g610.md)).

## 0. Summary

1. **Tip:** `349f989f5392bea65c20e7663289e86481e521e0`. **159 cases**
   have one of the three texts as their first refusal: TexVertex 61,
   TexKind 59 and FragCoordMediump 39. These are the counts of
   `mali-deqp-families.md` 19.1 at `e9033663`, but the case sets differ (2.2).
   All 159 are board Fail (NanoPC-T6, slices `37305608`). None is
   NotSupported.
2. **Each refusal site was measured, not read.** Section 3 lists the site of
   each case, from `eprintln!(line!())` at every `Err(MaliError::{TexVertex,
   TexKind, FragCoordMediump})` in a scratch copy that was thrown away:
   - TexVertex: one site for all 61.
   - TexKind: two sites.
   - FragCoordMediump: three sites, and the one that refuses 33 of the 39 is
     **not** the site the design guessed. `alu16`'s catch-all refuses an
     `Fabs` (the `isOk` harness's `abs(a - b)`) and an `Fadd` of 1 or 3
     lanes. The pair arms at `:3794` / `:3799` refuse only 6 (section 3.3).
3. **Mesa decides the look-up kind, not the source.** Mesa drops dead
   look-ups and folds a zero bias:
   - 6 of the "cube" random cases of the old split have no cube look-up in
     Mesa's words (it is dead). Mesa keeps their 2D look-up.
   - 2 cases whose bias folds to `0.0` are a plain `2d.computed` (new step
     **TM1**).

   So steps 3/4/5 are re-split here by Mesa's live `TEX_SINGLE` mode
   (section 4).
4. **21 new references** (section 5), all refused at the tip with their
   case's text. Every one of the 159 cases is now in a step row whose shape
   has a reference (section 4).
5. **Order** (section 6):
   - TV1 (step 1, 61) can start now: its dependency `intval-vs16` M4 has
     landed (`vsapi_mat2` is byte-equal, `vs_cmp_and`).
   - Then step 2 (32) with its `alu16` `Fabs` and odd-width arms.
   - Then steps 4 (30) and 3 (21) as one look-up session.
   - Then TM1 (2), step 5 (6) and step 6 (7).

   No text reaches 100 alone. Together they are 159.

## 1. Method

- **Tip and tree:**
  - The workspace at `349f989f` (`git describe`: `349f989f`).
  - The tip's `capi/Cargo.toml` defines the feature `vivante-st` twice
    (lines 41 and 85, from `75a7d901` and `110080eb`). This host's cargo
    (rustc 1.99.1) refuses the manifest ("duplicate key"), so no `cargo`
    command runs at the tip as committed.
  - The measure and the tests therefore ran in a scratch worktree of
    `349f989f` with line 85 removed (`349f989f-dirty` in `commit`). No
    compiler code differs.
  - That fix is outside this item (handoff).
- **CTS:** VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, prepared as
  follows:
  - fetched at that commit alone into `$CONNECTED_DEPS_DIR/VK-GL-CTS`;
  - `external/fetch_sources.py` run;
  - `tools/cts/cts-yakogl-target.patch` applied with `git apply`.
- **Command** (section 19's, 4 cores):

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" \
    --out "$PWD/build/measure" --jobs 3 --chunk 100
```

- **Result:**
  - All 10,597 cases began. 106 processes exited 1 and **none crashed**.
  - 4,709 cases have a host refusal (4,716 at `e9033663`).
  - The first refusal per case is `measure.tsv`'s `host_refusal`, mapped to
    its `MaliError` by the message's opening words (section 19's rule).
- **Sites:** each case's refusing program (`programs.tsv`, one per case) was
  compiled through `tests/mali_ref`'s `compile_pair` in a scratch copy with
  an `eprintln!("SITE {}:{}", file!(), line!())` in front of every
  `Err(MaliError::{TexVertex, TexKind, FragCoordMediump})` of `nir.rs` and
  `nir_fs.rs`. The copy also printed:
  - the `Es100Tex` kind at the fallback look-up arm;
  - the `AluOp` and width at `alu16`'s catch-all.

  All 159 reproduce their text this way. Line numbers are the tip's.
- **Shapes by Mesa's words:** every TexKind and FragCoordMediump case, plus
  the new TexVertex case, was drawn once with `tools/mali/deqp_ref.sh
  --instrument shim` into a scratch directory. The live look-ups and their
  `TEX_SINGLE` mode are read from `fs.fragment`. The 21 committed
  references are byte-identical to their scratch draws (apart from the
  `trace` name).

## 2. The counts at the tip

### 2.1 By text

| Text | Stage | Cases | Fail | NS | dEQP groups |
|---|---|--:|--:|--:|---|
| `TexVertex` | vs | 61 | 61 | 0 | `uniform_api.value` 44, `uniform_api.random` 11, `uniform_api.info_query` 6 |
| `TexKind` | fs | 59 | 59 | 0 | `shaders.random` 49, `shaders.texture_functions` 6, `texture.mipmap` 4 |
| `FragCoordMediump` | fs | 39 | 39 | 0 | `shaders.conversions` 32, `shaders.random` 6, `shaders.functions` 1 |
| | | **159** | 159 | 0 | |

### 2.2 Against the 144 of `ca7cdc52` (the texmisc notes)

- **TexVertex** 60 to 61: `uniform_api.random.6` is new. It is a cube
  `compare_vec4` beside int, `bvec4`, `vec3` and struct `mat4` uniform
  compares, and its vertex varying program is 118 words.
- **TexKind** 42 to 59:
  - 18 are new: `shaders.random.all_features.fragment.60`,
    `shaders.random.texture.fragment.{1,6,13,14,29,66,70,76,77,81,119,123,132,139,143}`,
    `shaders.texture_functions.fragment.texture2dproj_vec{3,4}_bias`.
  - 1 left: `.55`.
  - Inflow from the lowerings since `ca7cdc52`: these cases met an earlier
    refusal before.
- **FragCoordMediump** 42 to 39: the 32 `shaders.conversions` and the
  function case are unchanged.
  - New: `shaders.random.scalar_conversion.combined.40`.
  - Left: `exponential.fragment.87`, `scalar_conversion.combined.68`,
    `scalar_conversion.fragment.68` and `texture.fragment.87`.

## 3. The measured refusing sites at `349f989f`

### 3.1 TexVertex: one site, all 61

`src/shaderc/online/mali/nir.rs:6516` is the vertex walk's generic
`IrOp::Alu(a, s)` arm. It refuses a `Prec::Medium` operation that has an
operand lane of a `Kind::VsTex { medium: true }` look-up. These are the
`compare_vec4` harness's `fsub`, `fabs`, `flt` and `bcsel` of the fp16 texels.

The look-ups themselves (2D and cube at level 0) are taken. The other two
`TexVertex` sites (`:7304` a reuse at another width, `:7312` a look-up
without a level) are reached by none of the 61.

### 3.2 TexKind: two sites

| Site | What it refuses | Cases |
|---|---|--:|
| `nir_fs.rs:8574` | the fallback `ir::Op::Tex` arm: `es100_tex()` other than `Tex2D`. The kind printed is `Tex2DBias` (a bias, with or without projection) in 32 cases and `Tex2DProj` (projection, no bias) in 22 | 54 |
| `nir_fs.rs:8365` | the mediump `textureCube` bias arm, `f.uniform_dword(bias).is_none()`: a cube bias that is not a uniform lane | 5 |

Neither site checks liveness, and the walk refuses at the first look-up it
cannot build. Mesa's live look-up is often another one (4.3).

### 3.3 FragCoordMediump: three sites, all in `Fs::alu16` (`nir_fs.rs:3693`)

| Site | Condition | Cases |
|---|---|--:|
| `:3858` | `alu16`'s catch-all `_ =>`: `Fabs` of 2 lanes (12) or 4 lanes (5); `Fadd` of 1 lane (5) or 3 lanes (10) whose operand is not `lone16`, or whose width is odd (the `Fmul \| Fadd if w % 2 == 0` arm takes pairs only); `Fneg` of 1 lane (1) | 33 |
| `:3794` | the pair arm: neither half is a texel or a narrowed value (`own(p) \|\| own(q)` false) | 3 |
| `:3799` | the pair arm's second operand: a constant half beside a non-constant one | 3 |

`vec16_of` (`:3642`), `:3707`, `:3854`, `:3803` and `:10996` are reached by
none. Section 3.3 of the design guessed `:2965`-`:2974` (now `:3794`-`:3803`)
or `vec16_of` for the conversions. The measure shows that for 32 of the 33
conversion cases the walk builds the int round trip and the narrowing, and
then refuses the **harness**: the `isOk` function's `abs(a - b)` (`Fabs`) and
the subtraction of an odd number of lanes. The int lanes are not what it
refuses.

## 4. Every case in a step row

Step numbers 1-6 are `mali-deqp-texmisc.md` section 4 (`texmisc-lower-1..6`).
TM1 is new here. "Refs" are the references a step must byte-match:

- `fm_` is `deqp_fmaloc_`;
- `tm_` is `deqp_texmisc_`;
- **bold** is new in this item (`deqp_texmiscres_`).

### 4.1 The table

| Group | Step | Cases | Mechanism (one line) | Site | Refs | Top dEQP groups |
|---|---|--:|---|---|---|---|
| TV-2 | 1 | 14 | `vs_cmp_and`'s fp16 compare fold with texel operands: `FADD(.._IMM).v2f16` of the texel pair and a constant pair, `FMAX.v2f16 .abs` chain, one `FCMP_OR.v2f16.lt`, `S16_TO_S32`, `CSEL`, `FMA.f32 r, r, 0, r` | `nir.rs:6516` | `tm_tv_2d_compare` | `uniform_api.value` 12, `.info_query` 2 |
| TV-A | 1 | 22 | the same over three sampler-array look-ups joined by `FMAX` before one compare | `nir.rs:6516` | `tm_tv_2d_array_compare` | `uniform_api.value` 20, `.info_query` 2 |
| TV-C | 1 (TV1c) | 14 | the same after the unfolded constant-direction cube sequence (`CUBEFACE1 r0, 0x0, 0x0, 0x0` ...) | `nir.rs:6516` | `tm_tv_cube_compare` | `uniform_api.value` 12, `.info_query` 2 |
| TV-R1 | 1 | 7 | random 2D: both constant pairs immediates | `nir.rs:6516` | `tm_tv_rnd_2d` | `uniform_api.random.{0,14,22,30,32,49,95}` |
| TV-R2 | 1 (TV1c) | 3 | random cube sampler array, the other uniforms' compares in the same chain | `nir.rs:6516` | `tm_tv_rnd_cube_array` | `uniform_api.random.{11,33,78}` |
| TV-R3 | 1 (TV1c) | 1 | a single cube compare joined with int, `bvec4`, `vec3` and struct `mat4` compares (the vs16 M3/M4 uniform words beside the texels) | `nir.rs:6516` | **`tv_rnd_cube_mixed`** | `uniform_api.random.6` |
| FC-A | 2 | 4 | scalar harness: a lone `Fadd` of the narrowed `I2f` lane and a uniform (`lone16` must accept the narrowed int lane) | `:3858` `Fadd` w=1 | `tm_fcm_int_to_float` | `conversions.{scalar,vector}_to_scalar` |
| FC-B | 2 | 12 | `Fabs` of two fp16 lanes in `alu16` (the harness's `abs(a - b)` as `.abs` on the `FCMP_OR`/`FMAX` source); `ftrunc` and `F16_TO_S32` forms of the round trip | `:3858` `Fabs` w=2 | `tm_fcm_ivec2_to_vec2`, `tm_fcm_int_bool_to_vec2`, `tm_fcm_int_to_mat2` | `conversions.{vector_to_vector,vector_combine,scalar_to_vector,scalar_to_matrix,matrix_combine}` (`*vec2`, `*mat2`) |
| FC-C | 2 | 10 | an odd width (3) in `alu16`: a pair plus a lone lane (`FADD.v2f16 r0, r0^, u.neg` and `FADD.v2f16 r1, r1^.h00, u.neg.h00`), the two compare masks joined by `LSHIFT_AND.v2i16` | `:3858` `Fadd` w=3 | **`fcm_ivec3_to_vec3`**, **`fcm_bool_ivec2_to_vec3`**, **`fcm_int_to_mat3`** | `conversions.*_to_vec3`, `*_to_mat3` |
| FC-D | 2 | 5 | `Fabs` of four lanes: two pair harnesses, three `LSHIFT_AND.v2i16` | `:3858` `Fabs` w=4 | **`fcm_ivec4_to_vec4`** | `conversions.*_to_vec4`, `ivec4x4_to_mat4` |
| FC-E | 2 | 1 | `mat4(int)`: the pair of a zero half and a narrowed `ftrunc` lane (`MKVEC.v2i16 r1, 0x0.h0, r0.h0`, `U16_TO_U32`) is refused before it is built | `:3794` | **`fcm_int_to_mat4`** | `conversions.scalar_to_matrix.int_to_mat4` |
| TK-P1 | 3 | 6 | projection of a varying read whole: `FRCP.f32` of `.z`/`.w`, two `FMA.f32 .. 0x0.neg`, `2d.computed` | `:8574` `Tex2DProj` | `fm_tk_proj_vec3`, `fm_tk_proj_vec4`, `fm_tk_rnd_proj`, `tm_tk_rnd_proj_swizzle` | `texture_functions.texture2dproj_vec{3,4}`, `random.texture.{19,23,56,131}` |
| TK-P2 | 3 | 15 | projection of a built or computed coordinate: fp16 sum or `FMA.v2f16` (or fp32 products beside an int lane), `F16_TO_F32`, then TK-P1; 3 of them (`.34`, `.115`, `.144`) also hold a dead cube look-up | `:8574` `Tex2DProj` | `tm_tk_rnd_proj_sum`, **`tk_rnd_proj_mul`**, **`tk_rnd_proj_const_lane`**, **`tk_allf_proj_fma`** | `random.texture.{34,69,80,91,94,97,98,115,119,123,132,136,142,144}`, `random.all_features.60` |
| TK-B1 | 4 | 5 | 2D bias of a uniform or a varying read whole: the 8.8 pack (`FMA.f32.clamp_m1_1` by 1/16, `FMA.f32` by 4096, `F32_TO_S32.rtz`, `U16_TO_U32`), `2d.computed_bias` | `:8574` `Tex2DBias` | `fm_tk_bias_uniform`, `fm_tk_bias_varying` | `texture_functions.texture2d_bias`, `texture.mipmap.2d.bias.*` 4 |
| TK-B2 | 4 | 2 | projection **and** bias: TK-P1's divide, then TK-B1's pack as the third staging word | `:8574` `Tex2DBias` | **`tk_proj_vec3_bias`**, **`tk_proj_vec4_bias`** | `texture_functions.texture2dproj_vec{3,4}_bias` |
| TK-B3 | 4 | 13 | random 2D bias (computed coordinate, folded constant bias as a FAU word, `b * b` in fp16); 2 of them (`.125`, `.127`) also hold a dead cube look-up | `:8574` `Tex2DBias` | `fm_tk_rnd_bias`, `tm_tk_rnd_bias_fragcolor` | `random.texture.{7,13,14,17,31,48,59,63,66,84,124,125,127}` |
| TK-B4 | 4 | 6 | random projection with bias: a varying read whole or a computed coordinate, a bias sum or constant | `:8574` `Tex2DBias` | **`tk_rnd_proj_bias_whole`**, **`tk_rnd_proj_bias_comp`** | `random.texture.{1,6,29,76,81,143}` |
| TK-D | 4 | 2 | every look-up dead: Mesa has no `TEX_SINGLE` | `:8574` `Tex2DBias` / `Tex2DProj` | `tm_tk_rnd_bias_two`, `tm_tk_rnd_bias_proj` | `random.texture.{41,108}` |
| TK-DP | 4 then 3 | 2 | a dead bias look-up first; the live one is a projection (`2d.computed`) | `:8574` `Tex2DBias` | **`tk_rnd_bias_dead_proj`** | `random.texture.{78,99}` |
| TK-Z | **TM1** | 2 | a bias that folds to `0.0` (`float(bool(b))` of a constant 0, `float(false) * x`): Mesa's look-up is the plain `2d.computed`, no bias word; `.26` also has a dead cube look-up and an all-constant coordinate (`MOV.i32 r0, u2.w0` / `r1, u2.w1`) | `:8574` `Tex2DBias` | **`tk_rnd_bias_zero`**, **`tk_rnd_bias_zero_dead_cube`** | `random.texture.{26,70}` |
| TK-C | 5 | 6 | cube with a computed bias (a varying, a sum, `x + 0.75`): the cube sequence (constant `1.0` / `-3` direction lanes as FAU words), the 8.8 pack, `cube.computed_bias`; `.77` and `.139` hold a dead 2D look-up first | `:8365` (5), `:8574` `Tex2DBias` (`.139`) | `fm_tk_cube_bias`, `tm_tk_rnd_cube_bias`, **`tk_rnd_cube_bias_const`**, **`tk_rnd_cube_bias_dead2d`** | `texture_functions.texturecube_bias`, `random.texture.{52,77,121,139,147}` |
| FC-E6 | 6 | 2 | random: Mesa's program has no int word left (folded), a pair of two uniform halves | `:3794` | **`fcm_rnd_cmp_ops`** | `random.comparison_ops.26`, `random.scalar_conversion.fragment.26` |
| FC-F | 6 | 3 | random: `S32_TO_F32` of an int uniform or a highp lane beside a constant half (`MKVEC.v2i16 r1, u4.w0.h0, r1^.h0`) | `:3799` | `tm_fcm_rnd_int_lanes`, **`fcm_rnd_conv_const`** | `random.scalar_conversion.{combined,fragment}.40`, `random.texture.88` |
| FC-G | 6 then 5 | 1 | a lone fp16 `Fneg`, then a cube look-up with a computed bias (`cube.computed_bias`) | `:3858` `Fneg` w=1 | **`fcm_rnd_cube_bias_neg`** | `random.texture.37` |
| FC-H | 6 | 1 | the function call's int arithmetic behind a scalar harness | `:3858` `Fadd` w=1 | `tm_fcm_eval_order` | `functions.misc.argument_eval_order_1` |
| | | **159** | | | | |

### 4.2 Per step

| Step | Cases (was at `ca7cdc52`) | Groups | Text |
|---|--:|---|---|
| 1 `texmisc-lower-1` (TV1, TV1c) | 61 (60) | TV-2 14, TV-A 22, TV-C 14, TV-R1 7, TV-R2 3, TV-R3 1 | TexVertex |
| 2 `texmisc-lower-2` (FC1) | 32 (32) | FC-A 4, FC-B 12, FC-C 10, FC-D 5, FC-E 1 | FragCoordMediump |
| 3 `texmisc-lower-3` (TK1 projection, TK2) | 21 (14) | TK-P1 6, TK-P2 15 | TexKind |
| 4 `texmisc-lower-4` (TK1 bias, dead look-ups) | 30 (17) | TK-B1 5, TK-B2 2, TK-B3 13, TK-B4 6, TK-D 2, TK-DP 2 | TexKind |
| 5 `texmisc-lower-5` (TK3) | 6 (11) | TK-C 6 | TexKind |
| 6 `texmisc-lower-6` (FC2) | 7 (10) | FC-E6 2, FC-F 3, FC-G 1, FC-H 1 | FragCoordMediump |
| **TM1** (new) | 2 (0) | TK-Z 2 | TexKind |
| fmaloc T1 (train 4) | 0 | none as a first refusal; T1's cube sequence and `cube_dir_measured` are a dependency of TK-C and TV1c | |
| | **159** | | |

Step 5 shrank from 11 to 6. Mesa keeps no cube look-up in 6 of the old
"cube" random cases (`.26`, `.34`, `.115`, `.125`, `.127`, `.144`), so those
are 2D steps here. Steps 3 and 4 grew by the 18 new TexKind cases.

### 4.3 What Mesa does with the look-ups

The `TEX_SINGLE` modes of the 59 TexKind cases' `fs.fragment` (scratch
draws):

| Mode | Cases |
|---|--:|
| `2d.computed` | 25 |
| `2d.computed_bias` | 26 (the 4 `texture.mipmap` ones `.wait1`) |
| `cube.computed_bias` | 6 |
| none | 2 |

- **Dead look-ups:** in 12 cases the walk's first refused look-up is not
  Mesa's live one: TK-D 2, TK-DP 2, TK-Z's `.26`, the 5 dead cubes of
  TK-P2/TK-B3, and the dead 2D look-ups of `.77` and `.139`. The design's
  TK1 part 3 (one liveness pass before the refusing arms) is what lets
  them reach the live look-up.
- **Zero bias (TM1):** a bias Mesa folds to `0.0` gives `2d.computed`, a
  plain look-up with no bias word. This is new. The walk must take a
  constant-zero bias as `Tex2D`/`Tex2DProj`, not pack it.

## 5. The references

Drawn with `tools/mali/deqp_ref.sh <vert> <frag>
tools/mali/testdata/deqp_texmiscres_<shape> --instrument shim`. This is Mesa
26.2.1's gallium panfrost on drm-shim, one 64x64 draw with
`--uniform-values`.

- **Shim build:** `drm_shim_ref.sh build` from
  `$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz`, with
  `LLVM_DIR=/usr/lib/llvm-21` (this host).
- **Shim check:** it reproduces
  `deqp_texmisc_tk_rnd_bias_fragcolor.expect.json` byte for byte.
- **`.nir.sha`:** from `tools/mali/nir_refs.sh 'deqp_texmiscres_*'` (21
  written, 0 errors, each at `--uniform-values`).
- **Pairs:** verbatim from the measure's `programs/`. Each `.license` names
  the case.

| Reference `deqp_texmiscres_*` | Case (`dEQP-GLES2.functional.`) | FS words | Group | Mesa's words that matter |
|---|---|--:|---|---|
| `tv_rnd_cube_mixed` | `uniform_api.random.6` | 10 (vs.varying 118) | TV-R3 | `TEX_SINGLE...16.cube.zero` at the constant direction, the texel compares in one `FMAX` chain with the int, `bvec4`, `vec3` and `mat4` member compares |
| `tk_proj_vec3_bias` | `shaders.texture_functions.fragment.texture2dproj_vec3_bias` | 18 | TK-B2 | `FRCP.f32 r3` (w), two `FMA.f32`, the pack from `r0` (`clamp_m1_1` by `u2.w0`, by `u2.w1`, `F32_TO_S32.rtz`, `U16_TO_U32`), `MOV.i32 r4..r6`, `2d.computed_bias @r4:r5:r6` |
| `tk_proj_vec4_bias` | `...texture2dproj_vec4_bias` | 15 | TK-B2 | the same, the divisor `r2`, staging in place `@r0:r1:r2` |
| `tk_rnd_proj_mul` | `shaders.random.texture.fragment.119` | 18 | TK-P2 | fp32: `F32_TO_S32.rtz`, `IADD.s32 .. u2.w0`, `S32_TO_F32` (the int lane), three `FMA.f32` products, `FRCP.f32`, `2d.computed` |
| `tk_rnd_proj_const_lane` | `.123` | 12 | TK-P2 | the constant lane `true` folded: `FRCP.f32` of the varying, one `FMA.f32 r1, u2.w0, r0`, the coordinate `@r2:r3` |
| `tk_allf_proj_fma` | `shaders.random.all_features.fragment.60` | 25 | TK-P2 | three varyings narrowed, `MKVEC.v2i16 r5, r1.h1, r2.h0`, two `FMA.v2f16` (`.h10` / `.h11` swizzles), three `F16_TO_F32`, `FRCP.f32`, `2d.computed` |
| `tk_rnd_bias_zero` | `.70` | 11 | TM1 | `FRCP.f32`, one `FMA.f32`, `MOV.i32 r1, 0x0` (the folded `false` lane), `2d.computed` (no bias) |
| `tk_rnd_bias_zero_dead_cube` | `.26` | 9 | TM1 | no cube; the all-constant coordinate as FAU words `MOV.i32 r0, u2.w0` / `r1, u2.w1`, `2d.computed` |
| `tk_rnd_bias_dead_proj` | `.99` | 19 | TK-DP | the bias look-up dropped; `FADD_IMM.v2f16 #0x3C003C00`, `FADD.v2f16 .. u2.w0`, three `F16_TO_F32`, the projection, `2d.computed` |
| `tk_rnd_proj_bias_whole` | `.81` | 19 | TK-B4 | the bias `FADD.f32 r0, r1^, r0^` (c + b), the projection of the `vec4` varying, the pack, `2d.computed_bias` |
| `tk_rnd_proj_bias_comp` | `.76` | 25 | TK-B4 | `S32_TO_F32 u2.w0`, fp32 subtractions of the coordinate, `FRCP.f32`, the bias `b + 8.25` in fp16 (`FADD.v2f16 .. u2.w1`, `F16_TO_F32`), the pack, `2d.computed_bias` |
| `tk_rnd_cube_bias_const` | `.77` | 27 | TK-C | the dead 2D look-up dropped; `CUBEFACE1 r2, r1, 0x3F800000, u2.w1` (constant lanes 1.0 and -3), the bias `b + 0.75` in fp16, the pack, `cube.computed_bias` |
| `tk_rnd_cube_bias_dead2d` | `.139` | 31 | TK-C | the dead 2D projection dropped; the fp16 direction difference (`MKVEC.v2i16` of uniform halves, `FADD.v2f16 .neg`), `CUBEFACE1 .. u3.w1`, the bias `2.25 - c`, the pack, `cube.computed_bias` |
| `fcm_ivec3_to_vec3` | `shaders.conversions.vector_to_vector.ivec3_to_vec3_fragment` | 35 | FC-C | `V2F32_TO_V2F16`, `FMA.v2f16 * 0x3C03` for the pair and `FMA.v2f16 r1, r2^.h00, r1^` for the third lane, three `F16_TO_S32.rtz` / `S32_TO_F32` / narrowing, the harness as a pair and a `.h00` lane, `LSHIFT_AND.v2i16` twice, `MUX.v2i16` |
| `fcm_bool_ivec2_to_vec3` | `...vector_combine.bool_ivec2_to_vec3_fragment` | 32 | FC-C | the `ftrunc` form (`F16_TO_F32`, `FROUND.f32.rtz`, narrowing) for both int lanes, the bool lane `FCMP_OR.v2f16.ne.f1`, the width-3 harness |
| `fcm_int_to_mat3` | `...scalar_to_matrix.int_to_mat3_fragment` | 26 | FC-C | `ftrunc` form, `U16_TO_U32` / `MKVEC.v2i16 0x0.h0` columns, seven `FMAX.v2f16` (`.abs` of the column distances and of FAU words), one `FCMP_OR.v2f16.ge.f1` |
| `fcm_ivec4_to_vec4` | `...vector_to_vector.ivec4_to_vec4_fragment` | 40 | FC-D | two `V2F32_TO_V2F16`, two `FMA.v2f16`, four round-trip lanes, two pair harnesses, three `LSHIFT_AND.v2i16` |
| `fcm_int_to_mat4` | `...scalar_to_matrix.int_to_mat4_fragment` | 33 | FC-E | `ftrunc` form, `MKVEC.v2i16 r1, 0x0.h0, r0.h0` + `U16_TO_U32`, four `FADD.v2f16` column distances, `FMAX.v2f16` tree with FAU `.abs` words, two `FCMP_OR.v2f16.ge.m1` |
| `fcm_rnd_cmp_ops` | `shaders.random.comparison_ops.fragment.26` | 9 | FC-E6 | everything folded: `MKVEC.v2i16 r0, u2.w0.h0, u2.w1.h0`, `FADD.v2f16 r0, u3.w0, r0^.neg`, `FADD_IMM.v2f16 #0x48804000` |
| `fcm_rnd_conv_const` | `shaders.random.scalar_conversion.fragment.40` | 13 | FC-F | `S32_TO_F32 r1, u2.w0`, narrowing, `MKVEC.v2i16 r1, u4.w0.h0, r1^.h0`, three `FMA.v2f16` |
| `fcm_rnd_cube_bias_neg` | `shaders.random.texture.fragment.37` | 29 | FC-G | `FMA.v2f16 r0, u2.w1, u2.w0, r0^.neg` (the fp16 negation folded into an FMA), the cube sequence of a computed direction, the bias `x * x` fp32, the pack, `cube.computed_bias` |

All 21 are refused at the tip with their case's text
(`tests/mali_deqp_texmiscres.rs` `REFUSED`; `LOWERED` and `DIFFERS` are
empty). Together with the 17 `deqp_texmisc_*` and the 7 `deqp_fmaloc_tk_*`,
every group of section 4 has a reference.

## 6. Dependencies and order

| Order | Step | Cases | Needs | Can start |
|--:|---|--:|---|---|
| 1 | **1** TV1 / TV1c | 61 | `intval-vs16` M4, **landed** (`vsapi_mat2` is byte-equal in `tests/mali_deqp_unitype.rs`, `nir.rs` `vs_cmp_and`): extend its fold to `VsTex { medium }` operands at `nir.rs:6516`. TV1c also needs a constant-direction cube sequence in the vertex walk that is not folded. TV-R3 needs vs16's uniform words beside it. 21 `_both` cases also need their fragment partner (fclane / TexCoord, `mali-deqp-texmisc.md` 1.1) | now (train 5 or later; `src/shaderc/online/mali/**` is held by the trains) |
| 2 | **2** FC1 | 32 | `alu16` arms for `Fabs` (2 and 4 lanes), an odd width (3 = pair + lone lane), `lone16` of a narrowed `I2f`/`Ftrunc` lane, the zero-half pair (`:3794`) for `mat4`. Smallest first: `tm_fcm_int_to_float`, then widths 2, 3 and 4. 3 bool-lane cases need `intval-rest`'s `i2b` (landed in train 1) | now |
| 3 | **4** then **3** (one look-up session) | 30 + 21 | the liveness pass first (TK-D, TK-DP, 5 dead cubes); the 8.8 pack for D2 (`LodMode::ComputedBias` for `dim: D2`); then the projection divide. TK-P2's computed coordinates use fmaloc's `coord_lane` sums; the int lane of `.98` / `.119` is intval | after step 2 or beside it (different functions of `nir_fs.rs`) |
| 4 | **TM1** | 2 | step 4's bias arm: a constant-zero bias is a plain look-up | with step 4 |
| 5 | **5** TK3 | 6 | step 4's pack on `cube.computed_bias`; fmaloc T1's cube sequence for computed directions (train 4) | after step 4 and T1 |
| 6 | **6** FC2 | 7 | step 2; `opform`/`mixprec` uniform halves (FC-E6, FC-F); step 5 for `.37` | after step 2 |

Forecast (HOST-ONLY: first refusals removed from these three texts; a case
can meet a later refusal): step 1, 61; step 2, 32; steps 3+4+TM1, 53; step
5, 6; step 6, 7. Of the pinned references, these become byte-compare targets:

- step 1: the 5 `tm_tv_*` and `tv_rnd_cube_mixed`;
- step 2: the 4 `tm_fcm_*` conversion references and the 5 new `fcm_*`
  conversion references;
- steps 3/4/TM1: 13 `tk_*` here, 6 `tm_tk_*` and the 6 2D `fm_tk_*`.

**Pending trains move four of the new pins.** The other items' result commits
are reachable here, and with this item's files on them `refusals_pinned` gives:

- `13259474` (lower-train-2), `8180095c` / `cbb6b45b` (lower-vsloop) and
  `2c0c6596` (lower-vsconst) move none.
- `e8327e42` (lower-fsint) moves `tk_rnd_cube_bias_dead2d` and
  `tk_rnd_proj_bias_comp` to `Arithmetic`. The int-to-float lanes of their
  dead statements now reach an arithmetic refusal before the look-up.
- `81b3ce19` (lower-mixprec3) moves `fcm_rnd_cmp_ops` to `FsUniform` and
  `fcm_rnd_cube_bias_neg` to `TexKind`.

Whichever lands after this item must update those four `REFUSED` entries.
After those trains, the step 6 random cases and two TK-B4 / TK-C cases have
other texts first: re-measure before starting steps 5 and 6.

## 7. What this note does not know

- The words of a lowering at an odd fp16 width beyond the three FC-C
  references (`vec2_ivec2_float_float_int_bool_bool_to_mat3` and
  `float_bvec4_ivec2_bool_bool_to_mat3` mix bool, float and int lanes): draw
  them if the step's words differ.
- The two `uniform_api.random` TexVertex cube arrays and `random.6` carry
  vs16 uniform shapes (struct `mat4`, `bvec4`): whether TV1 finishes them
  alone depends on those arms.
- Counts are HOST-ONLY first refusals at `349f989f`, never a render verdict.
