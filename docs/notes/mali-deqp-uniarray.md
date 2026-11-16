<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): dEQP-GLES2 uniform array refusals, study

Plan #377 item `study-uniarray` (phase 1, study only: no code, no reference).
Tip `38ad30eb`. Companion of `docs/notes/mali-uniform-arrays.md` (float array
at a run-time index, lowered for case 108 and the fp32 forms),
`mali-uniform-index.md` (vec4 array, `LD_PKA`) and `mali-ubo-rows.md` (the
32-row `UboLayout`). Source of the counts: `refusals.tsv` of the four slices
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` (the first refusal per
case; a `_both` case is refused in its vertex shader first, so its fragment
shader's refusal is not counted here). Section M of
`docs/es2-cts-failures.md` (lines 230, 248, 289, 293-305) holds the totals:

| refusal | VS | FS |
|---|---|---|
| `a uniform array is not lowered yet` | 218 Fail + 2 NS | 47 Fail + 2 NS |
| `a uniform read at a run-time index other than ...` | 0 | 2 Fail + 1 NS |

dEQP's generated shaders are C++-made; they are not in the tree. The shapes
below are read from the case names; the exact GLSL of each is to be taken from
the `measure` item's `tools/mali/testdata/deqp_shapes/` pairs, and where this
note says "expected" it is dEQP's documented generator behaviour, not measured.

## 1. Refusal sites

`MaliError::UniformArray` (`mod.rs:130`, text `mod.rs:417`):

- `nir.rs:1004-1012` `uniform_of` (vertex stage and the shared walk): an array
  (`array_len > 1`) is admitted only as `GL_FLOAT_VEC4` with `dwords ==
  4 * array_len`; every other element type, and a float array, refuse here.
  This is the site of almost all the 218 VS refusals.
- `nir_fs.rs:5866-5890` `uniform_extent` (fragment): admits vec4 arrays and
  `GL_FLOAT` arrays (`dwords == n`); anything else refuses (line 5885).
- `nir_fs.rs:5900-5903` `uniform_load` (highp fragment): a float array has no
  highp constant-index form.
- `nir_fs.rs:4875-4878` (`ir::Op::LoadUniform`): a float array's element at a
  **constant** index: "no reference draws it" (case 108 reads at run-time
  indices only).

`MaliError::UniformIndexed` (`mod.rs:134`, text `mod.rs:418-419`), the three
`uniform array read at a run-time index` cases (all `shaders.indexing.
uniform_array.*_dynamic_read_fragment`: vec2, vec3, vec4):

- `nir_fs.rs:4885-4925` the `ir::Op::LoadUniformIdx` arms: mediump takes only a
  float element (`comps == 1`, no vec4 read beside it) or a vec4 element
  (`comps == 4`, first and only read, `f.nubo == 0`) whose index is a raw int
  uniform word (`int_word`, 4905-4906); `MAX_UBO_READS` (`nir_fs.rs:60`) caps
  the reads. vec2 and vec3 elements (`comps` 2, 3) fall to line 4899/4920.
- `nir.rs:1071-1099` `uniform_idx` (highp, vertex): `comps == 1` float or
  `comps == 4` vec4 only; the index an int uniform or (vec4 only) a 32-bit
  `Iadd`.
- `nir_fs.rs:10153-10206` `ubo_load` / `ubo_offset`: the array must start the
  block (`lv.ubo.word(dword) != Some(0)`), `words == 2` (an 8-byte mediump vec4
  element), the index a pushed int.

## 2. Shapes and counts

All counts are the first-refusal cases of the four slices. Total compile
refusals this note takes: VS 218 + FS 47 (+ 4 NS, + 3 for the run-time form).

| shape | cases | refused at |
|---|---|---|
| A. `uniform_api.value.{assigned,initial}.*` over arrays: `basic_array.<T>_{vertex,fragment,both}`, T = float, int, bool, ivec4, bvec4, `*_api_int/float` (bool read with `glUniform*i`/`*f`), `array_in_struct.float_vec4_*`, `nested_structs_arrays`, `multiple_*`, plus the `by_pointer`/`by_value`/`basic_array_assign_{full,partial}`/`get_uniform`/`render` groups | VS 138 assigned + 40 initial = 178; FS 28 + 10 = 38 | `nir.rs:1011`; float-array FS at `nir_fs.rs:4877` |
| B. `uniform_api.info_query.*` (`basic_array`, `array_in_struct`, `multiple_basic_array`, `nested_structs_arrays`, `multiple_nested_structs_arrays`, `unused_uniforms.mat4_mat2`) | VS 22 (10 basic_array + 2 each of the others), FS 6 | same |
| C. `uniform_api.random.{10,15,37,49,51,61,65,66,67,69,70,72,76,77}` | VS 14 | same |
| D. `shaders.indexing.uniform_array.*_static_read_vertex` (float, vec2, vec3), `*_static_loop_read_fragment` (float, vec2, vec3, vec4), `float_static_read_fragment` | VS 3, FS 5 | `nir.rs:1011`; FS float at 4877 |
| E. `shaders.indexing.uniform_array.*_dynamic_read_{vertex,fragment}` (vec2, vec3 VS; vec2, vec3, vec4 FS) | VS 2 + FS 3 as `UniformArray` (+ `UniformIndexed` 3 on FS) | see section 1 |
| F. `shaders.struct.uniform.{array_member,nested_struct_array}_{vertex,fragment}`, `state_query.shader.uniform_value_array` | VS 3, FS 1, VS 1 | `nir.rs:1011` |

Element types across A (per refusing VS/FS case, counted by the case name):
float 30 cases, `float_vec4` (struct member) 24 + 18, int 20, ivec4 20, bool 10
+ 20 (`_api_int`, `_api_float`), bvec4 10 + 20, the group names without a
type 22. **Mat arrays do not occur** in the refused set: only `mat4_mat2`
(info_query, two plain uniforms refused as "unused uniforms", 2 VS) and no
`mat*[N]` case; GLES2 has no mat arrays in uniform_api. Dynamic indexing
exists only in E (`uniform_array` with an int-uniform or loop-counter index).

Not all of A-F is an array defect. A shader with an `int`/`ivec4`/`bool`/`bvec4`
array also needs the integer or boolean uniform words (the `unitype` family,
`mali-int-uniforms.md`) and, for `int`/`bool` values, the `an integer or
boolean value` lowering (`intval`). A lowering for `uniarray` unblocks A-F only
where the element type is float/vec2/vec3/vec4; the int/bool arrays (about
half of A and B) stay refused behind `unitype` and `intval` and must be
re-measured after those. Estimate of what `uniarray` alone unblocks:
float/vec4 element groups of A (about 30 + 24 + 18 + 8 per slice
by case name, roughly 95 of 265), B's float/float_vec4 members (14), C (14 if
their other uniforms are float types), D (8), E (5), F (5) : about 140, the
rest waiting on `unitype`/`intval`.

## 3. UBO 0 layout Mesa 26.2.1 uses

`st_glsl_to_nir.cpp:714-715` runs `nir_lower_uniforms_to_ubo(nir,
PackedDriverUniformStorage, ...)`; with dword packing every `load_uniform` base
is multiplied by 4 (`nir_lower_uniforms_to_ubo.c:67-87`) and becomes
`load_ubo(0, byte_offset)`. The block is the linked default block in
front-end dword order (`select.rs:278` `UboLayout` mirrors it; `uniform_extent`
`nir_fs.rs:5866` gives each uniform's dword extent):

- **Element stride is a whole number of dwords, never packed in pairs**:
  float 1 dword (4 bytes, also for mediump: a 16-bit load at a 4-byte slot,
  `mali-uniform-arrays.md` section "What Mesa does" 2), vec2 2 dwords (8
  bytes), vec3 **3 dwords (12 bytes)** (GLSL packed storage, not the std140
  16), vec4 4 dwords (16 bytes), int/bool 1 dword, ivec4/bvec4 4. A mediump
  vec4 element is 4 dwords in the block (`mali-uniform-index.md`; `uniform_extent`
  reads it as `4 * n`), yet its `LD_PKA.i64` loads 8 bytes: mediump vec4
  elements are fp16 pairs held at a 8-byte stride (`mali-uniform-index.md`
  section 2). **vec3 and vec2 arrays have no reference**; the strides above are
  from the packed-storage rule and must be **pinned on the shim first**.
- A uniform's base is the next free dword; ints and floats before the array
  shift its byte base (`IADD_IMM #base` after the shift;
  `mali-uniform-arrays.md` "What differs", `nir_fs.rs:10190-10210`).
