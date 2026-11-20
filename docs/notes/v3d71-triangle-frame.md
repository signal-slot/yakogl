<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1: the clear-then-triangle frame (M5 on the Raspberry Pi 5)

Raspberry Pi 5 port (#7), milestone M5 (P6 of `pre-study-rpi5.md` section 8):
what a V3D 7.1 job drawing one triangle after a clear holds, packet by
packet, and the BCM2712 D0 shader record it needs. The code is
`driver/src/v3d/v3d_frame71_tri.c` (`v3d_frame_build_triangle` built with
`-DV3D_CL_VER=71`, declared in `v3d_frame.h` under `V3D_CL_VER == 71`); the
check is `tools/v3d/check71.d/triangle71.sh`. The clear part is the clear
frame of `v3d71-clear-frame.md`, unchanged.

Sources:

- `T` = `tools/v3d/pi5/triangle.clif`: Mesa 26.2.2's `V3D_DEBUG=clif` dump of
  `tools/v3d/gl_ref.py triangle` on the board (V3D 7.1.10.2, BCM2712 D0): a
  64x64 RGBA8 target cleared to 0xffff0000, then one Gouraud triangle.
  `tools/v3d/pi5/README.md` says how it was taken and how it differs from
  the drm-shim's `tools/v3d/shim71/triangle.clif` (`S`, V3D 7.1.7).
- `H` = `tools/v3d/tri-shaders71/mesa_passthrough_interp_color.h` (and its
  JSON): Mesa's three programs out of `T`, their uniform streams and the
  record values read at the D0 layout (`tools/v3d/clif_shaders71.py`).
- `X` = `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml` and the
  committed `driver/src/v3d/generated/v3d_packet_v71_pack.h`.

Mesa's gallium driver source is not vendored and was not at hand for this
note: every value below is backed by `T` (and `S`), not by a line of
`v3dx_draw.c`. Where the builder writes a rule rather than a constant (a
flag from the program, the viewport), the rule is the 4.2 builder's, and
`T` shows its 7.1 result for this one program.

## 1. The job

| Buffer (in `T`) | What | Bytes |
|---|---|---|
| `CL_0x11000` | the binning list (BCL) | 126 (`@add_bin` end `+0x7e`) |
| `CL_0x119000` | the indirect buffer: uniform streams, shader record, attribute records, generic tile list | 187 (`+0x00..+0xbb`) |
| `CL_0x125000` | the render list (RCL) | 96 (`@add_render` end `+0x60`) |
| `resource_0x1000` | the QPU code: FS `+0x60`, VS `+0xa8`, CS `+0x160` (to `+0x1d8`) | Mesa's; the first 0x60 bytes are not code |
| `resource_0x15000` | the vertex buffer: 3 vec4 positions, then 3 vec4 colours at `+0x30` | 96 |
| `resource_0x9000`, `tile_alloc_0x11d000`, `TSDA_0x121000` | the target, the tile allocation, the tile state | referenced only |

The submit is the clear's with one draw: `ADD_BIN` (BCL start, end, tile
allocation, its size 16384, tile state) and `ADD_RENDER` (RCL start, end,
tile allocation). 16384 is what `v3d_frame_alloc_sizes` gives for one draw
at 4 KiB pages (4096 + 8192 + 64, rounded up), the board's (16 KiB buffers)
and the shim's alike; the clear-only frame's 12288 is where the two differ.

The render list (96 bytes) and the generic tile list (27 bytes) are the
clear frame's, byte for byte (`v3d71-clear-frame.md` 1.2 and 1.3): the
draw only adds to the binning list and the indirect buffer. Our tile list
stores the target as raster with a byte stride where Mesa's GL target is
UIF (`Memory Format`, `Height in UB or Stride`), as in the clear.

## 2. The binning list, 126 bytes

