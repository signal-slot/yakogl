<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: wiring tiled targets, tile status, fast clear and early depth into the back end (design)

Plan #328, item tiled-wire-design (issue #1204). Design only: no code was
changed and no board was run. It turns steps 2 to 5 of
`docs/notes/gc-tiled-targets.md` ("The change list") and rank 1(a) of
`docs/notes/gc-slow-terrain.md` into fix items. Each item is meant to take
about 2 hours or less. They are listed in order.

Sources:

- **The back end** as of `723a75c8`. Every `file:line` below is at that
  commit. Items that are running now will move the lines: tiled-copy-rs
  (`backend.rs`, `tiled_copy.rs`, `texture.rs`), resolve-wide (`ds_cache.rs`),
  mip-one-kick (`mipmap.rs`). The function names stay valid. Two rule texts
  were corrected against the built code and cite the tree as it stands, and
  say so at their head: W5's early-depth rule and W10's sampler words.
- **The three pure modules**: `src/vivante/layout.rs`, `ts.rs` and
  `early_z.rs` (items gc-layout, gc-ts-words, gc-early-z). They are **not on
  the integration tip** this note was written from. No `src/vivante/layout.rs`
  exists, `src/vivante/mod.rs:31-54` does not name them, and the remote could
  not be reached from this host. So this note uses the contracts in
  "The interfaces this design assumes", and the items adapt to the names that
  get merged. Joining them to the module tree is item gc-groundwork-mod.
- **Mesa 26.2.1** etnaviv, read from
  `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`. File names are as in
  `gc-tiled-targets.md` ("Sources"). Register names are from
  `driver/src/vivante/hw/state_3d.xml.h` (the in-tree copy).
- **Golden streams**: `tools/vivante/cs_golden.c`, `tools/vivante/README.md`
  ("The golden command stream"). Item ts-golden adds a mode for a tiled,
  TS-cleared target. It is in review and not on this tip, so this note calls
  it "the ts-golden mode".

## Status (plan #355, 2026-11-12): W1 to W5 built on the host, behind `gc_tiled_rt`

W1 to W5 are in the tree, behind the knob `gc_tiled_rt` (`gc_debug_set_tiled_rt`,
`test-knobs`, default off; gl_test and glmark2 set it with
`CONFIG_GL_TEST_GC_TILED_RT` and `CONFIG_GLMARK2_GC_TILED_RT`). With the knob
off every stream is today's. No board has run them: see
`x2-glmark2-slow-scenes.md`.

| item | where | host test |
|---|---|---|
| W1 registers | `regs.rs` (the `GC_TS_*`, RS, `PE_MEM_CONFIG` and `TX_CTRL` bits) | `regs::mesa_header::ts_rs_consts_match_state_3d` |
| W2 one RS emitter | `backend.rs` `emit_rs` and `RsBlit` already served the resolve, the flip study and `tiled_copy`; the shadow's tile-in and copy back are its fourth and fifth callers (`tiled_rt::tile_in`, `untile`) | `tiled_rt::tests`, the RS interpreter of `backend_tests_tiled_rt.rs` |
| W3 depth layouts | `ds_cache.rs` `Surface::layout`, `select_in`, `layout_free`, `pe_private` | `a_depth_surface_keeps_its_layout_while_it_holds_contents` and the three after it |
| W4 shadows | `tiled_rt.rs`, `backend.rs` `tiled_rt_spec`, `tiled_rt_enter`, `frame_submit` | `a_depth_tested_draw_renders_into_a_tiled_shadow_the_rs_copies_back` and the five after it |
| W5 early depth | `backend.rs` `tiled_ez`, `draw_ez`, `emit_pe_state`, `emit_ra_state`, `pe_key` | `a_draw_into_a_tiled_shadow_takes_early_depth_and_nothing_else_does` |

Where the code differs from the design below:

- **The RS strides.** A TILED side's stride is its row stride times 4 with
  SOURCE_TILED or DEST_TILED in RS_CONFIG, and **no** `TILING` bit: Mesa sets
  that bit for a super-tiled side only (`etna_compile_rs_state`,
  `etnaviv_rs.c:84-132`, read in Mesa 26.2.1), as `tiled_copy.rs` already does.
  The W4 text above says `| TILING`.
- **One round trip a stream, no cross-kick state.** The note keeps a shadow
  in step with the storage across kicks (`InStep | ShadowNewer | StorageNewer`)
  and tiles in only after a CPU write, through a `storage_current` hook in the
  GL core. The code tracks one shadow in the open stream (`tiled_rt::Track`:
  `Active`, `InStep`), tiles in with the RS at the first shadow draw of a
  stream unless that draw opens with a whole colour clear, copies back at the
  stream's end or before the first draw that takes no shadow, and forgets it
  at the kick. That costs one more RS copy a stream when the draw does not
  clear, and needs no GL-core change: the board has to say whether the cost
  matters (session A).
