<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): mediump fragment-shader uniform shapes beyond `deqp_fsunif`'s eight

**Status: study** (plan #402, item `ref-fsunif2`; issue #20). Nothing under
`src/` changed. The references are `tools/mali/testdata/deqp_fsunif2_*`,
pinned by `tests/mali_deqp_fsunif2.rs`. Measured HOST-ONLY at the tip
**`ef5ce720`** (2026-11-25); nothing here was run on the NanoPC-T6. The
refusal and its other shapes are in `docs/notes/mali-deqp-fsunif.md`.

## 1. Measure

The 272 cases of `refusals.tsv` of `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`
holding the refusal went through `tools/mali/deqp-host-measure.sh --cases`
(features of the script, VK-GL-CTS `93bca018` with the yakogl target at
`$CONNECTED_DEPS_DIR/VK-GL-CTS-host`). 272 began, 265 refuse on the host (the
text of this refusal: 261 `unitype`, 4 `misc`), 11 are not reproduced
(7 pass compile and link on the host, 4 refuse with a different text).

## 2. Shapes without a reference at the start of this item

| Shape | Cases | Representative case | Reference |
|---|--:|---|---|
| `struct_mat_array`: struct of `mat4`, `mat2` and arrays of them | 9 | `uniform_api.value.assigned.by_pointer.render.array_in_struct.mat4_mat2_fragment` | `deqp_fsunif2_struct_mat_array` |
| `struct_mat_nested`: the same, nested structs | 5 | `...by_pointer.render.nested_structs_arrays.mat4_mat2_fragment` | `deqp_fsunif2_struct_mat_nested` |
| `multi_basic_array`: `float[3]`, `ivec3[3]`, `bvec2[3]` | 9 | `...by_pointer.render.multiple_basic_array.fragment` | `deqp_fsunif2_multi_basic_array` |
| `multi_nested`: three nested structs of float/vec4 arrays | 7 | `...by_pointer.render.multiple_nested_structs_arrays.fragment` | `deqp_fsunif2_multi_nested` |
| `api_random`: `uniform_api.random.{4,9,13,23,31,34,39,42,47,62,80,81,89,93}` | 14 | `uniform_api.random.13` | `deqp_fsunif2_random_13` (one program of the 14; drawn with `--zero-uniforms`: a sampler member) |

All 44 cases refuse with the `FsUniform` text first, in the fragment shader,
each vertex shader compiles. `random.23` (more than 64 uniforms) cannot be
drawn on Mesa 26.2.1 on the shim ("export failed", GL_INVALID_OPERATION in
`glDrawArrays`): no reference. The other 12 random programs are other
random combinations of the same member types and were not drawn (the shapes
differ only in which members the program reads).

## 3. Shapes the eight earlier references and others already cover

Unchanged from `mali-deqp-fsunif.md` section 3: `arr_bvec`, `arr_ivec`,
`arr_mat`, `struct_int`, `mat16`, `mat32`, `struct32` have `deqp_fsunif_*`;
`arr_bool`, `arr_int`, `struct_bool`, `rnd_two` are covered by
`deqp_unif2_fs_{bool,int,struct}_uniform` and `deqp_unif2_fs_other`;
`arr_float` by `deqp_uniarray_api_float`; `rnd_lane` by
`deqp_intval3_rsg_swizzle12_fs`; `invariance` by `deqp_misc2_invariant_cse`.
This item re-checked that no other `deqp_*` reference has an `ivec3`/`bvec2`
array or a matrix member in a struct.

## 4. What the references hold

The five were drawn with `tools/mali/deqp_ref.sh --instrument shim`; the
four non-random ones with `--uniform-values` (gl_ref.py gives leaf `k` the
values `16 (k + 1) + c`, so the struct tests compare FAU words with
`mali_ref::differences_members`). All five refuse at the tip with the
`FsUniform` text, pinned in `refusals_pinned`; each test is `#[ignore]`d.
A lowering of the shape removes the `#[ignore]` and moves its entry from
`REFUSED` to `MATCHED`.

## 5. The refusal site of each shape (item `probe-fsunif2-sites`)

Read HOST-ONLY at the tip **`53b04621`** (2026-11-25), file:line of that
commit. Method: every `MaliError::FsUniform` raise in `src/shaderc/online/mali/**`
was tagged with `file!()`/`line!()` in the working tree (reverted, nothing
under `src/` is changed), then the five references were compiled with
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fsunif2 -- --ignored --nocapture`.
The raise that ends each compile is `select.rs:628` for all five, in
`UboLayout::fs16` (`src/shaderc/online/mali/select.rs`), the `_ =>` arm of
the match on `u.gl_type`: a read uniform whose type and `array_len` no arm
takes (`GL_INT`/`GL_BOOL` and the vecs need `array_len <= 1`, `GL_FLOAT_MAT2/3/4`
too; a vector or `mat4` array is no arm). `fs16` walks the uniforms in
declaration order, so the construct below is the first read uniform that
falls through; later ones would refuse too. (The tag at `mod.rs:1603`
printed in every run is the eagerly evaluated argument of `ok_or` for
`fp32_uniforms`, not a raise: none of these has more than 64 uniforms.)

| Shape | Refusal site (tip `53b04621`) | Triggering construct | Step that takes it |
|---|---|---|---|
| `struct_mat_array` | `select.rs:628` | `u_var.m2`, a `mediump mat4[3]` (`GL_FLOAT_MAT4` with `array_len` 3: the `array_len <= 1` guard at `select.rs:622`); `m3[3]` (`mat2[3]`) after it. `m0`, `m1` are taken | the `arr_mat` row of `mali-deqp-fsunif.md` §5 (`deqp_fsunif_arr_mat4`): a `mat4`/`mat2` array row in `fs16` at 8 (`mat4`) or 2 (`mat2`) words an element, then the struct's member order of `deqp_unif2_fs_struct_uniform` |
| `struct_mat_nested` | `select.rs:628` | `u_var.m1.ms1`, a `mediump mat4[2]`; `ms2[].mss0` (`mat4`) and `mss1` (`mat2`) after it | as `struct_mat_array`, plus the nested-struct flattening the three `deqp_unif2_fs_struct_uniform` leaves already use |
| `multi_basic_array` | `select.rs:628` | `u_var_1`, a `mediump ivec3[3]` (`GL_INT_VEC3` needs `array_len <= 1`, `select.rs:610`); `u_var_2`, a `bvec2[3]`, after it. `u_var_0` (`float[3]`) is taken | the `arr_ivec`/`arr_bvec` rows (`deqp_fsunif_arr_ivec4`, `arr_bvec4`): layout only, one 32-bit word a lane, plus the integer compares of `fsint`/`intval`; no fp16 word |
| `multi_nested` | `select.rs:628` | `u_var_1.m1.ms1`, a `mediump int[2]` (`GL_INT` needs `array_len <= 1`, `select.rs:603`); then `ivec4` arrays and the `bvec4[2]`/`bool` members of `u_var_2`. `u_var_0`'s `float`/`vec4`/`float[2]` members are taken (fp32 struct members) | the `struct_int` row (`deqp_fsunif_struct_int_ivec4`): int and ivec arrays inside structs, layout only; the `bool`/`bvec4` members are `deqp_unif2_fs_struct_uniform`'s |
| `random_13` | `select.rs:628` | `u_var1.m2.m1.m2`, a `mediump mat4[4]` (the first array of a type the arms refuse in read order); then `ivec2[2]`, `ivec3[5]`, `bvec3[3]` arrays in `u_var2`, `u_var3`. `u_var0` (`mat3`), `float[5]`, `vec2`, `mat2`, `bool[1]` are taken. `u_var2.m0` is a `sampler2D` member (drawn with `--zero-uniforms`) | the `arr_mat` row for the `mat4[4]`, the `arr_ivec`/`arr_bvec` rows for the others; no step of its own, but it needs all of them together |

The line of each guard is the arm that did not match; `select.rs:628` is
the one line that raises. For the step names: `design-fsunif` and
`proto-fsunif` left no document in the tree, so "step" here is the lowering
list of `docs/notes/mali-deqp-fsunif.md` §5 (layout rows in `fs16`;
`arr_mat` the one that adds fp16 words), which this note's own shapes extend.
Taking `select.rs:628` is not enough for any of the five: once `fs16` accepts
the type, the reads reach the `nir_fs.rs` sites of that note's §2
(`10089`, `10102`, `9678`, `9683`), which this item did not probe.