| Packet (`T` name) | Bytes | Value in `T` | 7.1 against the 4.2 triangle |
|---|---|---|---|
| `NUMBER_OF_LAYERS` | 2 | 1 | 7.1 only (the clear's) |
| `TILE_BINNING_MODE_CFG` | 9 | 64x64, log2 tile 3x3, blocks 128 B / 64 B | the tile size, not the RT count and bpp (the clear's) |
| `CLEAR_VCD_CACHE`, `OCCLUSION_QUERY_COUNTER_ENABLE`, `START_TILE_BINNING` | 1 + 5 + 1 | | same |
| `CLIP` | 9 | 0, 0, 64, 64 | same rule |
| `CFG_BITS` | 4 | both faces, not clockwise, depth ALWAYS, `Z Clipping mode` 1 (`MIN_ONE_TO_ONE`) | no early-Z bits; a 1-bit line rasterization (diamond exit, 0); the Z clipping mode, -1..1 for GL's default `clip_halfz` false |
| `POINT_SIZE`, `LINE_WIDTH` | 5 + 5 | 1.0, 1.0 | same |
| `CLIPPER_XY` | 9 | 2048.0, 2048.0 | in 1/64 pixel: half width x 64 (4.2: x 256), the 7.1 clipper granularity |
| `CLIPPER_Z`, `CLIPZ` | 9 + 9 | 0.5 / 0.5; 0.0 .. 1.0 | same |
| `VIEWPORT_OFFSET` | 9 | fine 8192, coarse 0 | same u14.8 field; the CLIF prints its raw value (32 pixels x 256) |
| `BLEND_ENABLES`, `COLOR_WMASKS`, `BLEND_CCOLOR` | 2 + 5 + 9 | 0, 0, 0 | same (the masks cover 8 RTs at 7.1) |
| `ZERO_ALL_FLATSHADE_FLAGS`, `..._NOPERSPECTIVE_...`, `..._CENTROID_...` | 3 | | same |
| `TRANSFORM_FEEDBACK_SPECS` | 2 | 0 specs | same; Mesa's decoder reopens the list after it (a second `@format ctrllist` line in `T`) |
| `OCCLUSION_QUERY_COUNTER_ENABLE`, `SAMPLE_STATE` | 5 + 5 | null; mask 15, coverage 1.0 | same |
| `VCM_CACHE_SIZE` | 2 | 4, 4 | same |
| `GL_SHADER` | 5 | 2 attribute arrays, record at indirect `+0x60` | same |
| `VERTEX_ARRAY_PRIMS` | 10 | TRIANGLES, 3, first 0 | same |
| `FLUSH` | 1 | | same |

2 + 9 + 1 + 5 + 1 + 9 + 4 + 5 + 5 + 9 + 9 + 9 + 9 + 2 + 5 + 9 + 3 + 2 + 5 +
5 + 2 + 5 + 10 + 1 = 126.

## 3. The indirect buffer

| Offset | What |
|---|---|
| `+0x00` | the FS uniform stream: empty here, so its address is the VS stream's |
| `+0x00` | the VS stream, 16 words: viewport X scale, 8 constants (0..7), Y scale, Z scale, Z offset, 4 constants (0..3) |
| `+0x40` | the CS stream, 8 words: X scale, 4 constants, Y scale, 2 constants |
| `+0x60` | the shader record, 32 bytes, 32-byte aligned (section 4) |
| `+0x80` | two attribute records, 16 bytes each |
| `+0xa0` | the generic tile list, 27 bytes, to `+0xbb` |

There are **no default attribute values** at 7.1: the 4.2 record's
`Address of default attribute values` is gone (`pre-study-rpi5.md` 3.2), and
the 4.2 builder's 256-byte block in front of the streams is not written. The
uniform rule is the 4.2 builder's (`v3d_uniforms.c` `v3d_write_uniforms` for
the contents M5 supports, the same refusals) with the viewport X and Y scales
x 64 instead of x 256 (`0x45000000` = 2048.0 = 32 x 64 in `T`).

The attribute records (Address, vec size 4, float, values read by the CS 4
and 0, by the VS 4 and 4, divisor 0, stride 16, maximum index 0xffffff) are
the 4.2 builder's, with its rule for what the CS must load. At 7.1 the
stride is a 32-bit field.

## 4. The shader record: the BCM2712 D0 layout

V3D 7.1.10 (the D0 stepping, Mesa's `v3d_device_has_draw_index`, rev >= 10)
reads `GL Shader State Record Draw Index` (`X`: "exclusive to 2712D0"),
32 bytes, which the 7.1.7 drm-shim's plain `GL Shader State Record` does not
match: four bits are inserted (`cs_basevertex` 5, `cs_drawindex` 6,
`vs_basevertex` 10, `vs_drawindex` 11) and every flag from bit 5 up moves 2
to 4 bits higher. Bytes 3 to 31 (the varyings count, VPM sizes, code and
uniform addresses with their 4-way / final-section / propagate-NaNs bits)
are at the same bits in both.

| D0 bit | Field | Value written | `T` (this program) |
|---|---|---|---|
| 0 | Point size in shaded vertex data | false (a triangle) | 0 |
| 1 | Enable clipping | true | 1 |
| 2, 3 | Vertex ID / Instance ID read by the CS | `cs->uses_vid`, `uses_iid` | 0, 0 |
| 4 | Base Instance ID read by the CS | 0; a program reading it is refused | 0 |
| 5, 6 | `cs_basevertex`, `cs_drawindex` | 0 (our programs report neither) | 0 |
| 7, 8 | Vertex ID / Instance ID read by the VS | `vs->uses_vid`, `uses_iid` | 0, 0 |
| 9 | Base Instance ID read by the VS | 0; refused as for the CS | 0 |
| 10, 11 | `vs_basevertex`, `vs_drawindex` | 0 | 0 |
| 12 | FS does Z writes | `fs->writes_z` | 0 |
| 13 | Turn off early-Z test | `fs->disable_ez` | 0 |
| 15 | FS uses real pixel centre W | `fs->uses_center_w` | 1 |
| 16 | Sample rate shading | false (no MSAA) | 0 |
| 17, 18 | Any shader reads / insert primitive ID | `fs->uses_pid` | 0, 0 |
| 19 | Turn off scoreboard | false | 0 |
| 20 | Scoreboard wait on first thread switch | `fs->lock_scoreboard_on_first_thrsw` | 0 |
| 21 | Disable implicit point/line varyings | `!fs->uses_implicit_point_line_varyings` | 1 |
| 22, 23 | No prim pack, never defer FEP depth writes | false | 0, 0 |
| 24..31 | varyings in the FS | `fs->num_inputs` | 4 |
| bytes 4..7 | CS / VS output VPM segment size, min output 0, input size, min input 1 | the program's sizes | 1, 0, 0, 1 for both |
| bytes 8..31 | FS, VS, CS code address + 4-way + final section + propagate NaNs; uniforms address | threads == 4, `single_seg`, NaNs **clear** | 4-way all; final section VS and CS; NaNs 0 |

The flag bits come to 0x208002 (bits 1, 15, 21), `H`'s `MESA71_TRI_REC_FLAGS`.

What differs from the 4.2 record besides the layout: no default attribute
values address; no "separate input and output VPM blocks" flags, so the
input segment size is the program's own, 0 when shared (4.2 writes 1 for a
shared segment); the three propagate-NaNs bits clear (4.2 sets them; `T`
has them clear).

**How Mesa's dump shows it.** Mesa's CLIF decoder prints the record with the
plain 7.1 layout on the board too, so `T` shows "uses real pixel centre W"
0, "Insert Primitive ID as first varying" 1 and "Disable implicit
point/line varyings" 0: those are D0 bits 12, 15 and 18 read at their plain
positions. Bits 10, 11 and 21 to 23 have no plain field and are not in `T`
at all. `triangle71.sh` therefore compares in three ways, none of which
ignores a flag: our record decoded the same plain way equals `T`'s line for
line; `frame_check --triangle71` unpacks our bytes at the D0 layout
(`V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX_unpack`) and checks every field
and the 24 flag bits against `H`; and our D0 reading equals `S`'s plain
record (7.1.7 reads its own layout) in all 37 non-address fields, while the
two plain decodes differ in exactly those three flags. The five bits `T`
cannot show are `H`'s assumption (`clif_shaders71.py` `UNSEEN`: the shim's
values), and our builder writes the same.

