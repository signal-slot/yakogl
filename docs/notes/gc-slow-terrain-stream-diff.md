<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: terrain's streams against Mesa etnaviv's, pass by pass and state by state

Plan #328, item terrain-stream-diff, issue #1204. `gc-slow-terrain.md` counts
terrain's 52 kicks and prices them. `gc-slow-stream-diff.md` (item
stream-diff, on main at `31869a4c`) puts shadow's and refract's streams beside
Mesa's and gives terrain one summary table. This note does terrain in full.
For each pass of `gc-slow-terrain.md` "The 52 kicks", it lists our state
against Mesa's register by register: PE, RA, TS, tiling, cache flushes,
semaphores and stalls, RS and BLT. Then it ranks the differences by their
cost on the board and says which planned items already cover each one.

Host only. No board was run and no code was changed. Code is cited as
`path:line` at `d0f5813e`. Mesa is cited as `mesa:path:line` in Mesa 26.2.1.

## Answer in short

- **The shim's GC7000UL entry does not describe the i.MX8MP.**
  `tools/vivante/mesa-cs-golden.patch` gives it customer id 0. With that id,
  Mesa's hwdb lookup finds NXP's informal row `gc7000UL_6200` (revision
  0x6202), not `gc7000ULN_v123` (0x6204, customer 0x11), which is the row
  `src/vivante/profile.rs:101-107` and `backend.rs:760-768` take for the
  board. The two rows differ in 27 fields, among them `V4Compression` (1 /
  0), `REG_ZCompression` (1 / 0) and `Streams` (8 / 16). This note ran Mesa
  twice, once with each row:
  - **Row 6202** (the patch's entry, the row `gc-slow-stream-diff.md` read):
    colour and depth TS compression, and 6 in-place RS resolves a frame.
  - **Row 6204** (customer 0x11, the board's row as the code takes it): no
    compression, no resolves, TS buffers twice as large.

  Everything below compares against row 6204 unless a row 6202 value is
  given. Fixing the shim entry is new issue 1.
- **Mesa records terrain's frame as 2 submits, we record 52.** The words
  are about the same: 20,288 against our 18,964 (20,446 on row 6202). Mesa's
  frame has 51 draws and 11 RS fills of tile-status (TS) buffers. Ours has
  59 draws (8 of them clear quads) and no RS work. Mesa splits the frame in
  two only because its stream buffer is full (`mesa:src/etnaviv/drm/etnaviv_cmd_stream.c:63-66`:
  0x4000 words at most). Before every draw, both drivers write the same
  semaphores and stalls.
- **The differences that cost the most are already covered by planned
  items:**
  - the shaders' instruction counts (noise-compiler-fix,
    terrain-compiler-fix): the largest term, though not a stream state;
  - kick boundaries (mip-one-kick, gc-frames-ring, fence-irq): 9 to 14 ms;
  - early depth on the terrain and overlay draws (gc-early-z, gc-tiled-wire
    W4/W5, tiled-depth): 0 to 14 ms;
  - clears drawn as quads instead of TS fills (gc-tiled-wire W7 to W9; the
    RS fill of the surface is stream-diff's finding 1, which has no item):
    1.7 to 8.9 ms;
  - CPU tiled copies of the height and normal maps (tiled-copy-rs): 4 to 6 ms;
  - linear textures, where Mesa samples super-tiled ones (gc-tiled-wire
    W10/W11): not modelled.
- **New, with no item:**
  1. The shim row above. It changes what Mesa programs for TS
     (compression, resolves, TS sizes), so gc-ts-words and gc-tiled-wire W8
     should read row 6204's words.
  2. Terrain's `GL_DEPTH_COMPONENT16` renderbuffer is stored and tested as
     D24S8. Mesa uses D16, so we move twice the depth bytes: **1.3 to
     2.5 ms**.
  3. Draws into the XRGB8888 window lose PE OVERWRITE. tilt_v writes
     COMPONENTS 0x7 with no OVERWRITE, so the PE reads the destination
     back. Mesa's rule writes 0xf with OVERWRITE for an X format: **0.5 to
     1 ms** in terrain, and the same for every full-screen layer of every
     glmark2 scene.
  4. A clear quad into a colour-only target still runs the PE depth stage,
     against a scratch D24S8 surface, while Mesa's draws there have
     `DISABLE_ZS`: **0 to 1.5 ms**. It goes away if clears stop being quads.

  Proposed issues for all four are at the end.

## How the streams were made

Both sides ran glmark2 at the pinned `22c527cb` (the dependency cache's
`glmark2-src`), at 800x600, with `-b :nframes=3 -b terrain`.

**Ours.** The GC7000 build of the glmark2 host harness, unmodified, with the
Linux OSAL's stream dump (`src/osal/linux.c:409-431`). A wrapper script set
the environment, because this session's sandbox refuses command lines that
set environment variables:

```sh
YAKOGL_CS_DUMP=<dir>/terrain.cs GM2_WRITE=0 GM2_FRAMES=3 GM2_ENTRIES='^terrain$' \
  tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" <out> vivante
```

The run settings were `YAKOGL_GC_CARVEOUT_BYTES=245642240` (`gc-settings.txt`),
and the scene ended `ok frames=3 draws=25 refused=0 gl_errors=0`. The file
holds 238 submits: setup is 0-81, and the three frames are 82-133, 134-185
and 186-237. All three frames have the same 52 submit sizes. The frame used
here is 186-237.

**Mesa's.** Mesa 26.2.1, the archive.mesa3d.org tarball (sha256
`c47e81bd...a4442`, the same as `third_party/mesa-26.2.1/UPSTREAM`), patched
with `tools/vivante/mesa-cs-golden.patch` and built with
`tools/vivante/README.md`'s recipe ("Building it"). Ninja ran all 1022
steps. The tree is `$CONNECTED_DEPS_DIR/mesa-26.2.1-etnaviv-golden/mesa-26.2.1`
and the build is `../build-host`.

This host has no `pycparser`, and it could not be fetched. So
`src/etnaviv/hwdb/pycparser/` in that tree is a stand-in of about 200
lines: it reads the five vendor `gc_feature_database.h` files into the node
classes that `hwdb.h.py` visits, and `hwdb.h.py` itself runs unchanged. The
generated `hwdb.h` was checked field by field against
`nxp/gc_feature_database.h`: all 891 fields of all 11 GC7000 rows are equal.
A host that has `pycparser` needs no stand-in.

The tree's shim also has one entry that the patch does not have,
`C11GC7000UL`: the patch's GC7000UL entry with customer id 0x11. The shim
matches `ETNA_SHIM_GPU` as a prefix
(`mesa:src/etnaviv/drm-shim/etnaviv_noop.c`, `strncasecmp`), hence the name.

glmark2 was built as its `null` flavour (`GLMARK2_USE_NULL`, upstream
`main.cpp` and `native-state-null.cpp`, plus `run.sh`'s list of sources)
against Mesa's EGL headers. It ran off-screen against the shim, with
`<gpu>` set to `GC7000UL` (row 6202) or `C11GC7000UL` (row 6204):

```sh
LD_PRELOAD=$B/src/etnaviv/drm-shim/libetnaviv_noop_drm_shim.so \
LD_LIBRARY_PATH=$B/src/egl:$B/src/mesa/glapi/es2api:$B/src/gallium/targets/dri \
MESA_LOADER_DRIVER_OVERRIDE=etnaviv ETNA_SHIM_GPU=<gpu> MESA_SHADER_CACHE_DISABLE=true \
ETNA_SHIM_CS_DUMP=<dir>/terrain.cs ETNA_SHIM_MEM_DUMP=<dir>/terrain.mem \
  glmark2-es2-null --off-screen --size 800x600 -b :nframes=3 -b terrain
```

Both runs printed `GL_RENDERER: Vivante GC7000 rev 6204`. Each file holds 8
submits: 3 of setup and the first frame, then 2 a frame (row 6204: 15,910 +
4,378 words; row 6202: 16,040 + 4,406). The frame used here is submits 6
and 7. The frames before them have the same sizes.

**Reading them.** A throwaway script, not committed, walked both files with
`tools/vivante/cs_decode.py`'s register names (Mesa's `state_3d.xml.h` and
`state.xml.h` from that tree). It cut each stream into operations (a draw,
an RS kick, a BLT kick) and printed, for each one, the flushes, semaphores
and stalls written since the previous operation and the state it ran with.
Mesa's texture descriptors were read from the memory dump with
`cs_decode.Mem` and `TexDesc`. Unlike `cs_decode.py`'s own decoder, the
script sizes `DRAW_INSTANCED` (opcode 0x0c, 4 words) and `DRAW_PRIMITIVES`
(4 words). `cs_decode.py`'s table names 0x0e for `DRAW_INSTANCED` and steps
over these commands one word at a time. That is harmless for a register
diff, but it miscounts draws.

**One difference in the shape of the run** (as in `gc-slow-stream-diff.md`).
Under `--off-screen`, Mesa's canvas is an RGBA8 FBO with D24 depth, tiled
with TS. On the board and in our harness, the canvas is the XRGB8888 EGL
window (`driver/zephyr/lib/board/imx8mp/board_imx8mp.c:333`). So the canvas
clear and tilt_v are not like for like. Every other pass is the same GL
calls on both sides.

## The same in every pass

Notation: `F27` is GL_FLUSH_CACHE 0x27 (colour, depth, texture, shader L1),
`F07` is the same without shader L1, `F10` is the vertex-texture flush and
`F03` is colour and depth. `S`/`T` is a semaphore and stall-token pair,
`St` is the FE to PE STALL command and `TSF` is TS_FLUSH_CACHE.

| state | ours | Mesa |
|---|---|---|
| sync before a draw | `F27 F10 S(RA>PE) T TSF S(FE>PE) St(FE>PE) S(RA>PE) T`, every draw (`backend.rs:1790-1795`, `:2162`, `:2177`) | the same sequence. `F07` replaces `F27` before a u_blitter mip level. The first draw of a submit also has `F3000`, and a draw after a TS fill also has `F03 S T TSF` |
| sync around an RS kick | none (no RS work) | `F03 S(RA>PE) T [TSF]` before it |
| kick start | context init and `F3000` in every kick (`backend.rs:2186-2219`, `:2254`); on the board, the OSAL adds prologue and epilogue around it (`driver/rust/src/gpu/vivante/cmdbuf.rs:157-190`) | the same, once a submit: twice a frame |
| colour surface | linear: `PE_COLOR_FORMAT` without SUPER_TILED, `PE_LOGIC_OP` SINGLE_BUFFER 1 (`backend.rs:1649-1683`, `:1734-1745`) | super-tiled: `PE_COLOR_FORMAT` 0x00112f06 (SUPER_TILED, SUPER_TILED_NEW, OVERWRITE), SINGLE_BUFFER 2, or 3 with a 16-bit depth (`gc-tiled-targets.md:167`) |
| TS | `PE_MEM_CONFIG` 0 and `TS_MEM_CONFIG` 0 in every draw (`backend.rs:1751-1752`) | a pass's draw has colour TS: `TS_MEM_CONFIG` 0x2 (COLOR_FAST_CLEAR), `PE_MEM_CONFIG` 0. Row 6202: 0x382 (adds COLOR_COMPRESSION, format 3), `PE_MEM_CONFIG` 0x01000000. Mip-level draws have no TS |
| TS fill sizes (RS window, 16 x n words) | none | row 6204: 800x600 colour 16x132, 800x600 depth16 16x68, 256x256 colour 16x16. Row 6202: half that (16x68, 16x36, 16x8) |
| textures | linear: `CONFIG0` ADDRESSING_MODE linear, HALIGN 0 (`texture.rs:271-299`), `TX_CTRL` 0 (`texture.rs:597`) | super-tiled: `CONFIG0` ADDRESSING tiled, `CONFIG1` HALIGN 2 (0x08321000 / 0x08521000). A chain's level-0 read goes through the sampler TS (`TX_CTRL[0]` 0x42 with TS_ENABLE, `TS_SAMPLER_CONFIG[0]` 0xf1; row 6202: 0x33) |
| BLT | none | none: the GC7000UL has no BLT engine (`gc-tiled-targets.md:117`) |

## Pass by pass

The ms in this section are those of `gc-slow-terrain.md`'s model and of
`gc-slow-stream-diff.md`'s: 4.1 ns a pixel for a quad clear, an RS fill
rate of 2 to 8 GB/s (unmeasured), and a TS fill taking microseconds. "K"
numbers are our kicks within the frame.

### canvas clear (`canvas_.clear()`, the window)

| | ours | Mesa |
|---|---|---|
| operations | K1 alone (308 words): one 800x600 colour and depth quad. `PE_COLOR_FORMAT` 0x00010f05 (X8R8G8B8, OVERWRITE), `PE_DEPTH_CONFIG` 0x00041711 (D24S8, ALWAYS, write) | 2 RS TS fills (`RS_CONFIG` 0x4606, `RS_CLEAR_CONTROL` 0x0001ffff, fill 0x11111111, 16x132 for colour and for depth), recorded right after the end of the previous frame |
| why the kick ends | the noise pass changes the target (`backend.rs:2903`) | no kick |

Differences: the clear quad (1.97 ms of GPU) and one kick boundary.

### noise (height map, `SimplexNoiseRenderer`, 256x256)

| | ours | Mesa |
|---|---|---|
| operations | K2 (572 words): a clear quad, then the noise quad (6 vertices) | a TS fill (16x16), then the draw (6 vertices) |
| clear's PE depth | `PE_DEPTH_CONFIG` 0x00040711: D24S8, ALWAYS, no write, **no DISABLE_ZS**, on a ds_cache scratch surface (`backend.rs:1448`, `:1518`) | no clear draw; draws into a colour-only target use `PE_DEPTH_CONFIG` 0x01000700 (DISABLE_ZS) |
| draw's PE | linear, 0x00010f06, depth 0x01040711 (DISABLE_ZS) | super-tiled with colour TS (`TS_MEM_CONFIG` 0x2), depth 0x01000700 |
| RA | `RA_EARLY_DEPTH` 0x15000030 | 0x15000030 |
| shader | 840 instructions (`gc-terrain-shaders.md`) | etnaviv's count (same note) |

On row 6202, Mesa adds an **in-place RS resolve of this same target**
between the TS fill and the draw (`RS_CONFIG` 0x4686, source = destination,
`RS_SOURCE_STRIDE` 0xa8001000). glmark2 leaves the height map bound on
texture unit 2, and a compressed TS cannot be sampled without V4
compression (`mesa:src/gallium/drivers/etnaviv/etnaviv_texture.c:147-149`),
so the draw's sampler validation resolves it (`:189-199`). That costs 0.26 MB
read and written. Row 6204 has no compression, so no resolve.

### normal map (`NormalFromHeightRenderer`, 256x256)

The same pattern. Ours is K3 (560 words): a clear quad with the depth stage
as in noise, then the draw. Mesa's is a TS fill, then the draw (TS 0x2).
Mesa's draw samples the super-tiled height map (`TX_CTRL[0]` 0x40, no TS);
ours samples it linear. Shader: 25 against 21 instructions.

### terrain (`TerrainRenderer`, 800x600 with a depth16 renderbuffer)

| | ours | Mesa |
|---|---|---|
| before the kick | `finish()`, then the CPU tiles the height map and the normal map (`tiled_copy.rs:369-371`, `:426-429`, from `backend.rs:2873`), because this core's vertex stage does not read a linear texture (`tiled_copy.rs:170-180`) | nothing: vertex samplers 4 to 6 point at the two render targets themselves (descriptors' `LOD_ADDR[0]` = the noise and normal targets, super-tiled; `TX_CTRL` 0x50, 0x54, 0x58) |
| clear | a 800x600 colour and depth quad (`PE_DEPTH_CONFIG` 0x00041711) | 2 TS fills: colour 16x132, depth 16x68 |
| colour | linear 0x00010f06, stride 0xc80, no TS | super-tiled 0x00112f06, TS |
| depth | **D24S8**, stride 0xd00 (4 B a pixel), row-linear (`backend.rs:1497-1525`) | **D16**, stride 0x680 (2 B a pixel), super-tiled |
| depth test | **late**: `RA_EARLY_DEPTH` 0x15000030 (WRITE_DISABLE, no TEST_ENABLE), `PE_DEPTH_CONFIG` 0x00041311 (`backend.rs:1770-1784`) | **early**: `RA_EARLY_DEPTH` 0x05000031 (TEST_ENABLE), `PE_DEPTH_CONFIG` 0x05051301 (EARLY_Z, DISABLE_ZS, SUPER_TILED, D16, LEQUAL, write) |
| TS | `TS_MEM_CONFIG` 0 | 0xb: COLOR and DEPTH_FAST_CLEAR, DEPTH_16BPP. Row 6202: 0x3cb (adds COLOR and DEPTH_COMPRESSION) and `PE_MEM_CONFIG` 0x05000000 |
| `PE_LOGIC_OP` | 0x000e410c (SINGLE_BUFFER 1) | 0x000e436c (SINGLE_BUFFER 3) |
| the draw | 393,216 vertices, non-indexed, `F27 F10 S T TSF S St S T` | the same vertices and the same sync |

