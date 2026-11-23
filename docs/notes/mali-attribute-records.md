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

The board effect (shaders.operator.sequence, the shaders.random vertex
cases, fragment_ops.interaction, default_vertex_attrib mat2/mat3) is for the
plan's board rerun to record.