`GL_SHADER_STATE_RECORD_DRAW_INDEX` is in `v3d_cl_packets.h`'s 7.1 list,
so `cl_golden.sh` checks its packer against Mesa's (63 packets and records
at 7.1). The builder writes only the D0 record: an older 7.1 part would
read its flags at other bits, and the Pi 5 is the only 7.1 target.

## 5. What the 7.1 builder refuses

Everything the 4.2 builder refuses (a zero-sized frame, no programs, thread
counts other than 1/2/4, a vertex count that is not whole triangles, no
attribute record or an attribute read without one, 1..4 components only, a
uniform contents it does not write, non-zero varying flags or more than 24
varyings, a field out of range, a list that does not fit), and, at 7.1:

- a VS or CS with separate VPM segments or a non-zero VPM input size (no
  record flag for it; the only 7.1 reference shares the segments);
- a VS or CS reading the base instance (no 7.1 reference);
- an attribute with an instance divisor (M5 draws one instance).

`frame_check --triangle71` builds 20 such frames; each must be refused.

## 6. Open

- The board has not run this job from our builder yet (item probe-tri, the
  Zephyr probe `rpi5_triangle`, and the board session board-m5). The probe
  links `v3d_frame.c` and `v3d_frame71_tri.c` built at 7.1 and supplies the
  three programs (`H`), the vertex data and the buffers.
- Our own 7.1 compiler's programs (P8) will set the record from their
  prog_data; any field `H` holds constant here (propagate NaNs, the VPM
  input size, the base-instance and draw-index bits) needs a case of its own
  before a program that differs is accepted.
- `xbuild71.sh` cross-compiles `v3d_frame.c` but not yet `v3d_frame71_tri.c`.
