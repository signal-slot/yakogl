<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: glmark2 terrain and refract and the PowerVR render pool

Plan item baseline3-am62p, 2026-10-17, written at main `c854e0ae`. Host
investigation only: no code changed.

## Summary

- On the board's 64 MiB carve-out, terrain and refract do not fit. This is a
  real shortfall in total memory, not fragmentation and not a leak. About
  25224 KiB (24.6 MiB) of the 59200 KiB (57.8 MiB) render pool is taken
  before the first scene, so a scene has 33976 KiB (33.2 MiB). NPOT mip
  chains are laid out at their power-of-two extent, so terrain needs about
  37.7 MiB and refract about 36.6 MiB, plus the 6 MiB more that the board
  held at refract.
- The 21848 KiB request that `am62p-common-int.md` puts in shadow is
  **refract's** 1600x1200 colour mip chain (entry 24). Shadow allocates no
  such texture. Refract's framebuffer goes incomplete because the refused
  chain grow had already released the texture's level-0 storage.
- Main already has the fix: `504d4fae` gives the glmark2 image the dEQP
  image's 256 MiB carve-out, and the board run at `e3749a25` shows 0
  `render pool exhausted`, 0 incomplete framebuffers and 0 GL errors.
  This document explains why that works and gives a host test that pins it.

## Sources and method

- Board records, read with `git show ab43a20c5:<path>` (removed from the
  tree on 2026-10-16, `docs/README.md`):
  - `docs/board-sessions/am62p-common-int.md` (cd738b1a, 64 MiB): lines
    132, 140, 144, 154, 170;
  - `tools/cts/log/benchmarks-baseline/am62p.md` (f8741799, 64 MiB): line
    90, the same two refusals with timestamps and entry numbers;
  - `docs/board-sessions/am62p-common-carveout.md` (e3749a25, 256 MiB):
    lines 183, 191.
- The code at `c854e0ae` (file:line below) and glmark2 at `22c527cb`
  (`$CCFARM_DEPS_DIR/glmark2-src`).
- A **scratch replay** through the real pool code
  (`driver/rust/src/gpu/pvr/pool.rs`, `mem.rs`). It is the existing
  `pool/tests/replay.rs` sequence with three changes: the freelists sized
  by carve-out as `driver/src/pvr_render.c:632-675` sizes them, terrain's
  two 800x600 targets grown to their POT-extent chains after its first
  draw, and refract's colour texture grown to its chain after its depth
  buffer. It ran as an untracked module next to `replay.rs` with
  `cargo test --manifest-path driver/rust/Cargo.toml scratch_npot -- --nocapture`
  and was deleted afterwards. Its design is the host test in the last
  section. "Model" below means this replay.
- The glmark2 host harness (`tools/hostharness/glmark2/run.sh`) did **not**
  run: this session was not allowed to start it. It also does not model
  the PowerVR pool (its README, "How it works"). The conclusions that
  still need it are listed at the end.

How well the model matches the board:

| Point | Board | Model | Difference |
|---|---|---|---|
| live at jellyfish's end, 64 MiB | "about 38 MB" (am62p-common-int.md line 170) | 36800 KiB (37.7 MB, 35.9 MiB) | about 0 |
| live at the run's end, 64 MiB | 26910720 B = 26280 KiB, frees held 96 KiB (benchmarks-baseline/am62p.md line 90) | 25968 KiB | -312 KiB |
| gap at refract's chain request, 64 MiB | 2744 KiB | 3676 KiB (with the 24 "unnamed" blocks, driver/rust/src/gpu/pvr/pool/tests/replay.rs:395) | +932 KiB |
| gap at terrain's chain request, 64 MiB | 3648 KiB, one of two chains served | 1816 KiB, both refused | model holds about 3.6 MiB too much in terrain |
| bump at the end, 256 MiB | 91356 KiB | 90544 KiB | -812 KiB |
| live at refract's end (its frees still held), 256 MiB | 142553088 B = 139212 KiB | 139356 KiB (refract's peak) | +144 KiB |

