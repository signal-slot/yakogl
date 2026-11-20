<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: the uniform refusals, shapes and Mesa references (unif2)

Plan #387, item unif-ref: a study and references, no compiler change. Method
as `docs/notes/mali-deqp-families.md` sections 1-3, 6 and 7. The earlier
uniform notes (`mali-deqp-unitype.md`, `mali-deqp-uniarray.md`,
`mali-uniform-types.md`, `mali-uniform-arrays.md`, `mali-uniform-index.md`,
`mali-int-uniforms.md`) stay the record of what each form is; this one is the
map of what is left at the tip.

## 1. What was measured

**Counts at the tip `314fdc9c`** (the integration tip of plan #387), HOST-ONLY:
the compile and link outcomes of the Mali library at that tip, no stream run.
Command, with the VK-GL-CTS checkout at `93bca01861b0e3ef3c387027a9791e6d065f900c`
(the project's pin, with the yakogl target files of
`tools/cts/cts-yakogl-target.patch` added):

```sh
tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS 93bca01 tree> --out <out>   # all four 2026-11-15 slices, no --filter
```

The slices are `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608` (the
default): 10,597 refused cases, all began on the host; 6,132 still have a host
compile refusal at the tip. The counts below are the cases of `measure.tsv`
whose host first refusal belongs to family `unitype` or `uniarray` (the first
refusal per case, Fail + NotSupported by the board's status), 1,044 cases in
all: unitype 841 (640 F + 201 NS), uniarray 203 (197 F + 6 NS). The other 5,088
cases with a host refusal belong to other families; the 4,465 board refusals
that no longer refuse on the host compile (the script's "7,243 not
reproduced" also counts the cases whose first refusal moved to another text).

By refusal text (first refusal per case, tip):

| Text | Cases |
|---|---:|
| `a uniform other than a float, vec2, vec3, vec4 or mat4` | 462 |
| `a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int ...` | 280 |
| `a uniform array` | 193 |
| `a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than ...` | 99 |
| `a uniform read at a run-time index other than a float array's element or a mediump fragment ...` | 10 |
| **all uniform texts** | **1,044** |

By stage: vertex 764 (unitype 561 + uniarray 203), fragment 280.

**Was at `abcc9013`** (the run at `37305608`, which the first version of this
note quoted for lack of a CTS checkout): 1,092 cases (820 unitype, 272
uniarray; 991 Fail, 101 NotSupported). The old figure of each shape stays in
its row as "was". The shape *names* are the measure's `deqp_shapes/index.tsv`
names; the measure's shape classifier has moved since `abcc9013` (a vertex
int uniform is classified as `vs_int_uniform` before `vs_mediump_basic`
and so on), so a row's old and new figures compare the shape by name, not case
by case; `vs_mat_uniform`, `vs_bool_uniform`, `fs_highp_uniform` and the float
array no longer appear as a first refusal at all.

Each shape's *current outcome* is what `tests/mali_deqp_unif2.rs` pins on its
reference (section 2's last column).

## 2. Shapes, counts and the reference that covers each

Fail + NotSupported = cases. "Existing" is the `deqp_unitype_*` or
`deqp_uniarray_*` reference whose vertex and fragment shader are byte-identical
to the shape's (compared by hash), with its pin in `tests/mali_deqp_unitype.rs`
or `tests/mali_deqp_uniarray.rs` at the tip. "New" is a `deqp_unif2_<shape>`
reference of this item; its pin is in `tests/mali_deqp_unif2.rs`.

| Shape (stage, type, precision, index) | Cases at the tip (Fail + NotSupported by the board); was at abcc9013 | dEQP groups (example) | Reference | Pin at the tip |
|---|---:|---|---|---|
| VS mat2/mat3 uniform, highp or mediump, dynamic matrix subscript | 0 (compiles: no refusal); was 138 | `shaders.matrix.*_vertex` | New `deqp_unif2_vs_mat_uniform` | LOWERED (byte-equal) |
| VS struct uniform (nested, array member, int/bool members), mediump | 116 (106 F + 10 NS); was 134 | `uniform_api.value.assigned.*.array_in_struct` | New `deqp_unif2_vs_struct_uniform` | REFUSED: other than float..mat4 |
| FS mat2 uniform, highp | 79 (79 F); was 119 | `shaders.matrix.*_fragment` | New `deqp_unif2_fs_mat_uniform` | REFUSED: FragColor lane |
| VS mediump int uniform (`tmp[ui_zero]`, `i < ui_two`) | 346 (208 F + 138 NS); was 117 | `shaders.indexing.matrix_subscript.*_vertex` | New `deqp_unif2_vs_int_uniform` | REFUSED: other than float..mat4 |
| VS bool/bvec uniform, run-time compare chain | 0 (no first refusal); was 67 | `uniform_api.value.assigned.by_pointer.get_uniform.basic` | New `deqp_unif2_vs_bool_uniform` | REFUSED: arithmetic other than +,-,* |
| FS struct uniform | 75 (75 F); was 55 | `uniform_api...array_in_struct.*_fragment` | New `deqp_unif2_fs_struct_uniform` | REFUSED: mediump FS uniform |
| VS mediump struct (float/vec4 members) | 42 (42 F); was 53 | `uniform_api...by_pointer.get_uniform.basic_struct` | New `deqp_unif2_vs_mediump_struct` | REFUSED: FragColor lane |
| FS mediump int array / int uniform | 35 (35 F); was 52 | `uniform_api...basic_array_assign_full.basic_array.int_fragment` | New `deqp_unif2_fs_int_uniform` | REFUSED: mediump FS uniform |
| FS bool array | 41 (41 F); was 33 | `...basic_array.bool_fragment` | New `deqp_unif2_fs_bool_uniform` | REFUSED: mediump FS uniform |
| VS mediump vec4 array (compare against api values) | 2 (2 NS); was 25 | `...basic_array_assign_full.basic_array.vec4_both` | New `deqp_unif2_vs_mediump_array` | REFUSED: FragColor lane |
| FS other (two mediump floats beside a computed lane) | 50 (50 F); was 7 | `shaders.random.basic_expression.combined.*` | New `deqp_unif2_fs_other` | REFUSED: mediump FS uniform |
| array: bool array, `uniform bool u_var[3]` | 90 (90 F); was 95 | `...basic_array_assign_full.basic_array.bool_both` | New `deqp_unif2_arr_bool_array` | REFUSED: a uniform array |
| array: mediump int array | 47 (47 F); was 46 | `...basic_array.int_both` | New `deqp_unif2_arr_int_array` | REFUSED: a uniform array |
| array: struct array `uniform S s[2]` | 1 (1 F); was 2 | `shaders.struct.uniform.nested_struct_array_vertex` | New `deqp_unif2_arr_struct_array` | REFUSED: other than float..mat4 |
| array: mat3 array + ivec4 | 21 (21 F); was 1 | `uniform_api.random.10` | New `deqp_unif2_arr_mat_array` | REFUSED: a uniform array |
| array in struct (`float_vec4_both`) | 34 (34 F); was 77 | `...array_in_struct.float_vec4_both` | `deqp_uniarray_struct` | REFUSED: FragColor lane |
| float array | 0 (no first refusal); was 36 | `...basic_array.float_both` | `deqp_uniarray_api_float` | REFUSED: mediump FS uniform (FsUniform) |
| vec array, dynamic read | run-time index, 10 (4 F + 6 NS), the three rows together; was 12 | `shaders.indexing.uniform_array.vec2_dynamic_read_fragment` | `deqp_uniarray_dyn_vec2_fs` | LOWERED (assert_matches) |
| run-time index in a loop | (in the row above); was 3 | `shaders.indexing.uniform_array.float_static_loop_read_fragment` | `deqp_uniarray_loop_float_fs` | LOWERED |
| FS vec array, constant index | (in the row above); was 2 | `shaders.indexing.uniform_array.vec2_static_read_fragment` | `deqp_uniarray_vec2_const_fs` | LOWERED |
| FS highp uniform (`clear_color`) | 0 (no first refusal); was 17 | `fragment_ops.scissor.clear_color` | `deqp_unitype_fshighp_color` | MATCHED |
| VS mediump basic (`depth_range`) | 55 (4 F + 51 NS); was 1 | `builtin_variable.depth_range_vertex` | `deqp_unitype_depthrange_vs` | MATCHED |

The shapes sum to 841 (unitype) + 203 (uniarray) = 1,044 at the tip (was
820 + 272 = 1,092 at `abcc9013`). The shape rows of the table above that stand
for one measure shape: the three run-time index rows (`vec array, dynamic
read`, `run-time index in a loop`, `FS vec array, constant index`) are the
measure's one shape `runtime_index` (10: 4 F + 6 NS); the `array in struct`
row is `array_in_struct` (34); `mat3 array + ivec4` is `mat_array` (21). Fifteen
shapes have no existing reference and got one here; the other seven are covered
by the references named (their sources are byte-identical, so no second copy
was drawn). The existing references of the families that are not byte-identical
to a shape (`deqp_unitype_vsapi_mat2`, `vsbvec3`, `fsivec4`, `fsmulti`,
`fsmat3_mediump`, the `deqp_uniarray_*_vs` run-time index ones, `struct_member_*`)
are the sub-shapes the unitype and uniarray notes already pin; their pins
(DIFFERS: `vsmat3_lowp`, `vsapi_mat2`, `vsapi_struct`, `vsstruct_add`,
`depthrange_fs`, `vsbvec3`; REFUSED: `vsmat3_light`, `fsmat3_mediump`, `fsivec4`,
`fsmulti`, `struct_member_vs`/`_fs`, `dyn_vec{2,3,4}_vs`, `loop_float_vs`) stay
as in their tests.

By stage and shape group at the tip, over the 15 new references: vertex
shader int/struct (`vs_int_uniform` 346, `vs_struct_uniform` 116,
`arr_struct_array` 1): 463 cases; the fragment shader's mediump uniform
(`fs_bool` 41, `fs_int` 35, `fs_other` 50, `fs_struct` 75): 201; a uniform
array (`arr_bool` 90, `arr_int` 47, `arr_mat` 21): 158; the FragColor lane after
a uniform that now loads (`fs_mat` 79, `vs_mediump_array` 2,
`vs_mediump_struct` 42): 123; the bvec chain (`vs_bool`): 0; compiled
(`vs_mat_uniform`): 0 refusals. Not among the 15: `vs_mediump_basic` 55,
`array_in_struct` 34, `runtime_index` 10. (Was at `abcc9013`: 253, 147, 142,
197, 67, 138.)

## 3. Mesa's layout and loads, per new reference

Drawn with `--uniform-values` (an int as its integer, a bool true where the
value is even, floats `16*(k+1)+c`) except the struct shapes (section 4). The
words of the FAU and the programs are in each `.expect.json`; the load forms
below are read from Mesa's shaderdb disassembly of each draw.

| Reference | Mesa's loads and ops of note |
|---|---|
| `vs_mat_uniform` | mat2 columns as fp32 FAU words; the program is `FMA.f32` of attribute and FAU words; compiles byte-equal |
| `vs_int_uniform` | the three mediump ints are FAU words read through `F16_TO_F32`/`FMA.v2f16`; the loop bound is `ICMP_OR.s32.lt.m1` against a FAU word (a uniform-bound loop is kept, not unrolled) |
| `vs_bool_uniform` | `bvec2` words (`ffffffff` true) fed to `LSHIFT_OR.i32` and `CSEL.u32.eq`: one mask per compare, then a select |
| `vs_mediump_array`, `vs_mediump_struct` | each mediump 32-bit word of an element its own fp16 FAU load; `S16_TO_S32` into the colour's `CSEL.u32.eq` |
| `vs_struct_uniform` | the struct is flattened to rows in member order, an array member one row an element (fp32 for int/ivec, mediump floats as fp16 words); `ICMP_AND/OR.u32.eq.m1` chains of the int compares |
| `fs_mat_uniform` | highp mat2 columns as FAU words in the fragment program |
| `fs_int_uniform`, `fs_bool_uniform` | an int or bool array element as one FAU word; `ICMP_AND/OR.u32.eq.m1` of the lanes (int), `CSEL`/no ALU (bool) |
| `fs_other` | two mediump floats read as fp16 lane pairs in `FMA.v2f16` |
| `fs_struct_uniform` | struct rows as in the vertex case, `LSHIFT_OR.i32` of the masks |
| `arr_bool_array`, `arr_int_array` | an element a word, constant index folded to that word; the bool as its own mask (c7c377d3) |
| `arr_mat_array` | `mat3 u_var0[5]` a column a row, fp16; `ivec4 u_var1` compared lane by lane |
| `arr_struct_array` | `s[2]`: elements laid back to back, each member its row, `FMA.v2f16` with `F16_TO_F32` |

The table is read from the op names of the shaderdb output (not a word-by-word
disassembly): it names the forms to look for, and the byte-compare of a lowering
is the authority. The record of each draw (work registers 32 for every region, the FAU length
per region) is in the reference's `expect.json`.

## 4. Commands (the references)

```sh
tools/mali/drm_shim_ref.sh build <mesa-26.2.1.tar.xz> <work>      # once; installs <work>/inst
tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/deqp_unif2_<shape>.vert \
    tools/mali/testdata/deqp_unif2_<shape>.frag <out>/deqp_unif2_<shape> --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract <out>/deqp_unif2_<shape>.trace --draw --attributes \
    > tools/mali/testdata/deqp_unif2_<shape>.expect.json
```

The sources are the `tools/mali/testdata/deqp_shapes/<family>_<shape>.{vert,frag}`
texts, byte for byte, each with its `.license` sidecar (Khronos, the dEQP case).
**The four struct shapes** (`arr_struct_array`, `fs_struct_uniform`,
`vs_mediump_struct`, `vs_struct_uniform`) were drawn *without*
`--uniform-values`: `gl_ref.py` stops with "does not set a structType uniform".
Their FAU words are therefore Mesa's with every uniform 0, and no test compares
them (they are refused); the lowering item for structs must first teach
`gl_ref.py` to set a struct's members (`glGetUniformLocation("u_var.m0")`,
`"u_var.m2[1]"`) and redraw those four.

## 5. What spike 74dabf88 covers

The spike (a read-only patch on `af043255`, `docs/notes/mali-deqp-uniarray.md`
section 8) lowers float, vec2, vec3 and vec4 arrays, in both stages: the
element rule (`nir::array_element`, `element_load`), the layout
(`UboLayout::push_uniform`: an fp16 array, or an fp32 vec3 array, one row an
element), the vertex fp16 words, the fragment constant and run-time index
(`LD_PKA.i32/.i48/.i64`), the unrolled loop's passes. It leaves int, bool and
matrix arrays (`UniformArray`), the vertex `dyn_vec{2,3,4}_vs` forms, a vertex
run-time loop, struct members as values, and `compare_float`'s `abs` and `?:`.
Against this item's shapes: it covers none of the 15 new references (their
arrays are int, bool, mat3 or struct, and the vertex int, struct and bool
shapes are not arrays). It covers, of section 2's existing references,
`vec2_const_fs`, `dyn_vec2_fs` and `loop_float_fs` (already LOWERED at the
tip) and moves nothing of `api_float`, `uniarray_struct` and the struct
members, which stay on constop and intval.

## 6. Lowering map, in one-session steps ranked by cases

Files: `src/shaderc/online/mali/fau.rs` (the FAU and the rows),
`nir.rs` (vertex walk), `nir_fs.rs` (fragment walk), `select.rs`/`pack.rs` for
the ops.

1. **Struct uniforms, both stages** (VS 116 + 42, FS 75, array 1 = 234 cases at the tip, was 244 at abcc9013;
   `vs_struct_uniform`, `vs_mediump_struct`, `fs_struct_uniform`,
   `arr_struct_array`). First extend `gl_ref.py --uniform-values` to a struct's
   members and redraw the four references. Then `nir::uniform_of` and
   `nir_fs::uniform_extent` flatten a struct to member rows in declaration
   order (an array member one row an element, as the spike's
   `push_uniform`); the int members' compares are `ICMP_AND/OR.u32.eq.m1`.
   The vec4/float members of the mediump struct reuse the spike's fp16 words.
   Blocks on `vs_mediump_struct`'s FragColor lane (step 4) for 42 of them.
2. **Vertex int uniform as a value** (346 cases at the tip, was 117; `vs_int_uniform`): `nir.rs`
   admits an int uniform beyond a loop bound: the FAU word through
   `F16_TO_F32` for a mediump use, the dynamic matrix subscript
   `tmp[ui_zero]` as an index select, the kept uniform-bound loop's
   `ICMP_OR.s32.lt.m1`. `fau.rs` carries the int word rows.
3. **Int and bool arrays** (90 + 47 + 21 + FS 35 + 41 = 234 cases at the tip, was 227;
   `arr_bool_array`, `arr_int_array`, `arr_mat_array`, `fs_int_uniform`,
   `fs_bool_uniform`): the spike's element rule extended to int (one word, no
   padding), bool (one word, its own mask) and mat3 (a column a row, fp16);
   the constant index folds to the word, the FS `ICMP_*.m1` chain of the
   compares comes from the intval lowering. `nir_fs.rs` drops the
   `UniformArray` refusal for these element types.
4. **FragColor lane from a loaded uniform** (123 cases at the tip, was 197; `fs_mat_uniform`,
   `vs_mediump_array`, `vs_mediump_struct`): the general lane source of the
   plan's second family, not a uniform change; these three become
   byte-comparable once it lands, in `nir_fs.rs`.
5. **Vertex bvec compare chain** (0 cases at the tip, was 67; `vs_bool_uniform`): the walk's
   reduction takes `LSHIFT_OR.i32` of the lane masks into one `CSEL.u32.eq`
   (the same form `vsbvec3` pins; `nir.rs`, `select.rs`).
6. **FS mediump floats beside a computed lane** (50 cases at the tip, was 7; `fs_other`): the
   `FMA.v2f16` fp16 pair of two mediump uniforms in `nir_fs.rs`.

Each step moves its references from REFUSED to LOWERED in
`tests/mali_deqp_unif2.rs` in the same commit, and removes the `#[ignore]`.
