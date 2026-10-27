<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: glmark2 terrain and refract and the PowerVR render pool

Plan item baseline3-am62p, 2026-10-17, written at main `c854e0ae`. Host
investigation only: no code changed. Moved from `docs/investigations/` to
`docs/notes/` by plan item pvr-pool-npot (2026-10-17), which built the host
test the last section designs; that section now describes the test as
built (`driver/rust/src/gpu/pvr/pool/tests/replay/carveout.rs`).

## Summary

- On the board's 64 MiB carve-out, terrain and refract do not fit. This is a
  real shortfall in total memory, not fragmentation and not a leak. About
  25224 KiB (24.6 MiB) of the 59200 KiB (57.8 MiB) render pool is taken
  before the first scene, so a scene has 33976 KiB (33.2 MiB). NPOT mip
  chains are laid out at their power-of-two extent, so terrain needs 38732
  KiB (37.8 MiB) and refract 45160 KiB (44.1 MiB): the glmark2 host
  harness's per-scene high-water on the PowerVR build.
- The 21848 KiB request that `am62p-common-int.md` puts in shadow is
  **refract's** 1600x1200 colour mip chain (entry 24). Shadow allocates no
  such texture. Refract's framebuffer goes incomplete because the refused
  chain grow had already released the texture's level-0 storage.
- Main already has the fix: `504d4fae` gives the glmark2 image the dEQP
  image's 256 MiB carve-out, and the board run at `e3749a25` shows 0
  `render pool exhausted`, 0 incomplete framebuffers and 0 GL errors.
  This document explains why that works and gives the host test that pins
  it (`driver/rust/src/gpu/pvr/pool/tests/replay/carveout.rs`, last
  section).

## Sources and method

- Board records, read with `git show ab43a20c5:<path>` (removed from the
  tree on 2026-10-16, `docs/README.md`):
  - `ab43a20c5:docs/board-sessions/am62p-common-int.md` (cd738b1a, 64 MiB): lines
    132, 140, 144, 154, 170;
  - `tools/cts/log/benchmarks-baseline/am62p.md` (f8741799, 64 MiB): line
    90, the same two refusals with timestamps and entry numbers;
  - `ab43a20c5:docs/board-sessions/am62p-common-carveout.md` (e3749a25, 256 MiB):
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
  and was deleted afterwards. The host test in the last section is its
  committed form and reproduces its numbers. "Model" below means this
  replay.
- The glmark2 host harness on the PowerVR build:
  `tools/hostharness/glmark2/run.sh "$GLMARK2_SRC" build/gm2-pool powervr`
  (glmark2 checked out at `22c527cb`, 3 frames per entry, one process per
  entry; the generated `FINDINGS.md` was put back). It has no PowerVR
  render pool (its README, "How it works"), but `src/osal/linux.c:194-205`
  prints each heap's high-water at exit. That is the GL core's and the
  back end's memory per scene; the render service's own blocks
  (freelists, statics, HWRT) are not in it. Section "Harness" below.

How well the model matches the board:

| Point | Board | Model | Difference |
|---|---|---|---|
| live at jellyfish's end, 64 MiB | "about 38 MB" (am62p-common-int.md line 170) | 36800 KiB (37.7 MB, 35.9 MiB) | about 0 |
| live at the run's end, 64 MiB | 26910720 B = 26280 KiB, frees held 96 KiB (benchmarks-baseline/am62p.md line 90) | 25968 KiB | -312 KiB |
| gap at refract's chain request, 64 MiB | 2744 KiB | 3676 KiB (with the 24 "unnamed" blocks, driver/rust/src/gpu/pvr/pool/tests/replay.rs:395) | +932 KiB |
| gap at terrain's chain request, 64 MiB | 3648 KiB, one of two chains served | 1816 KiB, both refused | model holds about 3.6 MiB too much in terrain |
| bump at the end, 256 MiB | 91356 KiB | 90544 KiB | -812 KiB |
| live at refract's end (its frees still held), 256 MiB | 142553088 B = 139212 KiB | 139356 KiB (refract's peak) | +144 KiB |

## Harness: per-entry GL and back-end high-water

