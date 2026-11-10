<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): a fragment shader reading part of the varyings (gl_test case 19)

gl_test case 19 (`es100`, issue #33) drew nothing on the NanoPC-T6. Its link
was refused with `REFUSED link: online shaderc: a Mali link whose fragment
shader reads part of a varying (the link keeps only those components) is not
lowered yet ('Tint')` (tools/cts/log/2026-11-07-t6-step4a/, sweeps 1 and 2).
Its fp16 vertex uniforms were already byte-equal (561bad55,
`mali-precision.md` Part 1). This note covers the link.

## The pair

The program is the corpus's `es100/textured.vert` and `es100/textured.frag`
(`driver/tools/pvrlab/gl_test_shaders_es100.h`, the same text as
`tools/mali/testdata/vsfp16_19_tex.{vert,frag}`):

| stage | varyings, in declaration order | location |
|---|---|---|
| vertex | `mediump vec2 TexCoord`, `lowp vec4 Tint`, `mediump float Unused` (`= 0.5`) | 0, 1, 2 |
| fragment | `lowp vec4 Tint`, `mediump vec2 TexCoord` | 0, 1 |

The fragment shader reads all of `Tint` and `TexCoord`. It does not declare
`Unused`. The vertex shader stores a constant in `Unused`, so
`vs_constants` finds a lane. "Part of the varyings" here means only some of
the vertex shader's outputs are read; no single varying is read in part.

## What Mesa 26.2.1 does

The reference is `tools/mali/testdata/vsfp16_19_tex.expect.json` (drm-shim,
`--uniform-values`, made for 561bad55). The GLSL linker pairs the
varyings by name. `nir_opt_varyings` then removes the outputs no fragment
shader reads (`Unused`) and compacts the remaining components. The fragment
shader's order sets the layout:

- `vs.varying` stores six words: `Tint`'s four at offset 0 and `TexCoord`'s
  two at 16. There is no store for `Unused`. A vertex's generic varyings
  take 24 bytes (`generic_size_B`, which the frame writes to `VARY_SIZE`).
- `fs.fragment` loads `.v4` at index 0x0 (`Tint`) and `.v2` at 0x10
  (`TexCoord`).

## What our link did

glLinkProgram builds the program's vertex shader for the fragment shader's
reads (`compile_vs_linked` with no map, backend.rs `vs_for_fragment`, then
mali/mod.rs `compile_vs_for_reads`). The reads come from the fragment
record (`fragment_reads`) as (location, component mask) pairs, at the
**fragment** shader's locations: `[(0, 0xf), (1, 0x3)]`. `compact_layout`
(nir.rs) looks each vertex output up by its **own** location. So `TexCoord`
(vertex location 0) took `Tint`'s mask 0xf, clipped to its two components,
and `Tint` (vertex location 1) took `TexCoord`'s 0x3. The pairing
(link.rs `pair`) then found `Tint` at the same offset in both layouts with
masks 0x3 and 0xf, and refused it as a part read (`MALI_PARTIAL_READ`).

The refusal named the wrong cause: each stage numbers its varyings in its
own declaration order, and the reads were not moved to the vertex numbering.
The test of 561bad55 (`the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1`)
worked around it by passing the reads by hand at the vertex locations,
`[(1, 0xf), (0, 0x3)]`.

## The lowering

`mali/mod.rs` `reads_at_vertex_locations` maps a `ProgramReads` to the
vertex shader's locations by name, as Mesa's linker pairs varyings. It
lowers both sources (`lower_source`) and finds, for each fragment location,
the fragment input at that location, then the vertex output of the same
name. It maps both kinds of location the reads carry:

- the read list's locations;
- the moved expressions' (motion.rs `Moved::location` and every
  `Node::Lane`), decoded and encoded again.

The fragment source's own inputs give the names, not the fragment record.
The record lists only the inputs it laid out, and a moved expression can
read one it does not list.

`vs_for_fragment` (backend.rs) applies the mapping before the reads become
the vertex record's key (`FsReads`). The cache therefore keys the program's
vertex shader by the locations it is built at. If a read has no vertex
output of its name, no vertex shader is built (1). The link then refuses
the pair as before (`fragment varying '…' is not written by the vertex
shader`).

The layout order needs no change. `compact_layout` walks the vertex
shader's layout and only looks each output's mask up in the reads, so the
order of the reads does not matter.

`Unused` keeps its record slot with mask 0 and no components (offset 24).
`src/mali/program.rs` sizes a vertex's varyings as the largest
`offset + 4 * comps`, and `comps` is the mask's popcount (mod.rs, the vertex
`VaryingSlot`). So the frame writes 24, Mesa's size. Generic slots are only
counted to check that a varying program exists (frame.rs). `src/mali` did
not change.

## Pins

`src/shaderc/online/mali/tests.rs`:

- `case_19_s_program_through_the_entry_points_is_byte_equal_to_mesa_26_2_1`
  runs the pair through `SHC_ONLINE` in the GL core's order: compile each
  stage, `compile_vs_linked` with no map, then link. The link returns 0.
  `vs.position`, `vs.varying`, `fs.fragment` and their facts equal
  `vsfp16_19_tex.expect.json`. The generic layouts are vertex
  `(1, 0, 0xf), (0, 16, 0x3), (2, 24, 0)` and fragment `(0, 0, 0xf),
  (1, 16, 0x3)`. The stage meta's `varying_bytes` is 24.
- `compile_linked`, the helper every link-step reference test uses, now
  maps the reads the same way. The fp16 test's hand-made reads are gone, and
  the other pairs, declared in one order, map to the same locations.

`link.rs` `each_mali_link_outcome_is_its_own_line` keeps `MALI_PARTIAL_READ`
for a vertex shader that was not built for the program's fragment shader.

## Left

- The board: case 19 has to PASS in a `panthor-gltest --sweep` on the
  NanoPC-T6. Only the host has run it so far (the plan's board sweep x2).
- A varying read in part in a pair declared in two orders has no reference
  of its own. The mapping is the same code path, but the bytes are pinned
  only for a pair in one order (link.rs, gl/program.rs `vcol.y`) and for
  case 19's.