## 1. What is live at the end of jellyfish, and why

`osal_mem_line` defines "live" as the bytes the pool handed out (bump plus
top region) minus the bytes on its free lists (`src/osal/zephyr.c:303-313`).
Frees the OSAL still holds for a fence count as live. glmark2 prints the
"end" line after the scene's teardown (glmark2 `src/main-loop.cpp` line 116 tears down,
line 121 calls `log_scene_result`; ours prints at `driver/samples/zephyr/glmark2/src/main.cpp:494-498`).

The "about 38 MB" at jellyfish's end is two parts:

| Part | KiB | Where it comes from |
|---|---|---|
| Driver and back end, kept for the run | 24780 | render statics with the 2 MiB parameter-buffer reserve 2148 (`driver/src/pvr_render.c:618-620`); freelists 12440: two global lists of 1536 pages (448 + 64 ready + 4 x 256 grow reserve, `driver/src/pvr_render.c:636-637`, `driver/rust/src/gpu/pvr/freelist.rs:112-115`) + the local list + PM arrays; render context 4; 4 frame sets 7488 (4096 of it the top region's 1 MiB arenas); back-end statics and index buffer 280; GL leftovers 520; the window's 800x608 depth buffer 1900 |
| HWRT members, kept | 444 | one set per target size seen so far (driver/rust/src/gpu/pvr/pool/tests/replay.rs:166-181) |
| **Persistent total** | **25224** | model, before jellyfish |
| jellyfish's own 45 frees, held | 11576 | deferred to the fence of the kick being recorded (`src/osal/zephyr.c:157-162`); the next completed kick, or the reclaim at the first refusal (`src/osal/zephyr.c:139-146`), gives them back |
| **Live at jellyfish's end** | **36800** | model |

So:

- **No leak.** At the run's end live is back to 26280 KiB with 96 KiB held
  (f8741799). The model's persistent set at the end, with every HWRT
  member of the run, is 25968 KiB. No texture or renderbuffer storage
  outlives its scene.
- **Freed storage is coalesced and returned.** A free merges with both
  neighbours (`driver/rust/src/gpu/pvr/pool.rs:490-514`). A top-region range at the floor goes back
  to the gap at once (`driver/rust/src/gpu/pvr/pool.rs:520-525`, `driver/rust/src/gpu/pvr/mem.rs:248-255`). At the end of
  the 256 MiB run the GENERAL lists hold 1316 KiB in 9 ranges after 5823
  frees of 2326320 KiB (am62p-common-carveout.md line 184).
- **glmark2 keeps nothing across scenes.** What looks live between scenes
  is the driver's persistent set plus one scene's frees waiting for their
  fence.

Not yet read on the board: the `frees held N (K KiB)` field of the
`glmark2-mem: entry 21 ... end` line, which gives the 11576 KiB directly.

## 2. Fragmentation or a real total

**A real total.** The pool is 59200 KiB at 64 MiB (am62p-common-int.md line 153).
Take away the persistent 25224 KiB and about 33976 KiB (33.2 MiB) is left
for a scene.

`render pool exhausted: R KiB requested, L KiB left` is the bump's line.
`L` is the gap between the bump and the top region's floor
(`driver/rust/src/gpu/pvr/mem.rs:197-205`). It is reached only after a best fit over both regions'
lists and the top-region allocation have failed (`driver/rust/src/gpu/pvr/pool.rs:400-422`).

| Request (board) | Gap (board) | Model at the request | Can any layout serve it? |
|---|---|---|---|
| terrain, 5464 KiB (800x600 chain) | 3648 KiB | lists 728 KiB, largest 684; gap 1816 | No. Terrain's peak with both chains is 63832 KiB live, more than the whole 59200 KiB pool (before NPOT chains it was 56656 KiB and fitted) |
| refract, 21848 KiB (1600x1200 chain) | 2744 KiB | lists 7736 KiB, largest 7500 (its own released level 0); gap 3676 | No. Lists + gap = 11412 KiB, about half of the request |

The per-entry live high-water from the model (KiB; 64 MiB with the board's
24 extra blocks from shadow on, driver/rust/src/gpu/pvr/pool/tests/replay.rs:395):

| Entry | 64 MiB, before NPOT chains | 64 MiB, POT-extent chains (as cd738b1a) | 96 MiB | 256 MiB |
|---|---|---|---|---|
| 21 jellyfish | 36800 | 36800 | 36800 | 106560 |
| 22 terrain | 56656 | refused | 63832 | 133592 |
| 23 shadow | 47636 | 47636 | 47636 | 117396 |
| 24 refract | 55248 | refused | 69596 | 139356 |
| pool | 59200 | 59200 | 91968 | 255808 |

Fragmentation matters only in a narrow window. At 80 MiB the model refuses
refract's chain with a 20060 KiB gap and 7736 KiB on the lists (largest
7500 KiB): 27796 KiB, but in two pieces. The 7500 KiB piece is refract's
level-0 colour storage, released by the grow; it lies in the top region
between the depth texture and the depth buffer, so it cannot join the gap. From
96 MiB on, everything fits.

