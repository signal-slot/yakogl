<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali frame refusals: face culling (4), per-instance attributes (52), RGBA16F targets (54)

Plan #153, item study-frame. The three `2026-10-23-s6-discard` sweep
refusals that are a frame-descriptor gap rather than a compiler one:

| Case | Refusal (sweep 1 log) | Raised at | What it needs |
|---|---|---|---|
| 4 `cull` | `a draw with face culling on is not built yet` | `src/mali/backend.rs:1230-1232` (`check`, `NOT_BUILT_CULL`, `backend.rs:157`) | two DCD Flags 0 bits |
| 52 `es3split` | `a vertex attribute other than float32 data ... or one per instance is not built yet` | `src/mali/backend.rs:1897` (`a.per_instance != 0`), then `src/mali/frame.rs:1951` (`NOT_BUILT_INSTANCED`, `frame.rs:133`) | Attribute descriptor frequency and divisor |
| 54 `es3chain` | `a colour target other than RGBA8888, XRGB8888 or RGB565 is not built yet` | `src/mali/backend.rs:1244` (`target_format`, `NOT_BUILT_TARGET_FORMAT`, `backend.rs:147`) | a float render target |

## 1. How Mesa's descriptors were obtained, and what is still owed

**The host drm-shim could not be used in this session.** The shim is not built
on this machine (`libpanfrost_noop_drm_shim.so` exists nowhere under the
dependency cache; only the `pan_shc` build `mesa-pan/mesa-26.2.1/build-pan`
does), and the session was not permitted to run `meson`/`ninja` to build it
(`tools/mali/drm_shim_ref.sh build` needs a Mesa tarball that is not in
`$CONNECTED_DEPS_DIR/downloads` either). Even built, `gl_ref.py` could not give
these three descriptors: it draws GLES2 with `glDrawArrays` into an RGBA8 FBO
(`--prim`, `--blend-func`, `--depth-stencil`...), with no `glEnable(GL_CULL_FACE)`,
no `glVertexAttribDivisor`/`glDrawElementsInstanced` (ES 3.0) and no float
target. Adding those options is a `gl_ref.py` change, outside this item.

So the files under `tools/mali/testdata/frame_*` are **derived, not dumped**:
Mesa 26.2.1's source (the tree `MESA_PAN_TREE` names), read at the cited lines,
joined with the descriptors of the existing board dumps. Each states what it is
in its head and marks the values only a dump can settle. Section 5 says exactly
what a board Mesa reference must record.

| File | Case |
|---|---|
| `tools/mali/testdata/frame_cull.expect.txt` | 4: the six draws' DCD Flags 0 bits and r57 words |
| `tools/mali/testdata/frame_attr_instance.expect.txt` | 52: the three Attribute descriptors and the Buffers |
| `tools/mali/testdata/frame_rt_rgba16f.expect.txt` | 54: the RGB Render Target, blend memory format, clear words, tile size |

## 2. Case 4: face culling

**GL calls** (`driver/tools/pvrlab/gl_test.c:2109-2202`): one yellow triangle,
wound CCW in GL window space, `glEnable(GL_CULL_FACE)`, then six draws of
`glDrawArrays(GL_TRIANGLES, 0, 3)`: front CCW + cull BACK (visible), CCW +
FRONT (gone), CW + BACK (gone), CW + FRONT (visible) on the default (window,
top-down) surface; then CCW + BACK (visible) and CCW + FRONT (gone) into an
RGBA8 texture FBO. A refused draw is the case's only blocker: the GL core
already maps the state (`src/gl/draw.rs:1376-1395`, `map_cull`: `GlesDraw.cull`
= `GLES_CULL_CW` or `GLES_CULL_CCW`, the winding of the faces culled; FRONT_AND_BACK
polygons never reach the backend, `culls_everything`).

**Mesa** (`pan_csf.c:1544-1559`, the CSF path the T6 takes; `pan_jm.c:524-536`
is the same for the job manager):

