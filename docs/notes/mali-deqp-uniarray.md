<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): dEQP-GLES2 uniform array refusals, study

Plan #377 item `study-uniarray` (phase 1, study only: no code, no reference);
section 7 and the settled points of sections 3, 4 and "Open" are the item
`ref-uniarray`'s references.
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
  **Pinned by `ref-uniarray` (section 7): the strides above hold only for a
  struct member.** A plain `mediump` array (every dEQP array outside a struct
  is `mediump`) is held as **fp16** in UBO 0 and read with 16-bit
  `load_ubo`s, each element rounded up to whole dwords: float 4 bytes (one
  fp16 in a dword), vec2 4 bytes (one fp16 pair), vec3 8 bytes, vec4 8 bytes
  (`deqp_uniarray_*_const_*`: `16` at 0/4/8/12, `32x4` at 0 (range 16),
  `16x3` at 0/8/16/24, `32x4` at 0 and 16 (range 32)), in the VS as in the FS.
  A struct's members stay **fp32**, one dword per component, packed with no
  vec4 alignment (`struct_member`: `s.a` 0, `s.b[0..2]` 4-12, `s.c` 16, range
  20; `struct`: m0 0, m1 4-16, m2 20-28, m3 32-76, range 80, `32x4` loads).
  `mediump int` index uniforms stay 32-bit words.
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
  **Pinned (section 7), and both inferences above are wrong for dEQP's
  mediump arrays:** the stride is a power of two in every case (fp16
  storage, section 3), so there is no `IMUL`: each read is `LSHIFT_OR.i32
  idx, 2` (vec2) or `3` (vec3, vec4), the shift count a byte lane (`.b22`,
  `.b33`) of one `MOV.i32 r0, 0x3020100` at the program's start, then `IADD_IMM #0x10` (the array follows the
  four index ints), then `LD_PKA.i32` (vec2), `LD_PKA.i48` (vec3) or
  `LD_PKA.i64` (vec4), the same in the VS and the FS (`dyn_*`). A vec2 element
  is an `.i32`, not an `.i64`.

## 5. Plan for `lower-uniarray` (one session)

Order matters: lower after `intval` and `unitype` land (they share `nir.rs:1011`
and the int words), or limit the item to the float/vec element groups and say
so in the commit.

1. **Constant index first (about 80 % of the cases).** Replace the `array_len
   > 1` branch of `nir.rs:1004-1012` and `nir_fs.rs:5866-5890` by an
   `(element type, stride dwords, count)` table: float 1/1, vec2 2, vec3 3,
   vec4 4 (existing); int/bool 1 (admitted with unitype). (These are the fp32
   struct-member strides; a plain mediump array is fp16, see section 7.) Drop the refusals at
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
with `--uniform-values` where `gl_ref.py` sets the uniforms, from the
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

## 7. References (`ref-uniarray`, made 2026-11-16)

Each reference is `tools/mali/testdata/deqp_uniarray_<name>.{vert,frag,
expect.json}`; the shader files are dEQP's text (Khronos, Apache-2.0: each
has a `.license` sidecar naming the case and the source). Every
`expect.json` was made with the method of `gm2_*`
(`README-pan_shc.md`, "The shim driver"), with `--uniform-values`. gl_ref.py
gives the `int` index uniforms of `dyn_*` their values; an array
declarator's location lookup is -1, so an array's words stay 0. It refuses
a struct uniform, so `struct` and `struct_member_*` are drawn without the
flag:

```sh
tools/mali/drm_shim_ref.sh run <shim> tools/mali/testdata/deqp_uniarray_<name>.vert \
    tools/mali/testdata/deqp_uniarray_<name>.frag <out>/deqp_uniarray_<name> --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_uniarray_<name>.trace --draw --attributes \
    > tools/mali/testdata/deqp_uniarray_<name>.expect.json
```

`<shim>` is Mesa 26.2.1 built with `drm_shim_ref.sh build`'s meson options
and installed into `<shim>/inst`. It was configured from the tree that
`build_pan_shc.sh` extracted from the 26.2.1 tarball, which carries only
`mesa-pan_shc.patch` (one `subdir('pan_shc')` line in
`src/panfrost/meson.build`, no compiler or driver code). The shim's bytes were
checked first: `gm2_arithconst_1` made again this way is byte-identical to
the committed `gm2_arithconst_1.expect.json`. All 23 references were made a
second time and came out identical. `deqp_ref.sh` (the `ref-tool` item) was not
on the tip, and the tip had no `deqp_shapes/` pairs.