On the board the ERR summary printed right after each refusal
(`driver/rust/src/gpu/pvr/pool.rs:426`, the `render pool general: free ... largest ...` line) shows
the real list state. These lines are in the cd738b1a and f8741799
records, which are not on this machine. They confirm the "lists" column on
the board.

## 3. Shadow's 21848 KiB and refract's incomplete framebuffer

**The 21848 KiB is refract's, not shadow's.**

- 1600x1200 RGBA with mipmaps is laid out at its POT extent, 2048x2048
  (`src/gl/texture.rs:1534-1538` `pot_chain_layout`, after Mesa pvr's
  rule quoted at `src/gl/texture.rs:1507-1533`). 4 x (2048² + 1024² + ... + 1) =
  22369620 bytes, 5462 pages = **21848 KiB**. That is the board's line
  `texture 1600x1200 x1 fmt 0 (11 levels, 22369620 bytes): out of GPU memory`.
- Shadow makes one 1600x1200 `GL_DEPTH_COMPONENT` texture with
  `GL_NEAREST` and no mipmaps (glmark2 `scene-shadow.cpp lines 85-108`). It is
  served at 64 MiB: no refusal in entry 23 in any run.
- Refract makes a 1600x1200 depth texture and a 1600x1200 RGBA texture
  with `GL_LINEAR_MIPMAP_LINEAR` (`scene-refract.cpp lines 177-208`). It calls
  `glGenerateMipmap` on it every frame (`scene-refract.cpp lines 429-430`).
