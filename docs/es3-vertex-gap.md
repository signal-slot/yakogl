<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 vertex attributes: the dEQP-GLES3 gap (item vertex-int-attribs)

This note classifies every dEQP-GLES3 failure in the vertex-attribute groups
of the AM62P run, names the root cause with its file and line, and says which
plan item fixes it. It also records what this item changed in
`src/gl/vertex.rs` and what is still missing elsewhere.

Plain words for the terms: a *current value* is what a vertex input reads
while its array is disabled (`glVertexAttrib*`); a *pure-integer* current
value is one set with `glVertexAttribI4*`; a *packed type* is
`GL_INT_2_10_10_10_REV` or `GL_UNSIGNED_INT_2_10_10_10_REV`.

## Sources

- **AM62P board run**, dEQP-GLES3 mustpass 42,810 cases, main `396d667a`,
  2026-09-26: the group table of `tools/cts/RESULTS.md` section 1r (lines
  1457-1493). The per-case verdicts come from the run's UART records, which
  are gitignored and live in the owner's checkout on the Lepton node under
  `records/am62p/`:
  `record_G3-img1-396d667a_adefee7e6bf72ccb.txt` (attribute_location,
  default_vertex_attrib), `record_G3-img5-396d667a_f66825a3c66ce205.txt`
  (instanced, negative_api), `record_G3-img18-396d667a_7643b20b974461b0.txt`
  (state_query) and `record_G3-img25-396d667a_eec5b9dfa4d38d9f.txt`
  (vertex_arrays). Each record lists one `deqp: Pass|Fail|NotSupported <case>`
  line per case, so every count below marked *exact* is a count of those
  lines. (`docs/board-sessions/am62p-gles3-percase.md` records that the later
  per-case session got no data; these records are the 396d667a run itself.)
- **dEQP source**: VK-GL-CTS `93bca01` (`$CCFARM_DEPS_DIR/VK_GL_CTS` on this
  node, copied from the farm), read to see what each case does.