Differences: early depth, the depth format, layout and TS, the colour TS,
the clear, and the CPU copies with their `finish()`.

### terrain's chain (9 levels, `base-renderer.cpp:84-91`)

| | ours | Mesa |
|---|---|---|
| operations | 9 kicks of 328 words, one a level (`mipmap.rs:266-281`), each with its own context init | 9 u_blitter draws in the open stream, about 200 register writes each |
| a level's target | linear, the chain's one stride 0xc80 (`mipmap.rs` `chain_layout`), `PE_DEPTH_CONFIG` 0x01040711 | super-tiled level surfaces, each with its own aligned stride (0x700, 0x400, ...), no TS, `PE_DEPTH_CONFIG` 0x01000700 |
| level 0 read | linear | super-tiled, through the sampler TS (`TX_CTRL[0]` 0x42) |
| sync | the draw sequence above, plus `F3000` at each kick start | `F07 F10 S T TSF S St S T` |

The difference is the kicks (and our CPU's waits behind them). The pixels
are the same, and the level-to-level flushes differ only in shader L1.

### bloom_h (`BlurRenderer`, 256x256, then 8 levels)

Ours: K14 (572 words) is a clear quad (with the depth stage as in noise)
and the 5-tap blur, then 8 one-level kicks. Mesa: a TS fill, the blur draw
(TS 0x2) and 8 levels in the stream. On row 6202, Mesa first adds an
in-place resolve of the 800x600 terrain target (1.92 MB): a compressed or
mipmapped view cannot use the sampler TS
(`mesa:src/gallium/drivers/etnaviv/etnaviv_texture.c:147-156`). Shader: 42
against 24.

### bloom_v (256x256, then 8 levels)

The same as bloom_h. On row 6202, Mesa first resolves bloom_h's 256x256
target in place. Shader: 47 against 24.

### overlay (into the terrain target, then its 9 levels again)

| | ours | Mesa |
|---|---|---|
| operations | one kick of 332 words: a 6-vertex quad, blended (`PE_ALPHA_CONFIG` 0x01400141, `PE_COLOR_FORMAT` 0x00000f06 without OVERWRITE), depth LEQUAL with write, late (0x00041311, RA 0x15000030); then 9 one-level kicks | the same quad and blend with **early depth** (RA 0x05000031, PE 0x05051301, `TS_MEM_CONFIG` 0xb), then 9 levels in the stream. Row 6202 resolves bloom_v's target first |

Differences: early depth, the D16 depth bytes, and the kicks.

### tilt_h (800x600, then 9 levels)

Ours: a clear quad (colour only, with the depth stage on a scratch D24S8
surface, 0x00040711) and the 9-tap blur, then 9 one-level kicks (580 words,
then 9 x 328). Mesa: a TS fill (16x132), the blur draw (TS 0x2) and 9 levels
in the stream. Shader: 66 against 46.

### tilt_v (into the window)

| | ours | Mesa (an FBO, not a window) |
|---|---|---|
| operations | the last kick (580 words): a colour and depth clear quad, then the blur | 2 TS fills (canvas colour and depth), then the blur. Row 6202 resolves tilt_h's 800x600 target in place first |
| `PE_COLOR_FORMAT` | **0x00000705**: X8R8G8B8, COMPONENTS 0x7, **no OVERWRITE** | 0x00112f06: COMPONENTS 0xf, OVERWRITE |
| depth | D24S8 window depth, late, LEQUAL with write (0x00041311) | D24S8 super-tiled, early (0x05051311, RA 0x05000031), `TS_MEM_CONFIG` 0x3 |

COMPONENTS 0x7 comes from `no_alpha` (`backend.rs:1460`, `:1473`): the window
is XRGB8888, so the GL core says the target has no alpha bits
(`src/gl/framebuffer.rs:982-988`). OVERWRITE is set only when the mask is 0xf
(`backend.rs:1656`). So the PE reads every destination pixel back to keep
its X byte. Mesa decides OVERWRITE from the format's own channels:
`full_overwrite = fo_allowed && util_format_colormask_full(desc, colormask)`
(`mesa:src/gallium/drivers/etnaviv/etnaviv_blend.c:145-151`). For an X8
format, a full RGB mask is full, so Mesa writes 0xf with OVERWRITE. Mesa's
off-screen canvas is RGBA, so this golden cannot show that case itself; the
rule is read from Mesa's source. Shader: 75 against 46.

## The differences, ranked

The ms are board estimates a frame, with the costs above and those that
`gc-slow-terrain.md` and `gc-tiled-wire.md` already use. "GPU" is time the
GPU spends; "CPU" is time the GPU waits for the CPU.

| rank | difference | where (ours) | GPU or CPU | ms a frame | covered by |
|---|---|---|---|---:|---|
| 1 | shader length in every pass (noise 840; terrain 148 / 56; tilt 66 / 75; bloom 42 / 47; normal 25) against etnaviv's | `src/shaderc/online/vivante/**` | GPU | the largest term: 41.9 for the non-noise shaders (`gc-terrain-shaders-rest.md`), plus the noise's share (`gc-terrain-shaders.md`) | noise-compiler-fix, terrain-compiler-fix |
| 2 | 52 kicks against 2: each mip level (`mipmap.rs:266-281`) and each target change (`backend.rs:2903`) ends a kick. On the CPU, `frame_open` waits on the 4-slot ring (`backend.rs:2229-2235`, `:77`) with the 1 ms poll | as cited | CPU (starvation) and GPU (overhead per kick) | 9-14 (`gc-slow-stream-diff.md` finding 2) | mip-one-kick; gc-frames-ring; fence-irq. Target changes inside one stream: stream-diff finding 2 (b), no item |
| 3 | late depth on the terrain and overlay draws (RA 0x15000030 / PE 0x00041311 against 0x05000031 / 0x05051301) | `backend.rs:1770-1784`, `:1513-1520` | GPU | 0-14, up to 19 at 0.8 ns (`gc-tiled-wire.md` W5) | gc-early-z, gc-tiled-wire W4/W5, tiled-depth (texkill-scan for the rule's input) |
| 4 | 8 clear quads (2.18 Mpx) against 11 TS fills | `backend.rs:2346-2396`, called from `:3097` | GPU | 8.5-8.9 with TS fills; 1.7-6.8 with an RS fill of the surface | gc-tiled-wire W7-W9 (gc-ts-words gives their words); the RS fill is stream-diff finding 1, which has no item |
| 5 | CPU tiled copies of the height and normal maps with a mid-frame `finish()`; Mesa's vertex samplers read the render targets | `tiled_copy.rs:346-429`, `backend.rs:2873` | CPU (the GPU idles) | 4-6 (`gc-slow-terrain.md` rank 3) | tiled-copy-rs, then gc-tiled-wire W10 |
| 6 | **depth16 stored as D24S8**: stride 0xd00 against 0x680. Twice the depth bytes in the terrain draw, its clear and the overlay draw: about 0.3 to 0.5 M fragments in terrain and 0.48 M in overlay, each reading and writing 2 bytes more, plus 0.96 MB more of clear | `backend.rs:1497-1525` (always `GC_PE_DEPTH_FORMAT_D24S8`); `ds_cache.rs` sizes every surface at 4 B a pixel | GPU (bandwidth) | 1.3-2.5 | **new** (tiled-depth changes the layout, not the format) |
| 7 | linear textures, where Mesa samples super-tiled ones: tilt_v's and bloom_v's vertical taps, the normal map's 3 taps and every chain read | `texture.rs:271-299` | GPU (texture cache) | not modelled (`gc-tiled-wire.md` W11: "texture-cache locality (unmodelled)") | gc-tiled-wire W10/W11 |
| 8 | **window draws lose OVERWRITE**: tilt_v writes COMPONENTS 0x7 with no OVERWRITE on the XRGB8888 window, so the PE reads back 1.92 MB | `backend.rs:1460`, `:1473`, `:1656` | GPU (bandwidth) | 0.5-1 (1.92 MB read at 2 to 4 GB/s); the same for every full-screen window layer in other scenes | **new** |
| 9 | **a clear quad into a colour-only target runs the PE depth stage**: noise, normal, bloom_h, bloom_v and tilt_h clear with `PE_DEPTH_CONFIG` 0x00040711 (D24S8, ALWAYS, no write, no DISABLE_ZS) on a ds_cache scratch surface; Mesa's draws there have DISABLE_ZS (0x01000700) | `backend.rs:1448` (`depth_test` true for every clear), `:1518` (DISABLE_ZS excludes clears) | GPU (a depth read of up to 3 MB, if the PE fetches depth for ALWAYS) | 0-1.5 | **new** (goes away anyway if rank 4 stops drawing clears as quads) |
| 10 | a level's flush `F27` against Mesa's `F07`, and the per-kick context init | `backend.rs:1791`, `:2186-2219` | GPU | about 0 (`gc-slow-stream-diff.md` findings 5 and 6) | none needed |
| - | the shim's customer id selects row 6202. That row adds TS compression (`TS_MEM_CONFIG` 0x382, 0x3cb, 0x3c9, 0x3c3), halves the TS buffers and adds 6 in-place resolves a frame (4 at 256x256, 2 at 800x600; 9.7 MB moved, 1.2 to 4.9 ms that Mesa pays) | `tools/vivante/mesa-cs-golden.patch:84` (`ETNAVIV_PARAM_GPU_CUSTOMER_ID` 0x0) | none on the board: it changes the reference | 0 | **new**: it also answers `gc-tiled-wire.md`'s "Compression, an open point". Row 6202's `V4Compression` is 1 and row 6204's is 0, which is why the README's golden showed compression |

The CPU side has only ranks 2 and 5. Every other difference is GPU time
spent in the PE, the RA or the texture unit. No difference shows up in the
semaphores, stalls or TS flushes between draws: the two drivers write the
same sequence.

## What this note does not show

- No board numbers. Ranks 6, 8 and 9 are bandwidth estimates at the 1.9 to
  4 GB/s that the quad clear measured on this board
  (`imx8mp-frame-floor.md`, "Step 3 result"). Whether the PE reads the
  destination under COMPONENTS 0x7, or the depth surface under ALWAYS
  without a write, is the hardware's choice and was not measured.
- Which customer id the board's GC7000UL reports was not read here. The
  code takes 0x11 (`src/vivante/profile.rs:107`, `backend.rs:768`,
  `docs/es2-cts-failures.md:88`). The patch's comment says its entry is
  "as the Verdin iMX8M Plus reports it", with 0. `driver/rust/src/gpu/vivante/hw.rs:176`
  reads the register on the board. New issue 1 asks for that read.
- Mesa's canvas is an RGBA FBO with TS (see "How the streams were made"), so
  the canvas clear and tilt_v are not like for like. On a real window, Mesa
  would also render tiled with TS and resolve at the swap (gc-tiled-wire W13).
- On row 6204, Mesa samples the TS'd noise and normal maps, and their
  vertex-stage copies in the terrain draw, with neither the sampler TS nor a
  resolve (`TX_CTRL` 0x40, 0x50, 0x54, 0x58). That is right only if no tile
  of either map is left in the cleared state, which their full-target quads
  ensure. Why Mesa's flush tracking allows it was not worked out.
- The streams were compared for state, sync, RS and tiling. Shader code is
  compared in `gc-terrain-shaders.md` and `gc-terrain-shaders-rest.md`.
- Our harness dumps no memory here (`YAKOGL_MEM_DUMP` was not set). Our
  descriptors were read from `texture.rs`, not from a dump.
