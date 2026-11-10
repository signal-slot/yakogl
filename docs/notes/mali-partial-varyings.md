<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): a program's varyings paired by name, case 19's partial read

**Status: implemented (link-partial, plan #336, issue #33).** The sweep
`tools/cts/log/2026-11-07-t6-step4a` refused gl_test case 19 es100 at the
link:

    REFUSED link: online shaderc: a Mali link whose fragment shader reads part of a varying (the link keeps only those components) is not lowered yet ('Tint')

This note says what the refusal was (not a partial read: the two stages'
varying numberings), what Mesa 26.2.1 does, what changed in
`src/shaderc/online/mali` and `src/shaderc/online/backend.rs`, and what the
host tests pin. It continues `notes/mali-varying-constants.md` (the link's
constant lanes, `VaryingConsts`) and `notes/mali-precision.md` Part 1 (case
19's fp16 vertex uniforms, byte-equal since `561bad55`), and repeats neither.
Locations are at the commit of this note.

## 1. The defect

gl_test case 19 links two programs. The first (`hello`, `vsfp16_19_hello`)
links. The second is an IntroducingPVRShell-style textured pair
(`driver/tools/pvrlab/gl_test_shaders_es100.h` `gl_test_es100_tex_{vs,fs}_src`,
byte copies in `tools/mali/testdata/vsfp16_19_tex.{vert,frag}`):

| | declares, in order | location |
|---|---|---|
| vertex | `TexCoord` (vec2), `Tint` (vec4), `Unused` (float, `= 0.5`) | 0, 1, 2 |
| fragment | `Tint` (vec4), `TexCoord` (vec2) | 0, 1 |

Each stage of ours numbers its varyings in its own declaration order
(`src/shaderc/online/ir.rs` `IoDecl::location`, the front end), so `Tint` is
the fragment shader's location 0 and the vertex shader's 1. The link
(`mali/link.rs` `pair`) pairs the two records by **name**, as it should, and
compares the slot of each pair: offset and component mask.

The program's vertex shader, though, is built for the fragment shader's reads
(`backend.rs` `vs_for_fragment`, Mesa's varying compaction): the reads were
taken from the fragment record as `(location, mask)` and handed to
`compile_vs_for_reads` as if they were the vertex shader's locations. For
this pair that stored `TexCoord` (vertex 0) for `Tint`'s mask and `Tint`
(vertex 1) for `TexCoord`'s `0b0011`: two lanes of `Tint`. `pair` then found
the vertex slot of `Tint` with mask `0x3` against the fragment's `0xf` and
reported it as a partial read. The vertex shader was built for the program
every time here because it stores a constant lane (`Unused = 0.5`), which
made `vs_for_fragment` take the constant-lane arm whatever the fragment
shader read; a pair without one and without compaction takes the vertex
shader's own layout and never met the defect.

The same by-location hand-over existed for the constant lanes
(`vs_constants`, `VaryingConsts` `(location, comp, bits)`): the fragment
shader folds a lane into the read of its *fragment* location
(`nir_fs.rs` `prec.consts.get(v, c)`). `Unused` sits at the vertex shader's
location 2, which this fragment shader does not declare, so case 19 lost
nothing there; a pair whose fragment shader numbers another varying under
the vertex shader's constant location would have folded the constant into
that varying's read (wrong pixels, no refusal). And a moved expression
(`motion.rs` `Node::Lane { location, .. }`, encoded from the fragment record)
named its lanes by fragment location too.

## 2. What Mesa does

- The GLSL linker pairs varyings by name: `gl_nir_link_varyings.c:3536-3609`
  walks the producer's outputs (`nir_foreach_shader_out_variable`), finds the
  consumer input of the same name (`get_matching_input`, `:3310`) and
  records the match (`varying_matches_record`, `:2728`); the slots
  (`VARYING_SLOT_VARn`) are assigned to the matches, so after the link both
  stages agree on one numbering.
- `nir_opt_varyings` then removes what the consumer does not read, folds
  constant and uniform-only lanes into the consumer, and compacts the rest
  (`nir_opt_varyings.c:5491` `compact_varyings`).
- panfrost builds the vertex shader's varying layout from its compacted
  outputs (`pan_shader.c:584-591`, `pan_build_varying_layout_compact`,
  `pan_nir_collect_varyings.c:413`) and gives the fragment shader that
  layout at link (`pan_shader.c:681-682`, `fs->vs_varying_layout =
  vs->vs_varying_layout`; `notes/mali-frame-refusals.md` section 6).

The reference `vsfp16_19_tex.expect.json` shows the result: the varying
program stores `Tint` at offset 0 (four lanes) and `TexCoord` at 16 (two),
`Unused` nowhere; the fragment program loads `.v4` at 0 and `.v2` at 16.

## 3. What changed

All in `src/shaderc/online/mali/mod.rs` unless named.

- `LocationPairs::of(fs, vs_outputs)`: the `(vertex location, fragment
  location)` pairs of the program, by name, from the fragment shader's
  record rows (`ShcShader::varyings`, named as the IR names them: a
  per-column or per-element input `m[1]` by that name, the same on both
  sides) and the vertex IR's output declarations. An input no output is
  named for has no pair; the link refuses such a program by name
  (`backend.rs` `link_by_sources`), as before.
- `program_reads(r, fs, vs_glsl, scratch)`: `fragment_reads` with every
  location, of the reads and of the moved expressions' lanes, carried to
  the vertex shader's (`decode`, relocate, `encode` for the moved bytes).
  The vertex source goes through the front end once more for its
  declarations (it compiled before, so what can fail is the scratch arena).
  `needs_program_vs(fs)` replaces `partial_reads`: the fragment record when
  it is compacted or holds moved code.
- `vs_constants_for(glsl, fs, scratch)`: `vs_constants` at the fragment
  shader's locations (`link.rs` `VaryingConsts::relocated`); a lane of an
  output the fragment shader does not declare is left out, as no read of
  it exists to fold and the vertex shader built for the reads stores none.
  `vs_constants` itself (vertex locations) stays for the vertex shader's
  own compile without its constant lanes (`compile_vs_without_consts`).
- `backend.rs`: `vs_consts_of(text, fs)` and `vs_for_fragment` call the
  two; nothing else moves. The record keys are unchanged in form
  (`FsReads::key`, `LinkConsts::key`) and now hold the vertex shader's
  locations for a vertex record and the fragment shader's for a fragment
  record, so two fragment shaders of one interface in either declaration
  order share the program's vertex record.
- `link.rs` `MALI_PARTIAL_READ` now reads as what it is: the vertex shader
  stores other components than the fragment shader reads, which only a
  pair not built for the program shows (the hand-made pair of the link.rs
  test), a back-end defect like `MALI_NO_VERTEX_SLOT`.

`nir.rs` `compact_layout` (the components stored, compacted, in the vertex
shader's class-sorted order) did not change: with the reads at the right
locations it already lays case 19 out as Mesa does.

What case 19's pair now goes through, in the GL core's order: the vertex
shader object is refused for its constant lane and compiled without it
(`or_without_consts`: `Tint` at 0, `TexCoord` at 16, `Unused` nowhere); the
program's vertex shader (`compile_vs_linked`) finds the only constant lane
has no fragment input of its name, so the link carries no lane over, the
fragment record is not compacted, and nothing is built (1: the object's
record is the program's); the link (`link_by_sources`, then `pair`) finds
each read at its vertex output's offset. Both records are the reference's
to the byte without a rebuild. A pair that does carry a lane over, or
reads a varying in part, is rebuilt as before, with the locations right.

## 4. Tests (`src/shaderc/online/mali/tests.rs`)

- `case_19_textured_pair_links_through_the_entry_points_byte_equal_to_mesa_26_2_1`:
  the pair through `SHC_ONLINE.compile`, `compile_vs_linked` (1, nothing to
  build) and `link`, as the GL core calls them; the link returns 0 with an
  empty log, both stages are `vsfp16_19_tex.expect.json` to the byte
  (words, FAU, preload, work registers, secondary offset), `Tint` at offset
  0 and `TexCoord` at 16 in both records, `Unused` stored nowhere. The
  testdata sources are checked against gl_test's.
- `the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1` no longer hands
  the vertex shader the reads at the vertex locations by hand: `check_linked`
  (the link step as the back end runs it) does it for all four pairs.
- `a_constant_lane_is_folded_by_name_not_by_location`: `v_k = vec2(a_col.y,
  0.5)` declared first in the vertex shader and second in the fragment
  shader; the lane is carried to the fragment shader's location 1, the pair
  in either order is the same three programs (`v_a.zw` at 0, `v_k.x` at 8),
  and a lane of an output the fragment shader does not declare is dropped.
- `check_linked` pairs the two records' slots by name now (it compared
  `varying` numbers before, which held only for same-order pairs).
- `compile_linked`, which every byte-equal link test uses, now runs the
  by-name step (`fragment_names` makes the fragment record's rows from the
  IR), so every reference pair also pins that same-order pairs are
  unchanged.

## 5. What is left

- **Compaction with another order inside a packing class.** The vertex
  layout sorts by packing class (vec4, vec2, scalar, vec3) and keeps the
  declaration order inside a class (`select.rs` `generic_layout`); the
  fragment layout does the same in its own order. A pair whose stages
  declare two varyings of one class in opposite orders links through the
  fragment shader compiled for the vertex order (`link.rs` `VaryingOrder`,
  case 209) only when the fragment record is not compacted and holds no
  moved code (`pair`'s `orderable`); with a partial read as well it still
  ends in `MALI_LAYOUTS_DIFFER`. Case 19 has a vec4 and a vec2, so the class
  sort places them alike. No sweep case needs the combination; the corpus
  verdicts are per shader and do not see links.
- The board: case 19 x2 in the next t6 session (the plan's board-sweep
  item), then the sweep.