- `Cull Front Face` = `polygon && (rast->cull_face & PIPE_FACE_FRONT)`
- `Cull Back Face` = `polygon && (rast->cull_face & PIPE_FACE_BACK)`
- `Front Face CCW` = `rast->front_ccw`
- `polygon` = `u_reduced_prim(mode) == TRIANGLES` (lists, strips and fans);
  points and lines are drawn even with every face culled.
- st/mesa: `front_ccw = (FrontFace == GL_CCW)`, inverted for a clip origin of
  upper-left and for a Y-0-bottom framebuffer (`st_atom_rasterizer.c:77-92`);
  `cull_face` from `CullFaceMode` (`:110-127`).

These are `DCD Flags 0` bits 16 (`Front Face CCW`), 17 (`Cull Front Face`), 18
(`Cull Back Face`) of the word the CS loads in r57 (`DCD0`,
`src/mali/frame.rs:2940`), `src/mali/genxml/v10.rs` `DcdFlags0.front_face_ccw`,
`.cull_front_face`, `.cull_back_face`. Board evidence that exists:
`tools/mali/board-dump/flip-2026-10-19.txt` r57 `0x121AB` (a window, CCW, bit 16)
and `0x21AB` (an FBO, CCW); every pandecode dump on file has
`Cull front face: false`, `Cull back face: false`, so the two cull bits have
**no Mesa dump yet**.

**How src/mali builds it.** `frame.rs:2937` already sets
`front_face_ccw: d.front_cw != d.flip_y` (the stencil's face needs it), with a
comment "culling is not built" (`frame.rs:2886`). Add the culled winding to
`DrawCall` (from `GlesDraw.cull`, replacing `q.cull = GLES_CULL_NONE` only in
the clear quad, `backend.rs:1169`) and set, in the same `DcdFlags0` literal:

```
polygon         = prim is TRIANGLES / STRIP / FAN
cull_front_face = polygon && cull != NONE && ((cull == CW) == front_cw)
cull_back_face  = polygon && cull != NONE && !cull_front_face
```

(`front_cw` is `glFrontFace(GL_CW)`; the six draws give `cull_front_face`
0,1,0,1,0,1 and `cull_back_face` 1,0,1,0,1,0, r57 `0x521AB, 0x321AB, 0x421AB,
0x221AB, 0x421AB, 0x221AB`, the table in `frame_cull.expect.txt`.) The `check`
branch at `backend.rs:1230` is removed and its row in `backend_tests.rs:903`
(`("cull", ..., not_built(NOT_BUILT_CULL))`, and the constant's entry at
`:1083`) becomes a build row.

**Pinned by.** A new `src/mali/frame_cull_tests.rs`, shaped like
`frame_flip_tests.rs` (build the draw, read the IDVS r57 word and compare it with the
board dump's): the six words above from a `tools/mali/board-dump/cull-<date>.txt`
cut from the Mesa reference of section 5, plus the point and line draws with
culling on (bits 17/18 clear). Until that dump exists the test can pin the
table in `frame_cull.expect.txt`, but only as a derived expectation.

## 3. Case 52: per-instance attributes

**GL calls** (`gl_test.c:7547-7674`): `gl_test_es300_inst2_vs_src` (ES 3.00:
`pos` vec4 location 0, `offs` vec2 location 1, `icol` vec4 location 2; **no
`gl_InstanceID`**, so the built-in-input study does not gate this case);
8x8 vertices (`float`, stride 16) with 294 `u16` indices in one buffer;
instance data of 40 records of 6 floats (stride 24) in another, `offs` at offset
0 and `icol` at offset 8 sharing it; `glVertexAttribDivisor(1, 1)` and `(2, 1)`;
`glDrawElementsInstanced(GL_TRIANGLES, 294, GL_UNSIGNED_SHORT, 0, n)` with
n = 8 opaque and 40 with `glBlendFunc(GL_ONE, GL_ONE)`. The "split" is the PowerVR
backend's option (`gl_test_prim_budget` is a no-op without `GL_TEST_PVR`,
`gl_test.c:823`): on the Mali both halves are the same draw. The formats are
float32 (the `GLES_ATTRIBF_FLOAT32` check at `backend.rs:1896` passes);
`per_instance` is the only reason for the refusal, and
`backend.rs:1900-1911` (neither staged data nor buffer) is not reached
(the arrays are buffer objects).

**Mesa's Valhall attribute** (`pan_cmdstream.c:4083-4117`,
`panfrost_pack_attribute`, packed when the vertex elements are created):