- **yakogl**: the integration tip `2f69c7d1`. Line numbers are of that tip
  (plus this item's edit of `src/gl/vertex.rs`).
- **Host dEQP run**: none in this session. The sandbox refused every `cargo`
  and `cmake` invocation, so the host numbers of `host-deqp-run.sh` (API
  verdicts of attribute_location, negative_api and state_query are real on
  the host; render verdicts are not) could not be taken. The commands are at
  the end.

The i.MX8MP and the Pi 4 have no dEQP-GLES3 run; their columns are
predictions from the code and are marked so.

## The counts and causes

| Subgroup (dEQP-GLES3.functional.) | Mustpass cases | AM62P fails | Root cause | Fixed by |
|---|---:|---|---|---|
| vertex_arrays.single_attribute.first.int2_10_10_10, .offset.int2_10_10_10, .strides.int2_10_10_10.buffer_* | 8 + 8 + 4 | **20, exact** (all of them; img25) | A packed array in a buffer is fetched in place as float32 words: `src/gl/draw.rs:818` `map_attrib_format` maps the packed types (and `GL_HALF_FLOAT`) to `GLES_ATTRIBF_FLOAT32`, so `draw.rs:3253` takes the array as native and never reaches `convert_attrib`; `draw.rs:3272` also sizes one element as components x 4 bytes. Client arrays (the `user_ptr` strides cases, normalize, output_types: 22 cases) are converted and pass. | not covered (GL core draw path); see task T2 |
| vertex_arrays.single_attribute.output_types.{byte,short,int}.components{2,3}_ivec4_*, .{unsigned_byte,unsigned_short,unsigned_int}.components{2,3}_uvec4_* | 24 | **21, exact** (img25; the 3 passing are unsigned components2 quads1) | An integer array narrower than its ivec4/uvec4 input gets its missing w filled with the float 1.0 (`draw.rs:868`, `attrib_row`): bits 0x3f800000, which the input reads as 1065353216. ES 3.0 2.8 fills (0, 0, 0, 1) in the input's type. | not covered (GL core draw path); task T3 |
| vertex_arrays, everything else | 920 | 0, exact | | |
| attribute_location.mixed.*, .mixed_time.{pre_attach,pre_link,reattach,relink} | 24 + 5 | **28, exact** (img1; mixed_time.post_link passes) | `glBindAttribLocation` is applied to an attribute that has `layout(location = N)`: `src/gl/program.rs:1545-1549` (`build_attrib_indices`) remaps the shader's location to the bound index whenever the name was bound. ES 3.0 2.12.3 and GLSL ES 3.00 4.3.8.1: the qualifier wins and the binding is ignored. dEQP binds `a_0` to 4 while the shader says 3 and expects 3; post_link passes because a binding made after the link has no effect until the next link. | not covered (GL core link); task T4 |
| attribute_location.mixed_max_attributes.* | 24 | **20, exact** (img1; mat3, mat3x2, mat3x3, mat3x4 pass) | The compiler places an `in` with no qualifier at the counter continuing past the previous explicit location (`src/shaderc/online/reflect.rs:471-485`). dEQP alternates `layout(location = L)` and bound attributes from the top location down, so for vec4 `a_0` (unqualified) takes 0 and `a_15` is `layout(location = 0)`: two inputs on one location, and `program.rs:1562` writes `attrib_app_index[0]` twice, so the bound name answers 0 instead of 15. The mat3 shapes fill 15 of 16 locations and never collide. ES 3.0 2.12.3: unqualified, unbound attributes get locations the linker chooses among those no qualifier uses. | not covered (shaderc reflection + GL core link); task T4 |
| attribute_location, everything else (bind*, layout*, mixed_hole, mixed_relink_hole, bind_time, mixed_time.post_link) | 221 | 0, exact | | |
| default_vertex_attrib.{int,ivec2,ivec3,ivec4}.vertex_attribi_4i{,v}, .{uint,uvec2,uvec3,uvec4}.vertex_attribi_4ui{,v} | 16 | **16, exact** (img1) | `glVertexAttribI4i`/`I4ui` stored the value converted to a float (`src/gl/vertex.rs`, the old lines 312-321), and the draw stages the current value's dwords as they are (`draw.rs:3208`), so an ivec4/uvec4 input read a float's bit pattern. | **this item**: the pure-integer value is stored as its bits (`vertex.rs:361`, `set_current_bits`) |
| default_vertex_attrib, float and matrix cases | 64 | 0, exact | | |
| instanced.{draw_arrays_instanced,draw_elements_instanced}.mixed.{2,4,20}_instances | 6 | **6, exact** (img5) | The mixed cases read `gl_InstanceID` and use divisors 2 and 1 (es3fInstancedRenderingTests.cpp:55-57, 598-602). The PowerVR takes divisor 1 only (`src/pvr/backend.rs:6630`, `max_instance_divisor`), so `draw.rs:4270` emulates the draw instance by instance, and `pvr/backend.rs:3927` refuses an emulated instance whose program reads `gl_InstanceID` (the PDS counts instances from 0 in every draw). i.MX8MP: the same refusal (`src/vivante/backend.rs:2167`, divisor 1 at `:2690`), so the same 6 are predicted to fail. Pi 4: the V3D takes any divisor (`src/v3d/backend.rs:4645`, 0xffff) and the draw stays on the GPU, predicted to pass. | back-end item, not covered; task T6 |
| instanced, everything else | 39 | 0, exact | | |
| negative_api.vertex_array.{draw_elements, draw_elements_incomplete_primitive, draw_elements_instanced, draw_elements_instanced_incomplete_primitive, draw_range_elements, draw_range_elements_incomplete_primitive} | 6 | **6, exact** (img5) | Each case ends with a transform-feedback section (es3fNegativeVertexArrayApiTests.cpp:430-462): it links a program with a captured varying, begins a capture and expects GL_NO_ERROR, then expects GL_INVALID_OPERATION from an indexed draw while the capture is active. At 396d667a transform feedback did not exist (every MAX_TRANSFORM_FEEDBACK_* was 0 and Begin was an error). | tf-api (merged): `tests/es3_tf_api.rs` `capture_is_refused_loudly` holds exactly this order; confirm on the board (board-am62p) |
| negative_api.vertex_array.{vertex_attrib_pointer, vertex_attrib_i_pointer, vertex_attrib_divisor, vertex_attribf, vertex_attribfv, vertex_attribi4, vertex_attribi4v, enable_vertex_attrib_array, disable_vertex_attrib_array, gen_vertex_arrays, bind_vertex_array, delete_vertex_arrays, draw_arrays*, draw_elements_invalid_program, draw_elements_instanced_invalid_program, draw_range_elements_invalid_program}, negative_api.state.{get_vertex_attribfv, get_vertex_attribiv, get_vertex_attribi_iv, get_vertex_attribi_uiv, get_vertex_attrib_pointerv, is_vertex_array} | 27 | 0, exact | | |
| state_query.shader.vertex_attrib_{size,type,stride,normalized,integer,enabled,divisor,array_buffer_binding,pointerv}, .current_vertex_attrib_{initial,float,int,uint,float_to_int}, state_query.integers.max_vertex_attribs_*, .vertex_array_binding_* | 21 | 0, exact (img18) | `current_vertex_attrib_float_to_int` accepts any integer from floor(x) to ceil(x) (es3fShaderStateQueryTests.cpp:62-74, 203-223), so the truncation the code did passed; the code now rounds to nearest as ES 3.0 6.1.2 says. `current_vertex_attrib_int`/`uint` use values below 2^24, which also round-tripped through the old float storage. | this item (rounding) |
| vertex_array_objects.*, default_vertex_array_object.* | 11 | 0, exact | | |

The other failures of negative_api (25) and state_query (63) in those records
are transform feedback, queries, textures, framebuffers and limits, outside
this item (`docs/es3-rest-failure-map.md` is their map).

Sum of the vertex-related failures: 20 + 21 + 28 + 20 + 16 + 6 + 6 = 117. Of
them 16 are fixed here, 6 wait for a board confirmation of tf-api, and 95
need files this item does not own (tasks T2-T6 below).

## What this item changed in `src/gl/vertex.rs`

1. **Pure-integer current values are exact.** `glVertexAttribI4{i,ui}{,v}`
   store the four 32-bit values as they are, in the float slots of
   `GlContext::current_attrib` (`set_current_bits`). That is the convention
   the draw already uses for integer arrays (`draw.rs` `attrib_component`
   widens an integer to 32 bits "whose bits travel in the float slot") and
   for the per-instance values it writes into `current_attrib` itself
   (`draw.rs:4190` `instance_attrib_value`). The draw stages the dwords
   unchanged (`draw.rs:3208`), so an ivec4/uvec4 input now reads i32::MIN to
   u32::MAX exactly. `glGetVertexAttribI{i,ui}v(GL_CURRENT_VERTEX_ATTRIB)`
   return those bits. Fixes the 16 default_vertex_attrib cases (to be
   confirmed on the board).
2. **`glGetVertexAttribiv(GL_CURRENT_VERTEX_ATTRIB)` rounds** the float value
   to the nearest integer, halves away from zero, saturating (ES 3.0 6.1.2).
   It truncated before.
3. **The ES 2.0 report answers ES 2.0.** With `report-es2`,
   `glVertexAttribPointer` says GL_INVALID_ENUM for `GL_HALF_FLOAT` and the
   two packed types (ES 2.0 has none of them; `GL_OES_vertex_half_float` and
   `GL_OES_vertex_type_10_10_10_2` are not claimed, and `GL_HALF_FLOAT_OES`
   0x8D61 was never accepted), and the four getters say GL_INVALID_ENUM for
   `GL_VERTEX_ATTRIB_ARRAY_INTEGER` and `GL_VERTEX_ATTRIB_ARRAY_DIVISOR` (no
   instanced-arrays extension is claimed). Before, both builds accepted them.
   The ES 3.0 entry points (`glVertexAttribIPointer`, `glVertexAttribI4*`,
   `glGetVertexAttribI*`, `glVertexAttribDivisor`) stay exported and
   functional in the ES 2.0 build, as every other ES 3.0 entry point does
   there (`src/gl/egl.rs` has no per-report filter and `src/gl/es3.rs` gates
   behaviour, not symbols); hiding them would need `egl.rs` and `capi/`.
   dEQP-GLES2 never uses these types or pnames (its negative state cases pass
   pname -1), so the ES 2.0 CTS answer does not move.

Not changed, because the code was already right (held by
`tests/es3_vertex_api.rs`): index, size and stride checks (GL_INVALID_VALUE),
unknown types (GL_INVALID_ENUM), `glVertexAttribIPointer` taking the six
integer types only, packed types with size other than 4
(GL_INVALID_OPERATION), a client pointer with a vertex array object bound
(GL_INVALID_OPERATION), `glVertexAttribDivisor` (index check only; any
divisor value is legal), the stored integer flag and divisor, and the getters'
GL_INVALID_ENUM for unknown pnames.

**A limit of the fix.** The context records a current value's dwords but not
whether a float or an integer command set them (`GlContext::current_attrib` is
`[[GLfloat; 4]; 16]`, `src/gl/context.rs:821`; that file belongs to item
gl-core-misc). ES 3.0 2.7 and 6.1.14 leave the mixed reads undefined (a float
input or `glGetVertexAttribfv/iv` on a pure-integer value, an integer input or
`glGetVertexAttribI*v` on a float value, the initial (0, 0, 0, 1) included),
and here they return the stored bits reinterpreted. Mesa converts instead.
dEQP exercises only the defined pairs. Task T1 adds the type tag.

## Back-end support still missing

- **PowerVR, GC7000UL: divisors above 1 together with `gl_InstanceID`.**
  Both back ends take divisor 1 only; the GL core emulates other divisors
  one instance per draw and the back ends then refuse a program that reads
  `gl_InstanceID` (`pvr/backend.rs:3927`, `vivante/backend.rs:2167`). The
  6 instanced.mixed cases need either the vertex fetch stepping by the
  divisor (Mesa pvr programs the PDS instance step; etnaviv has no divisor
  on GC7000 and emulates in the driver) or `gl_InstanceID` fed as a per-draw
  constant in the emulated path. Not in the plan: task T6.
- **GC7000UL: integer arrays in a buffer.** The Vivante back end fetches
  byte, short and fixed arrays in place (`vivante/backend.rs:2646`,
  `native_attrib_formats`), and `draw.rs:3253` does not look at
  `GlVertexAttrib::integer` before choosing the in-place fetch, so a
  `glVertexAttribIPointer` byte or short array in a buffer would be fetched
  as a float array on the i.MX8MP. The PowerVR (`native_attrib_formats` 0)
  and the V3D (float32 only) convert such arrays on the CPU, so the AM62P
  passed the integer `output_types.*.components4_*` cases. Untested on the
  i.MX8MP (no GLES3 run yet); task T2 covers the check.
- **Nothing is missing for the integer current values**: every back end
  receives the staged 16 bytes and fetches them with stride 0.

## Tasks for later items

- **T1, typed current value** (`src/gl/context.rs`, then `vertex.rs`): keep
  `current_attrib` for the ABI (`src/abi_layout.rs:592` pins its offset) and
  add a per-attribute tag (float, int, uint) reset with the other attribute
  state at `context.rs:1462`; `glGetVertexAttribfv/iv` then convert a
  pure-integer value and `glGetVertexAttribI*v` convert a float one, as Mesa
  does. The draw does not need the tag.
- **T2, ES 3.0 array formats in a buffer** (`src/gl/draw.rs`):
  `map_attrib_format` (`draw.rs:818`) must not call the packed types and
  `GL_HALF_FLOAT` float32; they, and any array with `integer` set on a back
  end whose `native_attrib_formats` covers the type, must go through
  `convert_attrib`. The element size at `draw.rs:3272` must be
  `attrib_elem_bytes`. Fixes the 20 vertex_arrays packed cases on the AM62P
  and the i.MX8MP integer-array hazard above.
- **T3, the integer fill** (`draw.rs:868`, `attrib_row`): the w of a
  narrower integer array is the integer 1, not the float 1.0. Fixes 21
  vertex_arrays.output_types cases.
- **T4, layout locations at link** (`src/gl/program.rs`,
  `src/shaderc/online/reflect.rs`, `src/shaderc/online/codegen.rs:979`,
  `program.rs:1353`): (a) `build_attrib_indices` must ignore a binding for an
  attribute with an explicit layout location; (b) unqualified attributes must
  be placed at link among the locations no qualifier uses (the reflection's
  running counter at `reflect.rs:471-485` collides with a later qualifier,
  and the deferred `layout_at_link` record drops the declared location at
  `codegen.rs:979` / `program.rs:1353`). Fixes 48 attribute_location cases.
  These verdicts are API-level and reproducible on the host with the command
  below.
- **T5, confirm tf-api on the board** (board-am62p): the 6
  negative_api.vertex_array indexed-draw cases.
- **T6, instancing with any divisor on the PowerVR and the GC7000UL** (back
  ends): the 6 instanced.mixed cases; also predicted on the i.MX8MP.
- **T7, the i.MX8MP and Pi 4 columns**: the first dEQP-GLES3 run of each
  (board-imx8mp, board-pi4) turns the predictions above into counts.

## Host measurement to take (HOST-ONLY: API verdicts only)

The run needs the VK-GL-CTS checkout of `docs/es3-host-env.md` (here at
`$CCFARM_DEPS_DIR/VK_GL_CTS`) and a working `cargo` and `cmake`:

```sh
export CTS="$CCFARM_DEPS_DIR/VK_GL_CTS"
for g in powervr vivante; do
  for f in attribute_location. negative_api.vertex_array negative_api.state.get_vertex_attrib \
           state_query.shader.vertex_attrib state_query.shader.current_vertex_attrib; do
    GPU=$g MODULE=gles3 OUT=build/host-deqp/vtx-$g-${f//[.\/]/_} \
      tools/cts/host-deqp-run.sh "dEQP-GLES3.functional.$f"
  done
done
```

Expected on the integration tip plus this item: attribute_location mixed*
48 fails until T4; negative_api.vertex_array and negative_api.state
get_vertex_attrib* 0 fails; state_query.shader vertex_attrib* and
current_vertex_attrib* 0 fails. default_vertex_attrib, instanced and
vertex_arrays are render verdicts and mean nothing on the host.

## ES 2.0 no-regression

The ES 2.0 build (`report-es2`) changes only in what it refuses: the ES 3.0
array types and pnames above. dEQP-GLES2 does not use them, so its
state_query (396/396) and negative_api (237 pass, 9 not supported) rows are
predicted unchanged. `tests/es3_vertex_api.rs` runs its argument,
current-value and getter tests in the ES 2.0 build too, and
`es2_build_refuses_the_es3_types_and_pnames` pins the refusals; the unit
tests in `vertex.rs` branch on `crate::REPORT_ES2` the same way.
`glGetVertexAttribiv(GL_CURRENT_VERTEX_ATTRIB)` now rounds instead of
truncating in both builds; dEQP-GLES2's conversion case accepts both.