- f8741799's record dates the refusal to entry 24
  (benchmarks-baseline/am62p.md line 90: "21848 KiB requested, 2744 KiB left
  (00:33:20, entry 24)"). am62p-common-int.md lines 139-142 put it under shadow:
  refract's first FPS line has no `[scene]` prefix on that console, so the
  line was read as part of the scene before.

**Why refract's framebuffer goes incomplete:**

1. The first `glGenerateMipmap` on the NPOT colour texture grows its
   one-level storage to the chain (`src/gl/texture.rs:3765-3781`
   `grow_npot_chain`, then `relayout_keeping_levels`, `src/gl/texture.rs:1091`).
2. `texture_alloc_laid_out` first **releases** the old storage
   (`src/gl/texture.rs:1598`), then asks for 22369620 bytes (`src/gl/texture.rs:1602`).
3. The pool refuses: `render pool exhausted`, then `out of GPU memory`
   (`src/gl/texture.rs:1605`) and `GL_OUT_OF_MEMORY`. The texture now has no
   storage at all, not even its level 0.
4. `attachment_storage` returns `None` for a texture whose `mem.impl_` is
   null (`src/gl/framebuffer.rs:430`). The framebuffer is
   `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` (0x8cd6), and each of refract's
   two binds of fb 1 per frame logs it: 1694 lines = 2 x 847 frames.
   Refract's draws into it are skipped, so the 85 fps at cd738b1a was a
   scene that drew nothing.

Terrain's `framebuffer 7 incomplete` and `0x502 in glGenerateMipmap` lines
are the same path for one of its two mipmapped 800x600 targets
(terrain_renderer and tilt_h_renderer, glmark2 `scene-terrain.cpp lines 82,112`,
mipmaps generated each frame by `scene-terrain/base-renderer.cpp lines 84-90`).

## 4. Which fixes let all 33 scenes fit, and their cost

| Fix | Fits at 64 MiB? | Cost and notes |
|---|---|---|
| Free on delete or orphan | no gain | Already done: frees wait only for their fence, and the reclaim gives them back at a refusal (`src/osal/zephyr.c:139-146`). Live returns to the persistent set after each scene |
| Coalescing | no gain | Already done (`driver/rust/src/gpu/pvr/pool.rs:490-525`); the lists hold under 1316 KiB |
| Allocation order (top region, 256 KiB threshold, `driver/src/pvr_fw_boot.c:181-186,210`) | no | Already on. It fixes the fragmentation window around 80 MiB, not the 64 MiB total. Growing the chain before releasing level 0 would add 7500 KiB at refract's peak: worse |
| GL-sized NPOT chains (no POT padding) | no | Saves 2.9 MiB per terrain chain and 11.6 MiB for refract (2559756 and 10239756 bytes). Still refused at 64 MiB (model). It also changes the layout the texture unit reads (Mesa's rule, `src/gl/texture.rs:1507-1533`): a hardware risk for no fit |
| A smaller persistent set at 64 MiB | no | It needs 4632 KiB (terrain) to 10436 KiB (refract) cut from the 25224 KiB. The only large part is the freelists' 4-grow reserve (8 MiB), which guards against a refused freelist grow (#3) |
| General-heap fallback for big textures | yes, in principle | New code in `mem.rs`, `pool.rs` and the Zephyr OSAL: the GPU MMU would map pages from outside the reserved, no-map carve-out with the same cache rules. The largest change and a new failure surface |
| **A bigger carve-out** | yes, from 96 MiB (model) | DRAM reserved for the GPU. At 128 MiB or more the render service also takes a quarter of the carve-out as the freelist grow pool (`driver/src/pvr_render.c:632-650`), so the persistent set jumps from 25224 KiB (120 MiB) to 62152 KiB (128 MiB) and 94984 KiB (256 MiB) |

## Recommended fix

**Keep the 256 MiB carve-out of the glmark2 image (`504d4fae`). It is on
main, and the board run at `e3749a25` proves it.** No driver change is
needed for the fit.

Files (already changed by `504d4fae`):

- `driver/samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.overlay:39-57`:
  `gpu-carveout@a0000000`, `reg = <0xa0000000 0x10000000>`, the GPU's
  `memory-region`.
- `driver/samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.conf:19-23`:
  `CONFIG_MAX_XLAT_TABLES=128`, `CONFIG_KERNEL_VM_SIZE=0x20000000`.

Why 256 MiB and not the 96 MiB the model says is enough:

- It is the dEQP image's carve-out. That layout ran the whole dEQP-GLES2
  tree with 0 resets, so the two images share one tested layout.
- The board's own peak is 148072 KiB handed out (bump 91356 + top 56716
  KiB high-water, am62p-common-carveout.md line 183) of 255808 KiB: 107736 KiB
  headroom. That covers the model's error of a few MiB many times over.
  At 96 MiB the headroom would be 22372 KiB, and terrain's list is a
  reconstruction.

The 64 MiB of grow pool that 256 MiB brings stays unused by glmark2 (0
grow requests at e3749a25). Only shrink the carve-out with the test below
in place.

## Host test design for pvr-pool-npot

Put it beside `driver/rust/src/gpu/pvr/pool/tests/replay.rs` (it needs the
private `Rig`, `Replay`, `before_jellyfish`, `TERRAIN`, `SHADOW`,
`REFRACT_SETUP`), or in `tests/pvr_pool*.rs` if that item exposes them.

Pieces:

1. `Cfg::for_carveout(bytes)`: the freelist sizes of
   `driver/src/pvr_render.c:632-675`. Below 128 MiB: 2 x (448 + 64 + 4 x 256) pages,
   PM arrays 4096 x 4 bytes. From 128 MiB: a grow pool of
   `(bytes / 4 / 4096) & !1023` pages, then 2 x 2048 pages, PM arrays
   `(2048 + pool) x 4` bytes. The local list is 26 pages. The pool offset is
   `POOL_AM62P` (6336 KiB) at every size.
2. `boot(cfg)`: `Replay::boot` with those freelists (grow pool first,
   16 KiB aligned, as `driver/rust/src/gpu/pvr/freelist.rs:68-79`).
3. The chains as the GL core makes them: `CHAIN_800 = 5_592_404` and
   `CHAIN_1600 = 22_369_620` (the board's byte counts, from
   `pot_chain_layout`). A `grow(scene, old, new)` that frees the level-0
   block first, then allocates, as `src/gl/texture.rs:1598-1602` does.
4. Terrain: `TERRAIN`, then both 800x600 targets grown to `CHAIN_800`.
   Shadow: `SHADOW` plus `UNNAMED_AT_THE_BOARD` 256 KiB blocks kept from
   there on. Refract: `REFRACT_SETUP`, the 1600x1216 depth buffer, then
   the colour texture grown to `CHAIN_1600`.
5. `live()`: (bump + top) minus every heap's list bytes, as
   `osal_mem_line`. Record its high-water per entry.

Tests:

- `glmark2_npot_chains_at_64_mib_are_refused_for_want_of_memory`: refused
  is exactly terrain's chain(s) and refract's `CHAIN_1600`. At each refusal,
  list bytes + gap < the request, so it is not fragmentation. The pin of
  cd738b1a.
- `glmark2_before_npot_chains_fits_64_mib`: the same run with level-0-only
  targets refuses nothing (the baseline the NPOT change broke).
- `glmark2_fits_the_glmark2_images_carveout`: read the size from
  `verdin_am62p_am62p54_a53.overlay` (`include_str!`, the `reg` of
  `gpu_carveout_glmark2`). Nothing is refused, and every entry's live
  high-water is under 3/4 of the pool (the rule the GC7000's glmark2 row
  already uses). Expected now: 139356 of 255808 KiB at refract. This test
  fails if someone shrinks the carve-out below what glmark2 needs.
- `the_replay_matches_the_256_mib_board_run`: end bump within 1 MiB of
  91356 KiB (model without the extra blocks: 90544), refract's peak within
  1 MiB of 139212 KiB (model: 139356). This keeps the reconstruction honest.
- Optional: `the_freelist_grow_pool_takes_a_quarter_from_128_mib`: the
  persistent set at 128 MiB is about 36928 KiB above that at 120 MiB.

## What still needs a harness run or a board record

- **Terrain's real allocation list.** The model holds about 3.6 MiB more
  than the board at terrain: the board served one of the two chains, the
  model refuses both. Run `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC"
  <out> powervr` and read each entry's `osal(linux): heap 0x...: ... at
  most` lines; they are the GL core's and back end's per-scene high-water.
  The harness has no PowerVR pool, so the driver's persistent set must
  still be added from the table in section 1.
- **The board's list state at each refusal**: the ERR
  `render pool general: free ... largest ...` line after each
  `render pool exhausted` in the cd738b1a and f8741799 records.
- **The jellyfish "frees held" field**: the `glmark2-mem: entry 21 ... end`
  line of either 64 MiB record.

None of these can change the answers above. The 64 MiB shortfall is
4632 KiB for terrain even with perfect packing, and 10436 KiB for refract.