- **The room for the copy back** is reserved (`tiled_rt::RESERVE_WORDS`, taken
  from the stream's capacity while a shadow is `Active`, as the occlusion
  query's suspend does), so a full stream cannot lose it.
- **An RS clear into the shadow** fills the shadow's whole (exact) size; the
  colour clear stays the RS fill and only the address and stride change.
- **A depth surface keeps its layout while a plane holds contents.** A draw
  that wants the other layout is refused with `-ENOTSUP` and a line
  (`ds_cache::select_in`); a whole clear's value is layout-free. A shadow
  draw whose depth is live and linear is drawn as before. This is the cost
  of W3's "a non-private surface keeps Linear" without a conversion: an
  application that alternates one depth renderbuffer between a shadow draw
  and a draw that cannot take a shadow, without clearing it, is refused.
- **Discard keeps the test early.** Mesa's rule (`etna_update_zsa`) for a
  program that discards is an early test and a late write; the host test pins
  that, not "no early depth at all" (`early_z.rs:141`, `:150`; W5).
- **A stencil test never reaches the rule.** `tiled_ez` keeps today's "never"
  for any draw whose stencil test is on, KEEP ops or not (`backend.rs:653`);
  the rule itself is not that strict -- Mesa takes a non-KEEP stencil early,
  and only a discard makes it both-late (W5, the rule's fourth bullet).
- **The TEXKILL scan** is not cached by code address (the `DiscardCache` of
  `shader.rs` stays unused): a scan is one pass over the program words per
  shadow draw, and a stale answer after a program's memory is reused would
  render wrongly.

## Answer in short

- **Early depth reaches terrain soonest, and with the least risk, through items
  W1 to W5.**
  - Terrain's 148-instruction pass (`gc-slow-terrain.md`, "The 52 kicks")
    renders into a 32-bit **TILED shadow** of the terrain target, under
    SINGLE_BUFFER(2). The shadow is the 64-bit `wide_target.rs` mechanism
    generalised to 32-bit pixels.
  - The same stream then copies the shadow back into the GL core's linear
    storage with the resolve engine (RS), in `frame_submit`, where the 4x
    MSAA resolve already sits (`backend.rs:2297-2303`).
  - Every reader downstream is unchanged: the mipmap chain, the overlay,
    tilt_h's sampling, `tiled_copy`, glReadPixels and the CPU readers. Each
    already waits for the render's kick, and that kick holds the copy back.
  - Terrain's depth is a renderbuffer that no CPU code ever reads
    (`gc-slow-terrain.md` (2): "Terrain's depth is a renderbuffer and is never
    sampled"). Its layout under SINGLE_BUFFER(2) therefore only has to agree
    between the PE and the rasteriser (RA). It is set super-tiled, as Mesa
    sets every depth surface (`etnaviv_resource.c:572-576`). The 64x64-padded
    allocation (`ds_cache.rs:277-278`) already fits that layout.
  - Early depth is then Mesa's rule (`early_z.rs`), applied only to draws into
    such a shadow.
  - Gain: terrain 0 to 14 ms a frame (up to 19 ms at 0.8 ns an instruction),
    `gc-tiled-targets.md` step 5. Cost: one 800x600 RS copy a frame, at a rate
    nobody has measured.
- **Tile status (TS) and fast clear come after that (W7 to W9), behind the
  board probe.** The probe is item gt-tiled-probe (`gc-tiled-targets.md`
  step 0 (iii)): it shows whether TS works with the MMU off.
  - W8 fast-clears the colour of shadow targets.
  - W9 fast-clears the depth of PE-private depth surfaces. No CPU code reads
    those, so W9 needs no TS-aware CPU reader.
- **Direct sampling, tiled chains and depth textures (W10 to W12) are where the
  CPU readers start to matter.** Shadow's and refract's 30 to 150 ms
  (TS-aware depth resolve) is W12. It depends on the probe's depth dump and on
  depth-sample-design: if the texture unit samples D24S8, the CPU resolve goes
  away.
- **What it shares with plan #310** is in "Plan #310". In short:
  - `format_words` and `build_descriptor` in `texture.rs`;
  - `pe_color_format` and the RS-format choice in `backend.rs`;
  - `tiled_copy::wide_texels`;
  - one GL-core line in `src/backend.rs` `storage_current`.

  The GL core's texture classes are not touched.

## The interfaces this design assumes

Each item names the function it calls. The names and shapes below are what the
wiring needs. If the merged modules differ, the items take the merged names.

`layout.rs` (item gc-layout):

- `enum Layout { Linear, Tiled, SuperTiled }`. `Tiled` is 4x4 tiles,
  row-major, the layout of `wide_target.rs:128-131` and `tiled_copy.rs:242-245`.
  `SuperTiled` is the 64x64 super-tile of a CACHE128B256BPERLINE core
  (`SUPER_TILED_NEW`, `regs.rs:514-515`).
- `padded(layout, w, h) -> (u32, u32)`: 16x4 for `Tiled` with RS alignment
  (`wide_target.rs:115-117`), 64x64 for `SuperTiled`, `(w, h)` for `Linear`.
- `offset(layout, x, y, padded_w, bpp) -> usize`: the byte offset of a pixel.
- `pe_stride(layout, padded_w, bpp) -> u32` (Mesa's `level->stride`) and
  `rs_stride(layout, stride) -> u32`. A tiled RS stride is the stride times 4,
  ORed with `TILING` and, for `SuperTiled`, `SUPER_TILED_NEW`
  (`backend.rs:1015-1017`).

`ts.rs` (item gc-ts-words):

- `ts_bytes(surface_bytes) -> usize`: one 4-bit entry per 128-byte tile,
  aligned to `0x100` (`etnaviv_resource.c:205-227`). That is 30 KB for
  1600x1200 x 4.
- `TS_CLEAR_WORD = 0x1111_1111` (`etnaviv_screen.c:884-886`).
- `ts_fill_rect(ts_bytes) -> (16, align4(ts_bytes / 0x40))`: the RS window of
  the TS fill (`etnaviv_rs.c:337-360`).
- `auto_disable_count(padded_w, padded_h) = padded_w * padded_h / 16`
  (`etnaviv_rs.c:382-387`, `:470-475`).
- `color_clear_value(surf_format, rgba) -> u32` and
  `depth_clear_value(depth, stencil, has_stencil) -> u32`. The second gives the
  D24S8 word, `0xffffff00` for 1.0 and stencil 0 (`tools/vivante/README.md`,
  mode 8).
- `whole_word_clear(do_depth, do_stencil, has_stencil_object) -> bool`.
  Mesa's TS condition is `new_clear_bits == 0xffff` (`etnaviv_rs.c:467`). With
  no stencil object, a depth clear counts as Mesa's X8Z24 case (`:449-451`).

`early_z.rs` (item gc-early-z):

- `decide(EarlyZIn) -> EarlyZ { test: bool, write: bool, disable_zs: bool }`.
  The input fields are:
  - `target_linear`, `has_colour`;
  - `depth_test`, `depth_write`, `stencil_late`;
  - `shader_writes_depth`, `shader_discards`.

  This is `etna_update_zsa`, `etnaviv_state.c:992-1080`.
- `pe_bits(&EarlyZ) -> u32`: `GC_PE_DEPTH_EARLY_Z` (`regs.rs:417`), and
  `GC_PE_DEPTH_DISABLE_ZS` when nothing happens late.
- `ra_word(&EarlyZ) -> u32`: `GC_RA_EARLY_DEPTH_BASE | FORWARD_Z | FORWARD_W`,
  plus `TEST_ENABLE` when `test`, plus `WRITE_DISABLE` unless `write`
  (`regs.rs:338-344`, `etnaviv_state.c:1055-1063`).

## What exists today, in one table

| piece | where (723a75c8) | what it does now |
|---|---|---|
| SINGLE_BUFFER choice | `backend.rs:567-579` `single_buffer` | 2 for a 64-bit shadow, else 1 (or the opt-in MSAA value) |
| colour surface words | `backend.rs:1647-1683` (format, stride), `:1717-1731` (pipe addresses), `:1734-1745` (PE_LOGIC_OP) | linear at the GL core's pitch; the 64-bit shadow at `wide_target::stride`; `wide_target::bind` is called from inside the PE writer (`:1664`) |
| depth words | `backend.rs:1513-1525` | D24S8, no `SUPER_TILED` bit, so the PE writes the surface row-linear (case 169, `:1507-1511`) |
| TS words | `backend.rs:1751-1752`, `texture.rs:597`, `backend.rs:1794` | `PE_MEM_CONFIG` 0, `TS_MEM_CONFIG` 0, `TX_CTRL` 0, `TS_FLUSH_CACHE` with every draw's flushes |
| early depth | `backend.rs:1770-1784` `emit_ra_state` | never: `WRITE_DISABLE` always, no `TEST_ENABLE`, no `PE_DEPTH_CONFIG_EARLY_Z` |
| clear | `backend.rs:2349-2396` `emit_clear`, from `record` `:3097` | a full-target quad, fused in front of the draw |
| RS | `backend.rs:938-960` `emit_rs_copy` (flip study), `:998-1035` `emit_resolve` (4x MSAA, from `frame_submit` `:2300-2303`) | linear to linear, and super-tiled to linear with downsample |
| 64-bit shadow | `wide_target.rs` (`prepare` `:155`, `tile_in` `:239`, `bind` `:262`, `retire` `:272`, `released` `:302`); called from `draw_one` `backend.rs:2880-2887`, `finish` `:3207`, `gc_finish_previous` `:3232`, `kick_pending_retired` `:3256-3261` | CPU tile-in before the first draw, CPU untile when the kick retires |
| depth surfaces | `ds_cache.rs` (`Surface` `:80-92`, `select` `:264-393`, `copy_plane` `:435-481`, `seed_depth` `:497-543`, `resolve_depth` `:562-615`) | one D24S8 surface a (depth, stencil) pair, 64-aligned, row-linear, no TS (`:20-25`) |
| tiled sampling copies | `tiled_copy.rs` (`wanted` `:118`, `wanted_vs` `:174-180`, `refresh` `:357-434`, `lookup` `:498-512`) | 4x4 CPU copies for layered, 64/128-bit and vertex-stage textures |
| descriptor | `texture.rs:251-300` `format_words` (ADDRESSING `:271-273`, HALIGN 0 `:298`), `:306-449` `build_descriptor` (LINEAR_STRIDE `:375-384`, level addresses `:422-447`) | linear, or the tiled copy at HALIGN FOUR |
| GPU mipmap | `mipmap.rs:129-206` `record_level`, `:243-287` `chain_gpu` | one linear quad a level into the GL core's storage |
| state dedup | `backend.rs:1986-2048` `pe_key`, `dedup.rs:87-102` | the PE group is keyed; `pe_key` returns false for a 64-bit shadow draw because `bind` runs inside the writer |
| GL-core hooks | `src/backend.rs:1906-1911` `storage_current` (V3D arm only), called by `src/gl/texture.rs:893-898` `storage_back` from `settle_texture` `:926` (every CPU write) and `src/gl/framebuffer.rs:2944` `gpu_idle_for_cpu` (glReadPixels, glCopyTex*, blits) | the V3D's lazy raster store; nothing for the GC |

The CPU readers of render targets, and what each one waits for:

- **glReadPixels and glCopyTexImage2D**: `read_pixels_sync` →
  `gpu_idle_for_cpu` (`src/gl/framebuffer.rs:2851`, `:2925-2944`). This calls
  `backend::finish` (all four slots, `backend.rs:3197-3209`) and then
  `storage_back`.
- **The depth resolve**: `src/gl/texture.rs:698-739` `resolve_depth` waits for
  the render's kick (`:709`), then `gc_depth_resolve` (`backend.rs:2995-3004`)
  calls `ds_cache::resolve_depth`. glBlitFramebuffer's depth reaches the same
  function through `src/gl/blit.rs:874`.
- **`tiled_copy::refresh`** waits for the target's kick, or a chain's kicks,
  with `finish()` (`tiled_copy.rs:369-374`). It then reads the linear storage
  on the CPU (`:424-430`).
- **The CPU mipmap filter and `gc_return_mipmap_chain`** (`mipmap.rs:352-365`)
  wait for the chain's fence.
- **The window copy at present**: `eglSwapBuffers` →
  `finish`/`finish_previous` (`src/gl/egl.rs:2413`) → `swap_window_buffers`
  (`:2325`) → the board's `board_show`. The window stays linear in every item
  except W13.

## The fix items, in order

Every item:

- ends with the pull request's CI gate on the current tip;
- runs `src/vivante/backend_tests.rs`'s rig with `dedup::set_verify(true)`, the
  `host-test` default, so that a missing input in a key shows up as a test
  failure;
- keeps every existing stream byte-identical when its knob is off.

Board checks are on the Armadillo-X2 only, under plan #323's rules
(`gc-tiled-targets.md`, "The change list"). They are listed as checks for the
item x2-gm-after; no item here runs the board.

The host tests take Mesa's words from the golden modes, by number. The tree
keeps no golden fixtures: tests such as `backend_tests.rs:2399`, `:2736` and
`:2936` copy the golden mode's words into their asserts, with the mode named.
Where the stand-in OSAL cannot run the GPU, a test runs a small **RS
interpreter** over the captured stream instead. The interpreter is a test
helper with Mesa's semantics for the subset used: a linear or tiled source and
destination, the stride times 4 for a tiled side, and fill mode. It works
through `layout::offset`, so it shows that the stream's strides and tiling bits
agree with `layout.rs`. It does not show what the hardware does.

### W1. The TS, RS and sampler-TS registers (regs only; about 1 h)

- **Touches**: `src/vivante/regs.rs`, which today stops at `GC_TS_MEM_CONFIG`
  (`:634-638`) and `GC_RS_FORMAT_*` (`:1087-1091`).
- **Registers added**, values from `state_3d.xml.h`:
  - `GC_PE_MEM_CONFIG_COLOR_TS_MODE(x)` (bit 24) and `_DEPTH_TS_MODE(x)`
    (bit 26), `:1180-1186`;
  - `GC_TS_MEM_CONFIG_DEPTH_FAST_CLEAR` 0x1, `_COLOR_FAST_CLEAR` 0x2,
    `_DEPTH_16BPP` 0x8, `_DEPTH_AUTO_DISABLE` 0x10, `_COLOR_AUTO_DISABLE` 0x20,
    `:1456-1461`;
  - `GC_TS_COLOR_STATUS_BASE` 0x1658, `_SURFACE_BASE` 0x165c,
    `_CLEAR_VALUE` 0x1660, and the depth three at 0x1664, 0x1668, 0x166c;
  - `GC_TS_DEPTH_AUTO_DISABLE_COUNT` 0x1670, `GC_TS_COLOR_AUTO_DISABLE_COUNT`
    0x1674, `GC_TS_COLOR_CLEAR_VALUE_EXT` 0x16bc, `:1473-1495`;
  - `GC_RS_CLEAR_CONTROL_MODE_ENABLED1` 0x10000 and `rs_clear_bits(x)`,
    `:1400-1409`;
  - `GC_RS_DEST_STRIDE_TILING` 0x80000000 and `_SUPER_TILED_NEW` 0x08000000,
    `:1384-1386`;
  - `rs_extra_ts_mode(x)` (bit 24), `:1423-1425`;
  - `GC_RS_KICKER_INPLACE` 0x16b0, `:1428`;
  - `ts_sampler_config(i)` 0x1720 and `ts_sampler_status_base(i)` 0x1740,
    `:1501-1512`;
  - `GC_TX_CTRL_TS_ENABLE` 0x2, `tx_ctrl_ts_mode(x)` and `tx_ctrl_ts_index(x)`,
    `:1931-1938`;
  - `GC_TEX_HALIGN_SIXTEEN` 1 and `GC_TEX_HALIGN_SUPER_TILED` 2, from
    `common_3d.xml.h:124-128`, for `texdesc_halign` (`regs.rs:829`).
- **Allocation**: none.
- **CPU readers**: none.
- **Host test**: a `mod mesa_header` twin for `state_3d.xml.h`, in the style of
  `regs.rs` `tex_format_consts_match_mesa`. Every new `GC_TS_*`, `GC_RS_*`,
  `GC_PE_MEM_CONFIG_*` and `GC_TX_CTRL_*` constant is checked against its
  `VIVS_` define. The test fails if a value is mistyped.

### W2. One RS emitter (about 1.5 h; after tiled-copy-rs)

- **Touches**: `backend.rs` `emit_rs_copy` (`:938-960`) and `emit_resolve`
  (`:998-1035`). Both become callers of one `emit_rs(c, &RsJob)`:
  - `RsJob` holds the source and destination addresses, their `Layout`,
    strides and format, `w` and `h`;
  - an optional fill: `clear_bits`, `fill`, `MODE_ENABLED1`;
  - an optional source TS: base, surface, clear value, mode;
  - `downsample` and `flip`;
  - `inplace_tiles`.

  The emitter writes Mesa's order: `GL_FLUSH_CACHE` COLOR|DEPTH, then the
  RA→PE stall, then the RS block (`etnaviv_rs.c:845-850`, `:262-277`).
- **Coordination**: item tiled-copy-rs (running) adds an RS linear→TILED copy
  to `backend.rs` and `tiled_copy.rs`. W2 makes that copy the third caller,
  not a fourth copy of the block. If tiled-copy-rs merges first, W2 only folds
  the existing blocks into its emitter.
- **Registers**: the `GC_RS_*` set of `regs.rs:1030-1086`, plus W1's
  `DEST_STRIDE_TILING`, `CLEAR_CONTROL` bits and `EXTRA_CONFIG` TS mode.
- **Allocation**: none.
- **CPU readers**: none.
- **Host tests**:
  - the 4x MSAA resolve stream (`backend_tests.rs`, the `msaa` cases) and the
    flip-study copy are byte-identical before and after;
  - new unit tests of `emit_rs` for the shapes the later items emit:
    - tiled→linear (W4);
    - linear→tiled (tiled-copy-rs, W4's tile-in);
    - the TS fill (W8, W9);
    - a TS-sourced resolve (W8).

    Each is checked against the words `etna_compile_rs_state` gives for the
    same `rs_state` (`etnaviv_rs.c`, the struct literals at `:337-360` and
    `:892-922`), and against the ts-golden mode once it is merged;
  - the RS interpreter, run over the tiled→linear words of a 64x8 surface
    filled through `layout::offset(Tiled, ..)`, gives back the linear image.

### W3. Depth surfaces carry a layout; PE-private surfaces (about 1.5 h)

- **Touches**: `ds_cache.rs`:
  - `Surface` (`:80-92`) gets `layout: Layout`, `Linear` at allocation;
  - `select` (`:264-393`) gets the draw's wanted layout and returns it;
  - `copy_plane` (`:435-481`), `seed_depth` (`:497-543`) and `resolve_depth`
    (`:562-615`) refuse any surface that is not `Linear`. `copy_plane` returns
    `-ENOTSUP` with a line, as it already does for another stride (`:444-454`).
    `resolve_depth` and `seed_depth` answer as for a multisampled surface:
    1 or `-ENOTSUP`.

  Add a predicate `pe_private(key)`: no plane comes from a texture, so neither
  id has bit 30 (the GL core's texture bit, `src/gl/context.rs:554-556`). It
  is not the window, so neither id has bit 31. And scale is 1.
- **Coordination**: resolve-wide and resolve-own-kick (running or ready) change
  `resolve_depth`. W3 adds only the early return at its top, so rebase it after
  them.
- **The rule this sets up**: a surface whose layout is not `Linear` belongs to
  draws whose colour target is a tiled shadow, and only a PE-private surface
  may become one.
  - A non-private surface keeps `Linear`, so the depth textures of shadow and
    refract are not touched.
  - When a surface's layout changes, its planes become `Undefined`, unless a
    whole-clear value is kept (`Plane::cleared_value`). The PE never read the
    surface in another layout, so no conversion is needed. A live plane in
    another layout makes the draw keep the linear path (W4 asks `select`
    first).
- **Registers**: none in W3. W4 writes them.
- **Allocation**: unchanged. `align64(w) * 4` by `align64(h)` (`:277-278`)
  already holds a 64x64 super-tiled surface at that stride. Mesa's super-tile
  padding is 64x64 (`etnaviv_resource.c:389-393`).
- **CPU readers**: the three in `ds_cache.rs` refuse a non-linear surface, and
  so does glBlitFramebuffer's depth (`src/gl/blit.rs:874`, through
  `resolve_depth`). This cannot happen for a private surface: only texture
  ids reach `resolve_depth` and `seed_depth`. The guard keeps a wrong read from
  being silent.
- **Host tests** (`ds_cache` unit tests, and `tests/gc_depth_resolve.rs`
  unchanged):
  - every `ds_retention` case on the host is unchanged;
  - a forced `SuperTiled` surface is refused by `copy_plane` across layouts,
    and copied within one layout;
  - `resolve_depth` of a non-linear surface returns 1 and writes nothing;
  - `pe_private` is false for a texture id, the window id and scale 2.

### W4. 32-bit TILED shadow targets, resolved by the RS in the same stream (about 2 h; knob, default off)

- **What**: a draw qualifies for a shadow when all of these hold:
  - it has one colour target of 4 bytes a pixel, of a format `pe_color_format`
    draws (`backend.rs:307`);
  - `w % 16 == 0` and `h % 4 == 0`, so the padded shadow is exactly the target
    and the RS writes nothing past the GL core's rows;
  - not MSAA, not drawn upward (`ms_via_scratch`, `negative_stride`);
  - the depth test is on, with a `pe_private` depth surface (W3). With the
    test off, the draw still takes the shadow when that surface is already
    `SUPER_TILED` and holds contents: drawn linear it would ask `ds_cache`
    for a live surface in another layout, which refuses the draw
    (`tests/gc_tiled_rt.rs`, case 6).

  Such a draw renders into a TILED shadow under SINGLE_BUFFER(2), with depth
  `SUPER_TILED`. At `frame_submit` the stream gets the RS tiled→linear copy of
  the shadow into the target's storage.

  That gives terrain's pass (800x600, depth16 renderbuffer) and no other
  terrain pass: only it tests depth into an FBO, and tilt_v draws into the
  window. A later draw into the same target in another kick (the overlay,
  `overlay-renderer.cpp:72-90`) finds the shadow in step with the storage, so
  it may take the shadow without a tile-in. A draw into the target that takes
  the linear path marks the shadow stale.
- **Touches**:
  - `wide_target.rs`:
    - `BPP` (`:60`) becomes a field of `Entry` and `Key`;
    - `padded` and `stride` (`:115-124`) take the bpp, or call
      `layout::padded` and `pe_stride`;
    - `wanted` (`:137-143`) takes the bpp set;
    - `prepare` (`:155-220`) skips the tile-in when the shadow is in step or
      the draw opens with a full, unscissored, unmasked colour clear (terrain
      clears first, `terrain-renderer.cpp:46-47`). Otherwise the tile-in is
      the RS linear→tiled job of W2, recorded before the draw, not the CPU
      loop of `tile_in` (`:239-258`), which stays for 64-bit;
    - `retire` (`:272-297`) leaves 4-byte entries alone, because the stream
      already wrote them back;
    - a new `state: InStep | ShadowNewer(kick) | StorageNewer`;
    - `ENTRIES` 8 (`:64`) is shared; the 64-bit entries keep their rules.
  - `backend.rs`:
    - `wide_target()` (`:552-554`) also asks the W4 rule, and `single_buffer`
      (`:567-579`) gives 2 for it;
    - `draw_one`: the `wide_target` block (`:2880-2887`) calls `prepare`
      before the frame opens. The shadow is bound **in `draw_one`, not in
      `emit_pe_state`**. Its address goes into a field the PE writer reads
      (`g().shadow_gpu`, beside `ms_dest_gpu`), so that `pe_key`
      (`:1986-2048`) can key it (`k.u64(shadow_gpu)`, `k.u32(sb)`) instead of
      returning false. Today `pe_key` returns false for the 64-bit shadow at
      `:1988-1990`; once `bind` has moved, that case becomes keyed too;
    - `frame_submit` (`:2297-2306`): after `emit_resolve`, if
      `g.shadow_dest != 0`, call `emit_rs(Tiled → Linear, A8R8G8B8, w, h)`
      into the target's storage at its pitch, then clear it. One frame has
      one target (`draw_one` `:2903`), so there is at most one shadow resolve
      a stream;
    - `Mark` (`:3033-3084`) saves and restores `shadow_dest` as it does
      `ms_dest` (`restore_ms_dest`, `:3080-3083`), so a draw recorded again in
      a new frame still resolves;
    - `emit_pe_state`: `PE_COLOR_STRIDE` = `layout::pe_stride(Tiled, w, 4)`
      at `:1665-1672`, and the colour address at `:1721-1722`. For depth,
      `pe_depth |= GC_PE_DEPTH_SUPER_TILED` (`:1513-1517`; `regs.rs:423`)
      when the draw's surface layout is `SuperTiled`.
  - `src/backend.rs` `storage_current` (`:1906-1911`): one `gpu-vivante` arm
    that calls `vivante::backend::gc_storage_current(b)`, which marks the
    shadow of `b.base_gpu` `StorageNewer`. This is the only GL-core change,
    and it does not touch the texture classes.
  - A knob, `gc_tiled_rt` (test-knobs, default off), until the X2 has run it.
- **Registers**:
  - `PE_LOGIC_OP` SINGLE_BUFFER(2);
  - `PE_COLOR_STRIDE`, `PE_PIPE_COLOR_ADDR` 0/1;
  - `PE_DEPTH_CONFIG` `SUPER_TILED`, `PE_DEPTH_STRIDE` (unchanged value);
  - the RS block: `RS_CONFIG` `SOURCE_TILED`, `RS_SOURCE_STRIDE`
    `(pitch << 2) | TILING`, `RS_DEST_STRIDE` linear, `RS_WINDOW_SIZE`,
    `RS_SINGLE_BUFFER`, `RS_KICKER`.

  This is Mesa's combination for a TILED colour target with a super-tiled
  depth: the 128-bit formats, which Mesa keeps TILED (`etnaviv_resource.c:592-594`)
  while depth stays super-tiled (`:572-576`), under the mode
  `etnaviv_state.c:532-540` gives a target that is not linear.
- **Allocation**: one shadow a target, `w * h * 4` from a page boundary, as
  `wide_target.rs:193-204`. Terrain adds 1.92 MB to the carve-out. No TS yet.
- **CPU readers**:
  - **glReadPixels and glCopyTexImage2D**: they wait for every kick
    (`gpu_idle_for_cpu`), and the kick that rendered the shadow also wrote the
    storage back. Nothing changes for them.
  - **A CPU write** (glTexSubImage2D and the others through `settle_texture`):
    `storage_back` → `storage_current` marks the shadow `StorageNewer`, and the
    next shadow draw that does not clear tiles it in.
  - **`tiled_copy::refresh`**: it waits for `target_kick` (`:369-374`), and
    that kick holds the copy back. Unchanged.
  - **`mipmap.rs`**: the chain reads level 0 from the storage in a later kick.
    The RS ran earlier in GPU order, and every draw flushes the texture cache
    (`emit_flushes`, `:1790-1795`). Unchanged.
  - **The depth resolve**: never reaches the surface (W3).
  - **The window copy**: not involved.
  - **`texture_released`** (`backend.rs:3293-3295`) drops the shadow, as
    today.
- **Host tests**, a new `tests/gc_tiled_rt.rs` in the stand-in-OSAL style of
  `tests/gc_tiled_copy_reuse.rs`:
  - the knob on, an FBO with a 256x256 RGBA8 texture and a depth
    renderbuffer, a depth-tested draw. The stream holds:
    - `PE_PIPE_COLOR_ADDR` = the shadow, `PE_COLOR_STRIDE` = 1024;
    - `PE_LOGIC_OP` SINGLE_BUFFER 2;
    - `PE_DEPTH_CONFIG` with `SUPER_TILED`;

    and the last RS block before the kick's end copies tiled to linear into
    the texture's storage. The reference words come from golden mode 9
    (render into a texture through an FBO) for `PE_LOGIC_OP` and the
    colour format, and from mode 8 for the depth bits. Mesa's mode 9 target
    is super-tiled, so the colour format's `SUPER_TILED` and
    `SUPER_TILED_NEW` bits (`regs.rs:513-515`) are expected to differ, and
    the test says so;
  - the RS interpreter on that stream: a pattern laid into the shadow through
    `layout::offset(Tiled, ..)` ends up linear at the storage's pitch;
  - the same draw at 250x250, with a depth texture instead of a renderbuffer,
    or with the knob off, gives today's stream unchanged;
  - glTexSubImage2D after the render, then a draw that does not clear: the
    stream holds the linear→tiled tile-in before the draw;
  - glReadPixels after the render calls `finish` once and reads the storage
    (the count of `finish` calls, as `gc_tiled_copy_reuse.rs` counts them);
  - `dedup` verify: two draws into the shadow in one stream keep the PE group
    (a hit), and a switch between shadow and linear rewrites it.

### W5. Early depth for draws into a tiled shadow (about 1.5 h; same knob)

- **What** (built; these `file:line` are at the tree as it stands, not at
  `723a75c8`): one decision per draw from Mesa's rule (`early_z::early_z`,
  `early_z.rs:131-179`), taken wherever the draw's words are emitted, not
  stored on `GcDraw`. `tiled_ez` (`backend.rs:651-664`) answers for a draw
  only when it opens no clear quad, the stencil test is off, the depth test
  is on, and the target takes a tiled shadow (`:653`,
  `tiled_rt::Track::takes`, `tiled_rt.rs:100`); every other draw keeps
  today's "never" through `draw_ez` (`:667-669`, `draw_early_z`,
  `early_z.rs:186-188`). What the rule reads of a draw that passes that gate
  (`EarlyZInput`, `early_z.rs:55-75`, built at `backend.rs:656-663`):
  - colour layout `Tiled`, and `depth_func_always`, `depth_write`;
  - `writes_z` = `fs.viv.ps_depth_out_reg != SHC_VIV_NO_REG`
    (`src/shaderc/mod.rs:1057`, `:1082`);
  - `uses_discard` = `fs_discards` (`backend.rs:673-682`): the compiler's
    `uses_discard` flag, or a TEXKILL in the program words
    (`shader::discards`, `src/vivante/shader.rs:421`, the opcode in
    `src/shaderc/online/vivante/isa_tables.rs:177-179`). One pass over the
    words per shadow draw, uncached (Status, "The TEXKILL scan"); no
    `ShcViv` field.

  And what the rule says of a shadow draw (`early_z.rs`; each case is a row
  of `the_decision_table_follows_etna_update_zsa`, `:221-314`):

  - a plain depth-testing, depth-writing draw with neither: the test and the
    write are both early (`:139-155`; row "terrain tiled", `:229`) --
    `PE_DEPTH_CONFIG` `EARLY_Z` and `DISABLE_ZS`, `RA_EARLY_DEPTH`
    `TEST_ENABLE` without `WRITE_DISABLE`;
  - a program that discards: an early test and a late write -- the Status
    section's "Discard keeps the test early". `:141` withholds the early
    write from `uses_discard`; the early test at `:150` does not look at it.
    Words: `EARLY_Z` without `DISABLE_ZS`, `TEST_ENABLE` with `WRITE_DISABLE`
    (row "discard tiled", `:246-247`). Not "no early depth at all";
  - a program writing `gl_FragDepth`: no early depth, both halves late
    (`:141`, `:150`; row "writes z tiled", `:249-250`);
  - an enabled stencil test, whatever its ops: no early depth at all -- and
    not from the rule but from the gate: `tiled_ez` refuses such a draw
    before consulting Mesa's rule (`backend.rs:653`), so `draw_ez` answers
    today's words. The design's `stencil_late` input does not exist. Mesa's
    rule is not that strict: a stencil with non-KEEP ops is enabled and
    modified (`stencil_flags`, `early_z.rs:115-128`), its write is early on
    this part (`:140-142`), and the early test follows an early write
    (`!stencil_modified || early_zs`, `:150`; row "stencil replace tiled",
    `:271-279`). Only a discard pushes the modified stencil's write late and
    takes the test with it -- a fragment the early test killed could not
    carry out the stencil update the late pass owed it (row "stencil replace
    + discard tiled", `:280-288`). The back end keeps the whole stencil
    family out of the rule;
  - a clear quad (`is_clear`) is always "no" (`backend.rs:653`): Mesa has no
    clear quad to copy, and both halves stay off as today.

  The two halves leave one decision and the stream says so: `emit_pe_state`
  ORs `early_z.pe_bits` in (`backend.rs:1670-1677`), `emit_ra_state` writes
  `early_z.ra_word` (`:1947-1966`, the fallback equal to today's word
  asserted at `:1962-1964`), and `pe_key` keys both words (`:2199-2208`).
- **Touches**: `backend.rs`:
  - `emit_pe_state` ORs in `early_z::pe_bits`, next to the existing
    `DISABLE_ZS` rule (`:1518-1520`);
  - `emit_ra_state` (`:1770-1784`) writes `early_z::ra_word` instead of the
    constant `WRITE_DISABLE`. The comment at `:1755-1769` stays: it is the
    reason both halves come from one decision;
  - `pe_key` adds the decision (`k.u32(ez.bits())`), because the fragment
    shader is not in the PE key today (`:1991-2047`). The RA group is unkeyed
    (`dedup.rs` `Group::Ra`) and compares words, so it needs nothing.
- **Registers**: `PE_DEPTH_CONFIG` `EARLY_Z` (and `DISABLE_ZS`) and
  `RA_EARLY_DEPTH` `TEST_ENABLE` and `WRITE_DISABLE`.
- **Allocation**: none.
- **CPU readers**: none. Early depth changes only which fragments the shader
  runs on, not what is stored. The PE-private rule of W3 keeps every reader
  away from these surfaces.
- **Host tests** (landed in `src/vivante/backend_tests_tiled_rt.rs`; the
  `tests/gc_tiled_rt.rs` integration test over the public entry points is the
  open item on top of it):
  - a depth-tested draw into a shadow writes `PE_DEPTH_CONFIG` and
    `RA_EARLY_DEPTH` as golden mode 8 has them for its depth-tested draw --
    both halves, in the same stream, from one decision:
    `a_draw_into_a_tiled_shadow_takes_early_depth_and_nothing_else_does`
    (`backend_tests_tiled_rt.rs:239-291`), and the halves never disagree over
    any input of the rule (`both_halves_follow_one_decision`,
    `early_z.rs:332-352`). Mode 8 is a sound reference for the early-depth
    fields even though Mesa renders super-tiled, not TILED: the rule asks
    only whether the first colour target is linear (`early_z.rs:136`,
    `ColorLayout`, `:32-41`);
  - the same test (`:257-288`): a discarding program -- the compiler's flag,
    and a TEXKILL in the words -- keeps the test early and the write late;
    only a shader writing `gl_FragDepth`, a stencil test, the knob off (a
    linear target) and the clear quad keep today's words (`WRITE_DISABLE`, no
    `EARLY_Z`). The design's "for a discarding shader ... both words are
    exactly today's" is what did not land: Mesa gives a discarding program an
    early test and a late write (Status, "Discard keeps the test early");
  - `tests/gc_texkill_scan.rs`: the TEXKILL scan finds the `discard` of a
    compiled test shader, finds none in `mipgen.frag`'s or the clear quad's
    programs, and agrees with the instruction decoder over a corpus.
- **Board check (session A, item x2-gm-after or a session of its own, X2
  only)**: run one image built from origin/main with W4 and W5, with
  `gc_tiled_rt` off and then on. Check:
  - gl_test case 3, the case that hung when the two halves disagreed;
  - case 44 (the 64-bit shadow) and case 169 (the depth canary);
  - the default set;
  - terrain's ms a frame and picture value.

  The off/on pair measures both the RS resolve's cost and early depth's gain
  (`gc-tiled-targets.md` step 5's 0 to 14 ms).

### W6. The knob on by default (about 0.5 h; after session A)

- **What**: if session A is green, `gc_tiled_rt` defaults on. If the RS resolve
  costs more than early depth saves, the W4 rule narrows to draws whose
  fragment program is long, using the instruction count the shader carries
  (`usc_dw / 4`).
- **Touches**: the knob's default in `backend.rs`.
- **Host test**: `tests/gc_tiled_rt.rs` without the knob set. The glmark2 host
  harness's 33 entries keep their GL errors and fallback counts.

### W7. TS buffers for shadow colour surfaces, programmed but never valid (about 1.5 h; after the probe)

- **Gate**: gt-tiled-probe (iii) shows a TS clear and resolve landing at the
  back end's addresses with the MMU off. Mesa never runs that configuration
  (`gc-tiled-targets.md`, "Risks").
- **What**: every W4 shadow gets a TS buffer of `ts::ts_bytes(w * h * 4)`,
  aligned to `0x100` inside the shadow's allocation, after the pixels. The
  shadow's draws then program:
  - `PE_MEM_CONFIG` `COLOR_TS_MODE(0)` (128-byte TS mode);
  - `TS_COLOR_STATUS_BASE`, `TS_COLOR_SURFACE_BASE` and
    `TS_COLOR_CLEAR_VALUE` (`_EXT` 0).

  `TS_MEM_CONFIG` stays 0 while the TS is not valid. This is Mesa's order
  (`etnaviv_emit.c:586-597`): `TS_FLUSH_CACHE` before any change of TS state
  (`:294-297`). Every draw already sends that flush (`backend.rs:1794`).
  Before the first fast clear, the TS is filled with the "not cleared" code by
  zeroing it on the CPU.
- **Touches**:
  - `wide_target.rs`: the allocation, and `ts_gpu` and `ts_valid` in `Entry`;
  - `backend.rs`: the TS writes in `emit_pe_state` replace `:1751-1752`, and
    `pe_key` keys `ts_gpu` and the valid bit;
  - `dedup.rs`: the PE `ROOM` of 128 words (`:94`) holds about 12 more words;
    check it in the test.
- **Registers**: `PE_MEM_CONFIG`, `TS_MEM_CONFIG`, `TS_COLOR_*`,
  `TS_FLUSH_CACHE`.
- **CPU readers**: none yet. With the TS never valid, the PE writes every tile,
  and W4's resolve copies them as before.
- **Host tests**:
  - the shadow draw's words carry the three TS registers, with the bases
    inside the allocation, `0x100`-aligned;
  - `TS_MEM_CONFIG` is 0;
  - with the knob off, the stream is today's.

### W8. Fast colour clear of shadow targets, and the TS-sourced resolve (about 2 h)

- **What**: in `record` (`backend.rs:3097`), a colour clear of a shadow
  target that is full, unscissored and has every channel
  (`clear_components == 0xf`) becomes:
  - the RS TS fill of the shadow's TS (W2's fill job:
    `ts::ts_fill_rect`, `TS_CLEAR_WORD`, `MODE_ENABLED1`, bits 0xffff,
    `etnaviv_rs.c:337-360`);
  - `TS_COLOR_CLEAR_VALUE` = `ts::color_clear_value`;
  - `TS_COLOR_AUTO_DISABLE_COUNT` = `ts::auto_disable_count`, and
    `TS_MEM_CONFIG` `COLOR_FAST_CLEAR | COLOR_AUTO_DISABLE`
    (`etnaviv_rs.c:373-387`).

  Then the TS is valid. A clear fused with depth keeps its quad for the depth
  only (colour mask 0) until W9. A scissored or masked clear keeps the quad,
  as Mesa's state tracker does (`etnaviv_screen.c:242`).

  The frame-end resolve of W4 becomes a TS-sourced copy, as Mesa's
  `etna_try_rs_blit` does (`etnaviv_rs.c:852-891`):
  - `TS_FLUSH_CACHE`;
  - `TS_MEM_CONFIG` `COLOR_FAST_CLEAR`;
  - `TS_COLOR_STATUS_BASE`, `SURFACE_BASE` and `CLEAR_VALUE` of the source;
  - `RS_EXTRA_CONFIG` `TS_MODE`.

  Cleared tiles then read as the clear value. A TILED source has no in-place
  resolve (`etnaviv_rs.c:164-181` requires super-tiled), but W4's copy is not
  in place anyway. It goes to the linear storage.
- **Validity**: valid after the fill. Invalid after a tile-in (an RS or CPU
  write outside the PE) and after `StorageNewer`. The TS stays valid after a
  resolve without compression (`etnaviv_rs.c:933-939`).
- **Compression, an open point**: the README's measurement of Mesa's GC7000UL
  row (`tools/vivante/README.md:694-701`) shows Mesa writing
  `TS_MEM_CONFIG` 0x43c2 in mode 8. That includes `COLOR_COMPRESSION` 0x80
  and `DEPTH_COMPRESSION` 0x40. `gc-tiled-targets.md` read V4Compression 0
  from the hwdb, and Mesa compresses only with v4 or MSAA
  (`etnaviv_resource.c:196-199`). So the shim's row and the hwdb row may
  differ on `v4_compression`. This design programs no compression. Until
  ts-golden settles which row is right, the W8 tests compare `TS_MEM_CONFIG`
  with bits 0x40, 0x80, 0xf00 and 0x4000 masked, and say so.
- **Touches**:
  - `backend.rs`: `emit_clear` (`:2349-2396`) is split into
    `emit_fast_clear` and the quad;
  - `record` picks between them;
  - `frame_submit` passes the TS to the resolve;
  - `wide_target.rs` keeps the valid bit.
- **Registers**: `TS_COLOR_CLEAR_VALUE`, `TS_COLOR_AUTO_DISABLE_COUNT`,
  `TS_MEM_CONFIG`, the RS fill block, `RS_EXTRA_CONFIG` `TS_MODE`.
- **CPU readers**: glReadPixels and the others read the linear storage, which
  the TS-sourced resolve has filled. `tiled_copy`, mipmap and the overlay are
  unchanged, as in W4.
- **Host tests**:
  - a cleared and drawn shadow: the stream holds the fill words of the
    ts-golden mode (Mesa's colour TS fill) and no clear quad;
  - the frame-end resolve carries the TS source registers;
  - the RS interpreter, extended with Mesa's TS semantics (a tile whose entry
    is the clear code reads the clear value), gives the clear colour where the
    draw did not cover, and the drawn pixels where it did;
  - a scissored clear keeps the quad.
- **Board check**: terrain's ms a frame (the four 800x600 target clears and
  tilt_h's, `gc-tiled-targets.md` change 4's 1 to 5 ms); gl_test's clear cases.

### W9. Depth TS and fast depth clear for PE-private surfaces (about 2 h)

- **What**: a `SuperTiled` (private) `ds_cache` surface gets a TS buffer of
  `ts::ts_bytes(stride * rows)`. A depth clear for which
  `ts::whole_word_clear` holds becomes:
  - the RS TS fill;
  - `TS_DEPTH_CLEAR_VALUE` = `ts::depth_clear_value`;
  - `TS_DEPTH_AUTO_DISABLE_COUNT`;
  - `TS_MEM_CONFIG` `DEPTH_FAST_CLEAR | DEPTH_AUTO_DISABLE`
    (`etnaviv_rs.c:467-480`).

  Terrain's depth16 has no stencil object, so this is the case Mesa
  fast-clears (`gc-tiled-targets.md`, "Fast clear"). The fused colour and
  depth clear of terrain's pass then becomes two TS fills with no quad.
- **CPU readers**: none. A private surface is never read by `resolve_depth`
  or `seed_depth`.
  - `copy_plane` between two private surfaces of one renderbuffer must
    resolve first: an RS in-place resolve of the source (super-tiled, so
    `RS_KICKER_INPLACE` with the tile count, `etnaviv_rs.c:164-181`), then
    `finish`. A smaller first step is to drop the TS (set it invalid after an
    in-place resolve) before the copy.
  - `Plane::cleared_value` and `Selected::restore` (`ds_cache.rs:237-244`) are
    unchanged in meaning. A restore is a clear like any other, so it becomes a
    TS fill.
- **Touches**:
  - `ds_cache.rs`: `Surface` gets `ts: Option<(gpu, valid)>`, `alloc`
    (`:626-629`) adds the TS bytes, and `copy_plane` resolves first;
  - `backend.rs`: `emit_pe_state` adds the depth TS bits to `PE_MEM_CONFIG`
    (`DEPTH_TS_MODE`) and writes `TS_DEPTH_*`; `record` and `emit_clear` route
    the depth clear; `pe_key` keys the TS.
- **Registers**: `PE_MEM_CONFIG` `DEPTH_TS_MODE`, `TS_DEPTH_STATUS_BASE`,
  `_SURFACE_BASE`, `_CLEAR_VALUE`, `TS_DEPTH_AUTO_DISABLE_COUNT`,
  `TS_MEM_CONFIG`, `RS_KICKER_INPLACE`.
- **Host tests**:
  - the fused clear of an FBO with a depth renderbuffer and no stencil gives
    two TS fills and no quad, with the depth fill's words as the ts-golden
    mode's depth TS clear;
  - with a stencil object, a depth-only clear keeps the quad (Mesa: RS fill
    with bits 0xeeee, golden mode 8);
  - `copy_plane` of a TS surface records an in-place resolve and a `finish`
    first;
  - every `ds_retention` case is unchanged.
- **Board check**: terrain's ms a frame; gl_test's depth and stencil cases
  (3, 9, 12, 23, 169) on the X2.

### W10. Single-level shadow targets sampled directly (about 2 h; after tiled-copy-rs)

- **What**: a 2D texture with one level whose storage has a shadow in step is
  sampled from the shadow. The words, at the tree as it stands -- recorded
  for a single-level TILED 32-bit shadow, terrain's 256x256 RGBA8 height and
  normal maps as the size:
  - CONFIG0, ADDRESSING `TILED` is the field's value **0**
    (`GC_TEX_ADDRESSING_TILED`, `regs.rs:829`, shift 20 `:826-828`;
    `TEXTURE_ADDRESSING_MODE_TILED` 0x0, `common_3d.xml.h:131`;
    `VIVS_TE_SAMPLER_CONFIG0_ADDRESSING_MODE`, `state_3d.xml.h:1605-1607`).
    Mesa writes the field only for a linear resource
    (`etnaviv_texture_desc.c:179-182`), and `format_words` writes the same
    zero for a tiled side (`texture.rs:356-358`);
  - CONFIG1 **0x04321000** = HALIGN SIXTEEN (`TEXTURE_HALIGN_SIXTEEN` 1,
    `common_3d.xml.h:125`; field bits 28:26: `texdesc_halign`,
    `regs.rs:858-860`, `VIVS_TE_SAMPLER_CONFIG1_HALIGN` mask 0x1c000000,
    `state_3d.xml.h:1682-1684`) over the RGBA8 base `0x00321000` -- the base
    of Mesa's own words without SWIZZLE_A = ONE (`texture.rs:361-363`).
    Mesa takes HALIGN from the resource: a TILED resource the resolve engine
    may touch is 16x4-padded and gets SIXTEEN (`etna_layout_multiple`,
    `etnaviv_resource.c:357-362`, `:384-388` -- W4's qualification
    `w % 16 == 0`, `h % 4 == 0` puts a shadow in exactly that shape), and the
    sampler descriptor ORs it in (`etnaviv_texture_desc.c:183-186`);
  - LINEAR_STRIDE **0x400** = 256 x 4: Mesa's `levels[0].stride`
    (`etnaviv_texture_desc.c:190`, laid out by `setup_miptree`,
    `etnaviv_resource.c:327-332`); the copy path computes the same number for
    the same size (`texture.rs:463-472`);
  - SLICE **0x40000** = the whole 256x256 layer: Mesa's
    `levels[0].layer_stride` (`etnaviv_texture_desc.c:192`); the copy's is
    `align4(w) * align4(h) * bpp` (`tiled_copy.rs:652`, used at
    `texture.rs:482-484`);
  - LOD 0 = the shadow -- and it is the only word that differs from the RS
    tiled copy's descriptor of the same size and format: Mesa's LOD words are
    `bo + levels[lod].offset` (`etnaviv_texture_desc.c:204-205`), and every
    other word of that copy is in place today
    (`texture.rs:449-455` with `tiled_copy::lookup`'s SIXTEEN,
    `tiled_copy.rs:640-654`).

  TX_CTRL stays **0** without W8: `write_sampler` writes 0, "no tile status"
  (`texture.rs:702-703`). With W8 it is **`0x42 | slot << 2`**: TS_ENABLE 0x2
  and TS_INDEX `slot << 2` (`state_3d.xml.h:1931-1938`, `regs.rs:1144-1150`)
  plus 128B_TILE 0x40 (`state_3d.xml.h:1939`), which Mesa sets whenever the
  TS mode field is 0 (`etnaviv_texture_desc.c:302-307`) -- and 0 is the
  128-byte TS mode W7 programs. 0x42 is Mesa's own word from the board:
  `TX_CTRL[0]` 0x42 with TS_ENABLE in terrain's stream, at slot 0 where
  `slot << 2` vanishes (`gc-slow-terrain-stream-diff.md:165`, `:231`).

  This replaces `tiled_copy`'s vertex-stage copy for terrain's height and
  normal maps (`tiled_copy.rs:174-180`), and the RS copy tiled-copy-rs adds in
  its place. The frame-end resolve of W4 is then left out for a target nobody
  samples linear: it becomes lazy, done by `gc_storage_current` (an RS
  tiled→linear job and a `finish`) before a CPU reader. With W8 in, the
  sampler reads through the TS (`TS_SAMPLER_CONFIG`, `_STATUS_BASE`,
  `_CLEAR_VALUE`, TX_CTRL `TS_ENABLE` and `TS_INDEX`; `etnaviv_texture.c:80-159`,
  level 0 only, `:151-156`).

  The shadow rule of W4 widens to such textures even without a depth test.
  Height and normal are 256x256, which is RS-sized.
- **Touches**:
  - `texture.rs`: `build_descriptor` (`:306-449`) asks
    `wide_target::sampled(t)` next to `tiled_copy::lookup` (`:310`);
    `write_sampler` (`:596-617`) writes TX_CTRL;
  - `tiled_copy.rs`: `wanted_vs` (`:174-180`) is false for a texture with a
    shadow;
  - `wide_target.rs`: a `sampled_by` kick, so a shadow a pending kick samples
    is never freed (as `tiled_copy::busy`, `:488-490`);
  - `backend.rs`: the frame-end resolve is skipped for lazy entries.
- **CPU readers**: this is the first item where the storage is behind the
  shadow after a kick. Every CPU reader must go through `storage_current`:
  - glReadPixels and glCopyTexImage2D through `gpu_idle_for_cpu`
    (`src/gl/framebuffer.rs:2944`) do already;
  - so does a CPU write through `settle_texture` (`src/gl/texture.rs:926`);
  - `tiled_copy::refresh` of a layered texture never meets a 2D shadow;
  - `mipmap.rs` `chain_gpu` (`:243-287`) reads level 0 linear, so it resolves
    first (record the RS job before the level kicks);
  - `src/gl/texture.rs` `resolve_depth` is not involved (colour only).

  A reader that bypasses `storage_current` would read stale texels. The host
  test below fails for each of them.
- **Registers**: `TEXDESC_CONFIG0` addressing, `CONFIG1` HALIGN, LOD 0,
  `NTE_DESCRIPTOR_TX_CTRL`, `TS_SAMPLER_*` (W8 and later).
- **Host tests**:
  - `tests/gc_tiled_copy_reuse.rs` changes: a rendered height map is sampled
    by the vertex stage with no copy and no `finish` (counted), and the
    descriptor's words equal the RS tiled copy's of the same size and format
    with LOD 0 alone changed (the list above) -- *not* golden mode 26's.
    Mode 26 cannot pin this case: its CONFIG1 is `0x08521000`
    (`texture.rs:367-369`), whose HALIGN field is 2 = SUPER_TILED
    (`regs.rs:858-865`, `state_3d.xml.h:1682-1684`), and HALIGN 2 comes only
    from a super-tiled resource (`etnaviv_resource.c:389-393`): Mesa samples
    that dEQP texture from a super-tiled one, and the word's base is the
    GL_RGB one with SWIZZLE_A = ONE, not RGBA8's. No golden word in the tree
    carries HALIGN SIXTEEN -- the recorded CONFIG1 words are 0x00321000,
    0x00321002, 0x00521000 (HALIGN 0) and 0x08321000, 0x08500000, 0x08521000
    (HALIGN 2), `tex_layout.rs:31-35`, `gc-slow-terrain-stream-diff.md:165`;
  - so HALIGN SIXTEEN for a TILED target is pinned by the host test of the
    identical words, not by a golden:
    `a_render_into_the_texture_is_tiled_by_the_rs_with_no_finish`
    (`tests/gc_tiled_copy_reuse.rs:500`) asserts SIXTEEN in the RS copy's
    CONFIG1 (`:582-583`), and Mesa's derivation is the source lines above
    (`etnaviv_resource.c:384-388`, the HALIGN-feature comment
    `etnaviv_texture.c:230-232`, `etnaviv_texture_desc.c:183-186`). A golden
    that pins it outright would be a new cs_golden mode sampling a
    single-level TILED 16x4-aligned texture -- Mesa's `rs_align` case,
    `etnaviv_resource.c:357-362 -- and none of the current modes samples
    one;
  - glReadPixels and glTexSubImage2D after the render resolve first (an RS job
    and one `finish`);
  - glGenerateMipmap of such a texture records the resolve before the first
    level kick.

### W11. Tiled mipmap chains (two items of about 2 h each)

- **W11a, layout and descriptor**: a 2D RGBA8 texture whose chain the GPU
  makes (`mipmap.rs` `chain_gpu`) gets a tiled chain beside its storage. The
  chain's levels are laid out as `layout::padded(Tiled, ..)` at 64-byte
  boundaries, `tiled_copy::tiled_layout`'s rule with 16x4 padding. The
  descriptor samples it: TILED, HALIGN SIXTEEN, one level address each
  (`texture.rs:422-428`, the same path the copy uses).
  - Host test: a unit test of the chain layout against `layout::offset`, and
    the descriptor words for a 3-level chain.
- **W11b, the chain drawn tiled**: `record_level` (`mipmap.rs:129-206`)
  renders level `l` into the tiled chain under SINGLE_BUFFER(2), sampling
  level `l - 1` tiled. Levels smaller than 16x4 are still drawn: the PE pads,
  only the RS needs 16x4. Level 0 is the W4 shadow.
  - `gc_return_mipmap_chain` (`:352-365`) and `storage_current` untile the
    chain on the CPU: through `layout::offset`, after the fence, levels RS-sized
    by the RS, and the rest on the CPU. Mesa does the same: a HALIGN_FOUR or
    small level is detiled in software, `etnaviv_transfer.c:357-361`.
  - Terrain, tilt_h and bloom then sample tiled chains with no resolve. Refract's
    1600x1200 colour chain needs its depth to be private, which it is not (a
    depth texture), so it waits for W12.
  - Coordination: mip-one-kick (ready) records the chain in one kick. W11b
    goes after it.
  - Host tests: `tests/gc_mipmap_gpu.rs` with tiled levels (the target
    addresses step through the tiled chain, `PE_LOGIC_OP` SINGLE_BUFFER 2);
    glReadPixels of level 2 after the chain returns the box filter's pixels
    through the CPU untile (the stand-in OSAL's memory is filled through
    `layout::offset` by the test, as the GPU would have).

### W12. Depth textures tiled, with a TS-aware CPU resolve (two items; after the probe and depth-sample-design)

- **Gate**: gt-tiled-probe (ii) dumps the D24S8 surface under SINGLE_BUFFER(2)
  with and without `SUPER_TILED`. depth-sample-design and the X2 depth-TPU
  session decide whether the CPU resolve stays at all.
- **W12a**: a depth texture's surface (not private) may be `SuperTiled` when
  its draws are shadow draws (refract's colour pass with W11), or when it has
  no colour target (shadow's depth pass, under SINGLE_BUFFER(2) with Mesa's
  dummy target, `etnaviv_state.c:397-407`).
  - `ds_cache::resolve_depth` and `seed_depth` read and write through
    `layout::offset(SuperTiled, ..)` instead of refusing (W3).
  - `copy_plane` converts between layouts on the CPU.
  - `ensure_no_colour_surface` (`backend.rs:785-797`) gets a tiled dummy.
  - Host test: `tests/gc_depth_resolve.rs` extended, bit-exact floats from a
    surface filled through the layout, at its (w, h, pitch) cases.
- **W12b**: depth TS on those surfaces (W9's code), and `resolve_depth` reads
  the TS first. A tile whose entry is the clear code takes the clear value's
  float, with no read of the surface. This is shadow's and refract's 30 to
  150 ms (`gc-tiled-targets.md` change 3).
  - Host test: a synthetic TS, with cleared tiles giving the clear float and
    written tiles giving the converted words, bit-exact against the full read.
  - If the texture unit samples D24S8 (depth-sample-gc), W12b goes: the
    sampler reads the TS itself (one-level depth textures,
    `etnaviv_texture.c:151-156`).

### W13. The window tiled, resolved at the swap (optional, last)

As `gc-tiled-targets.md` step 6: a shadow of the EGL back buffer, with the
frame-end resolve of W4 recorded in the last kick before
`finish_previous` (`src/gl/egl.rs:2413`), so the window copy
(`swap_window_buffers`, `:2325`) reads resolved pixels. glReadPixels on the
window goes through `gpu_idle_for_cpu`'s `finish`.

- Host tests: the swap's last stream ends with the RS resolve, and glReadPixels
  on the window reads after it.
- Highest risk: the LCDIF must never scan a buffer being written
  (`imx8mp-frame-floor.md`, #923).

## Order, dependencies and risk

| item | depends on | board gate before it ships on | gain (model) |
|---|---|---|---|
| W1 | gc-groundwork-mod not needed | none | none |
| W2 | W1, tiled-copy-rs | none | none |
| W3 | gc-groundwork-mod (layout), resolve-wide, resolve-own-kick | none | none |
| W4 | W2, W3 | session A (knob) | none by itself; costs one RS copy |
| W5 | W4, gc-early-z | session A | terrain 0 to 14 (19) ms |
| W6 | session A | none | as W5 |
| W7 | W4, gc-ts-words | gt-tiled-probe (iii) | none |
| W8 | W7 | an X2 run | terrain 1 to 5 ms, refract 2.5 to 7.5 ms (after W11, W12) |
| W9 | W3, W8 | an X2 run | terrain's depth clear, inside W8's figure |
| W10 | W4, tiled-copy-rs | an X2 run | terrain 4 to 6 ms (the part tiled-copy-rs has not already taken) |
| W11a, W11b | W10, mip-one-kick | an X2 run | the chains' resolves, and texture-cache locality (unmodelled) |
| W12a, W12b | gt-tiled-probe (ii), depth-sample-design | an X2 run | shadow and refract 30 to 150 ms, if the CPU resolve stays |
| W13 | W8 | an X2 run, every scene | terrain about 2 ms |

Why W1 to W5 is the least risk for terrain's early depth:

- **Nothing a CPU reads changes layout.** The storage is written back in the
  kick that renders it. The depth surface it uses is one only the PE touches
  (W3's rule).
- **Every register combination is one Mesa emits on this part**: a TILED colour
  target under SINGLE_BUFFER(2) with a super-tiled depth (Mesa's 128-bit
  targets), early depth from one decision, and an RS tiled→linear copy (Mesa's
  transfer and flush path).
- **The PE's TILED write is proven** on this board for 64-bit pixels (gl_test
  44, `wide_target.rs:25-32`).
- **What is not proven** is the RS reading a TILED (not super-tiled) source on
  the board: #8's 4x scratch was super-tiled and came back empty. So is the
  RA's early depth against a super-tiled depth surface under SINGLE_BUFFER(2).
  Session A answers both with the knob. Turning the knob off restores today's
  streams byte for byte.
- **No TS is needed**, so the open question of TS with the MMU off does not
  block it.

## Plan #310: what it shares with this work

Plan #310 is changing the `src/vivante` texture formats for ES 3.0 and the GL
core's texture classes (`docs/notes/gl-texture-classes-es3.md`). The overlap:

- **`texture.rs` `format_words`** (`:251-300`). #310 adds `gc_texfmt` arms
  (`:96`) and format_ext words. W10 and W11 change the ADDRESSING argument and
  CONFIG1's HALIGN (`:271-273`, `:298`). Keep HALIGN a parameter of
  `format_words` (today the `tiled` flag is its only layout input), so the two
  edits touch different lines.
- **`texture.rs` `build_descriptor`'s LINEAR_STRIDE and level addresses**
  (`:375-384`, `:422-447`). #310's 64-bit and 128-bit classes go through the
  tiled copy here (`copy_bpp`). W10 and W11 add a third source, the shadow, in
  the same `match`.
- **`tiled_copy.rs` `wide_texels` and `copy_bpp`** (`:144-155`). #310's
  classes decide which textures copy. W10 narrows `wanted_vs` for shadowed 2D
  textures only.
- **`backend.rs` `pe_color_format`** (`:307`), `GC_INTEGER_RT_FORMATS`
  (`:340-356`) and `ms_rs_format` (`:988-994`). W4's "4 bytes a pixel, a
  format the PE draws" rule and the RS format of the resolve come from these.
  Make the RS format one function, `rs_format_of(surf)`: A8R8G8B8 for every
  4-byte class (a bit copy), R5G6B5 or A4R4G4B4 for 16-bit, and none for
  64-bit (the RS copies no 64-bit format, `wide_target.rs:22-23`). #310's new
  render-target classes then get a shadow, or not, from that one place.
- **`wide_target.rs`**: #310's 64-bit integer render targets are its current
  users (`:3-4`). W4 generalises the module by bpp and must keep their
  CPU-retire path.
- **The GL core**: only `src/backend.rs` `storage_current` (W4, one arm). No
  texture-class or `src/gl/texture.rs` change is needed. `storage_back` and
  `settle_texture` already call it (`src/gl/texture.rs:893-898`, `:926`).

Rebase each W item on the current origin/main before the pull request's CI gate
runs. Grep `tests/` and `src` for struct literals of anything that
gains a field (`GcDraw`, `wide_target::Entry`, `ds_cache::Surface` are
private; `YakoglBackend` is not touched by any item).

## Session A: the run sheet

The X2 session that W4 and W5 name ("Board check (session A ...)", above). It
only measures: no code changes, and no default moves (W6 is a separate item,
ordered after plan #351's i.MX8MP board run). Every fact below is read from the
tree. No board address, host name, UART path or credential is written here,
and none may go into a record.

### Before any jump

1. **One commit.** `git fetch origin`, `SHA=$(git rev-parse origin/main)`, the
   same `SHA` for every image. It must carry W1-W5. Check, and stop with no
   board touched if any of these finds nothing:
   - `git grep -n 'TS_COLOR_STATUS_BASE' $SHA -- src/` (W1; today it finds
     nothing, "What exists today" above);
   - `git grep -n 'gc_debug_set_tiled_rt' $SHA -- src/ driver/samples/` (the
     knob and both samples' calls, `driver/samples/zephyr/glmark2/src/main.cpp:1469-1472`,
     `driver/samples/zephyr/gl_test/src/main_imx8mp.c:471-478`);
   - `git grep -n 'early_z::' $SHA -- src/vivante/backend.rs` (W5's
     `early_z::pe_bits` and `early_z::ra_word`, W5 "Touches" above);
   - `git ls-tree --name-only $SHA tests/gc_tiled_rt.rs` (the host test of
     W4 and W5, "Host tests" of each).

   Write down `SHA` and `<sha8>` (its first eight characters).
2. **Reachability from the leased worker, first.** This is the cause of the
   2026-11-11 stop (`docs/notes/x2-glmark2-slow-scenes.md:11-24`): ping by the
   board's name (`BOARD_HOST` from the environment) and ssh with the
   persisted key, not a per-task forward. Then `tools/board.sh x2 status`
   (reads the board, no jump). Any failure: no image is built for the board,
   no jump is made, the report says the session did not start and why.
3. **Two images from `SHA`, knob off and on, both from `origin/main` only.**
   `tools/cts/x2-glmark2-scenes.sh` refuses a commit that is not an ancestor of
   `origin/main`, tracked changes when it builds `HEAD`, and an image without
   `gc_wdog_arm: armed` (`tools/cts/x2-glmark2-scenes.sh:29-32`, `:108-119`,
   `:156-157`); `tools/board.sh` refuses such an image too (`tools/board.sh:56-58`).
   Check each image with `grep -aF 'gc_wdog_arm: armed' <zephyr.bin>`.
   `NETBOOT_NO_WATCHDOG=1` is never set; the script unsets it and
   `BOARD_UART` for every board call (`:33-34`, `:171-172`).
   - glmark2: `--commit "$SHA"` is the off image; the on image adds
     `--conf CONFIG_GLMARK2_GC_TILED_RT=y`, the only key accepted
     (`:18-23`, `:72`). On rows read `<sha8>+tiled_rt` in the commit column
     and their directories get `-tiled_rt`, so both arms share one `<outdir>`.
   - gl_test: `IMX8MP_BOARD=x2 tools/cts/imx8mp-gltest-build.sh <name> <first case> [-D...]`
     builds `armadillo_x2/mimx8ml8/a53` into `$IMX8MP_OUT/x2/<name>` with
     `CONFIG_GC_BOARD_WDOG_SELF_ARM=y` by default
     (`tools/cts/imx8mp-gltest-build.sh:21-23`, `:56-59`;
     `tools/cts/imx8mp-env.sh:18-22`). A sweep covers 48 case numbers from
     the first case (`driver/samples/zephyr/gl_test/Kconfig:198-210`), so
     cases 3 and 44 come from first case 0 and case 169 from first case 144
     (selector = case - first, `imx8mp-gltest-build.sh:8-9`: 3, 44, 25). Four
     images, each pair off and on, the on ones with
     `-DCONFIG_GL_TEST_GC_TILED_RT=y` (`Kconfig:365-374`). Build from a
     `git worktree add --detach` of `SHA`, never from a tree with changes.
   - `--dry-run` (the script) and `DRYRUN=1` (the build script) touch no board;
     run them first (`x2-glmark2-scenes.sh:40-41`,
     `imx8mp-gltest-build.sh:25`).
4. **Resources and the console.** `target:x2` and `target:x2-console`, and
   optionally `target:x2-display`. The console only reads: no character is
   sent to it, and `target:x2-display` (the NanoKVM) is observation only.
   `BOARD_UART` comes from the environment only (`tools/board.sh:189-191`).
   Never `target:imx8mp*`, `am62p*`, `rpi4*`, `rpi5*`, the NanoPC-T6 or the
   STM32MP257F-DK. Nothing is installed or persisted, the eMMC boot
   partitions, the U-Boot environment and swupdate are not touched, and the
   board is left as found (`docs/notes/gc-ts-probe.md:97-99`, `:472-484`).

### The order

Terrain, shadow, refract, then gl_test cases 3, 44 and 169. Each is run off,
then on, from the one `SHA`:

| # | what | off, then on |
|---|---|---|
| 1 | terrain | one script call each: a run without a selector, then PERF pages `0x1000`, `0x1001`, `0x1003` (and `0x1004` when `gm_perf_page.h` has five pages), `x2-glmark2-scenes.sh:128-132` |
| 2 | shadow | the same |
| 3 | refract | the same |
| 4 | gl_test case 3 (the case that hung when the two halves disagreed, W5 above) | one boot each |
| 5 | gl_test case 44 (the 64-bit shadow) | one boot each |
| 6 | gl_test case 169 (the depth canary) | one boot each |

The glmark2 scenes go before gl_test so that a case 3 hang in the on arm
still leaves their numbers.

### The invocations

From the checkout of `SHA` with `ZEPHYR_WS` and `GLMARK2_SRC` set
(`x2-glmark2-scenes.sh:43-46`, `:105-106`), `BOARD_HOST` from the environment,
and `<outdir>` outside the tree. One scene a call, so a stop is read between
scenes:

```sh
tools/cts/x2-glmark2-scenes.sh --dry-run --commit "$SHA" <outdir> terrain shadow refract
tools/cts/x2-glmark2-scenes.sh --dry-run --commit "$SHA" --conf CONFIG_GLMARK2_GC_TILED_RT=y <outdir> terrain shadow refract
tools/cts/x2-glmark2-scenes.sh --commit "$SHA" <outdir> terrain
tools/cts/x2-glmark2-scenes.sh --commit "$SHA" --conf CONFIG_GLMARK2_GC_TILED_RT=y <outdir> terrain
# the same two lines for shadow, then for refract
```

Each call runs `tools/board.sh x2 run <image> <logdir> <name>` once a run
(`x2-glmark2-scenes.sh:165-176`; usage `tools/board.sh:10`), with `SEL` set to
the page selector. The gl_test boots, one case each, the image of its first
case:

```sh
env -u NETBOOT_NO_WATCHDOG -u BOARD_UART SEL=3  tools/board.sh x2 run <gt0-off image>   <logdir> gt-case3-off
env -u NETBOOT_NO_WATCHDOG -u BOARD_UART SEL=3  tools/board.sh x2 run <gt0-on image>    <logdir> gt-case3-on
# SEL=44 on the gt0 images (gt-case44-off, -on); SEL=25 on the gt144 images (gt-case169-off, -on)
```

`tools/board.sh x2 stage <image> <logdir>` goes as far as the jump and never
calls it (`tools/board.sh:45-49`): use it to check an image and the board
again without a jump.

### The words to quote

- **The verdict line**, verbatim, one per run (`tools/board.sh:96-99`, `:895`):
  `board.sh x2 <name> rc=<rc> verdict=<v> boot=kexec-dev DEV boot_verdict=<bv> result=<...> resets=<n> record=<w0,w1,w2> back_after=<s>s`.
  A kexec-dev run says `DEV` and is never a row (`tools/board.sh:23-28`).
  `verdict=done` is the only pass; `done_with_resets` is not clean
  (`tools/board.sh:101-107`).
- **The record words**: the `record=` field, three words on the X2, w3 is not
  stored (`tools/board.sh:1128`). For a PERF page, quote the words and the
  decoded `v0..v3` (`yakogl-tools record --words 3`,
  `x2-glmark2-scenes.sh:185-193`): page 0's `v1` is the frame in us and gives
  `ms/frame` (`:25`); pages 1, 3 and 4 give the swap and kicks, the GPU and
  the CPU fallbacks (`driver/samples/zephyr/glmark2/src/gm_perf_page.h:17-34`).
- **gl_test**: the CASE record of cases 3, 44 and 169 (status, first bad check,
  got, want; `imx8mp-gltest-build.sh:8-9`).
- **The arm is the console's**: the on images print `glmark2: gc_tiled_rt=1 ...`
  (`main.cpp:1472`) or `gc_gl_test: gc_tiled_rt=1 ...` (`main_imx8mp.c:477`).
  An "on" row without its line is not an on row; an off row has neither.
- The `resets=` field is quoted as it is, never folded into a pass.

### The records

`<logdir>` is `records/x2/`. Each run writes
`records/x2/<YYYYMMDD-HHMM>-<name>-<sha8>/` with `meta.txt`, `console.txt`,
`INDEX.txt` and the backend's files (`tools/board.sh:118-124`), gitignored
(`.gitignore:9-10`); the copies are redacted (IPv4 to 192.0.2.n, the board's
host name to `<board-host>`, `tools/board.sh:124-125`). A note quotes
the verdict line and the words only. The `*.out` and `table.tsv` of
`x2-glmark2-scenes.sh` are read before anything of them is quoted: no address,
UART path or credential.

### Green, and a picture match

- **Green**, per arm: every run ends `verdict=done` with `resets=0`; the marker
  and (on) the knob line are in the console; the glmark2 table has an
  `ms/frame` for each scene; gl_test cases 3, 44 and 169 each end with a CASE
  record that passes; the on arm's page 4 fallbacks are no worse than off's.
  Session A is green when both arms are. A cost row is not a verdict: the
  criterion does not judge speed, and W6 is not decided here (it needs green
  and a match, and is ordered after plan #351's board run).
- **A picture match**: the on arm's picture equals the off arm's for terrain,
  shadow and refract. The compared value is the scene's picture value
  (five probes, crc16 of a 40x30 grid, the crc of the channels `>> 4`, the
  mean colour, `black`; `main.cpp:231-246`), which a console prints as the
  `glmark2-picture:` line (`tools/board.sh:167-168`, `main.cpp:87-89`).
  Quote it for both arms from the X2 console record; `target:x2-display`
  corroborates by eye and a picture seen without a value is written as such.
  Equal crc16 is a match; a different one is a match only inside the picture
  value's tolerance (`driver/docs/glmark2.md`, "The picture value"), the
  quantised crc and the mean decide. `black`, or a difference above it, or a
  case that passes in one arm only, is a mismatch: the knob stays off and the
  row says so.

### The stop rules

- A run that does not come back (`board.sh` rc 5 or 6, `verdict=not_back`): the
  whole session stops, no retry, the board is released and the report says it
  needs a person (`x2-glmark2-scenes.sh:35-38`, `:218-221`; `tools/board.sh:112-114`).
  The runner's limit is the limit (1500 s in all for x2 kexec-dev,
  `tools/board.sh:206-209`): it is not raised and `NETBOOT_NO_WATCHDOG=1`
  is not set.
- A refused run (rc 3): stop, nothing retried (`x2-glmark2-scenes.sh:36-37`);
  the cause is reported.
- A hang or `failed`/`died_after` verdict of an on arm run: that is the
  finding. No retry and no further on run in the session; the off rows taken
  stay valid. A fence timeout is the hang's own sign (`gc-ts-probe.md:91-96`).
- Any other verdict that is not `done` (rc 4) is a row, and the next run goes
  on (`x2-glmark2-scenes.sh:37-38`).
- The frame is `gc-ts-probe.md:78-99`: the supervisor feeds the self-armed
  WDOG1 and resets the SoC, so a hang is a verdict, never something to wait out.
- At every stop the board is left as found, `tools/board.sh x2 status` is run
  once more and quoted, and nothing goes into `tools/cts/RESULTS.md`,
  `STATUS.md` or `README.md`.

### The rows to fill

X2 numbers are development measurements (`x2-glmark2-scenes.sh:25-27`). With
`SHA`, the verdict lines and the record words:

- `docs/notes/x2-glmark2-slow-scenes.md`: a row per scene and arm (ms a frame,
  the pages 1, 3 and 4 words, the picture line, the verdict line) and a row
  per gl_test case and arm with its CASE record. The "What is left" section
  there is replaced by them.
- `docs/notes/gc-slow-terrain.md` and `docs/notes/gc-slow-shadow-refract.md`:
  the off/on ms a frame for their scenes, beside the prediction, which is
  compared and not edited. Terrain's row is W5's "0 to 14 ms".
- This note: "What this note does not settle" loses the bullets session A
  answers (the RS rate for 800x600, the RA's early depth with a TILED colour
  under SINGLE_BUFFER(2), how many of terrain's fragments early depth
  rejects), each with its row and commit.
- The report gives the comment texts for #1204, #1106 and #15, and the title
  and text of a new issue for each new defect. Agents never run `gh`.

## What this note does not settle

- The RS rate for an 800x600 tiled→linear copy, and so W4's cost. Session A
  measures it.
- Whether the RA's early depth agrees with a super-tiled depth surface when
  the colour is TILED under SINGLE_BUFFER(2). Mesa emits that combination only
  for 128-bit targets. Session A answers it.
- TS with the MMU off (gt-tiled-probe (iii)), and the compression bits Mesa's
  shim row writes (W8, "Compression, an open point").
- The depth layout under SINGLE_BUFFER(2) for depth textures (gt-tiled-probe
  (ii)), and whether the CPU resolve survives depth-sample-design.
- How many of terrain's fragments early depth rejects. That depends on
  `make_grid`'s draw order (`gc-tiled-targets.md` step 5). The off/on pair of
  session A measures it.
