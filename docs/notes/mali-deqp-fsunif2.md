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