Sources. "rebuilt" means the text was re-made from VK-GL-CTS `5bb4c7e8`
(main, 2026-11-16), substituting the template the way `StringTemplate` does:

- **IDX**: `modules/gles2/functional/es2fShaderIndexingTests.cpp:410-521`
  (`createUniformArrayCase`, `PRECISION` mediump at :493, `ARRAY_LEN` 4), with
  the case list at :1180-1202.
- **STR**: `modules/gles2/functional/es2fShaderStructTests.cpp:163-209`
  (`createStructCase`, the default VS/FS and the specialisation), with
  `array_member` at :1346-1373 and `LineStream` (`glsShaderRenderCase.hpp:71-79`,
  one `\n` per line).
- **QPA**: verbatim from the `measure` item's
  `tools/mali/testdata/deqp_shapes/<pair>` at its commit `3b926d42`, which was
  not yet on the tip. That text is the .qpa of a dEQP run (VK-GL-CTS `93bca01`).
  `uniform_api`'s values come from a seeded random generator, so they are taken
  from the log rather than rebuilt.

| reference | dEQP-GLES2.functional. case | source | today's first refusal (37305608) | what it pins for `lower-uniarray` |
|---|---|---|---|---|
| `float_const_vs`, `float_const_fs` | `shaders.indexing.uniform_array.float_static_read_{vertex,fragment}` | IDX, rebuilt | `a uniform array` (both) | fp16 element, 4-byte stride: four 16-bit FAU halves (`u2.w0.h00` ...), `FADD.v2f16`; FS 9, VS varying 23 instructions |
| `vec2_const_vs`, `vec2_const_fs` | `vec2_static_read_*` | IDX, rebuilt | VS `a uniform array`; FS `a mediump fragment shader uniform other than ...` | one fp16 pair per element, one 32-bit FAU word each |
| `vec3_const_vs`, `vec3_const_fs` | `vec3_static_read_*` | IDX, rebuilt | VS `a uniform array`; FS `a mediump fragment shader uniform ...` | 8-byte stride: an xy word and a z half per element; the FS's colour w is the promoted fp16 pair `0x3c003c00` (FAU word 12) |
| `vec4_const_vs`, `vec4_const_fs` | `vec4_static_read_*` | IDX, rebuilt | VS `a vertex shader uniform that is not highp`; FS `a fragment shader operand of a form ... (a uniform first)` | two fp16 pairs per element, 8-byte stride; FS `ATEST` takes alpha from `r1.h1` |
| `loop_{float,vec2,vec3,vec4}_fs`, `loop_float_vs` | `{float,vec2,vec3,vec4}_static_loop_read_fragment`, `float_static_loop_read_vertex` | IDX, rebuilt | FS float, vec4 `a uniform read at a run-time index other than ...`; vec2, vec3 `a uniform array`; VS `a loop in a vertex shader` | Mesa unrolls the loop: each is **byte-identical** to its `*_const_*` (`pan_ref_diff.py diff-ref`: 0 differ, the FAU words too) |
| `dyn_vec2_vs`, `dyn_vec2_fs`, `dyn_vec3_vs`, `dyn_vec3_fs`, `dyn_vec4_vs`, `dyn_vec4_fs` | `{vec2,vec3,vec4}_dynamic_read_{vertex,fragment}` | IDX, rebuilt | vec2, vec3 `a uniform array`; `vec4_dynamic_read_vertex` `a vertex shader uniform that is not highp`; `vec4_dynamic_read_fragment` `a uniform read at a run-time index other than ...` (read four times, the array not first in the block); dEQP reports all six as NotSupported | the index ints at dwords 0-3 (one `32x4` push), the array at byte 16: `LSHIFT_OR` by 2 or 3, `IADD_IMM #0x10`, `LD_PKA.i32` / `.i48` / `.i64` (section 4) |
| `struct_member_vs`, `struct_member_fs` | `shaders.struct.uniform.array_member_{vertex,fragment}` | STR, rebuilt | `a uniform array` (both) | fp32 struct leaves `s.a`, `s.b[3]`, `s.c` at dwords 0, 1-3, 4; `s.c` is `S32_TO_F32` of an int word (`intval`/`unitype`) |
| `struct` | `uniform_api.value.assigned.basic_array_assign_full.array_in_struct.float_vec4_both` | QPA (`uniarray_array_in_struct`) | VS `a uniform array` | the flattened fp32 struct `m0, m1, m2[3], m3[3]` = 20 dwords, `32x4` loads at 0-64, read in both stages; the compare functions (`abs`, `?:`) are other families' |
| `api_float` | `uniform_api.value.assigned.basic_array_assign_full.basic_array.float_both` | QPA (`uniarray_float_array`) | VS `a uniform array` | the bulk shape A: `mediump float u_var[3]`, fp16 at 0/4/8 in both stages; also needs the `compare_float` arithmetic |