| Field (pandecode) | per vertex (divisor 0) | per instance, divisor 1 |
|---|---|---|
| `Attribute type` | `1D` | `1D POT Divisor` (a power-of-two divisor), else `1D NPOT Divisor` |
| `Frequency` | `Vertex` | `Instance` |
| `Offset enable` | `true` | not set (false) |
| `Divisor R` | 0 | `ctz(divisor)`, 0 for divisor 1 |
| `Divisor E`, `Divisor D` | 0 | NPOT only: `pan_compute_npot_divisor` |
| `Table` | `PAN_TABLE_ATTRIBUTE_BUFFER` (2) | the same |
| `Format (v7)` | `pan_format_from_pipe_format(src_format)->hw` | the same |
| `Offset`, `Buffer index`, `Stride` | `src_offset`, `vertex_buffer_index`, `src_stride` | the same |

The Buffer is `{ Address = base + buffer_offset, Size = width0 - buffer_offset }`
(`:941-971`). The draw itself: `INSTANCE_COUNT` = `info->instance_count`
(`pan_csf.c:1706`), `src/mali/frame.rs:2963` already writes `d.instance_count.max(1)`.
Existing dumps show the vertex column (`mesa-uniidxi.pandecode:98-112`: `1D`,
`Offset enable: true`, `RGBA32F RGBA`, `Table: 2`, `Frequency: Vertex`); **no dump
on file has `Frequency: Instance`**. `frame_attr_instance.expect.txt` has the
three attributes: `RG32F RGB1` (vec2) and `RGBA32F RGBA` (vec4) with `Offset` 0 and 8.

**How src/mali builds it.** In `frame.rs` (the `Attribute` literal at
`:2683-2693`), `VertexAttribute` (`:993`) gains `instance_divisor: u32` (0 =
per vertex): `attribute_type` `_1D`/`_1D_POT_DIVISOR`, `frequency`
`AttributeFrequency::VERTEX`/`INSTANCE`, `offset_enable` true only for 0,
`divisor_r = ctz(divisor)`. `backend.rs:1896-1898` drops `per_instance` from
the refusal (keep a refusal for a divisor above 1: the Mali table's
`max_instance_divisor` is 0 = divisor 1, `backend.rs:2729`, so the core hands a
larger divisor as separate draws) and passes `a.divisor.max(1)` when
`per_instance`. `frame.rs:1951` (`require(d.instance_count <= 1, ...)`) goes,
and `NOT_BUILT_INSTANCED` with it (`frame_tests.rs:662`, `backend_tests.rs:3121`).
`src/mali` builds one Buffer per attribute (buffer n for attribute n,
`:2699-2704`); Mesa shares a Buffer between the two interleaved arrays of the
same buffer object. That differs in the Buffer table but not in what the
vertex shader reads; the Mesa dump will show which index Mesa gives them, and
the test pins the Attribute words and the Buffer address/size, not the index.

**Pinned by.** A new `src/mali/frame_instance_tests.rs` reading a
`tools/mali/board-dump/instsplit-<date>.txt` cut: the three Attribute
descriptors and the Buffers, `INSTANCE_COUNT` 8 and 40. Not a case the
drm-shim path can cover (`gl_ref.py` has no ES 3.0 instanced draw).

**Settled by the board dump (section 6).** The board's Mesa 25.0.7 writes
`Offset` 0 for `icol` and instead folds the offset 8 into the Buffer entry
(base + 8, size 952, its own buffer index 2); the picture is correct, so
src/mali does the same (not `Offset` 8, not a shared Buffer).

