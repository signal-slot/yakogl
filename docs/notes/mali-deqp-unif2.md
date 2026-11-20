<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: the uniform refusals, shapes and Mesa references (unif2)

Plan #387, item unif-ref: a study and references, no compiler change. Method
as `docs/notes/mali-deqp-families.md` sections 1-3, 6 and 7. The earlier
uniform notes (`mali-deqp-unitype.md`, `mali-deqp-uniarray.md`,
`mali-uniform-types.md`, `mali-uniform-arrays.md`, `mali-uniform-index.md`,
`mali-int-uniforms.md`) stay the record of what each form is; this one is the
map of what is left at the tip.

## 1. What was measured, and what was not

The shapes and counts below are the `deqp_shapes/index.tsv` rows of families
`unitype` and `uniarray` (HOST-ONLY: `tools/mali/deqp-host-measure.sh` on the
host, the library compiling every dEQP shader, no stream run). They sum to
1,092 cases (991 Fail, 101 NotSupported), the uniform texts' counts at
`37305608` (the run at `abcc9013`, the first refusal of each case).

**Not re-measured at the tip (`bee6896d`).** `deqp-host-measure.sh --stage
build` needs a complete VK-GL-CTS checkout at `93bca01` with `fetch_sources`;
the dependency cache holds only the sparse `VK-GL-CTS-gles3` corpus (`data/`),
which cannot build `deqp-gles2`. So the counts are those of `abcc9013`; each
shape's *current outcome* is what the new test pins on its reference. Where a
shape's reference now compiles (`vs_mat_uniform`, and the existing LOWERED
references of section 2), the cases of that shape no longer refuse on this text
if the reference stands for all of them, which only the re-measure can say:

```sh
tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS 93bca01 tree> --slice records/nanopc-t6/20261117-* \
    --filter uniform
```

## 2. Shapes, counts and the reference that covers each

Fail + NotSupported = cases. "Existing" is the `deqp_unitype_*` or
`deqp_uniarray_*` reference whose vertex and fragment shader are byte-identical
to the shape's (compared by hash), with its pin in `tests/mali_deqp_unitype.rs`
or `tests/mali_deqp_uniarray.rs` at the tip. "New" is a `deqp_unif2_<shape>`
reference of this item; its pin is in `tests/mali_deqp_unif2.rs`.

| Shape (stage, type, precision, index) | Cases | dEQP groups (example) | Reference | Pin at the tip |
|---|---:|---|---|---|
| VS mat2/mat3 uniform, highp or mediump, dynamic matrix subscript | 138 | `shaders.matrix.*_vertex` | New `deqp_unif2_vs_mat_uniform` | LOWERED (byte-equal) |
| VS struct uniform (nested, array member, int/bool members), mediump | 134 | `uniform_api.value.assigned.*.array_in_struct` | New `deqp_unif2_vs_struct_uniform` | REFUSED: other than float..mat4 |
| FS mat2 uniform, highp | 119 | `shaders.matrix.*_fragment` | New `deqp_unif2_fs_mat_uniform` | REFUSED: FragColor lane |
| VS mediump int uniform (`tmp[ui_zero]`, `i < ui_two`) | 117 | `shaders.indexing.matrix_subscript.*_vertex` | New `deqp_unif2_vs_int_uniform` | REFUSED: other than float..mat4 |
| VS bool/bvec uniform, run-time compare chain | 67 | `uniform_api.value.assigned.by_pointer.get_uniform.basic` | New `deqp_unif2_vs_bool_uniform` | REFUSED: arithmetic other than +,-,* |
| FS struct uniform | 55 | `uniform_api...array_in_struct.*_fragment` | New `deqp_unif2_fs_struct_uniform` | REFUSED: mediump FS uniform |
| VS mediump struct (float/vec4 members) | 53 | `uniform_api...by_pointer.get_uniform.basic_struct` | New `deqp_unif2_vs_mediump_struct` | REFUSED: FragColor lane |
| FS mediump int array / int uniform | 52 | `uniform_api...basic_array_assign_full.basic_array.int_fragment` | New `deqp_unif2_fs_int_uniform` | REFUSED: mediump FS uniform |
| FS bool array | 33 | `...basic_array.bool_fragment` | New `deqp_unif2_fs_bool_uniform` | REFUSED: mediump FS uniform |
| VS mediump vec4 array (compare against api values) | 25 | `...basic_array_assign_full.basic_array.vec4_both` | New `deqp_unif2_vs_mediump_array` | REFUSED: FragColor lane |
| FS other (two mediump floats beside a computed lane) | 7 | `shaders.random.basic_expression.combined.*` | New `deqp_unif2_fs_other` | REFUSED: mediump FS uniform |
| array: bool array, `uniform bool u_var[3]` | 95 | `...basic_array_assign_full.basic_array.bool_both` | New `deqp_unif2_arr_bool_array` | REFUSED: a uniform array |
| array: mediump int array | 46 | `...basic_array.int_both` | New `deqp_unif2_arr_int_array` | REFUSED: a uniform array |
| array: struct array `uniform S s[2]` | 2 | `shaders.struct.uniform.nested_struct_array_vertex` | New `deqp_unif2_arr_struct_array` | REFUSED: other than float..mat4 |
| array: mat3 array + ivec4 | 1 | `uniform_api.random.10` | New `deqp_unif2_arr_mat_array` | REFUSED: a uniform array |
| array in struct (`float_vec4_both`) | 77 | `...array_in_struct.float_vec4_both` | `deqp_uniarray_struct` | REFUSED: FragColor lane |
| float array | 36 | `...basic_array.float_both` | `deqp_uniarray_api_float` | REFUSED: mediump FS uniform (FsUniform) |
| vec array, dynamic read | 12 | `shaders.indexing.uniform_array.vec2_dynamic_read_fragment` | `deqp_uniarray_dyn_vec2_fs` | LOWERED (assert_matches) |
| run-time index in a loop | 3 | `shaders.indexing.uniform_array.float_static_loop_read_fragment` | `deqp_uniarray_loop_float_fs` | LOWERED |
| FS vec array, constant index | 2 | `shaders.indexing.uniform_array.vec2_static_read_fragment` | `deqp_uniarray_vec2_const_fs` | LOWERED |
| FS highp uniform (`clear_color`) | 17 | `fragment_ops.scissor.clear_color` | `deqp_unitype_fshighp_color` | MATCHED |
| VS mediump basic (`depth_range`) | 1 | `builtin_variable.depth_range_vertex` | `deqp_unitype_depthrange_vs` | MATCHED |

