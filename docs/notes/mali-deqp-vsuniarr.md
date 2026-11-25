<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex-shader uniform-array refusal at 8ff01055, its shapes and Mesa 26.2.1 references

**Status: study** (plan #402, item `ref-vsuniarr`; issue #20). Nothing under
`src/` changed. The references are `tools/mali/testdata/deqp_vsuniarr_*`, and
`tests/mali_deqp_vsuniarr.rs` pins them. Measured HOST-ONLY at the integration
tip **`ab919063`** (2026-11-25). Nothing in this note was checked on the
NanoPC-T6.

The refusal is `MaliError::UniformArray` (`src/shaderc/online/mali/mod.rs:430`):
"a uniform array is not lowered yet". On the board
(`docs/es2-cts-failures.md` section M) the vertex-shader text is 193 Fail at
`8ff01055`. `docs/notes/mali-deqp-uniarray.md` (plan #377) drew the float,
vec2, vec3 and vec4 arrays and lowered them; what is left is every other
element type.

## 1. How this was measured

- **Cases:** the 193 cases whose `refusals.tsv` text in the slices
  `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` holds this refusal.
  All are in S1, all are `dEQP-GLES2.functional.uniform_api.*`, all are board
  Fail, and all are refused in the vertex shader.
- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --jobs 8 --slice <each of the four> --cases
  <the 193 names>` (`--filter` matches case names, not refusal text, so the
  list went in through `--cases`). VK-GL-CTS is at `93bca018` (a depth-1
  clone of that commit, `external/fetch_sources.py` run, the three files of
  `cts-yakogl-target.patch` added). All 193 cases began; 2 processes exited 1.
  **All 193 refuse with this text first, in the vertex shader**; none is
  "not reproduced". The partner fragment shader's own refusal is `unitype`
  for 94 cases (the `_both` cases, whose fragment shader reads the same int
  or bool uniforms), `367:TEX_COORD` for 2 and none for 97.
- **Refusal site:** one, `nir.rs:1103` in `uniform_of`: an array
  (`array_len > 1`) that `array_element` (`nir.rs:1141`) does not take. That
  function takes `GL_FLOAT`, `GL_FLOAT_VEC2`, `GL_FLOAT_VEC3` and
  `GL_FLOAT_VEC4` elements only, so every int, bool and matrix array refuses
  here, whether it is a plain uniform or a struct's member.
- **Shapes:** each case's refusing program (the measure's `programs.tsv`) was
  classified by the arrays its vertex shader declares and reads: element
  type, plain or struct member, every element read or some.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_vsuniarr_<shape> --instrument shim`, on the
  drm-shim that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's
  self-check (`gm2_cf_ifelse_vs`) was byte-equal. All 8 were drawn a second
  time and came out identical. The pairs are verbatim from the measure's
  `.qpa`; the `.license` beside each shader names the case.

## 2. The shapes at the tip (HOST-ONLY)

| Shape | Cases | The vertex shader's arrays | Reference |
|---|--:|---|---|
| **`bvec4`** | **32** | `uniform bvec4 u_var[3]`, every element compared (`basic_array.bvec4`, with `bvec4_api_int` and `bvec4_api_float`) | **new**: `bvec4` |
| **`ivec4`** | **22** | `uniform mediump ivec4 u_var[3]` | **new**: `ivec4` |
| **`mat4`** | **16** | `uniform mediump mat4 u_var[3]` | **new**: `mat4` |
| **`multi`** | **18** | `mediump float u_var_0[3]`, `mediump ivec3 u_var_1[3]`, `bvec2 u_var_2[3]` in one block (`multiple_basic_array`) | **new**: `multi` |
| **`struct_mat`** | **14** | a struct's members `mat4 m2[3]`, `mat2 m3[3]` behind `mat4 m0`, `mat2 m1` (`array_in_struct.mat4_mat2`) | **new**: `struct_mat` |
| **`nested_mat`** | **10** | `mat4 ms1[2]` and the struct array `ms2[2]` (a `mat4` and a `mat2` each) of a nested struct (`nested_structs_arrays.mat4_mat2`) | **new**: `nested_mat` |
| **`unused_bvec4`** | **4** | a struct's `bool m2[3]`, `bvec4 m3[3]`; the shader reads `m0` and `m3[1]` alone (`unused_uniforms.bool_bvec4`) | **new**: `unused_bvec4` |
| **`unused_mat2`** | **4** | a struct's `mat4 m2[3]`, `mat2 m3[3]`; `m0` and `m3[1]` alone are read (`unused_uniforms.mat4_mat2`) | **new**: `unused_mat2` |
| `bool` | 32 | `uniform bool u_var[3]` (`basic_array.bool`, `bool_api_int`, `bool_api_float`) | covered: `deqp_unif2_arr_bool_array` |
| `int` | 22 | `uniform mediump int u_var[3]` | covered: `deqp_unif2_arr_int_array` |
| `random` | 19 | `uniform_api.random.{6,10,15,17,24,37,53,57,61,66,67,71,74,75,76,77,83,94,99}`: one program a case | none (past the 8 allowed); `random.10` is `deqp_unif2_arr_mat_array` |
| **Total** | **193** | VS 193 / FS 0; board 193 Fail / 0 NS | 8 new references |

Each named shape is half `_vertex` cases and half `_both` cases (87 and 87 of
the 174); the references are the `_vertex` program of each, so the fragment
shader is the varying alone and the pair is a vertex-shader compare.

What the plan's list of shapes becomes:

- **Constant-index elements of float, vec2, vec3, vec4 arrays:** none left
  in this refusal (`deqp_uniarray_*_const_vs` are byte-equal).
- **Matrix arrays:** `mat4` (plain, fp16), `struct_mat`, `nested_mat`,
  `unused_mat2` (struct members, fp32).
- **Arrays of structs:** `nested_mat`'s `ms2[2]`; a plain `uniform S
  u[1]` occurs once, in `random.6` (not drawn; `deqp_unif2_arr_struct_array`
  is the `shaders.struct` form).
- **Run-time-indexed arrays:** none. Every index of the 193 is a constant,
  so Mesa pushes every word that is read and no reference has an `LD_PKA`
  (no UBO 0 load). The push-or-UBO choice is not met by this refusal.
- **Arrays read in part:** `unused_bvec4`, `unused_mat2`, and 13 of the 19
  `random` cases.

The `random` cases not drawn: plain `ivec3[]` (66, 77), `bvec2[]` (37, 76),
`bvec3[]` (67), `mat2[]` (17), `mat4[]` (53, 71, 75), `bool[]` with `mat4[]`
(99), `ivec2[]` with a struct `mat4[]` (94), and struct members that are
`mat2`, `mat3`, `mat4`, `ivec3` or `bvec4` arrays (6, 15, 24, 57, 61, 74,
83). `random.6` also calls `textureCube` in the vertex shader, and
`random.24` has a sampler member.

## 3. The references

Every vertex shader is `gl_Position = a_position; v_vtxOut = 1.0; v_vtxOut
*= compare_T(u..., constant); ...` and every fragment shader writes
`v_vtxOut` to the colour. In all eight, `vs.position` is the same 24 words
and `fs.fragment` the same 10; the difference is `vs.varying` and the FAU.
FAU words 0 to 10 are the viewport transform and the three varying offsets
(`00000004 00000008 0000000c`); the uniforms start at word 11 (`u5.w1`).

| Reference | dEQP-GLES2.functional.uniform_api. case | `vs.varying` words | FAU words | What Mesa does |
|---|---|--:|--:|---|
| `bvec4` | `value.assigned.by_pointer.render.basic_array.bvec4_vertex` | 20 | 24 | 12 words of 32 bits (11 to 22). Each `==` with a constant bvec4 is folded into the words: `LSHIFT_AND.i32` over the lanes expected true, `LSHIFT_OR.i32` then `.not` over the lanes expected false; `CSEL.u32.eq` to 1.0 |
| `ivec4` | `...basic_array.ivec4_vertex` | 29 | 26 | 12 words (11 to 22); a lane is `ICMP_OR.u32.eq.m1` or `ICMP_AND.u32.eq.m1` with its constant: a byte lane of an embedded table (`0x7060504.b3`), `IADD_IMM #0xFFFFFFFD`, or an FAU constant (`fffffffb`, `fffffffc` at 23, 24) |
| `mat4` | `...basic_array.mat4_vertex` | 77 | 36 | **fp16**: 24 words of lane pairs (11 to 34), two a column, eight an element. `FADD_IMM.v2f16` of each word with the negated constants, an `FMAX.v2f16` tree of the 48 `abs`, one `FCMP_OR.v2f16.lt` with 0.05 (`2a662a66` at 35), `S16_TO_S32` |
| `multi` | `value.assigned.by_pointer.render.multiple_basic_array.vertex` | 43 | 36 | the bvec2 words first (11 to 16), then the float array as fp16, one element a word (17 to 19, read `.h00`), then the ivec3 words (20 to 28): not the declaration order (float, ivec3, bvec2). Constants at 29 to 35 |
| `struct_mat` | `value.assigned.by_pointer.render.array_in_struct.mat4_mat2_vertex` | 205 | 92 | **fp32** members in declaration order, 80 words (11 to 90): `m0` 16, `m1` 4, `m2[0..2]` 48, `m3[0..2]` 12. Each word pair is `MOV` and `V2F32_TO_V2F16`, then the fp16 tree as `mat4` |
| `nested_mat` | `value.assigned.by_pointer.render.nested_structs_arrays.mat4_mat2_vertex` | 245 | 108 | fp32, 96 words (11 to 106) depth first: `m0` 16, `m1.ms0` 4, `m1.ms1[0..1]` 32, `m1.ms2[0].mss0` 16, `.mss1` 4, `m1.ms2[1]` 20, `m2` 4 |
| `unused_bvec4` | `value.assigned.unused_uniforms.bool_bvec4_vertex` | 10 | 16 | only the words read are pushed: 5 (11 to 15) for `m0` and `m3[1]`. Their values (`ffffffff 00000000 ffffffff ffffffff 00000000` for `m0` true, `m3[1]` true, false, true, false) show `m0` is not first |
| `unused_mat2` | `value.assigned.unused_uniforms.mat4_mat2_vertex` | 55 | 32 | only `m0` (16 fp32 words, 11 to 26) and `m3[1]` (4 words, 27 to 30) are pushed, in declaration order |

Two rules of `docs/notes/mali-deqp-uniarray.md` section 7 hold for the new
element types:

- A plain `mediump` matrix array is **fp16** in Mesa's block, as a plain
  mediump float or vector array is; a struct's matrix member, array or not,
  stays **fp32** and is narrowed in the shader.
- Int and bool words are 32 bits each, a bool `ffffffff` or `00000000`.

With all 193 cases at constant indices, nothing here is read from UBO 0:
the largest table is `nested_mat`'s 108 FAU words.

### What the references do not show

`gl_ref.py --uniform-values` sets no element of a plain array (its location
lookup of the declarator is -1; `tools/mali/README-deqp_ref.md`, "Struct
uniforms"), so the array words of `bvec4`, `ivec4`, `mat4` and `multi` are 0
in the FAU dump. Which element and lane a word holds is to be read from the
code (the constant each word is compared with), not from the dump. The
struct references (`struct_mat`, `nested_mat`, `unused_*`) carry each
member's values (16 (k + 1) + c for member k, a bool true where c is even).

## 4. The test

`tests/mali_deqp_vsuniarr.rs`:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsuniarr`.
At `ab919063`, 1 test passes (`refusals_pinned`) and 8 are ignored. Each
reference is `#[ignore]`d with the `UniformArray` refusal quoted and pinned
in `REFUSED`. `refusals_pinned` fails when a `deqp_vsuniarr_*` reference is
in neither `REFUSED` nor `MATCHED`, or when a pinned refusal moves. A
lowering that takes a reference removes its `#[ignore]` and its pin and adds
it to `MATCHED`.

## 5. For the lowering (`lower-vsuniarr`)

- The 54 `bool` and `int` cases share the site with these references;
  `deqp_unif2_arr_bool_array` and `deqp_unif2_arr_int_array` are their
  references (`_both` programs: the fragment shader then needs its own int
  and bool uniforms).
- Half of every shape is `_both`. Once the vertex shader compiles, those
  cases stop at the fragment shader's `unitype` refusal (94 cases by the
  measure's partner column), so the host count of this refusal can reach 0
  while about half the cases still fail for the fragment shader.
- The comparisons are the other families': `compare_bvec4`, `compare_ivec4`
  (int and bool values, `==` of vectors) and `compare_mat*` (the fp16
  `abs`/`fmax`/`<` tree that `deqp_unitype_vsapi_mat2` and
  `vs_cmp_and` already make for a mat2).
- Mesa's push order is not always the declaration order (`multi`,
  `unused_bvec4`).