## 4. Case 54: RGBA16F colour targets

**GL calls** (`gl_test.c:7684-7813`): phases A and A2 are RGBA8 FBO textures
512..4 and six kicks each (these pass). Phase B, pass 1, is the refusal: for
sizes 128..4, a `GL_TEXTURE_CUBE_MAP` of `glTexImage2D(face, 0, GL_RGBA16F, n, n,
0, GL_RGBA, GL_HALF_FLOAT, NULL)`, each of six faces attached in turn
(`glFramebufferTexture2D`), `glClearColor(0,0,1,1)`, `glClear`, a uniform-colour
quad (`glDrawArrays(GL_TRIANGLES, 0, 6)`), no blend, nothing read back. The
sweep-1 log has 36 refused draws (6 sizes x 6 faces); nothing else is refused.
The GL core already hands `GlesDraw.target_format = GLES_SURF_RGBA16F`
(`src/backend.rs:299`). Only `RGBA16F` is needed for the case; the other
`GLES_SURF_*` (R16F, R32F, the integers, sRGB8_A8) stay refused.

**Mesa's descriptors** (RGBA16F is a blendable format on v9+:
`pan_format.c:117-121`, `BFMT_FLOAT(R16G16B16A16_FLOAT, RGBA16F)`):

| Descriptor | Field (pack name) | Value |
|---|---|---|
| RGB Render Target | `internal_format` | **board dump: `RAW64` (0x23), not `RGBA16F` (0x12)**; see section 6 |
| | `writeback_format` (bits 35..39; `writeback_float_format` overlays it) | **board dump: `RAW64` (5), not `FloatColorFormat::RGBA16F` (0x13)** |
| | `swizzle` | RGBA identity `0x688` (`get_rt_formats`, `pan_desc.c:677-714`) |
| | `writeback_block_format`, `writeback_msaa`, `dithering_enable`, `write_enable`, `srgb` | as RGBA8: Linear, Single, true, true, false |
| | `clear` (blue 0,0,1,1) | raw halves, `pan_pack_raw` (`pan_clear.c:61-113`: a float format is raw): `color_0 0`, `color_1 0x3C003C00`, `color_2 0`, `color_3 0x3C003C00` |
| Blend | `memory_format` | `Format::RGBA16F << 12 \| RGBA` = `0xDF000` (`BFMT_FLOAT`, `MALI_BLEND_RGBA16F`, `pan_format.c:34-37`; `pan_dithered_format_from_pipe_format`, `pan_format.h:355-363`) |
| Framebuffer | tile and `color_buffer_allocation` | 8 bytes a sample (`pan_format_tib_size`, float: raw, `pan_format.h:127-139`): tile `min(16384 >> 3, 1024)` = 1024 pixels, allocation 8 * 1024 = 8192 |
| Fragment shader | | not a function of the format for a blendable target on v9+ (`pan_shader.c:173-176, 335-350` are arch <= 5) |

**How src/mali builds it.** `TargetFormat` (`frame.rs:877`) gains `Rgba16f`
with `bytes()` 8, `formats()` `(RGBA16F, ..)` plus a way to name the writeback in
`FloatColorFormat` (the packer has two overlaying fields; set
`writeback_float_format` and leave `writeback_format` default), `swizzle()`
`s(0,1,2,3)`, `blend_memory_format()` and `preload_blend_format()` `Format::RGBA16F.0 << 12 | RGBA`,
`preload_view()` the `RGBA16F` texture format with a `RAW64` clump, `chan_size()` 16,
`has_alpha()` true. `tile_size()` (`frame.rs:1580`) takes the bytes a sample
from the format (it hard-codes 4). `clear_colour()` (`:1544`) packs fp16 halves
for this format (the `_ =>` arm packs 8-bit). `backend.rs:1239-1245` maps
`GLES_SURF_RGBA16F`. Texture-side: the sampler of a later pass reading the face
as `RGBA16F` is a texture-format matter and is not part of this case (no
read-back).

