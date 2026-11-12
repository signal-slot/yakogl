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

## The lanes the link keeps, and how the fragment shader loads them

Mesa 26.2.1's GL path, traced in the source (`$MESA_PAN_TREE`) and in the
drm-shim's NIR of every pass (`NIR_DEBUG=print_fs,print_vs`). Each step
names the code that does it and the reference in `tools/mali/testdata`
that measures it.

1. **Packing order** (select.rs `generic_layout`). The GLSL linker sorts
   the varyings: a vec4 first, then a vec2, a float, a vec3, equal ones in
   the vertex shader's order (gl_nir_link_varyings.c:2376-2390, the
   comparator :2494-2505). A matrix sorts by its own components
   (`glsl_without_array(var->type)`, :2694-2708): a mat2 as a vec4, a
   mat3 as a float. fsload_v3z_v2x lays the vec2 before the vec3;
   fsload_mat2_v2 the mat2 before the vec2.
2. **Lanes kept** (nir_fs.rs `kept_whole`). `lower_precision` converts a
   varying in each tree it lowers as a whole variable, `f2fmp` of the
   deref before the swizzle (lower_precision.cpp:724-733). The link
   (`nir_opt_varyings_bulk`, gl_nir_linker.c:1524) runs while that vector
   conversion reads every lane, so every lane is kept; the lanes no one
   reads go only in the fragment shader's own compile.
   - Kept whole: a mediump float varying of a mediump shader that an fp16
     operation reads (fsvec_*, discard_81_branchquad, intloop_*), or that
     a lowered colour takes as it is. The colour is lowered when every
     lane is a constant, a mediump varying's lane, a mediump uniform's
     component, an fp16 texel, gl_PointCoord or fp16 arithmetic (cmp_*,
     fsload_*, case 105).
   - The lanes read alone: a highp shader, a highp varying and an integer
     one (fsload_highp, fsload_hp_var2, intbool_89_flatlast); a colour
     with an fp32 lane, a highp varying's, uniform's or texel's or an fp32
     operation's (fsload_hp_var2, case 89); a varying only a cube
     direction reads raw (varyconst_101_cubebias).
   - A matrix is kept per column, each column its own deref (fsload_mat2:
     `.v2`; fsload_mat3: `.v3`).
   - Any other colour lane or reader is refused (`FsVaryingLoad`).
3. **Compaction** (nir_fs.rs `layout`). `compact_varyings`
   (nir_opt_varyings.c:5491-5560) gives every kept lane the next scalar
   slot in the order of step 1. A vertex shader's constant lane is
   propagated, not kept. Two equal lanes are merged (fsload_dup loads
   `.v2`); the link here refuses that vertex shader (`VaryingDuplicate`).
4. **Loads.** `nir_unlower_io_to_vars` (st_program.c:872) makes one
   variable of each slot's lanes. The fragment shader loads a slot it
   reads a lane of at that width (`component + num_components`,
   pan_nir_lower_fs_inputs.c:52-69, in `bifrost_postprocess_nir` before
   `nir_opt_shrink_vectors`, bifrost_nir.c:286, which has no case for
   `load_var_buf_pan`). A slot no one reads is not loaded, though the
   vertex shader stores it (fsload_v3x_v3x: six stores, one `.v4` load).
5. **Conversions** (nir_fs.rs `conversion_order`, `select`).
   - The lanes a lowered tree reads are converted where its `f2fmp` stands,
     in lane order (`nir_lower_alu_width`), at their load: a compare's
     before its discard, an operand's before its operation. So are lanes
     an operation reads that the walk folds (`x * 1.0`: sel_v_color_uv).
   - The colour's own lanes lose their conversion at the link
     (`f2f32(f2fmp(a))` is `a`, nir_opt_algebraic.py:1991) and are
     converted at the colour's store (`nir_lower_mediump_io`,
     bifrost_nir.c:914, its `f2fmp` right before the store,
     nir_lower_mediump.c:102-104): after everything before it, a discard
     included (fsconv_disc2, fsconv_discw, fsconv_disch, fsconv_discv,
     fsconv_discpc), in the colour's lane order across the loads
     (fsconv_cpq: q.y, c.z, p.x; fsload_order; intloop_ifloop).
   - `nir_opt_vectorize` makes two lanes of one load that are an aligned
     pair one `V2F32_TO_V2F16` at the first one's place, that lane low
     (nir_opt_vectorize.c:166-171, 559). A pair the selection converts
     apart (fsload_mk_pair; fsload_disc_col: the compare's y, then the
     colour's x) or with the odd lane low is refused (`FsVaryingLoad`).
   - A lane alone as the alpha is read from its own conversion
     (fsconv_alpha2, fsconv_alpha3, fsconv_alpha4: `ATEST ... r2.h0`).
6. **Load order.** `nir_opt_move` puts each load right before its first
   use (bifrost_nir.c:380-381). A load the trees read stands at the first
   read of its varying; a load only the colour's own lanes read stands
   before the first of their conversions, at the store (fsconv_slot:
   d.y's slot 1 first, then slot 0; fsload_v3x_v3z). An indexed uniform
   element only the colour's words read is sunk there too, after the
   conversions (uniidx_j).

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
