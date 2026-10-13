<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES3: the remaining groups — failure map

**Status (2026-10-14): finished record of the AM62P run on `396d667a`**
(ccfarm `01a119ca-708f-7040-9874-ea491427be1e`, written there as
`docs/es3-rest-failure-map.md`; it lands here the way main keeps the
shaders and fbo/fragops maps). The `tools/cts/RESULTS.md` line numbers are
main's at the time of landing (`cd738b1`). The `src/` line numbers are those
of the commit each citation names. Written before these landed on main, so
read them as history: `docs/es3-transform-feedback.md` is now on main, and
the transform feedback API and capture are built (`docs/es3-cts-failures.md`,
ledger row RC3); `docs/es3-backend-caps.md` is still reverted. What is
current is `docs/es3-cts-failures.md` ("State on f2a13877"); a later run
than `396d667a` exists there, but no per-subgroup split of it is written
down.

Plan #3 item `es3-rest-deqp-gap`. This map covers every group of
`tools/cts/gles3-mustpass.txt` (42,810 names) except `shaders`, `texture`,
`fbo`, `fragment_ops` and `fragment_out`, which the sibling maps cover:
the shaders record `tools/cts/log/investigations/es3-shaders-396d667a.md`,
the fbo/fragops record `tools/cts/log/investigations/es3-fbo-fragops-396d667a.md`
(Appendix B of `docs/es3-cts-failures.md`), and the texture map
`docs/es3-texture-failure-map.md`, which was never written. It is input for `es3-inventory`
(`docs/es3-cts-failures.md`) and for the scope of `gl-core-misc`,
`ubo-uniform`, `vertex-int-attribs`, `tf-api`, `tf-capture`, `tf-infolog`,
`occlusion-query` and `mrt4`. Documentation only: nothing was run here; every
number is a count of names in `tools/cts/gles3-mustpass.txt`, a figure copied
from the AM62P run, or arithmetic on those two (the arithmetic is shown).

The run is the only GLES3 run there is: AM62P, main `396d667a`, 2026-09-26,
`tools/cts/RESULTS.md` §1r (heading at line 1438). Abbreviations:

- **R-group** = the §1r per-group table, `tools/cts/RESULTS.md:1458-1495`
  (each row cited by line below).
- **R-bucket** = the "8,835 failures by message" table, `RESULTS.md:1500-1517`
  (only the 16 largest of 78 messages are in the repository; the other 62
  together are 8,835 − 7,530 = 1,305 cases and are not listed anywhere).
- **R-1q** = the §1q state_query bucket table, `RESULTS.md:1423-1431` (a
  state_query-only board run at `9134800c`, 2026-09-25; its 874 / 63 / 9 equal
  the §1r row, so its buckets apply to the §1r numbers).
- **R-1p** = `RESULTS.md:1409-1410` (fbo.api 20/20 pass; info 6/6 pass).
- **R-life** = the §1r lifetime note, `RESULTS.md:1524-1531`.
- **R-img** = the §1r per-image table, `RESULTS.md:1553-1596`.
- **Work** = the Work-list rows: `RESULTS.md:990-996` (AM62P GLES3 rows) and
  `:874` / `:879` (GLES3 host runs `01de3104` and `8b8c5ff8`; host-only,
  cited as such).