**Pinned by.** `frame_formats_tests.rs` (it reads `board-dump/formats-2026-10-19.txt`
for XRGB8888 and RGB565): an `f16rt` section with the render target, blend and
framebuffer words from the section 5 dump; until then `frame_rt_rgba16f.expect.txt`
is the derived expectation. `backend_tests.rs:881` (`("RGBA16F", ..., not_built(NOT_BUILT_TARGET_FORMAT))`)
becomes a build row with R16F in its place as the refusal example.

## 5. What a board Mesa reference must record

For `tools/mali/mesa-board-ref.sh` (the board's Mesa 25.0.7 reaches ES 3.1;
`2026-10-24-mesa-refs4` shows the harness; pandecode with
`linear,nocrc,dump,trace,sync`). Three modes in one `cases.c` (bodies copied
from `gl_test.c` with its ES 3.00 shaders, a 64x64 RGBA8 pbuffer + D24S8 for
4 and 52, an ES 3 context for 52 and 54), twice each, rc 0 and 0 kernel
messages checked as in the earlier sessions:

1. **`cull`** (case 4): the six draws in order. Record, for each draw's
   `Draw`/`DCD Flags 0`: `Front Face CCW`, `Cull front face`, `Cull back face`,
   and the CS words r57 (the table in `frame_cull.expect.txt`); and read the
   six pixels (visible, gone, gone, visible, visible, gone). A seventh draw
   with `GL_LINES` and cull back on, to show `Cull back face: false` for lines.
2. **`instsplit`** (case 52): the n = 8 opaque draw and the n = 40 additive
   draw. Record every `Attribute` (`Attribute type`, `Offset enable`, `Format (v7)`,
   `Table`, `Frequency`, `Divisor R/E/D`, `Offset`, `Buffer index`, `Stride`),
   every `Buffer` (`Address`, `Size`), the CS `INSTANCE_COUNT` word, the
   Blend descriptor's equation for `ONE, ONE`, and the centre pixel of both
   draws (the picture the case compares). The open (?) of section 3 is
   settled by the `Offset` of `icol`.
3. **`f16rt`** (case 54 phase B pass 1): one 128x128 face and one 4x4 face of a
   `GL_RGBA16F` cube map, cleared blue and drawn, plus the same face as `GL_RGBA`
   for comparison. Record the `RGB Render Target` words (`Internal Format`,
   `Writeback Float Format`, `Swizzle`, `Writeback Block Format`, the
   four `Clear` words), the Blend descriptor's `Format` word, the Framebuffer
   descriptor's tile size and `Color buffer allocation`, and the fragment shader's
   bytes in both runs (equal to each other, the claim above). A read of the
   fp16 face is not needed.

Then `tools/mali/board_dump_cut.py` cuts the `board-dump/{cull,instsplit,f16rt}-<date>.txt`
the three tests above read, and the three `expect.txt` files are replaced by
(or checked against) the cut.

## 6. Settled by the board dumps (2026-10-24-mesa-refs5)

Mesa 25.0.7 on the NanoPC-T6, two runs per mode, rc 0, no kernel messages;
`0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs5/` (README: every prediction, met or
not) and the cuts `tools/mali/board-dump/{cull,instsplit,f16rt}-2026-10-24.txt`.
Compared with `tools/mali/testdata/frame_{cull,attr_instance,rt_rgba16f}.expect.txt`:

