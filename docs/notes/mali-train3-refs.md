<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Train 3: the Mesa 26.2.1 reference of every schedule 5.4 step

Plan #387 item `train3-refgaps`, 2026-11-27, integration tip `7e648113`. The
lowering schedule (`docs/notes/mali-lowering-schedule.md` 5.4) orders Train 3's
commits. This note checks that each step has at least one Mesa 26.2.1
reference that pins its shape, names it, and says which shapes were
missing and how they were drawn. No `src/` file and no existing design, test
or reference was touched.

## 1. Result

Every step has a reference except one that cannot be drawn. Three gaps were
found; two are filled (four references):

| gap | step | what | now |
|---|---|---|---|
| `random.texture.fragment.145` | M2-3 (commit 15) | the design says "draw it from this case's `.qpa` when the step starts" (`mali-mixprec2-lowering.md` M2-3) and 5.4 commit 15 repeats it | drawn: `deqp_t3gap_mixprec2_rnd_tex145`, refused (mixed precision) |
| `struct.local.{array_member,struct_array,nested_struct_array}_dynamic_index_vertex` | V3 (commit 7) | `mali-vsunif-lowering.md` section 7: "3 NS, compiled, have no reference ... the next item should draw one before it counts them under V3" | drawn, all three: `deqp_t3gap_vsunif_{array_member,struct_array,nested_struct_array}_dynidx`, compile, differ |
| `uniform_api.random.23` | unifmp U8 | more than 64 uniforms; Mesa 26.2.1 on the shim cannot draw it ("export failed", `mali-deqp-fsunif2.md` section 2); the step stays refused | **none can be drawn** (the shim's limit, not ours); the design leaves U8 refused and 5.4 has no commit for it |

The nested struct array one differs by more than a word order: Mesa keeps
it in thread-local storage (70 `vs.position` words against our 24), so it is a
V4 shape, not V3 (section 4).

## 2. How they were drawn

* The three struct shapes are the C++ source of
  `es2fShaderStructTests.cpp` (VK-GL-CTS 93bca01) with `${DECLARATIONS}`,
  `${COORDS}`, `${DST}` and `${ASSIGN_POS}` expanded for a vertex case the way
  the existing `deqp_vsunif_local_*` references show them (`a_position`,
  `a_coords`, `v_color`, `gl_Position = a_position`; the fragment shader is
  `gl_FragColor = v_color`).
* `random.texture.fragment.145` is a generated shader. It was written by a
  `deqp-gles2` of the same CTS built with `-DDEQP_TARGET=null` (no GL: the
  `.qpa` keeps the shader sources), run with
  `--deqp-case=dEQP-GLES2.functional.shaders.random.texture.fragment.145`.
  The same run for `.140` reproduces `deqp_mixprec2_rnd_tex140.{vert,frag}`
  byte for byte, so the sources equal the host measure's.
* All four: `tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_t3gap_<shape>
  --instrument shim` on the G610 drm-shim of `tools/mali/drm_shim_ref.sh build`
  (Mesa 26.2.1, `--uniform-values`).

## 3. Their state at the tip (`tests/mali_deqp_t3gap.rs`, `STATE`)

| reference | state | first difference |
|---|---|---|
| `mixprec2_rnd_tex145` | refused: "Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet" | |
| `vsunif_array_member_dynidx` | compiles, differs | `vs.varying` instruction 0: Mesa's `MOV` and `ICMP_OR.s32.lt.m1` before the attribute load; ours loads first and builds an `ICMP_OR.u32.eq` / `CSEL` tree |
| `vsunif_struct_array_dynidx` | compiles, differs | the same, 38 words both |
| `vsunif_nested_struct_array_dynidx` | compiles, differs | `vs.position` 24 words ours, 70 Mesa's (thread-local stores) |

`state_pinned` (running) holds each state and that every `deqp_t3gap_*` on disk
is listed; the four per-reference tests are `#[ignore]`. A step that changes a
state moves it in `STATE` and drops the test's `#[ignore]` in the same commit.
Run:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_t3gap`.

## 4. Every step and its references

Step names are the designs' (`unifmp` U1-U8 are the schedule's FM1-FM7:
FM1 = U1, FM2 = U2, FM3 = U3, FM4-FM7 = U4-U7 in the schedule's commit 17).
The commit numbers are 5.4's. "t3gap" marks a new reference of this item.

| Commit | Step | References (the test file pins each) |
|--:|---|---|
| 1 | unifmp U1 + FM1 (`fs16` rows, vertex `array_element`) | `deqp_fsunif_{arr_bvec4,arr_ivec4,arr_mat4,struct_int_ivec4}`, `deqp_fsunif2_*`, `deqp_fsunif3_*`, `deqp_unif2_fs_{bool,int,struct}_uniform`, `deqp_unif2_arr_{bool,int}_array`, `deqp_vsuniarr_{bvec4,ivec4,multi}`, `deqp_vsuniarr2_{ivec3,bvec3}_part` |
| 2 | opform O1 | `deqp_arith_op_binary_*`, `op_unary_*`, `op_common_{abs,mix}_fs_mediump`, `op_geometric_*`, `op_exponential_exp2_fs_*` |
| 3 | opform O2 | `deqp_opform_{matadd,matsub}_uniform`, `deqp_arith_matrix_storeout_fs_mediump` |
| 4 | opform O3 | `deqp_opform_{ret_never,mul_float_vec4}`, `deqp_arith_other_storeoutput_fs_none` |
| 5 | unifmp U2 (FM2) | `deqp_unifmp_{interaction_16,inv_mediump_cse2,rsg_expr10}`, `deqp_intval3_rsg_swizzle12_fs`, `deqp_misc2_invariant_cse` |
| 6 | unifmp U3 (FM3) | `deqp_unifmp_{mat4_sub_mat4,vec4_mat4_mul,rsg_sconv12}`, `deqp_fsunif_{mat4_add_float,mat2_mul_mat2}`, `deqp_uniarray_api_float`, `deqp_unif2_fs_other` |
| 7 | vsunif V3 + V2 | V2: `deqp_vsunif_{vec2_dynwrite,tmp_dynwrite,mat2_dynwrite,varying_array}`. V3: `deqp_vsunif_{vec2_subscript,mat2_subscript,local_cond_assign,vec_subscript,tmp_array}`, and the local struct arrays at a run-time index: **t3gap** `deqp_t3gap_vsunif_{array_member_dynidx,struct_array_dynidx}` |
| 8 | mixprec2 M2-1 + M2-4a | `deqp_mixprec2_{rnd_sconv36,rnd_swz4,rnd_tex140,hp_out_param}`, `deqp_mixcol_hp_{param,seq}` |
| 9 | vsuarr VA1; vsunif V1 (VT1) | VA1: `deqp_vsuarr_init_{bool,int,bvec4,ivec4,multi}`. V1: `deqp_vsunif_{local_equal,local_nested_equal,struct_cmp}` |
| 10 | mixprec2 M2-4b | `deqp_mixprec2_hp_mat2_add` |
| 11 | mixprec2 M2-4c | `deqp_mixprec2_hp_{fract,sqrt_vec3,length_vec2}`, `deqp_mixprec3_hp_*`, `deqp_mixcol_hp_*` |
| 12 | vsuarr VA2 (with unifmp U8's mat3 element) | `deqp_vsuniarr_mat4`, `deqp_unif2_arr_mat_array`, `deqp_vsuniarr2_{mat3_part,mat4_mixed}`, `deqp_vsuarr_init_mat4`. (U8's own case, `uniform_api.random.23`, cannot be drawn: section 1; VA2's mat3 reference covers the element rule) |
| 13 | unif2 U5 + vsuarr VA3 | `deqp_unif2_{vs,fs}_struct_uniform`; `deqp_vsuniarr_{struct_mat,nested_mat,unused_mat2}`, `deqp_vsuniarr2_{struct_mat4_arr,mat2_struct,struct_vec3_mat2,struct_nested}`, `deqp_vsuarr_init_{struct_mat,nested_mat}`; U5's bool/bvec4 arrays: `deqp_vsuniarr_unused_bvec4` |
| 14 | opform O4 | `deqp_opform_{min_vec3,clamp_vec3_float}` |
| 15 | mixprec2 M2-2 + M2-3 | `deqp_mixprec2_{struct_nested,seq_side}` (M2-2, M2-3); `random.texture.fragment.145` (M2-3): **t3gap** `deqp_t3gap_mixprec2_rnd_tex145` |
| 16 | opform O5, O8, O6, O7 | O5 `deqp_opform_conv_vec3` (+ arith `other_storeoutput_fs_mediump`); O8 `deqp_opform_{rnd_splat18,rnd_constneg85,matmul_const}`; O6 `deqp_opform_matneg_mat3`; O7 `deqp_opform_ret_dynamic` |
| 17 | unifmp FM4-FM7 (U4-U7) | U4 `deqp_unifmp_{highp_mat3_mul,highp_vec3_mat3,struct_array,struct_nested}`; U5 `deqp_boolpix_lk_uniform_struct_{vec4,fragment_only_vec4}`, `deqp_audit_o_linkage_uniform_struct_vec4`; U6 `deqp_unifmp_struct_sampler`; U7 `deqp_unifmp_interaction_88` |
| after 17 | varyload (`varyload-lower-1`, no 5.4 commit) | `deqp_varyload_{rnd57,rnd31,cond36,basic33,cond19,swz41,basic8,tex110}` (`mali-deqp-varyload.md`) |

Left out of Train 3 by 5.4:

| Step | References |
|---|---|
| nonfin NF1 (unrolled `coords * 0.5` chain; constprod K2) | `deqp_cfold2_{zero_operand_inot,const_other_inot}` |
| nonfin NF2 (`partial_unroll`; loops S2) | `deqp_vsconst_mixloop_{tmp_array,varying_array,vector_subscript}` |
| nonfin NF3 (the NaN word) | `deqp_nonfin_{tmp_float_static_dynread,tmp_vec3_static_dynread,tmp_vec4_dyn,matrix_mat3_dyn,matrix_mat4_static_dynread,varying_float_static_dynread,varying_vec4_dyn}` |
| vsunif V4 / VT4 (thread-local array) | `deqp_vsunif_tmp_float_const`; **t3gap** `deqp_t3gap_vsunif_nested_struct_array_dynidx` (also thread-local) |
| vsunif V5 / VT5 (`LD_PKA` at a run-time index) | `deqp_vsunif_struct_dynidx` (`struct.uniform.struct_array_dynamic_index_vertex`) |
| FM8 | not lowered; none |

## 5. Not done here

* The G310 check (`tools/mali/g310_check.sh 'deqp_t3gap_*'`) was not run: it
  needs five more shim builds. `mali-g310-vs-g610.md` holds for 869 of 869
  references; run it with the next batch.
* The three struct sources are the CTS C++ expanded by hand, not a `.qpa`
  (the `.license` files say so). The vertex template matched the existing
  `deqp_vsunif_local_array_member` shader exactly in its header and tail.