- Struct members and arrays in structs (F, parts of A/B) are flattened by the
  front end into one dword range per leaf; each `uniform_api` struct uniform
  is `name.member[i]` in the block's order. Whether the front end of ours
  (`ir::UniformDecl`) keeps one decl per leaf with `dword_off`, as the code
  at `nir.rs:1004` assumes, is to be read in the measure's output.

## 4. How bifrost loads it

- **Constant index (the bulk: A, B, C, D, F, and the `_static_loop` forms
  once unrolled)**: NIR `load_ubo(0, const)`; `bi_is_pushable_ubo`
  (`bi_opt_push_ubo.c:21-31`: constant offset and UBO handle, `offset & 3 ==
  0`) accepts it and `bi_opt_push_ubo` (`bifrost_compile.c:4447-4448`; pass
  `bi_opt_push_ubo.c:150`) moves the word into a FAU slot: the read is a
  `MOV`/ALU source `uK.wH`, no memory access, and the FAU table carries the
  array's words. This is how femtovg's `frag[14]` (vec4 array) and the 64-row
  arm of `mali-ubo-rows.md` already work (`select.rs:278`,
  `fau.rs:112` `push_ubo`). Our `LoadUniform` path
  (`nir_fs.rs:4875`) already pushes vec4 array elements; **a float, vec2 or
  vec3 array element at a constant index is the missing leaf** (needs one
  `Kind::Uniform` per dword at the layout's `dword_off + i * stride`; the
  vertex stage's `uniform_of` returns a `(decl, extent)` and a lane per
  dword, which is how `nir.rs:1013-1030` handles a plain vec4 already).
  The FAU holds 128 words in total; a bigger array
  (`float_static_loop_read`'s are 4 elements; `uniform_api`'s arrays are 2-5)
  stays under it.