- **Caps** = `docs/es3-backend-caps.md`: merged at `b6004e19`, then reverted
  at `3f78982e` ("Vetoed by Claude's confirming review after it was merged
  without a consultation"); it is **not at HEAD**, and was read from
  `b6004e19`.
- **TF** = `docs/es3-transform-feedback.md` (plan item `tf-design`, branch
  `ccfarm/01a11864-6419-75df-a128-991589c367a2`, commit `f818f84e`; **not
  merged**, so not at HEAD).
- **OQ** = `docs/es3-occlusion-query.md` (at HEAD; added at `d3037128`).
- **cts** = `$CCFARM_DEPS_DIR/VK-GL-CTS/modules/gles3/functional/` (VK-GL-CTS
  pinned `93bca01`).

## 1. What is known, and what is not

Group totals from R-group (lifetime has no row there; its 56 cases have no
result, R-life):

| group | mustpass | pass | fail | n/s | other | source |
|---|---:|---:|---:|---:|---:|---|
| attribute_location | 274 | 226 | 48 | 0 | 0 | R-group `:1458` |
| buffer | 695 | 695 | 0 | 0 | 0 | `:1459` |
| clipping | 602 | 602 | 0 | 0 | 0 | `:1460` |
| color_clear | 19 | 19 | 0 | 0 | 0 | `:1461` |
| default_vertex_array_object | 1 | 1 | 0 | 0 | 0 | `:1462` |
| default_vertex_attrib | 80 | 64 | 16 | 0 | 0 | `:1463` |
| depth_stencil_clear | 10 | 10 | 0 | 0 | 0 | `:1464` |
| dither | 20 | 20 | 0 | 0 | 0 | `:1465` |
| draw | 322 | 272 | 50 | 0 | 0 | `:1466` |
| draw_buffers_indexed | 76 | 0 | 0 | 76 | 0 | `:1467` |
| fence_sync | 10 | 10 | 0 | 0 | 0 | `:1469` |
| implementation_limits | 49 | 33 | 16 | 0 | 0 | `:1472` |
| info | 6 | 6 | 0 | 0 | 0 | `:1473` |
| instanced | 45 | 39 | 6 | 0 | 0 | `:1474` |
| lifetime | 56 | — | — | — | — | no result, R-life |
| multisample | 64 | 0 | 16 | 48 | 0 | `:1475` |
| negative_api | 312 | 277 | 31 | 4 | 0 | `:1476` |
| occlusion_query | 62 | 0 | 62 | 0 | 0 | `:1477` |
| pbo | 49 | 17 | 26 | 6 | 0 | `:1478` |
| polygon_offset | 20 | 20 | 0 | 0 | 0 | `:1479` |
| prerequisite | 3 | 3 | 0 | 0 | 0 | `:1480` |
| primitive_restart | 504 | 504 | 0 | 0 | 0 | `:1481` |
| rasterization | 100 | 62 | 21 | 12 | 5 | `:1482` |
| rasterizer_discard | 51 | 0 | 51 | 0 | 0 | `:1483` |
| read_pixels | 48 | 24 | 24 | 0 | 0 | `:1484` |
| samplers | 42 | 42 | 0 | 0 | 0 | `:1485` |
| shader_api | 52 | 38 | 0 | 14 | 0 | `:1486` |
| state_query | 946 | 874 | 63 | 9 | 0 | `:1488` |
| transform_feedback | 882 | 0 | 461 | 421 | 0 | `:1490` |
| ubo | 2,355 | 2,168 | 187 | 0 | 0 | `:1491` |
| uniform_api | 2,181 | 2,136 | 27 | 18 | 0 | `:1492` |
| vertex_array_objects | 10 | 10 | 0 | 0 | 0 | `:1493` |
| vertex_arrays | 1,005 | 964 | 41 | 0 | 0 | `:1494` |
| **total** | **10,951** | **9,136** | **1,146** | **608** | **5** | |

The mustpass column is a count of names in `gles3-mustpass.txt`; in every row
it equals pass + fail + n/s + other (for example 322 = 272 + 50;
882 = 461 + 421; 100 = 62 + 21 + 12 + 5). The totals: 10,951 = 9,136 + 1,146 +
608 + 5 + 56 (lifetime, no result). 10,951 = 42,810 − 31,859, where 31,859 =
shaders 20,616 + texture 5,105 + fbo 2,028 + fragment_ops 3,170 +
fragment_out 940 (R-group `:1468`, `:1470`, `:1471`, `:1487`, `:1489`).

**There is no per-case or per-subgroup result for these groups in the
repository or the deps cache.** The §1r logs (`build/g3-396d667a/logs/`,
`tally-final-with197.txt`) lived in a worktree that was cleaned up
(`RESULTS.md:1598-1599`); `tools/cts/testdata` holds only parser fixtures.
What the repository does record, and all this map can use:

1. the group totals above;
2. the first-failure message buckets with the groups each reached
   (R-bucket), and for two buckets a split by subgroup (Work `:992`:
   transform_feedback 413 + 48 with interpolation 18 and
   random_full_array_capture 30);
3. the §1q state_query buckets (R-1q), a complete split of that group's 63
   fails and 9 n/s;
4. the host-run splits (Work `:874`, `:879`) — host-only, never a board
   measurement;
5. the per-image tallies (R-img), which mix groups and are not used for
   per-subgroup figures (the sibling fbo map shows why: image 2's filters
   match 1,905 mustpass names but the row reports 1,956 results).

So the "AM62P fails" column below is **"not recorded" unless a figure comes
from 1-4, with the source cited.** A figure derived by arithmetic from
recorded counts is marked "derived"; a match between a fail count and a
subgroup population is marked "a reading" and is not a record.

## 2. Group by group

Columns: subgroup | mustpass | AM62P fail | NotSupported | bucket | dEQP
source | plan item. "not recorded" means the repository has no figure for
that row. A plan item in parentheses after "not recorded" is the group's
assignment, not a per-subgroup record.

### attribute_location — 274 mustpass, 48 fail (R-group `:1458`)

Source for every subgroup: cts `es3fAttribLocationTests.cpp`. The third
level is 274 single-case leaves.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| bind | 24 | not recorded | 0 | not recorded | none |
| bind_hole | 24 | not recorded | 0 | not recorded | none |
| bind_max_attributes | 24 | not recorded | 0 | not recorded | none |
| bind_relink_hole | 24 | not recorded | 0 | not recorded | none |
| layout | 24 | not recorded | 0 | not recorded | none |
| layout_hole | 24 | not recorded | 0 | not recorded | none |
| layout_max_attributes | 24 | not recorded | 0 | not recorded | none |
| mixed | 24 | not recorded | 0 | 133 "Fail (no message)" (R-bucket `:1516`) | none |
| mixed_hole | 24 | not recorded | 0 | 133 (names `attribute_location.mixed*`) | none |
| mixed_max_attributes | 24 | not recorded | 0 | 133 | none |
| mixed_relink_hole | 24 | not recorded | 0 | 133 | none |
| bind_time | 5 | not recorded | 0 | not recorded | none |
| mixed_time | 5 | not recorded | 0 | 133 (names `mixed*`) | none |

Reading (not a record): the only recorded attribution in this group is the
133 bucket naming `mixed*` (five subgroups, 24 + 24 + 24 + 24 + 5 = 101
cases); whether any of the 48 sits in a bind/layout subgroup (through one of
the 62 unlisted buckets) is not recorded. Suspected cause (§3): attribute
location assignment when `glBindAttribLocation` and `layout(location)`
qualifiers are mixed, including relink holes
(`es3fAttribLocationTests.cpp:226-306`).

### buffer — 695 mustpass, all pass (R-group `:1459`)

| subgroup | mustpass | third level | source |
|---|---:|---|---|
| map | 400 | write 176, read_write 120, read 104 | cts `es3fBufferMapTests.cpp` |
| write | 176 | basic 72, basic_subdata 72, partial_specify 13, random 10, recreate_store 9 | cts `es3fBufferWriteTests.cpp` |
| copy | 119 | basic 56, single_buffer 56, subrange 7 | cts `es3fBufferCopyTests.cpp` |

### clipping — 602 mustpass, all pass (R-group `:1460`)

Source: cts `es3fClippingTests.cpp`.

| subgroup | mustpass | third level |
|---|---:|---|
| triangle_vertex | 518 | clip_three 264, clip_two 228, clip_one 26 |
| polygon | 49 | 49 single-case leaves |
| line | 15 | 15 single-case leaves |
| point | 11 | 11 single-case leaves |
| polygon_edge | 9 | 9 single-case leaves |

### color_clear — 19 mustpass, all pass (R-group `:1461`)

Source: cts `es3fColorClearTest.cpp`. 19 single-case subgroups:
complex_{rgb,rgba}, long_{masked_,}{rgb,rgba}, masked_{rgb,rgba},
masked_scissored_{rgb,rgba}, multiple_{rgb,rgba}, scissored_{rgb,rgba},
short_scissored_rgb, single_{rgb,rgba}, subclears_{rgb,rgba}.

### default_vertex_array_object — 1 mustpass, passes (R-group `:1462`)

`vertex_attrib_divisor`. Source: cts `es3fDefaultVertexArrayObjectTests.cpp`.

### default_vertex_attrib — 80 mustpass, 16 fail (R-group `:1463`)

Source: cts `es3fDefaultVertexAttributeTests.cpp`. The third level is 80
single-case leaves.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| float / mat4 / vec4 | 8 each | not recorded | 0 | not recorded | vertex-int-attribs (group) |
| mat2, mat2x3, mat2x4, mat3, mat3x2, mat3x4, mat4x2, mat4x3, vec2, vec3 | 4 each | not recorded | 0 | not recorded | vertex-int-attribs (group) |
| int, ivec2, ivec3, ivec4, uint, uvec2, uvec3, uvec4 | 2 each | not recorded | 0 | not recorded | vertex-int-attribs |

Reading (not a record): the eight int/uint subgroups are exactly 16 cases
(8 × 2), which equals the group's 16 fails. dEQP sets default vertex
attributes of integer types through `glVertexAttribI*`; the float/matrix
subgroups are the ES 2.0 API that already passes. The split is not recorded.

### depth_stencil_clear — 10 mustpass, all pass (R-group `:1464`)

Source: cts `es3fDepthStencilClearTests.cpp`. 10 single-case subgroups:
depth, depth_scissored, depth_scissored_masked, depth_stencil,
depth_stencil_masked, depth_stencil_scissored, stencil, stencil_masked,
stencil_scissored, stencil_scissored_masked.

### dither — 20 mustpass, all pass (R-group `:1465`)

disabled 10, enabled 10. Source: cts `es3fDitheringTests.cpp`.

### draw — 322 mustpass, 50 fail (R-group `:1466`)

Source: cts `es3fDrawTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| random | 151 | not recorded | 0 | not recorded | none |
| draw_elements | 36 | not recorded | 0 | not recorded | none |
| draw_elements_instanced | 35 | not recorded | 0 | not recorded | none |
| draw_arrays | 31 | not recorded | 0 | not recorded | none |
| draw_arrays_instanced | 31 | not recorded | 0 | not recorded | none |
| draw_range_elements | 28 | not recorded | 0 | not recorded | none |
| instancing | 10 | not recorded | 0 | not recorded | none |

Third level: `draw_elements.indices` 8, `draw_elements_instanced.indices` 7;
each of draw_arrays, draw_arrays_instanced, draw_elements,
draw_elements_instanced and draw_range_elements has its seven primitive modes
at 4 cases each (line_loop, line_strip, lines, points, triangle_fan,
triangle_strip, triangles) plus `first` 3 (draw_arrays, draw_arrays_instanced);
random (151) and instancing (10) are single-case leaves. The 50 fails are not
recorded per subgroup; no bucket names draw (R-bucket). Suspected (§3):
`draw.random` image compares.

### draw_buffers_indexed — 76 mustpass, 76 not supported (R-group `:1467`)

Source: cts `es3fDrawBuffersIndexedTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| random | 40 | 0 | 40 (derived) | not exposed (R-1q `:1423-1431`) | none |
| overwrite_common | 18 | 0 | 18 (derived) | not exposed | none |
| overwrite_indexed | 18 | 0 | 18 (derived) | not exposed | none |

Derived: the group row is 0 pass, 0 fail, 76 n/s (`:1467`), so every name in
it is n/s.

Third level: `random.max_implementation_draw_buffers` 20,
`random.max_required_draw_buffers` 20; the two overwrite subgroups are
18 single-case leaves each. The reason is recorded for the §1q state_query
mirror: "the 9 n/s are indexed.blend_* / color_mask_*, draw_buffers_indexed,
which is not exposed" (R-1q `:1423-1431`). Not supporting the extension is
conformant for GLES3 (it is `GL_EXT_draw_buffers_indexed`); the group is
`none`, related to `mrt4` only through the draw-buffer count.

### fence_sync — 10 mustpass, all pass (R-group `:1469`)

Source: cts `es3fSyncTests.cpp`. 10 single-case subgroups:
client_wait_sync_{finish, flush_auto, flush_manual, largedraw, noflush,
smalldraw, timeout_largedraw, timeout_smalldraw},
wait_sync_{largedraw, smalldraw}.

### implementation_limits — 49 mustpass, 16 fail (R-group `:1472`)

Source: cts `es3fImplementationLimitTests.cpp`. All 49 subgroups are
single-case names: aliased_line_width_range, aliased_point_size_range,
compressed_texture_formats, extensions, major_version, max_3d_texture_size,
max_array_texture_layers, max_color_attachments,
max_combined_fragment_uniform_components, max_combined_texture_image_units,
max_combined_uniform_blocks, max_combined_vertex_uniform_components,
max_cube_map_texture_size, max_draw_buffers, max_element_index,
max_elements_indices, max_elements_vertices, max_fragment_input_components,
max_fragment_uniform_blocks, max_fragment_uniform_components,
max_fragment_uniform_vectors, max_program_texel_offset, max_renderbuffer_size,
max_server_wait_timeout, max_texture_image_units, max_texture_lod_bias,
max_texture_size, max_transform_feedback_interleaved_components,
max_transform_feedback_separate_attribs, max_transform_feedback_separate_components,
max_uniform_block_size, max_uniform_buffer_bindings, max_varying_components,
max_varying_vectors, max_vertex_attribs, max_vertex_output_components,
max_vertex_texture_image_units, max_vertex_uniform_blocks,
max_vertex_uniform_components, max_vertex_uniform_vectors,
min_program_texel_offset, minor_version, num_compressed_texture_formats,
num_extensions, num_program_binary_formats, num_shader_binary_formats,
shader_compiler, subpixel_bits, uniform_buffer_offset_alignment.

The 16 fails are not recorded per case. Reading from the Caps gap table
(`b6004e19`; see §5): 13 of the 49 limits are below the ES 3.0 minimum, and
each such limit is exactly one failing case:

| limit case | reported today (Caps) | minimum | plan item |
|---|---|---:|---|
| max_color_attachments | 3 | 4 | mrt4 |
| max_draw_buffers | 3 | 4 | mrt4 |
| max_transform_feedback_interleaved_components | 0 | 64 | tf-api |
| max_transform_feedback_separate_attribs | 0 | 16 | tf-api |
| max_transform_feedback_separate_components | 0 | 128 | tf-api |
| max_uniform_block_size | 6,144 | 16,384 | ubo-uniform |
| max_combined_fragment_uniform_components | 1,536 | 50,048 | ubo-uniform |
| max_combined_vertex_uniform_components | 1,536 | 50,176 | ubo-uniform |
| num_compressed_texture_formats | 3 | 10 | none (owner `formats-*`) |
| max_varying_vectors | 8 | 15 | none |
| max_varying_components | 32 | 60 | none |
| max_fragment_input_components | 32 | 60 | none |
| max_vertex_output_components | 32 (V3D/GC: 60/32) | 64 | none |

That accounts for 13 of the 16; the other 3 are not recorded. The Caps
measurement is from `b6004e19` (2026-10-08), not from `396d667a`; the limits
are assumed unchanged between them (an assumption, §7).

### info — 6 mustpass, all pass (R-group `:1473`)

Source: `modules/gles3/tes3InfoTests.cpp` (not under `functional/`).
extensions, render_target, renderer, shading_language_version, vendor,
version. R-1p `:1410`: 6/6 pass, printing "OpenGL ES 3.0 YakoGL" and
"OpenGL ES GLSL ES 3.00".

### instanced — 45 mustpass, 6 fail (R-group `:1474`)

Source: cts `es3fInstancedRenderingTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| types | 21 | not recorded | 0 | 133 "Fail (no message)" (names `instanced`, `:1516`) | vertex-int-attribs (reading) |
| draw_arrays_instanced | 12 | not recorded | 0 | not recorded | none |
| draw_elements_instanced | 12 | not recorded | 0 | not recorded | none |

Third level: each of draw_arrays_instanced and draw_elements_instanced has
attribute_divisor 4, instance_id 4, mixed 4; types is 21 single-case leaves.
Reading (not a record): `types` is the only instanced subgroup that uses
`glVertexAttribIPointer` with `GL_INT` / `GL_UNSIGNED_INT`
(`es3fInstancedRenderingTests.cpp:515-519`); Caps row 12 says instancing
itself is OK. The 6-case split is not recorded.

### lifetime — 56 mustpass, no result (R-life `:1524-1531`)

Source: cts `es3fLifetimeTests.cpp`. The group's init builds a program with
interleaved transform feedback of `gl_Position` (`es3fLifetimeTests.cpp:97`);
transform feedback is not implemented, the link fails, dEQP ends the run with
`die()`. All 56 cases have no result. (The Zephyr data abort that `die()`
caused was fixed at `50aa3958`; the group stays unmeasurable until transform
feedback exists.)

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| attach | 10 (deleted_name 4, deleted_input 3, deleted_output 3) | no result | — | — | tf-api / tf-capture |
| delete | 10 | no result | — | — | tf-api / tf-capture |
| gen | 10 | no result | — | — | tf-api / tf-capture |
| bind | 8 | no result | — | — | tf-api / tf-capture |
| bind_no_gen | 8 | no result | — | — | tf-api / tf-capture |
| delete_bound | 8 | no result | — | — | tf-api / tf-capture |
| delete_active | 1 | no result | — | — | tf-api / tf-capture |
| delete_used | 1 | no result | — | — | tf-api / tf-capture |

`bind` uses `glBeginQuery(ANY_SAMPLES_PASSED)` as its "bind" action
(`es3fLifetimeTests.cpp:195-215`, OQ), so `occlusion-query` is a second
owner once transform feedback unblocks the group.

### multisample — 64 mustpass, 16 fail, 48 n/s (R-group `:1475`)

Source: cts `es3fMultisampleTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| default_framebuffer | 16 | not recorded | 0 | not recorded | none |
| fbo_4_samples | 16 | not recorded | 16 (reading) | not recorded | none |
| fbo_8_samples | 16 | not recorded | 16 (reading) | not recorded | none |
| fbo_max_samples | 16 | not recorded | 16 (reading) | not recorded | none |

All 64 are single-case leaves. Reading (not a record): the 48 n/s are the
three `fbo_*` subgroups, whose sample counts exceed MAX_SAMPLES = 1 (Caps);
the 16 fails are then `default_framebuffer`. Not recorded. Caps names the
gap "MAX_SAMPLES 4 required, 1 today — new: MSAA item; not in the plan", so
the plan item is `none` (§3).

### negative_api — 312 mustpass, 31 fail, 4 n/s (R-group `:1476`)

| subgroup | mustpass | AM62P fail | n/s | dEQP source | plan item |
|---|---:|---:|---:|---|---|
| texture | 103 | not recorded | not recorded | cts `es3fNegativeTextureApiTests.cpp` | see host split below |
| shader | 62 | not recorded | not recorded | cts `es3fNegativeShaderApiTests.cpp` | see host split below |
| state | 56 | not recorded | not recorded | cts `es3fNegativeStateApiTests.cpp` | see host split below |
| buffer | 37 | not recorded | not recorded | cts `es3fNegativeBufferApiTests.cpp` | see host split below |
| fragment | 27 | not recorded | not recorded | cts `es3fNegativeFragmentApiTests.cpp` | see host split below |
| vertex_array | 27 | not recorded | not recorded | cts `es3fNegativeVertexArrayApiTests.cpp` | see host split below |

All 312 are single-case leaves. The board's 31 fails and 4 n/s are not
recorded per subgroup. The closest recorded split is the host run at
`8b8c5ff8` (Work `:879`, HOST-ONLY): 32 of 312 left, named as transform
feedback 13 + bind_buffer TF 2 (→ `tf-api`), queries 4 (→ `occlusion-query`
/ `oq-gl` / `oq-v3d` / `oq-gc`; OQ cites the same 4 at Work `:874`), integer
formats 6 (→ `int-rt-gl`), blit/multisample 3 (→ none), uniformui-on-int 2
(→ `ubo-uniform`, reading), fbo attachment pnames 1 (→ `gl-core-misc`), fbo
read-pixels 1 (→ `gl-core-misc` / `int-rt-gl`). The earlier host run
`01de3104` (Work `:874`) named the same families. dEQP pins the query API's
error behaviour in `es3fNegativeFragmentApiTests.cpp:305-380` and
`es3fNegativeStateApiTests.cpp:1015-1050, 1240-1262` (OQ).

### occlusion_query — 62 mustpass, 62 fail (R-group `:1477`)

Source: cts `es3fOcclusionQueryTests.cpp`. All 62 are single-case names:
all_occluders (1), conservative_* (31), depth_* (12), scissor_* (15),
stencil_* (3). Every case fails: the query API is a stub
(`src/gl/es3.rs:233-329` per OQ — `glBeginQuery` answers
`GL_INVALID_OPERATION`, `glGetQueryObjectuiv` always fails). Plan item:
`occlusion-query` (design) with `oq-gl` (GL core), `oq-v3d` (V3D 4.2) and
`oq-gc` (GC7000UL) as the implementation items.

### pbo — 49 mustpass, 26 fail, 6 n/s (R-group `:1478`)

Source: cts `es3fPixelBufferObjectTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| renderbuffer | 47 | not recorded | not recorded | not recorded | gl-core-misc; int-rt-gl (reading) |
| native | 2 | not recorded | not recorded | not recorded | gl-core-misc |

All 49 are single-case leaves. Reading: the tests read the default
renderbuffer back through `glReadPixels` into a buffer object, with float,
integer and unsigned types; the integer types are `int-rt-gl`'s readback
territory. Not recorded which of the 26 are integer types, nor what the 6
n/s are.

### polygon_offset — 20 mustpass, all pass (R-group `:1479`)

Source: cts `es3fPolygonOffsetTests.cpp`. {default, fixed16, fixed24,
float32} × {displacement_with_units, enable, factor_0_slope,
render_with_factor, render_with_units}, 1 case each.

### prerequisite — 3 mustpass, all pass (R-group `:1480`)

clear_color, read_pixels, state_reset. Source: cts
`es3fPrerequisiteTests.cpp`.

### primitive_restart — 504 mustpass, all pass (R-group `:1481`)

Source: cts `es3fPrimitiveRestartTests.cpp`. Eight subgroups of 63
(basic, begin_restart, begin_restart_duplicate_restarts,
begin_restart_end_restart, begin_restart_end_restart_duplicate_restarts,
duplicate_restarts, end_restart, end_restart_duplicate_restarts), each with
its seven primitive modes at 9 cases.

### rasterization — 100 mustpass, 21 fail, 12 n/s, 5 QualityWarning (R-group `:1482`)

Source: cts `es3fRasterizationTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| fbo | 46 | not recorded | not recorded | not recorded | none |
| culling | 18 | not recorded | 0 | not recorded | none |
| interpolation | 12 | not recorded | 0 | not recorded | none |
| primitives | 10 | not recorded | 0 | not recorded | none |
| flatshading | 9 | not recorded | 0 | not recorded | none |
| fill_rules | 5 | not recorded | 0 | not recorded | none |

Third level: `fbo.rbo_multisample_4` 12, `fbo.rbo_multisample_max` 12,
`fbo.rbo_singlesample` 11, `fbo.texture_2d` 11; `interpolation.basic` 6,
`interpolation.projected` 6; the rest single-case leaves. Reading (not a
record): the 12 n/s are one of the two `rbo_multisample_*` subgroups (samples
above MAX_SAMPLES = 1, Caps). The 21 fails and the 5 QualityWarning are not
recorded per subgroup; no bucket names rasterization (R-bucket).

### rasterizer_discard — 51 mustpass, 51 fail (R-group `:1483`)

Source: cts `es3fRasterizerDiscardTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| basic | 17 | 17 (derived) | 0 | 133 "Fail (no message)" (names `rasterizer_discard`, `:1516`) | tf-capture |
| fbo | 17 | 17 (derived) | 0 | 133 | tf-capture |
| scissor | 17 | 17 (derived) | 0 | 133 | tf-capture |

Derived: the group row is 0 pass, 51 fail (`:1483`) and the group has 51
names, so every case fails.

All 51 are single-case leaves. TF records the cause: `GL_RASTERIZER_DISCARD`
is stored but changes nothing (`src/gl/state.rs:434-436` at `f818f84e`), so
discarded draws still write depth/stencil/colour, and 9 of every 17 cases are
about `glClear` under discard.

### read_pixels — 48 mustpass, 24 fail (R-group `:1484`)

Source: cts `es3fReadPixelsTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| alignment | 16 | not recorded | 0 | not recorded | gl-core-misc; int-rt-gl (reading) |
| rowlength | 16 | not recorded | 0 | not recorded | gl-core-misc; int-rt-gl (reading) |
| skip | 16 | not recorded | 0 | not recorded | gl-core-misc; int-rt-gl (reading) |

All 48 are single-case leaves. Exactly half the group fails; the split is not
recorded. Reading: the tests read back float, integer and unsigned types; the
integer types are `int-rt-gl`'s readback territory. The host run honoured the
pixel-store skips and pack row length (Work `:879`), so the board's 24 are a
board-side gap.

### samplers — 42 mustpass, all pass (R-group `:1485`)

Source: cts `es3fSamplerObjectTests.cpp`. Six subgroups of 7:
multi_{cubemap, tex_2d, tex_3d}, single_{cubemap, tex_2d, tex_3d}.

### shader_api — 52 mustpass, 14 n/s (R-group `:1486`)

Source: cts `es3fShaderApiTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| shader_source | 20 | 0 | 0 | — | none |
| program_binary | 14 | 0 | 14 (derived) | not recorded | none |
| program_state | 12 | 0 | 0 | — | none |
| compile_link | 3 | 0 | 0 | — | none |
| create_delete | 3 | 0 | 0 | — | none |

Third level: `program_binary.binary_persistence` 12, `program_binary.simple`
2; the rest single-case leaves. Derived: 52 − 38 pass = 14 n/s, and
`program_binary` is exactly 14 names, so the 14 n/s are the program_binary
subgroup. Reading: `glGetProgramBinary` is not implemented, which is what
makes dEQP report the subgroup as not supported. No plan item covers program
binaries (§3).

### state_query — 946 mustpass, 63 fail, 9 n/s (R-group `:1488`)

Level-2 subgroups (all 946 third-level entries are single-case leaves):

| subgroup | mustpass | dEQP source |
|---|---:|---|
| integers | 517 | cts `es3fIntegerStateQueryTests.cpp` |
| texture | 134 | cts `es3fTextureStateQueryTests.cpp` |
| boolean | 87 | cts `es3fBooleanStateQueryTests.cpp` |
| shader | 47 | cts `es3fShaderStateQueryTests.cpp` |
| floats | 44 | cts `es3fFloatStateQueryTests.cpp` |
| internal_format | 35 | cts `es3fInternalFormatQueryTests.cpp` |
| sampler | 18 | cts `es3fSamplerStateQueryTests.cpp` |
| fbo | 15 | cts `es3fFboStateQueryTests.cpp` |
| integers64 | 15 | cts `es3fInteger64StateQueryTests.cpp` |
| indexed | 14 | cts `es3fIndexedStateQueryTests.cpp` |
| buffer_object | 11 | cts `es3fBufferObjectQueryTests.cpp` |
| rbo | 5 | cts `es3fRboStateQueryTests.cpp` |
| string | 4 | cts `es3fStringQueryTests.cpp` |

The 63 fails and 9 n/s have a complete recorded split (R-1q `:1423-1431`;
the §1q run's totals equal the §1r row, so the buckets apply):

| bucket | fails | cases (prefix after `state_query.`) | plan item |
|---|---:|---|---|
| transform feedback | 29 | boolean.transform_feedback_*, indexed.transform_feedback_*, integers.transform_feedback_*binding*, integers.max_transform_feedback_*, shader.transform_feedback | tf-api |
| an ES 3.0 limit read as float or int64 | 18 | integers.max_{color_attachments, combined_texture_image_units, draw_buffers, fragment_input_components, samples, varying_components, varying_vectors, vertex_output_components, vertex_texture_image_units}_{getfloat, getinteger64} | gl-core-misc (the limit values themselves: mrt4 / none, §5) |
| 64-bit uniform limits | 6 | integers64.max_{combined_fragment_uniform_components, combined_vertex_uniform_components, uniform_block_size}_{getfloat, getinteger} | gl-core-misc (values: ubo-uniform, §5) |
| compressed texture formats list | 3 | integers.compressed_texture_formats_{getfloat, getinteger, getinteger64} | none (owner `formats-*`) |
| fbo attachment queries | 3 | fbo.framebuffer_attachment_{component_type, x_size_rbo, x_size_texture} | gl-core-misc |
| renderbuffer queries | 3 | rbo.renderbuffer_{component_size_color, internal_format, samples} | gl-core-misc |
| other | 1 | shader.program_binary | none |
| **total** | **63** | 29 + 18 + 6 + 3 + 3 + 3 + 1 = 63 | |

The 9 n/s are `indexed.blend_*` / `color_mask_*` — the draw_buffers_indexed
pnames, "which is not exposed" (R-1q `:1423-1431`).

### transform_feedback — 882 mustpass, 461 fail, 421 n/s (R-group `:1490`)

Source: cts `es3fTransformFeedbackTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| array | 378 (interleaved 189, separate 189) | not recorded | not recorded | 413 "glTransformFeedbackVaryings(): GL_INVALID_VALUE" (R-bucket `:1508`) | tf-api |
| basic_types | 378 (interleaved 189, separate 189) | not recorded | not recorded | 413 | tf-api |
| random_full_array_capture | 60 (interleaved 30, separate 30) | 30 (Work `:992`) | not recorded | 48 "Compile failed: ''" | tf-infolog (the 30); tf-api (the rest) |
| interpolation | 54 (centroid 18, flat 18, smooth 18) | 18 (Work `:992`) | not recorded | 48 "Compile failed: ''" | tf-infolog (the 18); tf-api (the rest) |
| point_size | 6 | not recorded | not recorded | 413 | tf-api |
| position | 6 | not recorded | not recorded | 413 | tf-api |

Recorded totals: 461 = 413 + 48 (Work `:992`); the 413 are named only as
`transform_feedback.*` (no subgroup split), the 48 as interpolation 18 +
random_full_array_capture 30. The 421 n/s are not recorded per subgroup; TF
records the mechanism: dEQP reads MAX_TRANSFORM_FEEDBACK_* = 0
(`src/gl/state.rs:1137-1139`) and skips the cases. The 48 are kept as a
separate defect (`tf-infolog`, Work `:994`): a compile refusal with an empty
info log, whatever transform feedback's state. `tf-capture` owns the capture
write path; `tf-api` owns the refusal path (`glTransformFeedbackVaryings`
`GL_INVALID_VALUE` at `src/gl/es3.rs:390-416`, `glBindBufferBase/Range`
`GL_INVALID_OPERATION` at `:959-961, 1014-1016`,
`glBeginQuery(TF_PRIMITIVES_WRITTEN)` `GL_INVALID_OPERATION` at `:285`, all
at `f818f84e`).

### ubo — 2,355 mustpass, 187 fail (R-group `:1491`)

Source: cts `es3fUniformBlockTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| single_basic_type | 968 (shared 363, std140 363, packed 242) | not recorded | 0 | not recorded | ubo-uniform |
| instance_array_basic_type | 344 (shared 129, std140 129, packed 86) | not recorded | 0 | not recorded | ubo-uniform |
| single_basic_array | 344 (shared 129, std140 129, packed 86) | not recorded | 0 | not recorded | ubo-uniform |
| random | 323 (all_shared_buffer 50, all_per_block_buffers 49, eight families at 25, nested_structs_arrays 24) | not recorded | 0 | 1,293 "Image compare failed" (names `ubo.random`, `:1503`) | ubo-uniform |
| single_nested_struct_mixed_matrix_packing | 192 (per_block_buffer 128, single_buffer 64) | not recorded | 0 | 1,293 (names `ubo.single_nested_struct*`) | ubo-uniform |
| multi_basic_types | 44 (per_block_buffer 22, single_buffer 22) | not recorded | 0 | not recorded | ubo-uniform |
| multi_nested_struct | 44 (per_block_buffer 22, single_buffer 22) | not recorded | 0 | 1,293 | ubo-uniform |
| single_nested_struct | 24 (per_block_buffer 16, single_buffer 8) | not recorded | 0 | 1,293 | ubo-uniform |
| single_nested_struct_array | 24 (per_block_buffer 16, single_buffer 8) | not recorded | 0 | 1,293 | ubo-uniform |
| single_struct | 24 (per_block_buffer 16, single_buffer 8) | not recorded | 0 | not recorded | ubo-uniform |
| single_struct_array | 24 (per_block_buffer 16, single_buffer 8) | not recorded | 0 | not recorded | ubo-uniform |

The 187 are not recorded per subgroup. The 1,293-case bucket names
`ubo.single_nested_struct*` (284 cases: 192 + 44 + 24 + 24) and `ubo.random`
(323) and says "not split yet" (`:1503`); 284 + 323 = 607 ≥ 187, so the
named subgroups can hold all 187, but which ones is not recorded. The limits
behind part of the group are in §5 (MAX_UNIFORM_BLOCK_SIZE 6,144 < 16,384).

### uniform_api — 2,181 mustpass, 27 fail, 18 n/s (R-group `:1492`)

Source: cts `es3fUniformApiTests.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| value | 1,422 (assigned 1,014, initial 408) | not recorded | not recorded | not recorded | ubo-uniform |
| info_query | 660 (indices_active_uniformsiv 240, active_uniform 210, consistency 210) | not recorded | not recorded | not recorded | ubo-uniform |
| random | 99 (99 single-case leaves) | not recorded | not recorded | not recorded | ubo-uniform |

The 27 fails and 18 n/s are not recorded per subgroup, and no bucket names
uniform_api (R-bucket). The host run kept one deviation here: setting a
`uniformui` with an int setter (Work `:879`, "Qt's deviation, kept").

### vertex_array_objects — 10 mustpass, all pass (R-group `:1493`)

Source: cts `es3fVertexArrayObjectTests.cpp`. all_attributes, diff_buffer,
diff_divisor, diff_indices, diff_integer, diff_normalize, diff_offset,
diff_size, diff_stride, diff_type.

### vertex_arrays — 1,005 mustpass, 41 fail (R-group `:1494`)

Source: cts `es3fVertexArrayTest.cpp`.

| subgroup | mustpass | AM62P fail | n/s | bucket | plan item |
|---|---:|---:|---:|---|---|
| single_attribute | 802 | not recorded | 0 | not recorded | vertex-int-attribs |
| multiple_attributes | 203 | not recorded | 0 | not recorded | vertex-int-attribs |

Third level: single_attribute — output_types 282, usages 234, strides 136,
normalize 58, first 52, offset 40; multiple_attributes — input_types 125,
stride 64, attribute_count 7, storage 7. The 41 are not recorded per
subgroup. Reading: `output_types` and `input_types` are the ES 3.0 input
types beyond float — `INT`, `UNSIGNED_INT`, `HALF`, `FIXED`,
`INT_2_10_10_10_REV` (`es3fVertexArrayTest.cpp:538-543`); GLES2's
vertex_arrays passed on every board, so the new types are the suspected
cause. `vertex-int-attribs` is the item for non-float vertex attributes
(`glVertexAttribIPointer`, refused today as "an attribute format other than
float32", `docs/online-shaderc/v3d-backend.md:3334`).

## 3. Failing subgroups with no plan item, ranked

Ranked by recorded AM62P fail count (n/s in parentheses). Entries with 0
fails are listed separately below. The 3 implementation_limits fails that no
named limit gap explains are not here: the item scope gives
implementation_limits to `gl-core-misc` (§4).

1. **draw — 50 fail.** Not recorded per subgroup; no bucket names it.
   Suspected: `draw.random` (151 random image-compare cases). Needs a
   per-subgroup rerun.
2. **attribute_location — 48 fail.** Named in the 133 "Fail (no message)"
   bucket as `mixed*` (`:1516`); the split among the five mixed* subgroups is
   not recorded. Suspected: location assignment when `glBindAttribLocation`
   and `layout(location)` are mixed, including relink holes.
3. **rasterization — 21 fail, 12 n/s, 5 QW.** Not recorded per subgroup.
   Suspected: the `fbo.rbo_multisample_*` cases (MAX_SAMPLES 1, Caps) and
   unknown others.
4. **multisample — 16 fail (48 n/s).** MAX_SAMPLES 1 vs the required 4; Caps
   names "a new MSAA item; not in the plan". The plan has no MSAA item —
   this is the largest gap with no owner at all.
5. **implementation_limits varying limits — 4 fail** (max_varying_vectors,
   max_varying_components, max_fragment_input_components,
   max_vertex_output_components). Caps says these need a usc measurement /
   V3D board run; no plan item covers them.
6. **negative_api — 3 fail (4 n/s) outside named families** (host split,
   Work `:879`: blit/multisample 3). The board's 4 n/s are not recorded.
7. **state_query.compressed_texture_formats list — 3 fail.** Owner is the
   `formats-*` family (NUM_COMPRESSED_TEXTURE_FORMATS 3 vs 10, Caps), which
   is outside this map's item list.

NotSupported only (0 fails), no plan item:

- **shader_api.program_binary — 14 n/s (derived).** Suspected:
  `glGetProgramBinary` not implemented. No plan item covers program
  binaries.
- **draw_buffers_indexed — 76 n/s.** The extension is not exposed
  (recorded, R-1q `:1423-1431`). Conforming for GLES3; no item needed unless
  the extension is wanted (then it pairs with `mrt4`).

## 4. Plan-item scope, as this map sees it

| plan item | groups / subgroups (recorded fails) | cases |
|---|---|---:|
| tf-infolog | transform_feedback.interpolation 18 + random_full_array_capture 30 — the empty info log (Work `:994`) | 48 |
| tf-api | transform_feedback 413 + 421 n/s; state_query transform feedback 29; negative_api TF 13 + 2 (host); implementation_limits max_transform_feedback_* 3 (reading); lifetime 56 blocked | 413 + 29 + 3 recorded; 421 + 56 blocked |
| tf-capture | rasterizer_discard 51; the transform_feedback capture write path | 51 |
| occlusion-query / oq-gl / oq-v3d / oq-gc | occlusion_query 62; negative_api queries 4 (host); lifetime.bind once TF exists | 62 |
| ubo-uniform | ubo 187; uniform_api 27 + 18 n/s; state_query 64-bit uniform limits 6; implementation_limits uniform limits 3 (reading); negative_api uniformui-on-int 2 (host) | 187 + 27 recorded |
| vertex-int-attribs | vertex_arrays 41; default_vertex_attrib 16 (reading); instanced 6 (reading) | 41 + 16 + 6 recorded |
| gl-core-misc | state_query 34 (63 − 29 TF); negative_api (rest of 31); implementation_limits 3 unattributed; pbo 26 + 6 n/s; read_pixels 24 | 34 + 31 + 3 + 26 + 24 recorded |
| int-rt-gl | read_pixels integer types (reading); pbo integer readback (reading); negative_api integer formats 6 (host) | 6 recorded (host) |
| mrt4 | implementation_limits max_color_attachments + max_draw_buffers 2 (reading); MAX_DRAW_BUFFERS / MAX_COLOR_ATTACHMENTS 3 vs 4 (Caps) | 2 (reading) |
| none | §3: draw 50, attribute_location 48, rasterization 21, multisample 16 (48 n/s), shader_api 14 n/s, draw_buffers_indexed 76 n/s, compressed formats 3, varying limits 4, negative_api outside named families 3 (its 4 n/s not counted: board value not recorded) — the same entries as §3 | 145 fail + 138 n/s |

## 5. Limits behind the failures (Caps `b6004e19`, reverted at `3f78982e`)

`docs/es3-backend-caps.md` measured the reported limits against the ES 3.0
minimums. It is not at HEAD (reverted at `3f78982e`); the rows below are from
`b6004e19`. Gaps that this map's groups hit:

- MAX_COLOR_ATTACHMENTS / MAX_DRAW_BUFFERS: 3 today, 4 required → `mrt4`
  (implementation_limits 2 cases, reading; draw_buffers_indexed n/s).
- MAX_SAMPLES: 1 today, 4 required → no plan item (multisample 16 + 48 n/s,
  rasterization 12 n/s reading).
- MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS / SEPARATE_ATTRIBS /
  SEPARATE_COMPONENTS: 0 today, 64 / 16 / 128 required → `tf-api`
  (implementation_limits 3 cases, reading; transform_feedback 421 n/s).
- MAX_UNIFORM_BLOCK_SIZE: 6,144 today, 16,384 required;
  MAX_COMBINED_{FRAGMENT,VERTEX}_UNIFORM_COMPONENTS: 1,536 today,
  50,048 / 50,176 required → `ubo-uniform` (implementation_limits 3 cases,
  reading; state_query integers64 6).
- NUM_COMPRESSED_TEXTURE_FORMATS: 3 today, 10 required → `formats-*`
  (state_query 3, implementation_limits 1).
- MAX_VARYING_VECTORS 8 vs 15, MAX_VARYING_COMPONENTS 32 vs 60,
  MAX_FRAGMENT_INPUT_COMPONENTS 32 vs 60, MAX_VERTEX_OUTPUT_COMPONENTS
  32/60/32 vs 64 → no plan item (implementation_limits 4, reading).
- OK rows relevant here: uniform blocks 12 + 12 and 24 bindings; 16 vertex
  attributes; instancing. Occlusion queries: GAP (`occlusion-query`).

## 6. Next steps

- `es3-inventory` (docs/es3-cts-failures.md) can paste §1-§4 as the
  non-shader/texture/fbo half of the inventory.
- A per-subgroup split is the single biggest missing fact. A host run
  (`tools/cts/host-deqp-run.sh`) or a narrow board filter over
  `draw.*`, `attribute_location.*`, `vertex_arrays.*`, `ubo.*`,
  `uniform_api.*`, `read_pixels.*`, `pbo.*`, `negative_api.*`,
  `multisample.*`, `rasterization.*`, `instanced.*`, `implementation_limits.*`
  would turn most "not recorded" cells into figures.
- `gl-core-misc` scope from §4: state_query 34 (limits read as float/int64,
  fbo/rbo pnames), pbo, read_pixels, negative_api rest, implementation_limits
  3 unattributed.
- `ubo-uniform` scope: ubo 187 + uniform_api 27 + the two uniform limit
  gaps.
- `vertex-int-attribs` scope: vertex_arrays 41, default_vertex_attrib 16,
  instanced 6 — all three suspected to be the ES 3.0 non-float attribute
  types.
- `mrt4` and the MSAA decision: multisample (64 cases) and
  draw_buffers_indexed (76 n/s) wait on MAX_SAMPLES / MAX_DRAW_BUFFERS;
  MSAA has no plan item (Caps).

## 7. Assumptions

- The §1q buckets (run `9134800c`) apply to the §1r state_query row because
  the two runs report identical totals (874 / 63 / 9); RESULTS.md `:1433-1435`
  states the same.
- The Caps limits at `b6004e19` (2026-10-08) equal the limits at `396d667a`
  (2026-09-26) for the 13 named gaps; not re-measured at `396d667a`.
- Host-run splits (Work `:874`, `:879`) describe the same code gaps as the
  board's but are not board measurements; they are cited as host-only.
- "not recorded" cells are left empty rather than estimated; readings
  (count matches like default_vertex_attrib 16 = 16 int/uint cases,
  shader_api 14 n/s = program_binary 14 names) are labelled as readings.
- The plan-item list used is the one this item names
  (tf-infolog, tf-api, tf-capture, occlusion-query, oq-gl, oq-v3d, oq-gc,
  ubo-uniform, vertex-int-attribs, gl-core-misc, int-rt-gl, mrt4) plus
  `none`; where the true owner is outside that list (`formats-*`, MSAA,
  program binaries) the row says `none` and names the owner in prose.

## Check

This map covers 10,951 mustpass cases. The sibling maps cover 31,859
(shader map 20,616; texture map 5,105; fbo-fragops map 6,138).
10,951 + 31,859 = 42,810 — the four maps together cover the whole
`gles3-mustpass.txt`, with no group covered twice and none left out.
(At landing, the texture map's 5,105 names have no map of their own on
main; the other three maps are on main and cover 37,705 = 42,810 − 5,105.)