KiB, from each entry's `osal(linux): heap ...: U bytes used at close, H at
most` lines (GENERAL `0x8000000000`, PDS `0xda00000000`, USC
`0xe000000000`). "Scene" is the sum of the heaps' highs minus what every
entry holds at close (the back end's frame sets, statics and the window
depth buffer: GENERAL about 6300 + PDS 3088 + USC 168). 0 GL errors, 0
refused draws in every entry.

| Entry | GENERAL max | all heaps max | Scene | Board pool live at 64 MiB (25224 + scene) | at 256 MiB (94984 + scene) |
|---|---|---|---|---|---|
| 0-20 (build ... ideas) | 8220-14344 | 11480-17672 | 1900-8100 | at most 33400 | at most 103100 |
| 21 jellyfish | 19740 | 23016 | 13468 | 38692 | 108452 |
| 22 terrain | 45064 | 48404 | 38732 | **63956**, over the 59200 pool | 133716 |
| 23 shadow | 23900 | 27172 | 17580 | 42804 | 112564 |
| 24 refract | 51568 | 54832 | 45160 | **70384**, over the 59200 pool | 140144 (board: 139212) |
| 25-32 (conditionals ... clear) | 8292 | 11548 | about 2000 | | |

The harness agrees with the replay on terrain (scene 38732 against the
replay's 38608 KiB) and on shadow (17580 against 16268). For refract it
measures 45160 KiB, 7700 KiB more than the replay's named blocks (37460):
that is the "unnamed" 6 MiB the board showed at refract
(`driver/rust/src/gpu/pvr/pool/tests/replay.rs:384-395`), now measured. With it, the
predicted live at refract on the 256 MiB board is 140144 KiB against the
board's 139212 KiB.

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
| refract, 21848 KiB (1600x1200 chain) | 2744 KiB | lists 7736 KiB, largest 7500 (its own released level 0); gap 3676 | No. Lists + gap = 11412 KiB, about half of the request; the harness puts refract's peak 11184 KiB over the pool |

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
| A smaller persistent set at 64 MiB | no | It needs 4756 KiB (terrain) to 11184 KiB (refract) cut from the 25224 KiB (harness figures). The only large part is the freelists' 4-grow reserve (8 MiB), which guards against a refused freelist grow (#3) |
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
  At 96 MiB the headroom would be about 21600 KiB (91968 - 70384), close
  to the error between the board and the host at terrain (below).

The 64 MiB of grow pool that 256 MiB brings stays unused by glmark2 (0
grow requests at e3749a25). Only shrink the carve-out with the test below
in place.

## Host test (pvr-pool-npot)

`driver/rust/src/gpu/pvr/pool/tests/replay/carveout.rs`, a child module of
`pool/tests/replay.rs` (it uses the replay's private `Replay`, `Rig`,
`before_jellyfish`, `TERRAIN`, `SHADOW`, `REFRACT_SETUP`). It runs in the
`tools/check.sh` row `test    pvr render pool (Rust; the C pool's golden
trace replayed, 33 tests)`, which asserts the count, and in the
`test    yakogl-drivers` row. Alone:

```text
cargo test -p yakogl-drivers --all-features gpu::pvr::pool::tests::replay::carveout -- --nocapture
```

What `replay.rs` gained for it (the 64 MiB tests of `replay.rs` are
unchanged and still pass to the byte):

1. `Cfg::for_carveout(bytes)`: the freelist sizes of
   `driver/src/pvr_render.c:632-676`. Below 128 MiB: 2 x (448 + 64 + 4 x 256)
   pages, PFN arrays 4096 x 4 bytes. From 128 MiB: a grow pool of
   `(bytes / 4 / 4096) & !1023` pages first (16 KiB aligned, as
   `freelist::pool_init`), then 2 x 2048 pages, PFN arrays
   `(2048 + pool) x 4` bytes. The local list is 26 pages with a 32-entry
   array (`bo_alloc_pm` rounds every array to 16 KiB). The pool offset is
   `POOL_AM62P` (6336 KiB) at every size, so the pool is 59200 KiB at
   64 MiB and 255808 at 256.
2. `Replay::with_carveout(bytes, top_above)` and `boot_for(&Cfg)`.
3. `Replay::grow(scene, what, chain)`: the texture's level-0 block is freed
   first, then the chain is asked for, as `src/gl/texture.rs:1598-1602`.
   The chains are the board's byte counts: `CHAIN_800` = 5592404 (a
   1024x1024 chain for 800x600) and `CHAIN_1600` = 22369620 (2048x2048
   for 1600x1200).
4. `Replay::live()`: (bump + top region) minus every heap's list bytes, as
   `osal_mem_line`; its high-water is kept per scene (`live_hw`), and
   GENERAL's state is kept at each refusal (`refused_at`).

The sequence (`glmark2_on`): the boot for the carve-out, entries 0-20,
jellyfish, terrain with both mipmapped 800x600 targets grown to `CHAIN_800`
after its first draw, shadow with the 24 unnamed blocks of
`UNNAMED_AT_THE_BOARD` kept from there on, refract with its 1600x1216 depth
buffer and then the colour texture grown to `CHAIN_1600`. The top region is
at the driver's threshold. The design's alternative of the harness's 7700
KiB in refract instead of the 24 unnamed blocks was not taken: the 24
blocks are what the `replay.rs` tests and this note's model are calibrated
on, and the test reproduces the model's numbers exactly (persistent set
25224 KiB; live high-water 36800 / 56656 / 47636 / 55248 at 64 MiB without
chains, 106560 / 133592 / 117396 / 139356 at 256 MiB; the gaps 1816 and
3676 KiB at the refusals).

The tests:

- `glmark2_npot_chains_at_64_mib_are_refused_for_want_of_memory`: the pin
  of cd738b1a. Exactly three refusals: terrain's two chains (5462 KiB each)
  and refract's (21846 KiB). At terrain's first chain the lists and the gap
  hold 2604 + 1816 KiB, at refract's 7736 + 3676: less than the request,
  so not fragmentation. Each scene's peak with its chains in place is more
  than the whole pool (terrain 63832 KiB, refract 69596, from the level-0
  run's peaks plus what the chains add). Terrain's second chain is refused
  with 4480 KiB free (largest 3752) and a 1816 KiB gap, more bytes than it
  asks for in total, only because the first refusal gave the first target's
  level 0 back; the board served one of the two chains. The harness's
  figures put terrain and refract over the pool and shadow under it.
- `glmark2_before_npot_chains_fits_64_mib`: the same run with the NPOT
  textures at their level 0 refuses nothing (the baseline the npot change
  broke).
- `glmark2_fits_the_glmark2_images_carveout`: the carve-out is read from
  `driver/samples/zephyr/glmark2/boards/verdin_am62p_am62p54_a53.overlay`
  (`include_str!`; the `reg` size of `gpu_carveout_glmark2`, which must be
  the GPU's `memory-region`). Nothing is refused and every scene's live
  high-water is under 3/4 of the pool (the GC7000 glmark2 row's rule):
  refract 139356 of 255808 KiB. Checked: with the overlay's `reg` set to
  `0x04000000` (64 MiB) the test fails with the three refusals above; at
  the overlay's `0x10000000` it passes.
- `the_replay_matches_the_256_mib_board_run`: refract's peak within 1 MiB
  of the board's 139212 KiB (model 139356). The end bump within 5 MiB of
  the board's 91356 KiB (model 96176), not the 1 MiB the design asked: the
  24 unnamed blocks are 256 KiB each, not above the top region's
  threshold, so they sit on the bump (6144 KiB of it) where the board's
  6 MiB mostly did not (without them the model's bump is 90032 KiB). Where
  the board put them changes no scene's live bytes. (The table above
  quotes the scratch replay's 90544 KiB for the end bump; the committed
  test's reconstruction gives 96176 with the unnamed blocks and 90032
  without.)
- `the_freelist_grow_pool_takes_a_quarter_from_128_mib`: the persistent set
  is 25224 KiB at 120 MiB, 62152 at 128 (a step of 36928) and 94984 at
  256, the figures of section 4's last row.

## What still needs a board record

- **Terrain on the board.** The harness agrees with the replay at terrain
  (38732 against 38608 KiB), yet the board had about 3.6 MiB more room
  there: it served one of the two chains, where the replay refuses both.
  The difference is on the board side (the persistent set at that moment,
  or when the first chain was asked for), not in terrain's list. It does
  not change the answer: terrain's peak is 4756 KiB over the pool.
- **The board's list state at each refusal**: the ERR
  `render pool general: free ... largest ...` line after each
  `render pool exhausted` in the cd738b1a and f8741799 records.
- **The jellyfish "frees held" field**: the `glmark2-mem: entry 21 ... end`
  line of either 64 MiB record.

None of these can change the answers above. The 64 MiB shortfall is
4756 KiB for terrain even with perfect packing, and 11184 KiB for refract
(harness scene + persistent set - pool).
