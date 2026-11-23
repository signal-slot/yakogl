<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): attribute records with an unread attribute between read ones

Plan #373, item `attrgap-fix` (from `inv-attrgap` and `vcube-magswitch`).
A vertex shader whose read locations are not one run from 0 (a `mat2` or
`mat3` attribute with a column read beyond column 0, an attribute declared
and unread or dead between read ones) was refused at the draw as
`NOT_BUILT_ATTRIBUTE_RECORDS` (`src/mali/backend.rs`, `vertex_attributes`).
dEQP-GLES2 meets it in shaders.operator.sequence, shaders.random (vertex),
fragment_ops.interaction and default_vertex_attrib (mat2, mat3).

## Citations

Mesa: `M:<path>:<line>`, `<path>` under `src/` of the Mesa 26.2.1 tree
(`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`). Ours: paths from the repository
root.

## Mesa's numbering

- The state tracker renumbers a vertex shader's inputs to the bits of
  `inputs_read`: `nir_recompute_io_bases` (`M:mesa/state_tracker/st_program.c:397`)
  makes an input's base the count of read locations below it, and the
  variant keeps `num_inputs = util_bitcount64(inputs_read)` and
  `vert_attrib_mask = inputs_read` (`M:mesa/state_tracker/st_program.c:423-424`).
  An unread location has no base, so it takes no slot.
- The vertex elements follow the same count: element
  `util_bitcount(inputs_read & BITFIELD_MASK(attr))` for attribute `attr`
  (`M:mesa/state_tracker/st_atom_array.cpp:178-183`), and a location read
  without an enabled array gets its current value as an element at the same
  index (`M:mesa/state_tracker/st_atom_array.cpp:371-385`). A matrix is one
  `gl_vert_attrib` per column, so a column that is not read is not an element.
- Panfrost's compiler records `attributes_read = inputs_read` and
  `attributes_read_count = util_bitcount64(attributes_read)`
  (`M:panfrost/compiler/pan_compiler.c:223-225`), and the driver packs one
  ATTRIBUTE descriptor per read input, `0..attributes_read_count`, each
  naming its element's Buffer (`M:gallium/drivers/panfrost/pan_cmdstream.c:2522-2548`,
  the Buffers themselves at `:941-971`).

So the attribute table is indexed by record `0..n` over the read locations
only, and the gaps vanish. Our compiler already numbers its records that way
(`MaliShader::attributes`, the `(location, record)` pairs), which is why an
unread `vec4` between read ones always drew.

## What was missing, and the change

The GL core stages one `GlesAttrib` per row of the shader's attribute
reflection (`ShcShader::attribs`), a matrix row split into one input per
column at `vi_reg + k` (`src/gl/draw.rs`, `vertex_inputs`). The back end
finds, for each record, the staged attribute whose `vi_reg` is the record's
location.

`record` in `src/shaderc/online/mali/mod.rs` built the reflection rows by
looking up each record's location among the reflection's attributes by
exact location. A matrix is listed once at its base location, so a record
reading column 1 or 2 found no row, the matrix was left out of
`ShcShader::attribs`, the GL core staged nothing for its columns and the
back end refused the draw.

The rows are now found by column range: a record's location `loc` belongs to
the attribute with `location <= loc < location + columns(gl_type)`
(`glsl::columns`, 1 for a non-matrix), and the row is written once, at the
base location with `vi_reg` the base, however many of its columns are read.
The GL core then stages every column at `base + k` and the back end finds
each read column's location among them; an unread column is staged and not
fetched, as an unread attribute between read ones is not a record (Mesa
leaves it out of the elements; we stage it and leave it out of the table,
which the frame cannot tell apart).

Nothing in the code words changes: the records, their numbering and the
ATTRIBUTE descriptors are the same; only the reflection rows the GL core
stages from are completed.

## The rows are the active attributes, the records are what the code reads