Dropped from section 6:

- **highp arrays** (1, 3): no refused dEQP case declares one. Every
  `uniform_api` collection and `random` type is mediump unless it is a bool
  (`es2fUniformApiTests.cpp:215-216, 348, 356, 366-367`), and `indexing` uses
  mediump (:493). A highp array would be a reduction, not a dEQP shape, so
  there is no highp reference.
- **`deqp_uniarray_int_const`** (7): the int and bool arrays are `uniform_api`
  shapes whose text is the .qpa's (`deqp_shapes/uniarray_{int,bool}_array`).
  They need the int words and values first (`unitype`, `intval`), so they are
  left to those references.

Added beyond section 6: `dyn_vec4_fs` (the third `UniformIndexed` case of
section 1), `struct_member` (F, whose text can be rebuilt) and `api_float`
(A's float group).

For `lower-uniarray`, this changes section 5 step 1. The `(element type,
stride)` table is **fp16 for a plain mediump array**: float 1 dword, vec2 1,
vec3 2, vec4 2, read as 16-bit halves. A struct member's array is fp32 at
1/2/3/4 dwords. The `UboLayout` of both, and the frame's upload of a mediump
array as fp16 (`src/mali/`, plan #373), must follow the same rule. In step 3,
`ubo_load`'s 8-byte element (`words == 2`) becomes a 4-byte `.i32` (vec2), a
6-byte `.i48` at an 8-byte stride (vec3) and the 8-byte `.i64` (vec4). Each
index is shifted, and the array is not first in the block (`#0x10`).

## 8. Lowering (`lower-uniarray` spike, on `af043255`)

Byte-equal (`tests/mali_deqp_uniarray.rs`, `assert_matches`: the
references are drawn with `--uniform-values`, section 7): `float_const_{vs,fs}`,
`vec2_const_{vs,fs}`, `vec3_const_{vs,fs}`, `vec4_const_{vs,fs}`,
`loop_{float,vec2,vec3,vec4}_fs`, `dyn_vec{2,3,4}_fs` (15 of 23).

- **One element rule.** `nir::array_element` gives a float, vec2, vec3 or
  vec4 array's element as (front-end stride, components): (1, 1), (2, 2),
  (4, 3), (4, 4); the front end pads a vec3 to 4 dwords (reflect.rs
  `type_layout`), Mesa's block does not. `nir::uniform_of` (and with it
  `nir_fs::uniform_extent`) admits every such array; `nir::element_load` is
  the element's load (its first dword, its components) for the vertex walk,
  the highp fragment walk (`uniform_load`) and the mediump walk's
  `uniform_load` bookkeeping. Int, bool and matrix arrays still refuse
  `UniformArray`.
- **Layout.** `UboLayout::push_uniform` lays an array whose element does not
  follow the last one's word as one row an element, each split (each word its
  own load): an fp16 array (any of the four) and an fp32 vec3 array (a struct
  member's). An fp32 float, vec2 or vec4 array stays one split row, as
  before (the same words, and `MAX_UBO_ROWS` is not spent). `fs16` admits
  vec2 and vec3 arrays (one fp16 row an element at the front-end stride),
  and a struct member's array as fp32 rows. So a mediump array is fp16 at a
  word (float, vec2) or two (vec3, vec4) an element in the vertex shader as
  in the fragment shader, as section 7 pinned.
- **Vertex shader.** `nir::fp16_uniform` takes a mediump float, vec2, vec3
  or vec4 array; the walk loads each 32-bit word of an fp16 element as its
  own 16-bit uniform node of at most two lanes (Mesa splits the element's
  load a word a piece), so the existing fp16 pair forms (`vs_pair16`'s
  `.h01` of a node's lanes 0 and 1) cover a vec4's zw. `fadd(a, 0.0)` over
  every lane is `a` in the fp16 paths (`vs_identity16`; GLSL IR already
  folds `res = vec4(0.0); res += u[0]`). A varying stored as `vec4(res,
  0.0, 1.0)` where `res` is an fp16 sum of fp16 uniforms alone keeps its
  constant lanes, flat (`uniform_sum16`, `kept_uniform`): the `f2f32` of
  the mediump constructor hides them from the link, which then promotes
  the whole slot to flat (`.v4.src_flat32`, `IADD_IMM.i32 r4, 0x0, #0x0`).
- **Fragment shader.** The constant-index refusal of a float array is
  gone. `prod(Op, AddA)` (the running sum first) is admitted, and a sum of
  fp16 uniform lanes beside 0.0 is a `U16_TO_U32` word (`uniform_sum`).
  A `LoadUniformIdx` whose index the walk folded to a constant (an unrolled
  loop's counter) is the constant-index read, clamped as the IR clamps it.
  An unrolled loop's nodes keep their pass (`Fs::pass`), which `Fs::key`
  adds to the loop's shared key: pass after pass, as Mesa's unrolled body
  (`loop_vec4_fs`: each pass's xy and zw `FADD.v2f16` before the next's).
- **Run-time index, fragment shader.** A vec2, vec3 or vec4 array read more
  than once (every read of one element width) takes case108's float path:
  per read `LSHIFT_OR` by 2 (float, vec2) or 3 (vec3, vec4), `IADD_IMM`
  of the base, the load sunk to its first reader, now `LD_PKA.i32`, `.i48`
  (`Op::LdPka { words: 2, half: true }`, pack.rs `LdPkaI48`) or `.i64`.
  `Leaf::UboWord(k, h)` is word `h` of read `k`; a vec3's z is a lane
  alone (`.h00`) and its word with a constant `MKVEC.v2i16`.
  `prod(Ubo, AddB)` is admitted. The single vec4 read at the block's start
  (uniidx_i) keeps its own path.

Left: `dyn_vec{2,3,4}_vs` (the vertex walk has no fp16 `LD_PKA` read; its
indexed lanes would also need the flat link rule of `kept_uniform`),
`loop_float_vs` (a vertex loop, `lower-loops-1`), `struct_member_{vs,fs}`
(the int member `s.c` as a value, `intval`), `struct` and `api_float`
(`compare_float`'s `abs` and `?:`, Mesa's FMAX of the abs values and one
FCMP; `constop`).

## 9. What landed (lower-constop, at 3497799a)

Section 8's spike is on the tip as `74dabf88`. Before this item, 15 of 23
references were byte-equal and 8 were ignored. This item adds one.

- **`loop_float_vs`, byte-equal and un-ignored.** Plan #367's
  lower-int-loops unrolls the vertex loop (`VsLoop::Unroll`). The
  counter then reaches `u_arr[i]`'s `LoadUniformIdx` as a constant, and
  `nir::uniform_idx` refused that, because it takes only an int
  uniform's index. The vertex walk (`nir.rs`, before its `match *op`) now
  does what the fragment walk does (`nir_fs.rs`, `walk_nodes`). A
  `LoadUniformIdx` whose index lane is a `Val::Const` becomes the
  constant-index `LoadUniform` at `base_dw + k * stride_dw`, with `k`
  clamped to the array as the IR reads it. That is Mesa's unrolling,
  which makes the deref constant before `nir_lower_uniforms_to_ubo`. The
  bytes equal `float_const_vs`'s, as the references are equal (section 7).
  `cases.txt` gains its row: pan_shc's fp32 array sum differs (`mediump`).

Left (pins unchanged):

| Test | Stops at | What it needs |
|---|---|---|
| `dyn_vec2_vs`, `dyn_vec3_vs` | `UniformIndexed` (`nir::uniform_idx`: float and vec4 shapes only) | a vertex fp16 element read at a run-time index: `LSHIFT_OR.i32` by the byte constant `0x3020100`'s `.b22` (2 or 3), `IADD_IMM #0x10`, `LD_PKA.i32`/`.i48`, the `FADD.v2f16` sums, then per lane `F16_TO_F32` and `FMA.f32 r, r, 0x0, r`, and the varying made flat (`src_flat32`, section 8's `kept_uniform` rule) |
| `dyn_vec4_vs` | `UniformPrecision` (unitype) | the same with `LD_PKA.i64` |
| `struct_member_vs`, `struct_member_fs` | `UniformType` / `FsUniform` | the int member `s.c` as a value (intval) |
| `struct`, `api_float` | `ColourForm` / `FsUniform` | `compare_float`'s `abs` and `?:` (Mesa's `FMAX` of the abs values and one `FCMP`) |

No glmark2 verdict moves (`mali_glmark2_corpus`, fixture byte-identical).
`tests/mali_deqp_unitype.rs`'s `vsmat3_light` still stops at the mediump
mat3-array uniform rule (`a vertex shader uniform that is not highp`),
which is unitype's, so its pin stays.

## 10. What landed (spike-uniarray-rest2, at 87388100)

At the start, 7 of 24 references were ignored. This item makes four
byte-equal and un-ignores them: `dyn_vec2_vs`, `dyn_vec3_vs`, `dyn_vec4_vs`
and `struct_member_vs`. Their pins are gone.

- **A vertex fp16 element at a run-time index** (`nir::uniform_idx16`). A
  `LoadUniformIdx` of a mediump vec2, vec3 or vec4 array (`fp16_uniform`)
  at an int uniform's index becomes a 16-bit `Kind::UniformIdx` node. Its
  lanes are the element's, two to a register, and each lane gets its own
  `f2f32` as a constant index's element does. The node is a load of
  16-bit words like the fp16 vertex look-up (`nir::vs_is_word16`, which
  `vs_vec16` and `vs_tex16_word` now ask), so a pair of its lanes is a
  register read whole (`FADD.v2f16 r1, r4^, r2^`) and a vec3's z is a lane
  alone (`r5^.h00`). `vs_is_uniform16` and `uniform_sum16` take it as an
  fp16 uniform's lane, so `vs_alu16u` makes Mesa's `FADD.v2f16` sums.
  `select.rs` `idx16_offset` emits `LSHIFT_OR.i32` by 2 (vec2) or 3 (vec3,
  vec4) and the `IADD_IMM.i32` of the base. `idx16_fetch` emits
  `LD_PKA.i32`, `.i48` or `.i64`, sunk to the first reader like the vec4
  path's `pending` load. `UboLayout::vs` lays the array as fp16 rows, as at
  a constant index (section 8).
- **The mediump int index.** The four `ui_*` ints are mediump. Mesa keeps an
  int 32-bit (`caps->int16` is false), so an int read only as such an index
  (`nir::index16_only`) passes the vertex precision rule (mod.rs).
- **The flat varying.** The sums' lanes read no attribute. Mesa's link makes
  them flat with the constants `vec4(res, 0.0, 1.0)` keeps
  (`.v4.src_flat32`, `IADD_IMM.i32 r4, 0x0, #0x0`). `nir::widened_sum16`
  counts such a lane as flat beside the `kept_flat` constants, where it
  counted as interpolated and met the "computed from no attribute" refusal.
- **`struct_member_vs`.** The int member `s.c` read only through `float(s.c)`
  (`nir::int_float_only`) is no longer `UniformType`: `S32_TO_F32 r2,
  u7.w0` of its pushed word.

No glmark2 verdict moves (`mali_glmark2_corpus` on a PowerVR
`tools/hostharness/glmark2/run.sh` corpus of 69 shaders;
`MALI_GLMARK2_WRITE=1` leaves the fixture byte-identical).
`tests/mali_deqp_unif2.rs` `arr_struct_array` moved its pin to the
vertex precision rule (`docs/notes/mali-deqp-unitype.md`, last section).

Left (pins unchanged):

| Test | Stops at | What it needs |
|---|---|---|
| `struct_member_fs` | `FsUniform` | the mediump fragment walk's leaf for the int member converted, `S32_TO_F32 r0, u3.w1` then `FADD.f32 r0.h0, r0^, 0x0.neg`, paired by `MKVEC.v2i16` with the narrowed fp32 member `b[1]` (`FADD.f32 r1.h0, u3.w0, 0x0.neg`); the other two members `V2F32_TO_V2F16 r0, u2.w0, u2.w1` |
| `struct`, `api_float` | `ColourForm` / `FsUniform` | `compare_float`'s `abs` and `?:` (Mesa's `FMAX` of the abs values and one `FCMP`), constop's fragment compare tree |

## Open

- ~~The exact generated GLSL of A-F is not read~~. The `indexing` and
  `struct.uniform` text is rebuilt and the `uniform_api` text is the .qpa's
  (section 7). The case counts per shape are still the `measure` item's.
- ~~vec3 array stride, vec2 array load width and the dynamic form of vec3~~
  are pinned (sections 3, 4, 7).
- ~~Whether `static_loop_read` unrolls~~: it does, and its bytes equal the
  static read's (section 7).
- The FAU slot that holds each UBO word is fixed by the instruction words, but
  no value pins it, because `gl_ref.py --uniform-values` does not set an array
  (it looks up the location of `u_arr[4]`, which is -1). A value-pinned
  layout needs that tool to set arrays element by element.
- A `_both` case's fragment shader is not counted; its fragment refusal may
  differ from the vertex's.