The shapes sum to 820 (unitype) + 272 (uniarray) = 1,092. Fifteen shapes have
no existing reference and got one here; the other seven are covered by the
references named (their sources are byte-identical, so no second copy was
drawn). The existing references of the families that are not byte-identical to
a shape (`deqp_unitype_vsapi_mat2`, `vsbvec3`, `fsivec4`, `fsmulti`,
`fsmat3_mediump`, the `deqp_uniarray_*_vs` run-time index ones, `struct_member_*`)
are the sub-shapes the unitype and uniarray notes already pin; their pins
(DIFFERS: `vsmat3_lowp`, `vsapi_mat2`, `vsapi_struct`, `vsstruct_add`,
`depthrange_fs`, `vsbvec3`; REFUSED: `vsmat3_light`, `fsmat3_mediump`, `fsivec4`,
`fsmulti`, `struct_member_vs`/`_fs`, `dyn_vec{2,3,4}_vs`, `loop_float_vs`) stay
as in their tests.

By stage and refusal text at the tip, over the 15 new references: vertex
shader int/struct (`vs_int_uniform`, `vs_struct_uniform`, `arr_struct_array`):
253 cases; the fragment shader's mediump uniform (`fs_bool`, `fs_int`,
`fs_other`, `fs_struct`): 147; a uniform array (`arr_bool`, `arr_int`,
`arr_mat`): 142; the FragColor lane after a uniform that now loads
(`fs_mat`, `vs_mediump_array`, `vs_mediump_struct`): 197; the bvec chain
(`vs_bool`): 67; compiled (`vs_mat_uniform`): 138.

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

1. **Struct uniforms, both stages** (VS 134 + 53, FS 55, array 2 = 244 cases;
   `vs_struct_uniform`, `vs_mediump_struct`, `fs_struct_uniform`,
   `arr_struct_array`). First extend `gl_ref.py --uniform-values` to a struct's
   members and redraw the four references. Then `nir::uniform_of` and
   `nir_fs::uniform_extent` flatten a struct to member rows in declaration
   order (an array member one row an element, as the spike's
   `push_uniform`); the int members' compares are `ICMP_AND/OR.u32.eq.m1`.
   The vec4/float members of the mediump struct reuse the spike's fp16 words.
   Blocks on `vs_mediump_struct`'s FragColor lane (step 4) for 53 of them.
2. **Vertex int uniform as a value** (117 cases, `vs_int_uniform`): `nir.rs`
   admits an int uniform beyond a loop bound: the FAU word through
   `F16_TO_F32` for a mediump use, the dynamic matrix subscript
   `tmp[ui_zero]` as an index select, the kept uniform-bound loop's
   `ICMP_OR.s32.lt.m1`. `fau.rs` carries the int word rows.
3. **Int and bool arrays** (95 + 46 + 1 + FS 52 + 33 = 227 cases;
   `arr_bool_array`, `arr_int_array`, `arr_mat_array`, `fs_int_uniform`,
   `fs_bool_uniform`): the spike's element rule extended to int (one word, no
   padding), bool (one word, its own mask) and mat3 (a column a row, fp16);
   the constant index folds to the word, the FS `ICMP_*.m1` chain of the
   compares comes from the intval lowering. `nir_fs.rs` drops the
   `UniformArray` refusal for these element types.
4. **FragColor lane from a loaded uniform** (197 cases; `fs_mat_uniform`,
   `vs_mediump_array`, `vs_mediump_struct`): the general lane source of the
   plan's second family, not a uniform change; these three become
   byte-comparable once it lands, in `nir_fs.rs`.
5. **Vertex bvec compare chain** (67 cases; `vs_bool_uniform`): the walk's
   reduction takes `LSHIFT_OR.i32` of the lane masks into one `CSEL.u32.eq`
   (the same form `vsbvec3` pins; `nir.rs`, `select.rs`).
6. **FS mediump floats beside a computed lane** (7 cases; `fs_other`): the
   `FMA.v2f16` fp16 pair of two mediump uniforms in `nir_fs.rs`.

Each step moves its references from REFUSED to LOWERED in
`tests/mali_deqp_unif2.rs` in the same commit, and removes the `#[ignore]`.
