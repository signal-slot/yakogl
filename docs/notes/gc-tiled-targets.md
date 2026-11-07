<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: tiled render targets, tile status, fast clear and early depth (study)

Plan #328, item tiled-target-study (issue #1204). Host study only. No code
was changed and no board was run. It answers F4 of
`docs/notes/imx8mp-frame-floor.md` ("a tiled colour target with tile status
and fast clear, and early depth") and the "Not done" of
`docs/notes/gc-terrain-tiled-copy.md` (rendering to a tiled texture
directly). It asks three things: what Mesa's etnaviv does on this part, what
the GC back end would need, and what each change would save in glmark2
terrain, shadow and refract.

Sources:

- **Mesa 26.2.1**, the release `third_party/mesa-26.2.1/UPSTREAM` names
  (commit `da14d65e`; the tree's `VERSION` reads 26.2.1). It was read from the
  full tarball tree in `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`.
  `third_party/` carries no etnaviv directory.
- Mesa citations name a file and its lines:
  - `etnaviv_*.c` and `.h` are under `src/gallium/drivers/etnaviv/`;
  - `hw/*.xml.h` is under `src/etnaviv/hw/`;
  - `hwdb` is `src/etnaviv/hwdb/nxp/gc_feature_database.h`;
  - `etna_hwdb.c` is `src/etnaviv/hwdb/etna_hwdb.c`.
- **The back end** as of main `a53b96b0`.
- **glmark2**: the source in `$CONNECTED_DEPS_DIR/glmark2-src` (the harness
  pins `22c527cb`, `tools/hostharness/glmark2/README.md:42`).
- **Pass list and GPU model for terrain**: `docs/notes/gc-slow-terrain.md`
  (bd-terrain).

## Answer in short

- **What Mesa does on this part.**
  - Every render target is tiled. A colour texture is super-tiled
    (`etnaviv_resource.c:577-581`) and the texture unit samples it as it is
    (`etnaviv_texture.c:218-220`).
  - Every render target gets a tile-status (TS) buffer
    (`etnaviv_resource.c:531-535`). A full glClear writes only that buffer
    (`etnaviv_rs.c:337-360`, `:373-398`, `:467-481`).
  - Unrendered tiles are filled in only when something other than the pixel
    engine (PE) reads the surface: an in-place resolve
    (`etnaviv_rs.c:164-181`). The texture unit reads a single-level texture
    through its TS directly (`etnaviv_texture.c:121-159`).
  - Early depth is on whenever the first colour target is not linear and the
    shader neither writes depth nor discards (`etnaviv_state.c:1005-1056`).
  - A linear render target exists only under the `linear_pe` debug flag
    (`etnaviv_screen.c:1094-1095`). There Mesa turns early depth off
    (`etnaviv_state.c:1018-1019`).
- **What the back end does.**
  - Every colour target is linear under SINGLE_BUFFER(1). The depth surface
    is row-linear (measured, `src/vivante/backend.rs:1496-1511`).
  - There is no TS (`:1750-1751`, `src/vivante/texture.rs:597`), and every
    glClear is a quad (`backend.rs:2345-2395`).
  - Early depth is never on (`backend.rs:1754-1783`).
  - A rendered texture that the vertex stage samples is tiled again on the
    CPU after a `finish()`, every frame (`src/vivante/tiled_copy.rs:357-434`).
- **What the changes save is a small part of these frames.**

  | change | terrain (250 ms) | shadow (500 ms) | refract (500 ms) |
  |---|---:|---:|---:|
  | 1. tiled vertex-stage textures | 4-6 ms | 0 | 0 |
  | 3. fast clear of depth | ≈0 | 7.5 ms | ≈0 |
  | 3. TS-aware CPU depth resolve | 0 | 30-150 ms | 30-150 ms |
  | 4. fast clear of colour | 1-5 ms | 0 | 2.5-7.5 ms |
  | 5. early depth | 0-19 ms | 0-1 ms | 0-2 ms |
  | 6. tiled window | ≈2 ms | 0-1 ms | 0-1 ms |

  - The fill gains come from the pixel model of
    `docs/notes/imx8mp-frame-floor.md` candidate 2: 3.9 ns a rasterised
    pixel, plus 0.6 to 0.8 ns an instruction a pixel (`gc-slow-terrain.md`,
    "The GPU calibration").
  - Early depth matters only on terrain's 148-instruction terrain pass. The
    grid's draw order decides how much it saves.
  - The largest single item is the TS-aware CPU depth resolve, which reads
    only tiles the depth pass wrote. If the texture unit learns to sample the
    D24S8 surface (item gt-depth-tpu, `docs/notes/gc-depth-sample.md`), the
    resolve goes away and so does that gain.
  - Terrain's GPU time is about 140 to 205 ms (`gc-slow-terrain.md`). The
    noise shader (33 to 44 ms) and the terrain pass (49 to 75 ms) are the
    largest parts. Shadow's and refract's time is mostly the CPU depth
    resolve (`docs/notes/imx8mp-glmark2-gap.md`, rank 1).
  - So these changes are worth doing, but they do not take terrain off 4 FPS
    or shadow and refract off 2 FPS on their own.
- **Order.**
  - 0. A board probe of the layouts and of the RS rates.
  - 1. Tile terrain's two vertex-stage maps on the GPU.
  - 2. Tiled depth under SINGLE_BUFFER(2).
  - 3. Depth TS and fast depth clear, with a TS-aware CPU depth resolve.
  - 4. Tiled colour textures rendered and sampled directly, with colour TS
    and fast clear.
  - 5. Early depth.
  - 6. Optionally, the window tiled and resolved to linear at the swap.

  Each step depends on the board answers of step 0. The list, with sizes,
  tests and board checks, is in "The change list".

## The part

The i.MX8MP's GC7000UL is model `0x7000`, revision `0x6204`, product
`0x70003`. Its two hwdb entries are `gc7000ULN_v123`, EcoID 0 at hwdb `:18839`
and EcoID 1 at `:19733`. They are byte-identical apart from the EcoID
(compared for this note). The features that decide this study, with the bit
in the EcoID 0 entry, the `ETNA_FEATURE` it becomes (`etna_hwdb.c`), and what
Mesa derives from it:

| hwdb bit | value | `etna_hwdb.c` | Mesa uses it as |
|---|---|---|---|
| `REG_FastClear` (`:18951`) | 1 | `:43` FAST_CLEAR | TS is allowed (`etnaviv_resource.c:153-155`). It needs MC20 or an MMU (`etnaviv_screen.c:986-992`); this part has `REG_MC20` 1 (`:19005`) |
| `REG_TileStatus2Bits` (`:18993`), `CACHE128B256BPERLINE` (`:19190`) | 1, 1 | `:56`, `:95` | `bits_per_tile` = 4 (`etnaviv_screen.c:880-882`). A TS tile is 128 or 256 bytes (`etnaviv_screen.h:113-126`). The TS clear word is `0x11111111` (`etnaviv_screen.c:884-886`) |
| `V4Compression` (`:19191`), `DEC400` (hwdb `:19278`) | 0, 0 | `:99`, `:107` | no colour compression except for MSAA (`etnaviv_resource.c:196-199`); the sampler can read TS (`etnaviv_texture.c:147-149`) |
| `REG_SuperTiled32x32` (`:18995`) | 1 | `:57` SUPER_TILED | `can_supertile` (`etnaviv_screen.c:878-879`) |
| `REG_SuperTiledTexture` (`:19050`) | 1 | `:69` | the texture unit samples a super-tiled resource (`etnaviv_texture.c:218-220`) |
| `REG_TextureHorizontalAlignmentSelect` (`:19035`) | 1 | `:60` TEXTURE_HALIGN | a TILED texture padded to 16 for the RS is still sampled (`etnaviv_texture.c:230-232`) |
| `REG_LinearPE` (`:19051`), `REG_LinearTextureSupport` (`:19037`) | 1, 1 | `:68`, `:66` | the PE can write linear, but Mesa turns that off unless debugging (`etnaviv_screen.c:1094-1095`); the texture unit samples linear |
| `REG_PEEnhancements3` (`:19117`) | 1 | `:84` SINGLE_BUFFER | `single_buffer` (`etnaviv_screen.c:975`); no multi-tiled layout (`:978-979`) |
| `NumPixelPipes` (`:18866`) | 1 | | one pixel pipe |
| `REG_BltEngine` (`:19174`) | 0 | `:87` | `use_blt` false (`etnaviv_screen.c:984`): clears and resolves go through the RS (`etnaviv_rs.c:1004-1009`). Scissored clears are not offered (`etnaviv_screen.c:242`), so the state tracker draws those as quads |
| `REG_NoEZ` (`:18967`), `REG_RAWriteDepth` (`:19158`), `REG_HierarchicalZ` (`:19010`) | 0, 1, 0 | `:49`, `:91` | early depth is allowed and the rasteriser can also write depth (`etnaviv_state.c:1001`, `:1026-1031`); no hierarchical Z |
| `REG_CorrectAutoDisable1` (`:19022`) | 1 | `:59` AUTO_DISABLE | the TS auto-disable count after a fast clear (`etnaviv_rs.c:382-387`, `:470-475`) |

## What Mesa does

### Layouts, and when each is chosen

- **`etna_resource_create`** (`etnaviv_resource.c:551-605`) starts from TILED.
  - A depth or stencil resource adds SUPER (`:572-576`).
  - So does any other resource the RS can tile, on a core with
    SUPERTILED_TEXTURE (`:577-581`). On this part, then, every colour
    texture, render target or not, is **super-tiled**.
  - It is LINEAR only for scanout, an explicit PIPE_BIND_LINEAR, or a
    compressed format (`:596-601`).
  - 128-bit formats stay plain TILED (`:592-594`).
- **Padding** (`etna_layout_multiple`, `:352-407`):
  - A super-tile is 64x64 (`:389-393`), HALIGN `SUPER_TILED`.
  - TILED is 16x4 with HALIGN SIXTEEN when the RS may touch it, else 4x4
    with HALIGN FOUR (`:384-388`, `rs_align` at `:361-362`). Without the
    BLT that is every render target, and on a TEXTURE_HALIGN core every
    texture.
- **Render-compatible** (`:88-111`): a LINEAR resource is render-compatible
  only with LINEAR_PE, which is off. A resource that is not
  render-compatible gets a tiled `render` shadow (`:113-145`). The colour
  path is `etna_set_framebuffer_state` (`etnaviv_state.c:244`), the depth
  path the same function (`:411`).
- **Sampler-compatible** (`etnaviv_texture.c:208-239`): super-tiled, linear,
  or TILED (with TEXTURE_HALIGN any padding). Anything else gets a TILED
  `texture` copy (`:241-269`), refreshed from `render` when that is newer
  (`:177-188`).
  - So on this part a rendered colour texture is **one super-tiled
    resource** that the PE writes and the texture unit reads. There is no
    copy (`res->render` and `res->texture` both stay NULL).
- **The descriptor** (`etnaviv_texture_desc.c:179-205`):
  - ADDRESSING_MODE is LINEAR only for a linear resource (`:181-182`);
    TILED is 0 (`hw/common_3d.xml.h:130`).
  - CONFIG1 carries the resource's HALIGN (`:186`).
  - Each level has its own LOD address (`:204-205`).
- **CPU access to a tiled or TS resource** (`etnaviv_transfer.c:350-403`) goes
  through a LINEAR staging resource that the RS fills (`:400-403`) and
  writes back (`:225-232`). Exceptions: a HALIGN_FOUR resource is detiled
  in software, because the RS cannot handle it (`:357-361`); so is a TILED
  resource whose `texture` copy is current (`:350-354`).
  `etna_manual_blit` is the software fallback for TILED-to-TILED copies
  (`etnaviv_rs.c:945-954`).

### SINGLE_BUFFER: one switch for every surface

- `etnaviv_state.c:532-540`: the mode is LINEAR (1) when any colour target
  is linear. Otherwise, on a SINGLE_BUFFER core, it is 2, or 3 for a 16-bit
  target or depth.
- The comment says there is one switch for colour and depth together. So
  the depth surface's layout follows the colour target's mode.
- With no colour target (shadow's depth pass), `target_linear` stays false
  and the mode is 2 (`:397-407` gives the dummy target).
- The depth surface is always tiled, and super-tiled when its resource is
  (`etnaviv_state.c:416`, `:421`, `:428-432`;
  `VIVS_PE_DEPTH_CONFIG_SUPER_TILED` `hw/state_3d.xml.h:943`).

### Tile status

- **Allocation**:
  - Every render-target or depth resource that is render-compatible gets a
    TS buffer (`etnaviv_resource.c:531-535`) if `etna_resource_can_use_ts`
    holds (`:147-170`): FAST_CLEAR, no array or 3D, RS-tileable, not 128-bit.
  - Size (`:223-227`): one 4-bit entry per 128-byte tile. TS_MODE_128B
    without compression (`:184`, `:204-220`; `hw/common_3d.xml.h:128`).
    A 1600x1200 D24S8 surface is 7.68 MB / 128 B = 60 000 tiles = 30 KB.
- **Per framebuffer** (`etnaviv_state.c:232-235`, `:304-324`, `:451-466`):
  - PE_MEM_CONFIG gets COLOR_TS_MODE and DEPTH_TS_MODE (`hw/state_3d.xml.h:1179-1185`).
  - TS_COLOR_STATUS_BASE, TS_COLOR_CLEAR_VALUE and the depth pair are set.
  - TS_MEM_CONFIG gets `COLOR_FAST_CLEAR` / `DEPTH_FAST_CLEAR` exactly while
    the level's TS is valid (`etna_update_ts_config`, `:877-945`;
    `hw/state_3d.xml.h:1455-1460`).
- **Emit** (`etnaviv_emit.c:586-597`): PE_MEM_CONFIG, TS_MEM_CONFIG, then the
  status and surface bases and the clear values. Before any change of TS
  state, TS_FLUSH_CACHE (`:294-297`).
- **Validity** (`etnaviv_resource.h:118-145`): a level's TS is valid after a
  fast clear and invalid after anything that writes the surface outside the
  PE.

### Fast clear

- `etna_clear_rs` (`etnaviv_rs.c:504-581`):
  - Flushes COLOR and DEPTH and stalls RA on PE (`:518-519`), then
    TS_FLUSH_CACHE if any target has TS (`:521-546`).
  - Clears colour, then depth (`:551-578`), then stalls again (`:580`).
- **Colour** (`etna_blit_clear_color_rs`, `:362-433`):
  - With TS: the new clear value goes into TS_COLOR_CLEAR_VALUE (`:373-380`),
    and the AUTO_DISABLE count is the tile count (`:382-387`).
  - The RS then fills **the TS buffer**, 16 wide and `ts_layer_stride / 64`
    high, with `0x11111111` (`etna_rs_gen_ts_clear_cmd`, `:337-360`). That
    is 30 KB written for a 1600x1200 target instead of 7.68 MB.
  - It marks the TS valid and the level unflushed (`:396-397`).
  - Without TS it is a plain RS fill of the surface (`:399-422`,
    `etna_rs_gen_clear_cmd` `:284-335`: tiled if the padded size is 16x4
    aligned, else linear, `:316-327`).
- **Depth** (`etna_blit_clear_zs_rs`, `:435-502`):
  - The TS path is taken only when the whole word is cleared
    (`new_clear_bits == 0xffff`, `:467`). That is depth on Z16 or X8Z24, or
    depth and stencil together on S8Z24 (`:447-465`).
  - A depth-only clear of an S8Z24 surface is an RS fill with bits `0xeeee`,
    the stream this repository's golden mode 8 records
    (`tools/vivante/README.md`, "How the two drivers clear depth":
    `RS_CLEAR_CONTROL 0x0001eeee`).
  - glmark2's depth textures (`GL_DEPTH_COMPONENT`) and terrain's
    `GL_DEPTH_COMPONENT16` have no stencil, so Mesa fast-clears them.

### Resolve: filling in the tiles that were only cleared

- **Self-to-self through the RS** (`etna_try_rs_blit`, `etnaviv_rs.c:677-943`):
  - Flushes COLOR and DEPTH first (`:845-850`).
  - Points the TS unit at the source with `COLOR_FAST_CLEAR`
    (`:852-891`), so cleared tiles read as the clear value.
  - On a single-buffer core with a super-tiled source, same address, format
    and stride, this is an **in-place resolve**: RS_KICKER_INPLACE with the
    tile count, which writes only the unrendered tiles (`:164-181`,
    `hw/state_3d.xml.h:1427`).
  - It does nothing when the TS is not valid (`:208-210`).
  - A TILED (not super) source has no in-place mode. Its self-copy rewrites
    the whole surface.
  - After a resolve without compression, the TS stays valid
    (`:933-939`).
- **Who resolves**:
  - the display flush (`etna_flush_resource`, `etnaviv_clear_blit.c:274-301`:
    `render` -> scanout resource, or the resource in place);
  - a sampler view that cannot use sampler TS (`etnaviv_texture.c:189-199`);
  - a CPU map (`etnaviv_transfer.c:355-403`);
  - a framebuffer target that cannot keep TS (`etnaviv_state.c:251-255`).
- **Sampler TS** (`etnaviv_can_use_sampler_ts`, `etnaviv_texture.c:121-159`):
  - The texture unit reads the TS itself when all of these hold: level 0's
    TS is valid, TEXTURE_TILED_READ (hwdb `REG_TextureTileStatus` 1,
    `:19076`), no compression without V4, and a view of **level 0 only**
    (`:151-156`).
  - Then TS_SAMPLER_CONFIG and the status base and clear value are set
    (`:80-115`), plus TX_CTRL's TS_ENABLE / TS_MODE / TS_INDEX
    (`etnaviv_texture_desc.c:302-307`, `:437-455`).
  - A mipmapped texture sampled over its chain is resolved instead.

### Early depth

`etna_update_zsa` (`etnaviv_state.c:992-1080`):

- `early_z_allowed` starts as `!NO_EARLY_Z` (`:1001`; true here). It is
  cleared when the first colour target is LINEAR (`:1005-1023`): "RA and PE
  disagree about the buffer layout in this mode".
- **Early test** (`:1036-1043`): depth test on, early allowed, no shader depth
  write, and stencil unmodified or written early.
- **Early write** (`:1025-1034`): RA_WRITE_DEPTH, early allowed, no alpha test,
  no depth write, no discard.
- **Encoding**:
  - `PE_DEPTH_CONFIG_EARLY_Z` follows the early test
    (`:1049`, `hw/state_3d.xml.h:935`).
  - `DISABLE_ZS` is set when nothing happens late (`:1050-1051`).
  - `RA_EARLY_DEPTH` is `0x30 | TEST_ENABLE` (`:1055-1056`,
    `hw/state_3d.xml.h:590`), plus FORWARD_Z/W, and WRITE_DISABLE unless
    the write is early (`:1058-1063`).
- `ETNA_DBG_NO_EARLY_Z` turns it off for debugging (`etnaviv_screen.c:1084-1085`).

## What the back end does today

| | the back end | file:line |
|---|---|---|
| colour layout | linear at the GL core's pitch, SINGLE_BUFFER(1); the exceptions are below | `src/vivante/backend.rs:1646-1682`, `:1733-1744`, `single_buffer` `:556-579` |
| 64-bit targets | a TILED shadow (no SUPER bit), padded 16x4, SINGLE_BUFFER(2). The CPU tiles it in before the first draw and untiles it when the kick retires. **Proven on the board**: gl_test 44 passes on the i.MX8MP (`tools/cts/RESULTS.md:379`) | `src/vivante/wide_target.rs:1-52`, `src/vivante/VIVANTE_NOTES.md:1494-1517` |
| 4x MSAA scratch | SUPER_TILED and SUPER_TILED_NEW, RS downsample into the linear target (`emit_resolve`). Opt-in; **the board found the scratch empty** after clear and after draw (#8, `docs/notes/gc-msaa-addresses.md:9-13`) | `backend.rs:1653-1660`, `:997-1034` |
| depth | always D24S8, no SUPER_TILED bit, **written row-linear by the PE under SINGLE_BUFFER(1)** (gl_test 169's dump, 2026-09-30), 64-aligned both ways | `backend.rs:1496-1524`, `src/vivante/ds_cache.rs:231` |
| CPU readers of depth | `copy_plane` (`ds_cache.rs:435`), `seed_depth` (`:497`), `resolve_depth` (`:562`, called from `backend.rs:3002` for `src/gl/texture.rs:698-738` and `src/gl/blit.rs:874`); all assume row-linear words and no TS (`ds_cache.rs:20-25`) | |
| tile status | none: PE_MEM_CONFIG 0, TS_MEM_CONFIG 0, sampler TX_CTRL 0; TS_FLUSH_CACHE sent with every draw's flushes | `backend.rs:1750-1751`, `texture.rs:597`, `backend.rs:1789-1794`, `src/vivante/regs.rs:634-638` |
| glClear | a full-target quad with the clear's colour, depth and stencil, fused in front of the draw | `backend.rs:2345-2395`, `:3096` |
| early depth | never: no RA TEST_ENABLE, WRITE_DISABLE always, no PE EARLY_Z, the rule Mesa applies to a linear target | `backend.rs:1754-1783` |
| sampling | the GL core's linear storage, ADDRESSING LINEAR, HALIGN FOUR (0). Some textures get a 4x4-tiled CPU copy instead: layered mipmapped or padded ones, 64-bit and 128-bit texels, and **every plain 2D texture the vertex stage samples**. The copy is rebuilt when a render or CPU write changes the key, after a `finish()` if the render is pending | `texture.rs:271-298`, `src/vivante/tiled_copy.rs:118-125`, `:174-180`, `:357-434` |
| GPU mipmap chain | one quad a level into the GL core's linear storage, one kick a level | `src/vivante/mipmap.rs:1-45`, `:270-279` |
| the RS | used only by the MSAA resolve (opt-in) and the flip study | `backend.rs:905-964`, `:997-1034` |

## Per-scene passes and the pixel model

Model (`imx8mp-frame-floor.md` candidate 2, `gc-slow-terrain.md` "The GPU
calibration"):

- 3.9 ns a rasterised pixel for raster, clear and PE;
- 0.6 ns an instruction a pixel for shading, 0.8 ns as the upper case;
- no term for texture-cache misses, which nothing on this part isolates.

A clear quad costs 3.9 ns a pixel of its target:

- 800x600: 1.87 ms;
- 1600x1200: 7.5 ms;
- 256x256: 0.26 ms.

**terrain** (pass list `gc-slow-terrain.md`, "The 52 kicks"):

- 8 clear quads, 8.5 ms:
  - the window twice (`canvas_.clear()`, glmark2 `main-loop.cpp:132`, and
    tilt_v's own, `scene-terrain/texture-renderer.cpp:57`);
  - the terrain target with its depth16 (`terrain-renderer.cpp:46-47`);
  - tilt_h;
  - the four 256x256 maps (height, normal, bloom_h, bloom_v).
- 9 passes, 5 GPU chains.
- The terrain pass shades 148 instructions a pixel with the depth test on,
  42.6 ms at full cover. Its cover times overdraw is 0.8 to 1.2.
- The height and normal maps (256x256, one level, not mipmapped:
  `scene-terrain.cpp:68-71`) are sampled by the vertex stage. That is what
  makes `tiled_copy` copy them every frame (`gc-terrain-tiled-copy.md`).

**shadow** (`scene-shadow.cpp`):

- The window clear: 1.87 ms.
- The depth pass into a 1600x1200 `GL_DEPTH_COMPONENT` texture
  (`:85-86`, `:106-113`): colour mask off, `glClear(GL_DEPTH_BUFFER_BIT)`
  (`:145-151`), then the horse (`:420-431`). The clear quad is 7.5 ms.
- The CPU depth resolve of all 1.92 M texels for the ground's sampling
  (`imx8mp-glmark2-gap.md` rank 1: 60 to 200 ms on the board, from the
  non-cacheable reads).
- The ground, then the horse, at 800x600 (`:434-459`).

**refract** (`scene-refract.cpp`):

- The window clear: 1.87 ms.
- A 1600x1200 colour target, mipmapped (`:177-178`, `:199-210`), with a
  1600x1200 depth texture. One fused clear of both (`:254`), 7.5 ms; then
  the horse with front faces culled (`:255`, `:414-425`).
- The colour chain on the GPU: 10 levels, about 0.64 M pixels, 2.5 ms
  (`:429-430`).
- The CPU depth resolve of 1.92 M texels.
- The refraction pass at 800x600, sampling the depth, the colour chain and
  the image (`:432-471`).

## The gap, per feature

### (a) Tiled colour targets the texture unit samples directly

What is needed, and what Mesa would do:

- **A tiled surface per rendered 2D texture.** It is TILED with 16x4 padding
  and HALIGN SIXTEEN (`etnaviv_resource.c:384-388`), or super-tiled like
  Mesa (`:577-581`, 64x64, HALIGN SUPER_TILED).
  - The back end keeps it beside the GL core's linear storage, as
    `wide_target.rs` keeps its shadow.
  - The descriptor then points at it, with ADDRESSING TILED
    (`texture.rs:271-273`), the surface's HALIGN instead of 0 (`:298`), and
    each level's address.
  - The GL core's storage stays the CPU's copy, written back only on demand.
  - Recommendation: TILED first. The board has drawn a TILED 64-bit shadow
    right (case 44), but never a super-tiled surface right (#8).
- **SINGLE_BUFFER(2) for a draw into it** (`etnaviv_state.c:538-539`), as
  `single_buffer` already gives the 64-bit shadow (`backend.rs:567-579`).
  That changes the depth surface's layout for the same draw. That is change
  2, so (a) for a target with depth depends on it.
- **The mipmap chain written tiled.** Each level in `mipmap.rs` becomes a
  draw into that level's tiled storage. A level is padded to 16x4, so a
  level under 16 wide is padded the same way.
- **CPU readers and writers.** Each one must get the linear copy back first.
  - Readers: `glReadPixels` and `glCopyTexImage2D` (through `finish`), the
    CPU mipmap filter and `gc_return_mipmap_chain` (`src/gl/texture.rs:878-885`),
    the layered tiled copy, and the depth readers (change 2).
  - Writer: `glTexSubImage2D` into a rendered texture, which must re-tile
    the surface or mark it stale.
  - Mesa does both directions with the RS (`etnaviv_transfer.c:225-232`,
    `:400-403`). The RS needs widths in 16s and heights in 4s
    (`etnaviv_rs.c:87-92` aborts otherwise) and a HALIGN that is not FOUR
    (`etnaviv_transfer.c:357-361`). Smaller levels are untiled on the CPU,
    as Mesa does.
  - `wide_target.rs` untiles at every retire (`wide_target.rs:36-42`). For
    32-bit targets rendered every frame that would cost a full CPU pass per
    frame. The write-back must happen only when a reader asks.
- **What it removes.** The `tiled_copy` path for rendered textures, which
  today copies every plain 2D vertex-stage texture after a `finish()`
  (`tiled_copy.rs:174-180`, `:369-374`). Layered and 64-bit textures that
  are only uploaded keep the copy.

### (b) Tile status and fast clear

- **A TS buffer per tiled colour surface and per depth surface**: 4 bits a
  128-byte tile, aligned to `0x100` (`etnaviv_resource.c:223-227`), in the
  carve-out. It is about 30 KB for 1600x1200, under 8 KB for 800x600.
- **State**:
  - PE_MEM_CONFIG's TS modes;
  - TS_MEM_CONFIG's FAST_CLEAR bits, set while the TS is valid;
  - the status, surface and clear-value registers (`etnaviv_emit.c:586-597`);
  - the AUTO_DISABLE counts (`etnaviv_rs.c:382-387`).
  - TS_FLUSH_CACHE stays where it is: every draw already sends it
    (`backend.rs:1793`).
- **A fast-clear path in `emit_clear`.** For a full, unscissored, unmasked
  clear of a surface that has TS, it records the RS TS-buffer fill
  (`etnaviv_rs.c:337-360`) instead of the quad. It keeps the quad for
  scissored or masked clears, as Mesa's state tracker does
  (`etnaviv_screen.c:242`).
  - A depth clear may write the whole word when the draw has no stencil
    object. This is Mesa's X8Z24 case (`etnaviv_rs.c:449-451`); glmark2's
    depth buffers have no stencil.
  - A fused colour and depth clear becomes two TS fills.
- **Validity tracking per surface.** Valid after a fast clear. Invalid after
  a CPU write, an RS copy into it, or a seed or copy of a depth plane
  (`ds_cache.rs`).
- **A resolve before every reader outside the PE**:
  - in place for a super-tiled surface (`etnaviv_rs.c:164-181`);
  - an RS self-copy for a TILED one (`:852-891`);
  - nothing for a single-level texture sampled through sampler TS
    (`etnaviv_texture.c:121-159`, TX_CTRL `etnaviv_texture_desc.c:302-307`).
- **The CPU depth readers.** They must either resolve first (an RS pass and
  a `finish()`), or read the TS: a tile whose entry is the clear code holds
  the clear value, so the CPU writes the clear's float for it without
  reading the surface. The second is the large gain for shadow and refract,
  because their resolve is paced by non-cacheable reads
  (`imx8mp-glmark2-gap.md` rank 1). It depends on the TS's entry code for a
  cleared tile (`0x1` in each nibble after a TS clear,
  `etnaviv_screen.c:884-886`) and on the tile-to-pixel map of the depth
  layout from change 2. Both are board facts the probe reads.

### (c) Early depth

- **The rule** is Mesa's (`etnaviv_state.c:1005-1063`), applied only when the
  draw's colour target is tiled (a), or when there is none (shadow's depth
  pass; with no colour target Mesa allows it, `:1010-1023`).
  - Early test: the depth test is on, the shader does not write depth, and
    stencil is unmodified or written early.
  - Early write: as well as that, no discard.
- **The encoding**:
  - `GC_PE_DEPTH_EARLY_Z` (`regs.rs:417`) in `emit_pe_state`;
  - RA_EARLY_DEPTH TEST_ENABLE, and WRITE_DISABLE only when the write is
    late (`emit_ra_state`, `backend.rs:1769-1783`).
  - Both halves from one decision. The comment at `backend.rs:1756-1768`
    records what disagreeing halves did (gl_test case 3: a kick that never
    completed).
  - The decision goes into the dedup key (`src/vivante/dedup.rs:68`).
- It does nothing for a linear target: Mesa itself refuses
  (`etnaviv_state.c:1018-1019`).

## The change list, in order

Sizes are estimated lines of non-test code. Gains are per frame, from the
model above. The board numbers are 250 ms (terrain) and 500 ms (shadow,
refract) at main `a917d123`.

Every board check runs on the Armadillo-X2 (target:x2) only, under plan
#323's rules:

- `tools/board.sh x2 run`, an image built from a commit on origin/main that
  carries `gc_wdog_arm: armed`;
- results from the SNVS record and the runner's verdict line;
- ms a frame from the glmark2 F page or PERF page 0 v0
  (`imx8mp-glmark2-gap.md`, "What the record can and cannot show"), per
  scene through item x2-gm-script;
- the fallback and wait counts from PERF page 4 (item fb-counters);
- the picture values in the record compared with the run before the change
  (by-eye pictures stay plan #317's).

### 0. Board probe: layouts and RS rates (gate for 2 to 6)

- **What.** A gl_test case, built and not run under this item, that:
  - (i) draws one known triangle into 64x64 RGBA8 TILED and SUPER_TILED
    colour surfaces under SINGLE_BUFFER(2), and dumps the words;
  - (ii) dumps the D24S8 surface of the same draw under SINGLE_BUFFER(2),
    with and without PE_DEPTH_CONFIG_SUPER_TILED, the way case 169 dumped
    it under (1);
  - (iii) fast-clears a TILED and a super-tiled target and its depth
    through the TS, draws a corner, resolves (in place and by self-copy)
    and compares with the CPU's expected image; and dumps the TS words
    after the clear and after the draw;
  - (iv) times a 1600x1200 clear as a quad, as an RS fill and as a TS fill,
    and an 800x600 RS tiled-to-linear copy (GPU busy, PERF page 1 v3).
- **Size.** About 250 lines of gl_test C and a record page.
- **Gain.** None by itself. It decides:
  - TILED or SUPER_TILED for (a);
  - the tile-to-pixel map that changes 2 and 3 untile with;
  - whether TS on this part with the MMU off works at all. Mesa never runs
    it that way: the back end turns the MMU off
    (`driver/rust/src/gpu/vivante/hw.rs:15-16`), and Mesa's FAST_CLEAR
    gate counts MC20 (`etnaviv_screen.c:986-992`);
  - the RS rates the gains of 4 and 6 depend on.
- **Host test.** The case's expected images and words come from the CPU
  tiling functions, `wide_target.rs` and `tiled_copy.rs` (unit tests
  beside them). Its streams are compared with a new `tools/vivante`
  golden mode (cs_golden vs cs_ours, `tools/vivante/README.md`, "The
  golden command stream") for a TS-cleared target.
- **Board check.** The case's verdict and dumps on the X2.

### 1. Terrain's vertex-stage maps tiled on the GPU (small; no layout change)

- **What.** This is `gc-slow-terrain.md` rank 3's first form. When
  `tiled_copy::refresh` needs a new copy of a 2D RGBA8 texture whose newer
  contents come from a render, record an RS linear-to-TILED copy into the
  copy's memory, in the stream after the render, instead of `finish()` and
  the CPU pass.
  - The RS supports a linear source and a tiled destination (stride times
    4, `etnaviv_rs.c:76-78`, `:95-102`, `:130-132`). The flush and stall
    come first (`:845-850`).
  - Only where the RS fits: width a multiple of 16, height a multiple of 4
    (`:87-92`). Terrain's 256x256 maps do.
  - The copy's layout today is 4x4-tiled with HALIGN FOUR
    (`tiled_copy.rs:207-220`, `texture.rs:298`), which the RS cannot write
    (`etnaviv_transfer.c:357-361`). At these sizes 4x4 and 16x4 padding are
    the same bytes, so the RS copy is padded to 16x4 and the descriptor's
    HALIGN set to SIXTEEN.
- **Size.** About 120 lines in `tiled_copy.rs` and `backend.rs`. It uses the
  RS block of `emit_resolve` (`backend.rs:997-1034`) without downsampling.
- **Gain.**
  - terrain 4 to 6 ms: the mid-frame `finish()` (4.5 ms) and 1.5 MB of
    non-cacheable CPU traffic (`gc-slow-terrain.md` (2), (3), rank 3).
  - shadow 0, refract 0.
- **Host tests.**
  - `tests/gc_tiled_copy_reuse.rs` changes: a render into the texture gives
    a new copy with no `finish` (counted), and the stream holds the RS copy
    after the render's draw.
  - A mock-stream unit test of the RS words against the tiling function.
  - `tools/check.sh --changed origin/main`.
- **Board check.**
  - terrain's ms a frame before and after;
  - PERF page 4: the `finish` count one a frame lower, and the
    `tiled_copy` CPU bytes from about 1 536 KiB to 0 a frame
    (`gc-slow-terrain.md` rank 3);
  - terrain's picture value unchanged.

### 2. Depth tiled as Mesa lays it out, under SINGLE_BUFFER(2) (medium)

- **What.** Draws with no colour target, or with a tiled one (the 64-bit
  shadow today, (a) later), program SINGLE_BUFFER(2) and the depth layout
  that step 0 measured.
  - A surface carries its layout (`ds_cache::Surface`). A draw whose mode
    differs converts it with the RS (32-bit words copy as A8R8G8B8) or the
    CPU.
  - The CPU readers (`copy_plane`, `seed_depth`, `resolve_depth`) untile
    by that layout.
  - The 64-bit shadow's own unmeasured depth case (`wide_target.rs:46-52`)
    is settled by the same probe.
- **Size.** About 200 lines: `ds_cache.rs`, `backend.rs` `emit_pe_state`
  and `single_buffer`.
- **Gain.** None by itself. It is the precondition for 3 and 5 on depth.
- **Host tests.**
  - `tests/gc_depth_resolve.rs` extended: bit-exact floats from a tiled
    surface filled through the layout function, at the same (w, h, pitch)
    cases.
  - The mock stream carries PE_DEPTH_CONFIG and PE_LOGIC_OP as Mesa's golden
    mode for a depth-only draw (shadow's shape) has them.
  - dEQP-GLES2 fbo and depth rows on the host harness unchanged.
- **Board check.**
  - gl_test's default set on the X2: case 169's canary, the depth-texture
    and shadow-compare cases;
  - shadow's and refract's picture values unchanged.

### 3. Depth TS, fast depth clear, TS-aware CPU depth resolve (large)

- **What.**
  - (b) for depth surfaces: a TS buffer per `ds_cache` surface, the TS
    state, the RS TS fill for a full depth clear (the whole word when there
    is no stencil object, `etnaviv_rs.c:449-451`, `:467`), and validity
    tracking.
  - `resolve_depth` reads the TS first. Tiles whose entry is the clear code
    get the clear value's float without a read of the surface; the others
    are read and untiled.
  - `copy_plane` and `seed_depth` resolve first (RS self-copy) or
    invalidate the TS.
  - A fused colour and depth clear keeps its quad for the colour until
    change 4.
- **Size.** About 350 lines: `ds_cache.rs`, `backend.rs`, `regs.rs` (TS
  registers from `hw/state_3d.xml.h:1455-1500`).
- **Gain.**
  - terrain about 0: its depth16 clear is fused with the colour clear.
  - shadow 7.5 ms (the 1600x1200 depth clear), plus the resolve's skipped
    reads: 30 to 150 ms. That is the share of tiles the horse does not
    cover, which is not measured; put at half to three quarters of
    rank 1's 60 to 200 ms. The float writes stay.
  - refract 0 on the clear (fused, see change 4), the same 30 to 150 ms on
    the resolve.
  - If gt-depth-tpu lets the texture unit sample the D24S8 surface, the
    resolve goes, and so does its part of this gain. The texture unit can
    then read the TS through sampler TS, since the depth textures have one
    level (`etnaviv_texture.c:151-156`), so the fast clear still needs no
    resolve.
- **Host tests.**
  - TS words and the RS TS fill in the mock stream against a golden mode
    for a depth-only TS clear;
  - `resolve_depth` with a synthetic TS: cleared tiles give the clear
    float, written tiles the converted words, bit-exact against the full
    read;
  - every `ds_retention` case on the host unchanged.
- **Board check.**
  - shadow's and refract's ms a frame before and after (PERF page 0 v0);
  - PERF page 4's CPU bytes read for the resolve: falls by the cleared
    share;
  - the picture values unchanged;
  - gl_test's depth cases on the X2.

### 4. Colour targets tiled and sampled directly, with colour TS and fast clear (large)

- **What.**
  - (a) for every rendered 2D texture: a tiled surface, SINGLE_BUFFER(2),
    a tiled descriptor, a tiled GPU mipmap chain, write-back on demand.
    This subsumes change 1.
  - (b) for those surfaces: fast colour clear; sampler TS for single-level
    views; a resolve before a chain is sampled and before the CPU reads.
- **Size.** About 600 lines:
  - generalise `wide_target.rs` to 32-bit pixels with lazy write-back;
  - `texture.rs`, `mipmap.rs` and `backend.rs`;
  - a GL core hook so a CPU reader asks for the linear copy. The GL core's
    `finish` and `kick_pending` calls are what `wide_target.rs` relies on
    today (`wide_target.rs:36-42`). Keep the shared GL change small: plan
    #310 is changing the GL core's texture classes.
- **Gain.**
  - **terrain 1 to 5 ms.** The four FBO clear quads at 800x600 and the four
    256x256 ones are 4.8 ms. Height and normal are single-level, so
    sampler TS applies and they need no resolve. The terrain target,
    tilt_h, bloom_h and bloom_v are mipmapped and need a resolve before
    their chain is sampled: about 0 (in place, super-tiled, fully drawn)
    to a full-surface RS copy (TILED), at the rate step 0 measures.
    Texture-cache locality of tiled sampling for the 9-tap blurs is a
    possible further gain that the model has no term for.
  - **shadow 0.** It has no colour FBO.
  - **refract 2.5 to 7.5 ms.** The fused 1600x1200 colour and depth clear
    goes (7.5 ms), minus the resolve of level 0 before the chain and the
    refraction pass sample it. The resolve writes the cleared share of
    7.68 MB in place, or all of it by self-copy.
- **Host tests.**
  - `tests/gc_tiled_copy_reuse.rs`: a rendered 2D texture is sampled from
    its tiled surface with no copy;
  - `tests/gc_mipmap_gpu.rs` with tiled levels;
  - `glReadPixels` and `glCopyTexImage2D` after a render return the
    rendered pixels (resolve plus write-back);
  - `glTexSubImage2D` after a render re-tiles;
  - mock streams against golden modes for render-to-texture and sample;
  - the glmark2 host harness's 33 entries unchanged in their GL errors and
    fallback counts.
- **Board check.**
  - the gl_test default set and the dEQP-GLES2 fbo rows on the X2;
  - terrain's and refract's ms a frame and picture values;
  - PERF page 4 shows no tiled copies.

### 5. Early depth (small, after 2 and 4)

- **What.** Mesa's rule and encoding ("The gap", (c)), for draws whose
  colour target is tiled or absent.
- **Size.** About 60 lines in `backend.rs` and `dedup.rs`.
- **Gain.**
  - **terrain 0 to 14 ms** (up to 19 at 0.8 ns): the hidden fragments of
    the terrain pass. Cover times overdraw is 0.8 to 1.2 at cover about
    0.8, so 0 to a third of 480 000 fragments, at 148 instructions each.
    How many are rejected depends on whether `make_grid`'s order
    (`terrain-renderer.cpp:243`) draws near rows first; this is the widest
    range in this note. tilt_v's depth-tested full-screen quad gains
    nothing.
  - **shadow 0 to 1 ms.** The depth pass shader is trivial, and the horse
    is drawn after the ground it covers.
  - **refract 0 to 2 ms.** The horse's own overdraw with culling is small.
- **Host tests.**
  - mock stream: PE_DEPTH_CONFIG EARLY_Z and RA_EARLY_DEPTH set together,
    and neither for a discarding shader, a depth-writing shader or a linear
    target;
  - the same words as Mesa's golden mode for a depth-tested draw into a
    tiled target.
- **Board check.**
  - gl_test case 3 (the case that hung when the two halves disagreed) and
    the default set;
  - terrain's ms a frame. One image with `no_early_z` behaviour and one
    without settles the 0 to 14 ms range.

### 6. The window tiled, resolved to linear at the swap (optional, last)

- **What.** The EGL window's draws go to a tiled shadow with TS. At the
  swap, an RS resolve into the linear buffer the present copies or scans
  (`etna_flush_resource`, `etnaviv_clear_blit.c:274-301`), ordered before
  the present.
- **Size.** About 150 lines: `backend.rs`, plus the EGL swap and the board
  present.
- **Gain.**
  - terrain about 2 ms: the two window clears, 3.7 ms, minus a 1.92 MB RS
    copy (step 0 measures it). Early depth also becomes possible on window
    draws.
  - shadow 0 to 1 ms, refract 0 to 1 ms: one window clear minus the copy.
- **Risk.** The highest of all:
  - the LCDIF must never scan a buffer being written (`imx8mp-frame-floor.md`,
    #923);
  - the resolve must complete before the copy or the scan.
- **Host tests.**
  - the swap stream ends with the RS resolve after the last draw;
  - `glReadPixels` on the window reads resolved pixels.
- **Board check.**
  - every glmark2 scene's picture value and ms a frame on the X2;
  - the Slint and cluster rows' pictures if they are in the X2 set.

## Risks

- **The PE's output for the display.** The window stays linear until
  change 6. Changes 1 to 5 touch only FBO surfaces, so the present path
  (`board_show`, the LCDIF) is unchanged by them.
- **One SINGLE_BUFFER switch for all surfaces** (`etnaviv_state.c:532-535`).
  - A depth surface used with a linear colour target and with a tiled one
    has two layouts and must be converted between them (change 2).
  - A draw with several colour targets mixing linear and tiled cannot be
    programmed: `RT_CONFIG` has no tiled bit of its own
    (`wide_target.rs:44-47`). It keeps the linear path.
- **Super-tiled PE writes have not worked on this board yet** (#8's empty 4x
  scratch). A TILED surface has: gl_test 44 passes with the 64-bit TILED
  shadow. That is why (a) starts TILED and why the probe comes first.
  TILED has no in-place resolve (`etnaviv_rs.c:171`). Its resolves are
  full self-copies, which reduces the gains of change 4.
- **CPU readers of rendered textures.** Each must get resolved, linear
  contents, or it reads stale or tiled bytes:
  - `ds_cache`'s three depth readers (`copy_plane`, `seed_depth`,
    `resolve_depth`) and `src/gl/blit.rs:874`;
  - `glReadPixels` and `glCopyTexImage2D`;
  - the CPU mipmap fallback and `gc_return_mipmap_chain`;
  - the layered `tiled_copy`;
  - `glTexSubImage2D`, the writer.

  Each needs a host test that fails without the resolve.
- **RS limits.** Widths in 16s and heights in 4s, or the RS hangs or
  scribbles (`etnaviv_rs.c:87-92`), and no HALIGN_FOUR
  (`etnaviv_transfer.c:357-361`). Small mipmap levels and odd sizes fall
  back to the CPU or keep the linear path.
- **TS cache coherence.**
  - Flush COLOR and DEPTH and stall RA on PE before an RS clear or resolve
    (`etnaviv_rs.c:515-519`, `:845-850`).
  - Flush the TS cache before any TS reconfiguration
    (`etnaviv_emit.c:294-297`, `etnaviv_rs.c:545-546`, `:860-861`).
  - A wrong order shows up as stale tiles in a later frame, not as an error.
- **TS with the MMU off.** This is not a configuration Mesa runs on this
  part. The probe (step 0, (iii)) must show a TS clear and resolve landing
  at the addresses the back end gives.
- **Carve-out.** The TS buffers are small (30 KB at 1600x1200). The tiled
  colour surfaces double the memory of every rendered texture while both
  copies exist: terrain's targets are about 4.5 MB, refract's 1600x1200
  chain about 10 MB.

## What this note does not settle

- Every gain is a model figure. Steps 0 to 6 replace each with an X2
  measurement.
- Shadow's and refract's per-pass instruction counts, and the horse's cover
  in the 1600x1200 depth targets, are item bd-shadow-refract's. The 30 to
  150 ms of change 3 depends on that cover.
- Whether the noise shader's 840 instructions are ours or would also be
  Mesa's (`gc-slow-terrain.md` rank 1 (b)) is outside this study, and is
  worth more than any change here.