- **Run-time index (E, 6 cases)**: `ishl idx, log2(stride bytes)`, `iadd
  base` if the array is not first, then `LD_PKA.i16/.i32/.i64/.i128` from UBO
  0 (`mali-uniform-arrays.md` section 4-5, `nir_fs.rs:10170-10206`). Not
  constant, so not pushable (`bi_opt_push_ubo.c:23-24`) and UBO 0 must be
  uploaded (`ubo_mask` bit 0, `frame.rs:485`). The int index word is pushed.
  A vec3 element is three 32-bit loads (stride 12 is not a power of two: Mesa
  emits `imul idx, 12` or `idx * 3 << 2`, an `IMUL`/`IADD` form, **pin on the
  shim**); a vec2 element is one `LD_PKA.i64` (stride 8, the same as a
  mediump vec4's, a different decl but the same load); dEQP's index (a
  `uniform int` or a counter of a loop that Mesa unrolls) decides whether the
  read stays a `load_ubo` at all.

## 5. Plan for `lower-uniarray` (one session)

Order matters: lower after `intval` and `unitype` land (they share `nir.rs:1011`
and the int words), or limit the item to the float/vec element groups and say
so in the commit.

1. **Constant index first (about 80 % of the cases).** Replace the `array_len
   > 1` branch of `nir.rs:1004-1012` and `nir_fs.rs:5866-5890` by an
   `(element type, stride dwords, count)` table: float 1/1, vec2 2, vec3 3,
   vec4 4 (existing); int/bool 1 (admitted with unitype). Drop the refusals at
   `nir_fs.rs:4875-4878` and `5900-5903` for a constant index, emitting the
   lanes as FAU pushes at `dword_off + i * stride` (as `uniform_lane`). Byte
   test against the references of `ref-uniarray` for float, vec2, vec3, vec4
   in the VS and the FS.
2. **`UboLayout` rows (`select.rs:278`, `MAX_UBO_ROWS = 32` at 301)**: an
   array past 32 rows already refuses (`mali-ubo-rows.md`); `uniform_api`'s
   arrays are far below, no change expected.
3. **Run-time index for vec2/vec3 (E).** Extend `shape` at `nir_fs.rs:4905`
   and `nir.rs:1090` to `comps` 2 and 3, `ubo_load`'s `words != 2` check to
   accept the `LD_PKA` width of the element and the vec3 stride; `ir::Op::
   LoadUniformIdx` already carries `comps`. These are 6 cases: do them last,
   and only if the vec2 reference shows the vec4 shape's `.i64` form.
4. **Frame upload** (`src/mali/frame.rs`, `backend.rs` `uniform_words`) is
   plan #373's; check that glUniform1fv/2fv/3fv/4fv of a consecutive array
   lands as consecutive dwords (`mali-uniform-arrays.md` "The upload"). No
   change expected, but the `uniform_api.value` cases render with arrays set
   by `glUniform*v` of count < array_len (`assign_partial`): the untouched tail
   must stay zero.
5. Test `tests/mali_deqp_uniarray.rs`: byte-compare each reference below with
   the online compiler (feature set `host-test,gpu-mali,shaderc-online,
   no-shader-table`); add the new shapes' rows to `tools/mali/testdata/
   cases.txt` and verdicts to `tools/mali/corpus-verdicts.txt`; if a glmark2
   verdict moves (the glmark2 vec4 arrays do not), move
   `tests/fixtures/mali_glmark2_verdicts.txt` in the same commit.

## 6. References to make (`ref-uniarray`, `tools/mali/testdata/deqp_uniarray_*`)

Each pair via the `pan_shc`/drm-shim method of `tools/mali/testdata/gm2_*`,
without `--uniform-values` (int uniforms are refused by `gl_ref.py`), from the
`measure` item's representative pair of each shape:

1. `deqp_uniarray_float_const` : `uniform float u[4]` read at constants 0..3,
   VS and FS, highp and mediump (the FS/VS split: four references).
2. `deqp_uniarray_vec2_const`, `deqp_uniarray_vec3_const`: same for vec2 and
   vec3 (the vec3 stride pins the packed block).
3. `deqp_uniarray_vec4_const`: vec4 array at constants in the VS and highp FS
   (the mediump FS exists: femtovg).
4. `deqp_uniarray_struct`: `struct { float a[2]; vec4 b[2]; }` (F,
   `array_in_struct.float_vec4`), pins the flattened offsets.
5. `deqp_uniarray_loop`: `static_loop_read` (a `for (i<3) sum += u[i]` Mesa
   unrolls to constants, or keeps; decides whether this is shape 1 again).
6. `deqp_uniarray_dyn_vec2`, `deqp_uniarray_dyn_vec3`,
   `deqp_uniarray_dyn_vec4_vs`: dynamic read at an int uniform, VS and FS.
7. If `unitype`/`intval` references contain `int u[N]`/`bvec4 u[N]` they are
   theirs; add only the array part here (`deqp_uniarray_int_const`).

## Open

- The exact generated GLSL of A-F is **not read**: this note takes shapes from
  the case names. The count of float-only cases, and whether the uniform_api
  shaders sum every element (and so pull in `intval` for `int`/`bool`), come
  from the `measure` item.
- vec3 array stride, vec2 array load width and the dynamic form of vec3 are
  **inferences from packed storage**, unpinned on the shim.
- A `_both` case's fragment shader is not counted; its fragment refusal may
  differ from the vertex's.