`ShcShader::attribs` is also what `glGetActiveAttrib`, `GL_ACTIVE_ATTRIBUTES`
and `glGetAttribLocation` answer from. Since the program's vertex shader is
built for its fragment shader's reads (`vs_for_fragment`,
docs/notes/mali-product-route.md), an attribute that feeds only a varying
the fragment shader does not read is loaded by no code and has no record
(gl_test case 1: `v_color = a_color` under a fragment shader that reads no
varying). Rows made from the records alone dropped it from the program's
active attributes, where the PowerVR, Vivante and V3D back ends list it
(`ShcAttribBinding::active`: the shader statically reads it, ES 2.0 section
2.10.4), and gl_test case 1 failed on the NanoPC-T6 (#11239).

So `record` lists a row for every attribute the vertex shader statically
reads (a load anywhere in the front end's IR, taken or not, less the side
of a constant `if` not taken: next section) and for every
attribute a record names, in declaration order. An attribute with a row and
no record is staged by the GL core and left out of the attribute table, like
the unread matrix column above.

## A read Mesa's compile drops, and the GL location (#11307)

dEQP-GLES2 attribute_location.bind_aliasing.max_inactive_* declares two
attributes per run of locations of its type over the 16 (32 floats, 8
mat4s), reads one of each pair under `if (u_A != 0.0)` and the other under
`if (0 != 0)`, and binds the pair to its run. The link must pass: the ones
in the never-taken side are not active and take no location.

Mesa:

- The compile runs `do_common_optimization` once
  (`M:compiler/glsl/glsl_parser_extras.cpp:2291`, from `opt_shader` at
  `:2496`), whose `do_if_simplification` (`:2598`) puts the side an `if` of
  a constant condition takes in its place and drops the other
  (`M:compiler/glsl/opt_if_simplification.cpp:87-98`). After it nothing
  reads `a_16`..`a_31`.
- The link removes the inputs nothing reads
  (`remove_dead_varyings_pre_linking`,
  `M:compiler/glsl/gl_nir_linker.c:1183-1191`, called at `:4103-4104`)
  before it assigns a location to any (`:4108`). An ES 3.00 vertex shader
  is the exception (`:4103`): its declared attributes all take one.

Ours had two faults on the way. With the rows of #11239 every load in the
front end's IR counted, the `if (0 != 0)` side's too (the front end keeps
the `if`): 32 rows, more than the record holds, and the compile was refused
(`Defect(IrIndex)`). Before #11239 the rows were the records' 16, but each
at the front end's location, which counts every declared attribute: in the
float case `a_8`, the 17th declared, has location 16, and the GL core
refused the link (`attribute 'a_8' has location 16, beyond the 16 vertex
inputs`).

So `record` now:

- leaves out a load in the side of an `if` whose condition the front end's
  constants alone decide, the side not taken (`dead_sides`, `const_lane`:
  integer and bool operations, and the float ones `fold::fold_float` folds
  exactly). A condition it does not fold keeps both sides, so a read is
  dropped only where Mesa drops it;
- gives the rows of an ES 1.00 shader the GL locations of the active
  attributes from 0, in declaration order (the GL core's own layout,
  gl/program.rs `attrib_layout`, makes the same choice), and keeps `vi_reg`
  the front end's location, which the records name. An ES 3.00 shader keeps
  the front end's locations (`layout(location)` among them).

Not the same as Mesa yet: a condition that reads a `const` variable (a load
of a local in our IR) is not folded, so its never-taken side still counts;
and Mesa places the unbound attributes largest first
(`M:compiler/glsl/gl_nir_link_varyings.c:60-70, 1453`), ours in declaration
order.

## Two attributes bound to one location (#11388)

dEQP-GLES2 attribute_location.bind_aliasing.cond_* bind two attributes to
one location and read each under one side of `u_A` (`a_0` when
`u_A != 0.0`, `a_1` when `u_A == 0.0`); max_cond_* do it for all 16
locations, 32 declared and read. ES 1.00 allows it when no path reads both,
and so does Mesa (`M:compiler/glsl/gl_nir_link_varyings.c:1224-1275`).

Mesa compiles the vertex program at link, after the locations are
assigned (`M:compiler/glsl/gl_nir_linker.c:4108`): the two variables have
one location, the inputs are numbered over `inputs_read`
(`M:mesa/state_tracker/st_program.c:397, 423-424`), and one `LD_ATTR_IMM`
of the location serves both names. The references
`tools/mali/testdata/deqp_misc3_bind_*` show it: `cond_float` is one
`LD_ATTR_IMM.v4` at `index:0x0`; `max_cond_mat4`'s locations 12, 8, 4, 0 are
indices 3, 2, 1, 0; `max_cond_float` has 16 loads, `index:0xF` down to 0.

Ours had two faults:

- The GL core relinks a program whose bindings alias with the link's map
  (gl/program.rs `link_attrib_layout`), and `compile_vs_mapped_blocks`
  always refused (`LinkedVertexNotLowered`): 8 cases.
- glCompileShader of a max_cond_* shader built it at the front end's
  locations, where the 32 read attributes are 32 records: record 16 does not
  fit LD_ATTR_IMM's 4-bit index and the packer failed (`Defect(Encode)`):
  6 cases (`max_cond_mat4` reads one column of each mat4, 8 records, so it
  failed only at the relink).

The change:

- `mali::compile_vs_mapped_blocks` moves each attribute's loads,
  declaration and row to the map's location
  (`powervr::abi::with_attrib_map`, the GC7000's and the V3D's step), then
  builds the vertex shader for the program's fragment shader as
  `compile_vs_for_reads` does (online backend.rs `vs_for_fragment` with the
  map). Two names on one location are one record; the record list names a
  location once (`compile_stage`), and the rows list it once (`record`:
  the first row that covers it), so the GL core stages it once and the
  frame builder finds it there. The rows keep the map's locations; after
  the relink the GL core answers glGetActiveAttrib from the map.
- A vertex shader whose active attributes take more than 16 locations is
  compiled to a record with no code that waits for the link
  (`mali::deferred_vertex`, `codegen::deferred_record`, the other GPUs'
  `layout_at_link` record). Active is the rule of the rows above, so
  max_inactive_* (16 active of 32) keeps its code. Its program's vertex
  shader without the link's locations is refused by name
  (`AttribLocationsAtLink`).
- The GL core counted a matrix's locations by its rows, and the Mali lists a
  matrix as one row: a mat2 bound to 1 took only location 1, so a vec4 bound
  to 2 was not seen as an alias (cond_mat{2,3,4}_offset_1 then built three
  records where Mesa has two). `attrib_layout` now takes the columns from
  the type too.

Not the same as Mesa yet (both seen without aliasing too):

- dEQP's shaders are mediump; their vertex arithmetic is fp32 here and fp16
  in Mesa (`FCMP_OR.v2f16`, `MUX.v2i16`), the mediump vertex walk of #4536.
  So the 14 cases compile with a program that is not Mesa's, as every
  mediump vertex shader does today.
- With the shaders made highp (`deqp_misc3_bind_hp_*`), 4 of 17 are byte
  equal. In the other 13 the only difference is the order of the first two
  instructions: ours puts the first `LD_ATTR_IMM` before the first `FCMP`,
  Mesa after it. The same happens with one float attribute read in two `if`
  sides under a uniform's condition, with no aliasing, so it is not the
  layout's.
- An attribute that is not active (read only in a side a constant
  condition does not take) and is in a program whose bindings alias makes
  the relink refuse its load (`with_attrib_map`, `AttribOutOfRange`): Mesa
  drops that side first. No dEQP case has the shape.

## The tests

- `tests/mali_attr_active.rs` (GL on the Mali mock): gl_test case 1's pair,
  plain and with `a_pos` bound to 3; the active attributes and their
  locations, and a draw that is not refused. And the 7 max_inactive_*
  shaders with dEQP's bindings: the link passes, 16 / columns attributes
  are active, each at its binding, and the never-read ones have location -1.
- `src/shaderc/online/mali/tests.rs`,
  `a_read_in_a_side_a_constant_condition_does_not_take_is_no_row`: reads
  under `if (0 != 0)`, `if (2.0 < 1.0)` and `if (false)` give no row, a read
  under a uniform's condition and under a folded true one do; the GL
  locations run from 0.

- `src/shaderc/online/mali/tests.rs`,
  `attribute_rows_cover_the_matrix_columns_read`: a `mat2` with column 1
  read, a `mat3` with columns 1 and 2 read (one row, not two), each with a
  tail attribute, and `a0`, unread `a1`, `a2`; every record's location lies
  in the column range of a row's `vi_reg` (the base location), and the GL
  locations run from 0 over the rows.
- `tests/mali_attr_gap.rs` (GL on the Mali mock, through `glLinkProgram`
  and `glDrawArrays`): the control, an unread attribute last or between read
  ones, a dead read between read ones (`v_c = a1; v_c = a2;`), the `mat2`
  with one column read and a tail, the `mat3` read from default values
  (`glVertexAttrib4f`) only at column 2, and the `mat3` with two columns read
  and a tail. Each asserts no refusal, no refusal line and `GL_NO_ERROR`.
- `src/mali/backend_tests.rs` keeps the back end's own refusal for a
  location the draw does not stage.
- `tests/mali_attr_active.rs`,
  `attributes_bound_to_one_location_link_and_draw`: the 17 bind_aliasing
  cond_* / cond_*_offset_1 / max_cond_* shapes with dEQP's bindings (the 14
  of #11388 and the 3 offset ones) link, every attribute is active at its
  binding, and a draw with each side of `u_A` is not refused.
- `tests/mali_deqp_misc3.rs`: the byte compares of the section above.
- `src/shaderc/online/mali/tests.rs`,
  `more_than_16_active_locations_wait_for_the_link`.

The board effect (shaders.operator.sequence, the shaders.random vertex
cases, fragment_ops.interaction, default_vertex_attrib mat2/mat3) is for the
plan's board rerun to record.
