# V3D 7.1.10 triangle: the shader record Mesa wrote and the one our pass-through programs need

Covers the D0 `GL Shader State Record Draw Index`, its two attribute
records and the three uniform streams of Mesa's Pi 5 triangle
(`tools/v3d/pi5/triangle.clif`), against our 7.1 online compiler's
`passthrough+interp_color` program (`tools/shaders/gl_test_es100/passthrough.vert`
and `interp_color.frag`, triangles). It is the input of the probe's "ours"
option (M5, `docs/notes/pre-study-rpi5.md` section 8, P6) and of the
board's second run.

Read at commit `2a621965d2fe4b50a624fce4d50c7bc7a0979aa3` (host only,
nothing here was run on the board). "Ours" values were printed by a
throwaway test calling `chain71::compile_program71(.., Prim::Triangles,
TARGET71, ..)` on that pair (feature set
`host-test,gpu-v3d,shaderc-online,no-shader-table`); that test is not
committed. Re-read them from `chain71` when the compiler moves: the
ours-shaders item commits the binaries under `tools/v3d/tri-shaders71/`.
The layout is `driver/src/v3d/generated/v3d_packet_v71_pack.h`,
`V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_pack` (32 bytes, not changed
here).

Verdicts: **same** (Mesa's value is the one ours needs), **differs: x**
(ours is x), **cannot be derived: reason** (our compiler has no output for
it; the value to use is named and why).

## What our compiler gives (all `chain71.rs` unless named)

| Stage | threads | single_seg | words | VPM in / out | vcm cache | attribute sizes |
|---|---|---|---|---|---|---|
| FS | 4 | false | 12 | n/a | n/a | n/a |
| VS | 4 | true | 28 | 0 / 1 | 4 | `[4, 4, 0 ..]` |
| CS | 4 | true | 20 | 0 / 1 | 4 | `[4, 0, 0 ..]` |

FS: `num_inputs` 4, `input_slots` `(32,0) (32,1) (32,2) (32,3)`, flat,
noperspective and centroid flags all 0, `writes_z` false, `disable_ez`
false, `uses_center_w` **true**, `lock_scoreboard_on_first_thrsw` false,
`uses_implicit_point_line_varyings` false, `uses_pid` false, `rt_mask` 1,
no discard. VS and CS: `writes_psiz` false, `uses_vid`, `uses_iid`,
`uses_biid` false, `separate_segments` false. `attributes` is
`[(0, Some(0)), (1, Some(1))]` (location, record).

Where each comes from: `threads` is `build71` (4 when the values fit 4
threads' registers and no look-up writes more than 4 TMU inputs, else 2;
`chain71.rs`, `build71`, with `under_pressure71`); FS `single_seg` is the
literal `false` in `compile_program71_keyed`'s `FsProg71`; VS/CS
`single_seg` is `starts_in_last_section` (no `Thrsw` in the selected ops,
`vs_prog71`); VPM sizes and `vcm_cache_size` are `vpm_fields`
(`vs_prog71`); `num_inputs`, `input_slots` and the flag words are
`fragment_inputs` (prepare.rs) plus the flat/centroid loop in
`compile_program71_keyed` (noperspective is the literal 0); `writes_z`,
`disable_ez`, `uses_center_w`, `implicit_point_varyings` come from
`select_fragment`'s `Selected` (select.rs); `writes_psiz` is
`vsp.psiz.is_some()`; `vattr_sizes` and `uses_vid/iid` come from
`select_vertex_keyed`'s `Selected` (select.rs); `attributes` is the
record loop (`compile_program71_keyed`, chain.rs:473-502's copy).

## 1. The D0 shader state record

Mesa's column is the decoded `@format shadrec_gl_main` of the clif.

| Field | Mesa | Ours and where it comes from | Verdict |
|---|---|---|---|
| point size in shaded vertex data | 0 | `vs.writes_psiz` / `cs.writes_psiz` = false (`vs_prog71`, `vsp.psiz`); a triangle draw | same |
| enable clipping | 1 | not a compiler output; the draw state (4.2 frame code sets true, `driver/src/v3d/v3d_frame.c`) | same: set 1 |
| vertex id read by CS / VS | 0 / 0 | `uses_vid` false (`Selected.uses_vid`) | same |
| instance id read by CS / VS | 0 / 0 | `uses_iid` false | same |
| base instance id read by CS / VS | 0 / 0 | `uses_biid` false (literal in `vs_prog71`) | same |
| cs_basevertex, cs_drawindex, vs_basevertex, vs_drawindex | 0 | chain71 has no such fields; the pass-through reads none (no vid/iid/base) | same: set 0 |
| FS does Z writes | 0 | `fs.writes_z` false (`Selected.writes_z`) | same |
| turn off early-z test | 0 | `fs.disable_ez` false | same |
| FS uses real pixel centre W in addition to centroid W2 | 0 | `fs.uses_center_w` **true** (`Selected.uses_center_w`, select.rs) | differs: 1. The perspective-correct varying interpolation needs W; Mesa's FS did not set it. Take it from the program, not from Mesa |
| enable sample rate shading | 0 | `fs.force_per_sample_msaa` false | same |
| any shader reads hardware-written primitive id | 0 | `fs.uses_pid` false (literal in `compile_program71_keyed`) | same |
| insert primitive id as first varying to FS | **1** | `fs.uses_pid` = 0 would be the 4.2 frame code's mapping (`v3d_frame.c`: `= fs->uses_pid`) | cannot be derived: Mesa's reason for 1 with a program that does not read the primitive id is not in the tree (Mesa's `v3dx_draw.c` is not vendored). Mesa's FS has the same four varying reads as ours (v_color, 4 words) and runs with 1; see the list at the end |
| turn off scoreboard | 0 | no field; chain71 never sets it | same |
| do scoreboard wait on first thread switch | 0 | `fs.lock_scoreboard_on_first_thrsw` false | same |
| disable implicit point/line varyings | 0 | `fs.uses_implicit_point_line_varyings` false, so the 4.2 mapping `= !uses_implicit` (`v3d_frame.c`) gives 1 | cannot be derived: Mesa's 0 with a triangle FS that reads no implicit varying is not explained by the compiler's flag. Ours says "not used" (1 would be consistent); Mesa's 0 is known to run. Triangles read no implicit varying either way; see the list |
| no prim pack | 0 | no field | same |
| never defer FEP depth writes | 0 | no field, no Z write | same |
| number of varyings in FS | 4 | `fs.num_inputs` = 4 (`fragment_inputs`) | same (value; the primitive-id row decides whether the hardware counts one more) |
| CS output VPM segment size | 1 | `cs.vpm_output_size` 1 (`vpm_fields`) | same |
| min CS output segments in play (in addition to VCM cache size) | 0 | 0 (the 4.2 frame code's fixed value; chain71 has no field) | same |
| CS input VPM segment size | **0** | `cs.vpm_input_size` 0 (`vpm_fields`, the first element of its tuple) | same. Not the 4.2 frame code's 1: copy the program's 0 |
| min CS input segments in play | 1 | no field; the packer asserts it is at least 1 and writes it minus 1 | same: set 1 |
| VS output VPM segment size | 1 | `vs.vpm_output_size` 1 | same |
| min VS output segments in play | 0 | 0 | same |
| VS input VPM segment size | 0 | `vs.vpm_input_size` 0 | same |
| min VS input segments in play | 1 | set 1 | same |
| FS 4-way threadable | 1 | `fs.threads` 4 (`build71`) | same |
| FS start in final thread section | 0 | `fs.single_seg` false | same |
| FS propagate NaNs | 0 | no field; the 4.2 frame code sets true | cannot be derived: no compiler output. Mesa's 0 is the value that runs on this board; set 0 |
| FS code address | `resource_0x1000+0x60` | the probe places `fs.code.words` (bits 2..0 of the field carry the flags, so 8-byte aligned; words are 8 bytes) | differs: the address of our own FS words (a different code, 12 words against Mesa's 9) |
| FS uniforms address | `CL_0x119000+0x0` | `fs.code.uniforms` is empty (0 words): any valid address, Mesa's equals the VS's | same (an empty stream) |
| VS 4-way threadable | 1 | `vs.threads` 4 | same |
| VS start in final thread section | 1 | `vs.single_seg` true (`starts_in_last_section`) | same |
| VS propagate NaNs | 0 | no field; 4.2 frame code sets true | cannot be derived (as the FS): set 0 |
| VS code address | `resource_0x1000+0xa8` | our VS words (28 words) | differs: our address |
| VS uniforms address | `CL_0x119000+0x0` | the VS stream of section 3 (20 words) | differs: our stream's address |
| CS 4-way threadable | 1 | `cs.threads` 4 | same |
| CS start in final thread section | 1 | `cs.single_seg` true | same |
| CS propagate NaNs | 0 | no field | cannot be derived (as the FS): set 0 |
| CS code address | `resource_0x1000+0x160` | our CS words (20 words) | differs: our address |
| CS uniforms address | `CL_0x119000+0x40` | the CS stream of section 3 (12 words) | differs: our stream's address |

Not in the record but decided by the same step: the `GL_SHADER` packet's
attribute-array count is 2 in Mesa and
`program.attributes.len()` / the records' count 2 in ours (same); the
`VCM_CACHE_SIZE` packet is 4 / 4 in Mesa, `cs.vcm_cache_size` /
`vs.vcm_cache_size` 4 / 4 in ours (`vpm_fields`; same).

## 2. The attribute records

| Field | Record 0 Mesa | Record 1 Mesa | Ours | Verdict |
|---|---|---|---|---|
| address | `resource_0x15000+0x0` (positions) | `+0x30` (colours) | the probe's vertex buffer; the compiler's `attributes` says location 0 is record 0 and location 1 is record 1 | cannot be derived: the data's place is the probe's. Keep Mesa's layout: 3 vertices of 16 bytes for each array, positions first, colours 0x30 after (the clif's `resource_0x15000` words) |
| vec size | 4 | 4 | 4 (both attributes are `vec4`) | same |
| type | float (2) | float | float | same |
| signed / normalized / read as int | 0 / 0 / 0 | 0 / 0 / 0 | float attributes | same |
| number of values read by CS | **4** | **0** | `cs.vattr_sizes` = `[4, 0, ..]` (`Selected.vattr_sizes` of the CS, which prunes the colour) | same |
| number of values read by VS | 4 | 4 | `vs.vattr_sizes` = `[4, 4, ..]` | same |
| instance divisor | 0 | 0 | 0 | same |
| stride | 16 | 16 | 16 (tightly packed vec4) | same |
| maximum index | 16777215 | 16777215 | 0xffffff (the 4.2 frame code's value) | same |

The 4.2 frame code's GFXH-930/1602 rule (force one CS value when the CS
loads none) does not fire: record 0 has 4.

Vertex data in Mesa's triangle: positions (-0.75,-0.75,0,1),
(0.75,-0.75,0,1), (0,0.75,0,1); colours red, green, blue (alpha 1).

## 3. The uniform streams

The stream is the compiler's `Code71.uniforms`, `(contents, data)` pairs,
written one 32-bit word each (`write_uniforms` in `v3d_frame.c` is the
4.2 writer: `QUNIFORM_CONSTANT` writes `data`; the viewport entries write
a float). Both generations' streams are in `select.rs`'s `QUNIFORM_*`
numbering (CONSTANT 0, X_SCALE 2, Y_SCALE 3, Z_OFFSET 4, Z_SCALE 5).

**FS.** Mesa: 0 words (its address equals the VS's). Ours: 0 entries.
Same. Neither reads a uniform.

**VS.** Ours (20 words), in order:
`c0 c1 c2 c3 c4 c5 c6 c7`, `X_SCALE`, `c0`, `Y_SCALE`, `c1`, `Z_SCALE`,
`Z_OFFSET`, `c2 c3 c4 c5 c6 c7` (`cN` is CONSTANT with data N). Mesa's
dump has 16 words at `CL_0x119000+0x00`: `2048.0, 0..7, 2048.0, 0.5, 0.5,
0, 1, 2, 3` (the words are in the clif; their roles below are read
against the compiler's layout, not stated by the dump: X and Y scale
and Z scale and offset cannot be told apart in it, being equal pairs).
differs: ours is a different program with a different length and
order; the contents and where they go are known (below), so nothing is
missing.

**CS.** Ours (12 words): `c0 c1 c2 c3 c0 c1 c2 c3`, `X_SCALE`, `c4`,
`Y_SCALE`, `c5`. Mesa's: 8 words at `+0x40`: `2048.0, 0, 1, 2, 3,
2048.0, 4, 5`. differs, as the VS.

How the probe fills a stream, from the dump:

* CONSTANT: `data` verbatim.
* X_SCALE: 2048.0 (`0x45000000`) for the 64x64 target = viewport half
  width 32 px × 64. **At 7.1 the clipper granularity is 64.0, not the
  4.2 writer's `* 256.0f`** (the facts file, section 2; Mesa's own
  `CLIPPER_XY` carries the same 2048.0). The 4.2 `write_uniforms` must
  not be reused unchanged for this entry.
* Y_SCALE: 2048.0 (positive, as Mesa's `CLIPPER_XY` half height).
* Z_SCALE 0.5, Z_OFFSET 0.5 (Mesa's `CLIPPER_Z`).

Every entry is a constant or a viewport value, so every stream is fully
derivable from the viewport; nothing in the pass-through needs a value
from a GL uniform or a texture. One word of room has to follow each
stream (the QPU prefetches the next uniform; `v3d_frame.c` says so).
Each stream starts 4-byte aligned; the three may follow one another
(FS, VS, CS) as in Mesa.

## What probe-tri's ours option must set

1. Compile `passthrough+interp_color` at 7.1 with `compile_program71`
   (triangles, `TARGET71`) or load the committed
   `tools/v3d/tri-shaders71/ours_*` words; place FS, VS and CS words at
   8-byte aligned addresses (the low 3 bits of a code address are the
   flags).
2. Threads: set `4-way threadable` of all three stages from
   `threads == 4` (all 4). Final-thread-section: FS 0, VS 1, CS 1, from
   `single_seg`.
3. VPM: CS and VS output segment size 1, input segment size **0** (not
   1 as on 4.2), min input segments in play 1, min output segments in
   play 0; `VCM_CACHE_SIZE` 4 and 4. Take them from `vpm_input_size`,
   `vpm_output_size`, `vcm_cache_size`.
4. Varyings: number of varyings in FS = `fs.num_inputs` = 4.
5. `uses_center_w` is true for ours: set "FS uses real pixel centre W
   in addition to centroid W2" = 1 (Mesa had 0).
6. Flat, noperspective and centroid flags: all 0, so the `CFG`-side
   `ZERO_ALL_*` packets Mesa's clif has stay as they are (no
   per-varying flag packets).
7. Point size flag 0; vertex, instance and base-instance id bits 0;
   base-vertex and draw-index bits 0; Z writes 0; early-z test not
   turned off; sample rate shading 0; hardware primitive id read 0;
   enable clipping 1; scoreboard bits 0; no prim pack 0; never defer
   FEP depth writes 0.
8. Where our compiler has no output, copy Mesa's board value, which is
   known to run on this V3D 7.1.10: propagate NaNs 0 for all three
   stages (not the 4.2 frame code's true); insert primitive id as first
   varying 1; disable implicit point/line varyings 0. If the ours run
   fails and Mesa's own run passed, bisect these three first (each is
   one record bit), then `uses_center_w`.
9. Attribute records: two, float vec4, stride 16, max index 0xffffff,
   divisor 0; record 0 reads 4 values in the CS and 4 in the VS, record
   1 reads 0 in the CS and 4 in the VS (`cs.vattr_sizes`,
   `vs.vattr_sizes`). The vertex data is Mesa's (positions, then colours
   0x30 after).
10. Uniform streams: write FS (empty), VS (20 words) and CS (12 words)
    in the compiler's order; CONSTANT entries verbatim; X_SCALE and
    Y_SCALE 2048.0 (64.0 times the half size in pixels, **not** 256.0);
    Z_SCALE 0.5; Z_OFFSET 0.5. Put the three addresses in the record
    (an empty FS stream may share the VS's address, as Mesa's does), one
    word of room after the last stream.
11. The 4.2 record's `address of default attribute values` and the two
    `separate VPM block` flags do not exist at 7.1; the 32-byte D0
    record has no field for them (use the 7.1 packer).