- **cull: no differing byte.** r57 0x521AB, 0x321AB, 0x421AB, 0x221AB, 0x421AB,
  0x221AB for the six draws, as the table; the `GL_LINES` draw has `Cull front
  face` and `Cull back face` false and r57 0x121AB (the (?) of the line case:
  settled, bits 17 and 18 clear, the rest equal to frame 1's).
- **instsplit differs in three values.** (1) `Offset` of `icol` is **0**, not 8
  (the expect file's attr 2 row): Mesa puts the offset in the Buffer (entry
  base + 8, `Size` 952 = 960 - 8). (2) There are three Buffer entries, not two:
  index 0 the grid (1024), 1 the instance data (base, 960), 2 `icol`'s
  (base + 8, 952); each attribute has its own index (1 and 2 for the instanced
  pair). The (?) types are settled as predicted: `1D POT Divisor`, `Offset
  enable` false, `Divisor R/E/D` 0, `Frequency: Instance`, `RG32F RGB1` /
  `RGBA32F RGBA`, stride 24. (3) The n = 40 additive centre pixel is
  `0xffff1400` (the prediction of the README, 0xffff0a0a, was wrong; the case
  only checks "not black"). `INSTANCE_COUNT` (r34) is 8 and 0x28. The
  `ONE, ONE` draw is `Mode: Fixed-Function`, `Load Destination` true, `A Src,
  B Dest, C Zero inverted` for RGB and alpha, mask 15; the opaque draw is
  `Mode: Opaque`. The (?) of the open paragraph of section 3 (Mesa sets
  `Offset` unconditionally) is therefore answered the other way: Mesa's
  `Offset` is 0 and the hardware reads the shifted base.
- **f16rt differs in the render target's formats, not in the rest.** A
  `GL_RGBA16F` cube face is written as **`Internal Format` `RAW64` (0x23) and
  `Writeback Format` `RAW64` (5)**, not `RGBA16F` (0x12 / 0x13) as derived from
  Mesa 26.2.1's source (the table of section 4: both rows corrected above;
  `writeback_float_format` is not used). Everything else of the derivation
  holds: `Swizzle` 0x688, `Writeback Block Format` Linear, `Writeback MSAA`
  Single, `Clear` 0, 0x3C003C00, 0, 0x3C003C00, blend `Format (v7): RGBA16F
  RGBA`, `Effective Tile Size` 1024, `Color Buffer Allocation` 8192 (RGBA8
  frames 4096), and the fragment shader buffer is byte-identical in the 16F
  and RGBA8 frames. Row strides: 1024 (128x128 float face), 512 (RGBA8); the
  4x4 faces are **64** for both formats, not 32 and 16 (a row is padded to 64
  bytes). The centre texels read back (1, 0, 0, 1) and (255, 0, 0, 255).
- Caveat: these are Mesa **25.0.7** (the board's), the derivation read Mesa
  **26.2.1**'s source; where they differ (RAW64) the board's dump is what the
  hardware was shown to accept. A drm-shim reference under 26.2.1 would say
  whether 26.2.1 also packs RAW64.
- Consequence of RAW64: the tile buffer holds the face as raw 8-byte texels,
  a layout the fixed-function blender does not read, and 25.0.7's
  `panfrost_get_blend` (`pan_context.c:178-197`) runs fixed function on a
  non-blendable format only for an opaque write, a blend shader otherwise.
  The dump covers only the opaque draw of case 54, so `src/mali` builds an
  RGBA16F target for opaque writes of every channel only: blending that is
  not `ONE, ZERO` or a colour mask other than 0xf is refused by name
  (`frame::NOT_BUILT_FLOAT_BLEND`, `check_draw()`), and a blend shader into
  it stays `NOT_BUILT_BLEND_SHADER` (`blend::shader_equation` is RGBA8888
  only). The colour preload of an RGBA16F target (a frame without a clear)
  follows the opaque draw's pattern (conversion `RGBA16F`, `raw` false over
  the RAW64 buffer); no dump pins it, and a board item should say so.

## 7. Order of work

Case 4 is the smallest (a DCD word) and independent of the others. Case 52 is a
frame change plus removing a backend refusal. Case 54 touches the most places
(`TargetFormat` is matched everywhere in `frame.rs`) but is mechanical once the
dump fixes the five values above. None needs a compiler change. After each:
the sweep's PASS set may only grow (cases 4, 52, 54 are REFUSED in
`2026-10-23-s6-discard`), and `MALI_TESTS` in `tools/mali/gate.env` moves with
the tests added.
