<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: varyings paired by name in the program's build (case 19, `link-partial`)

**Implemented** (plan #336 item `link-partial`, issue #33). The Mesa 26.2.1
source is `MESA_PAN_TREE` (`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`); line
numbers are that tree's. The reference is the one 561bad55 recorded,
`tools/mali/testdata/vsfp16_19_tex.expect.json` (Mesa 26.2.1's GL driver on
drm-shim). No new reference was needed: the words below are compared to it
on the host. This note builds on `mali-varying-constants.md` (constant lanes
folded at the link) and `mali-precision.md` Part 1 (case 19's fp16 vertex
uniforms).

## 1. What case 19 refused

gl_test case 19 `es100` draws `tools/shaders/es100/textured.vert` and
`.frag`. The sweep of 2026-11-07 (`tools/cts/log/2026-11-07-t6-step4a/`)
reads:

```
REFUSED link: online shaderc: a Mali link whose fragment shader reads part of a varying
(the link keeps only those components) is not lowered yet ('Tint')
```

The fragment shader does not read part of `Tint`: it reads `Tint` (lowp
vec4) and `TexCoord` (mediump vec2) whole. The line came from the program's
vertex shader being built for the wrong varyings:

- Each stage numbers its varyings in its own declaration order
  (`reflect.rs`, one counter per stage). The vertex shader declares
  `TexCoord`, `Tint`, `Unused` (locations 0, 1, 2); the fragment shader
  `Tint`, `TexCoord` (0, 1).
- `Unused = 0.5` is a constant lane, so glLinkProgram builds the program's
  vertex shader (`compile_vs_linked` with no map, backend.rs
  `vs_for_fragment`): the fragment shader is compiled with the vertex
  shader's constant lanes folded in (`compile_fs_for_vs`), then the vertex
  shader for that fragment shader's reads (`compile_vs_for_reads`).
- The reads were handed over at the **fragment** locations: `(0, xyzw)`,
  `(1, xy)`. The vertex shader took them at its own: location 0 is
  `TexCoord` (two lanes, asked for four), location 1 is `Tint` (asked for
  two). The link (link.rs `pair`) then found `Tint`'s slots at the same
  offset with different masks, which is the part-read line.

The fp16 test of 561bad55 (`the_fp16_vertex_uniforms_are_byte_equal_to_
mesa_26_2_1`) had worked around this by hand, with the reads given at the
vertex locations `[(1, 0b1111), (0, 0b0011)]`.

## 2. What Mesa does

Mesa's GLSL linker pairs a producer's output with the consumer's input of
the same name (`gl_nir_link_varyings.c:3601-3609`, `get_matching_input` at
3310, then `varying_matches_record`), and gives each matched pair one
location (`varying_matches_assign_locations`, 2829). Both stages' NIR then
name a varying by the same `VARYING_SLOT_VAR<n>`, which is what
nir_opt_varyings (constant lanes, compaction, code motion) and
pan_varying_layout work on. A declaration order is never seen past the
link.

## 3. The lowering

One map, `Locations` (mali/mod.rs), from one stage's location to the other
stage's location of the varying of the same name. It is built from both
sources' reflection rows (name, first location; a row holds its locations up
to the next row's, so array elements and matrix columns keep their offset
from the first). It is used in both directions:

- **reads, fragment to vertex** (`reads_at_vertex_locations`): the program
  reads (`ProgramReads::reads`) and the moved expressions' locations (the
  stored lane `Moved::location` and every `Node::Lane`, decoded and encoded
  again) go to the vertex shader's locations before `compile_vs_for_reads`
  and before they become the record key. A name the vertex shader does not
  declare is left as it is: the link refuses that pair by name, as before.
- **constant lanes, vertex to fragment** (`consts_at_fragment_locations`,
  `VaryingConsts::relocated`): the fragment shader looks a folded lane up at
  its own location (nir_fs.rs `prec.consts.get`, motion.rs
  `prec.consts.flat`), so the vertex shader's lanes go to the fragment
  locations. A lane of a varying the fragment shader does not declare is
  left out: no read finds it, and if it stayed it could land on another
  varying's location there. backend.rs `vs_consts_of` does this for both of
  its callers (`vs_for_fragment` and `link_by_sources`), so the fragment
  shader the program's vertex shader is built for and the one the link
  compiles have the same record key.

Without the constant map the reads map alone would have opened a silent
miscompile: a reordered pair whose fragment shader reads a constant lane
would fold the constant into the varying at the vertex location's number in
the fragment shader, which is another varying.

### Case 19 after the change

`Unused` is the only constant lane, and the fragment shader does not
declare it, so the relocated set is empty. `vs_for_fragment` then has
nothing to build (it answers 1), and the vertex shader's own record is the
program's: its compile stores no constant lane (`or_without_consts`, Unused
listed with mask 0), and it lays out `Tint` at 0 and `TexCoord` at 16, the
offsets the fragment shader reads. The link pairs both. The varying
program's store offsets are push words (`u8.w0` to `u10.w0` in the
reference), so the FAU fact check compares the layout; the frame's
`varying_bytes` (VARY_SIZE) is 24, `Tint`'s 16 bytes and `TexCoord`'s 8.
`src/mali` is not changed.

## 4. Tests (src/shaderc/online/mali/tests.rs)

| test | what |
|---|---|
| `case_19_through_the_program_link_is_byte_equal_to_mesa_26_2_1` | textured.vert/.frag through `SHC_ONLINE.compile`, `compile_vs_linked` (answers 1) and `link`: both IDVS programs and the fragment program, with every fact, byte-equal to vsfp16_19_tex.expect.json |
| `the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1` | the hand-given vertex locations removed: `compile_linked` maps the reads itself |
| `varying_locations_are_paired_by_name` | both maps on a reordered pair: reads and a moved lane to the vertex locations, an undeclared read kept, constant and flat lanes to the fragment locations, a vertex-only lane dropped |
| `a_part_read_in_another_order_is_built_at_the_vertex_locations` | a fragment shader reading `b` whole and `a.y`, declared `b, a` against the vertex shader's `a, b`: the program's vertex shader stores `b` at 0 and `a.y` at 16 and the link pairs it; built at the fragment locations, the link refuses it |

`MALI_TESTS` 378 -> 381. The corpus verdicts (`corpus-verdicts.txt`, one
line per source compiled alone) do not move: case 19 was a link verdict.

## 5. Open

- No reference draws a reordered pair with a part read or a constant lane
  that the fragment shader reads; the two host tests check pairing, not
  Mesa's words. Mesa's layout is made after its own link, so such a pair
  should be byte-equal to the same pair in the vertex shader's order, but
  link.rs still refuses a compacted fragment shader whose offsets differ
  from the vertex shader's (`MALI_LAYOUTS_DIFFER`, no `VaryingOrder` for a
  compacted record).
- Case 19 on the board: the next sweep of plan #336.
