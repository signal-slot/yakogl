<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the program's vertex shader for the fragment reads, case 19

gl_test case 19 es100 (#33) was refused at glLinkProgram with
`a Mali link whose fragment shader reads part of a varying (the link keeps
only those components) is not lowered yet ('Tint')`
(tools/cts/log/2026-11-07-t6-step4a, both sweeps). The program is the
second of the case, the PowerVR SDK textured pair:
`tools/shaders/es100/textured.vert` and `textured.frag`, the same text as
`tools/mali/testdata/vsfp16_19_tex.{vert,frag}`. Its first program
(HelloAPI, `prec_19`) linked before.

## What the fragment shader reads

The fragment shader reads `Tint` whole and `TexCoord.xy`, all of both.
It does not read part of a varying. The refusal came from a location
mix-up in the link step:

- Varying locations are each stage's declaration order (`reflect.rs`,
  `varying_loc`). The vertex shader declares `TexCoord` (0), `Tint` (1),
  `Unused` (2). The fragment shader declares `Tint` (0), `TexCoord` (1).
- The vertex shader stores a constant lane (`Unused = 0.5`). So the link
  builds both stages for the program (`backend.rs` `vs_for_fragment`): the
  fragment shader with the constants (`compile_fs_for_vs`), then the vertex
  shader for that record's reads (`compile_vs_for_reads`). The same path
  runs for a fragment shader that reads a varying in part
  (`mali::partial_reads`).
- The record's reads are `(location, mask)` at the **fragment** locations:
  `[(0, 0xf), (1, 0x3)]`. `compile_vs_for_reads` takes them as **vertex**
  locations (`nir.rs` `compact_layout`, `motion::apply`). So it stored
  `TexCoord.xy` at offset 0 and `Tint.xy` at 8. Where the fragment shader
  reads `Tint` at offset 0 with mask `0xf`, the vertex slot of `Tint` had
  mask `0x3`. `link.rs` `pair` reported that as `MALI_PARTIAL_READ`.

Before this change, `tests.rs` `the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1`
passed for `vsfp16_19_tex` only because it gave `compile_vs_for_reads` the
reads at the vertex locations by hand (`[(1, 0xf), (0, 0x3)]`,
docs/notes/mali-precision.md Part 1). The GL path had no such mapping.

## What Mesa does

Mesa's GLSL linker pairs an output with an input by name
(`gl_nir_link_varyings.c:3310-3336` `get_matching_input`, a hash look-up
of `output_var->name` in the consumer's inputs). It then gives both
stages one location per pair (`assign_initial_varying_locations`,
`:3602-3606`), and `nir_opt_varyings` removes the unread `Unused` and
keeps only the components read. The Valhall layout is the producer's slot
order. On drm-shim (`vsfp16_19_tex.expect.json`, Mesa 26.2.1,
`--uniform-values`), the varying program stores `Tint` (attribute index 2)
in words 0-3 and `TexCoord` (index 1) in words 4-5 (`vs.varying`: four
`STORE.i32.estream` of `r2..r5` after `LD_ATTR_IMM.v4 ... index:0x2`,
then two of `LD_ATTR_IMM.v2 ... index:0x1`). The fragment shader's
`LD_VAR_BUF_IMM` reads `Tint` at 0 and `TexCoord` at 16. Nothing is
stored for `Unused`.

## The lowering

`link.rs` `reads_at_vertex_locations(vs_glsl, fs_glsl, reads)` takes the
fragment record's reads to the vertex shader's locations. Both sources go
through the front end's interface (`reflect::interface`). Fragment
location `l` lies in the fragment row with the greatest location `<= l`.
It maps to the vertex row of the same name, at the same distance from the
row's first location, so a matrix column or array element keeps its
place. The moved expressions (motion.rs) carry fragment locations as
well: each one's slot and every `Node::Lane` in it are mapped the same
way, then encoded again.

`backend.rs` `vs_for_fragment` applies it to the reads before it builds
the program's vertex shader (one line). The tests' `compile_linked` does
the same. A pair whose stages declare their varyings in the same order
maps to the same locations, so every earlier reference stays byte-equal.

If either source does not reflect, or a read has no vertex output of its
name, the reads are returned unchanged. The link then refuses by its own
rules ("is not written by the vertex shader"), as before.

`MALI_PARTIAL_READ` stays. It is now met only by a vertex shader that was
not built for the program.

## Checked by

- `src/shaderc/online/mali/tests.rs`
  `case_19_textured_pair_reads_its_varyings_by_name`: the record's reads
  `[(0, 0xf), (1, 0x3)]` map to `[(1, 0xf), (0, 0x3)]`. Both stages and
  every fact are byte-equal to `vsfp16_19_tex.expect.json`. The layouts
  are `Tint` at 0 mask `0xf` and `TexCoord` at 16 mask `0x3` in both
  stages (vertex locations 1 and 0, fragment 0 and 1), with no store for
  `Unused`.
- `the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1`: the hand
  mapping is gone. All four `VSFP16_POPULATION` pairs go through
  `compile_linked`.
- `src/shaderc/online/backend.rs`
  `case_19_textured_pair_links_through_the_entry_points`: the textured
  pair goes through `online_compile`, `online_compile_vs_linked` and
  `online_link`, as `gl::program` calls them. The program links, and its
  `fs.fragment`, `vs.position` and `vs.varying` words are Mesa's.
- `link.rs` `fragment_reads_move_to_the_vertex_locations_of_their_names`:
  a `vec4`, `mat2`, `vec2` set declared in reverse order maps column 1 of
  the matrix to its own column. A moved expression's slot and lane are
  mapped too. A pair in one order, and a read with no vertex output of its
  name, are left as they were.

The corpus verdicts (`tools/mali/corpus-verdicts.txt`) do not move:
`textured.vert` and `.frag` were `ok` alone, and the refusal was the
link's. The board run (case 19 x2, then the sweep) is plan #336's
board-sweep item.

## Not covered

A fragment shader that reads a varying in part *and* declares its
varyings in another order than the vertex shader. The vertex shader is
now built for the right lanes, but it lays them out in its own slot order
(`compact_layout`). The fragment layout (`nir_fs.rs` `layout`) packs them
in the fragment order, and the `VaryingOrder` recompile is not tried for
a compacted record (`link.rs` `pair`, `orderable`). Such a pair is
refused as `MALI_LAYOUTS_DIFFER`. No gl_test case has this shape, and no
reference draws it.
