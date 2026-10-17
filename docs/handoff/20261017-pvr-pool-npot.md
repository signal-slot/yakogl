<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pvr-pool-npot: the AM62P pool replay with the NPOT mip chains, 2026-10-17, item pvr-pool-npot

## Open work

- Under item 1, the AM62P glmark2 bullets: replace the am62p-render-pool
  note's "pin the 256 MiB fix on the host (pvr-pool-npot)" bullet with
  - **AM62P glmark2 render pool: the 256 MiB fix is pinned on the host**
    (pvr-pool-npot). `driver/rust/src/gpu/pvr/pool/tests/replay.rs`
    (docs/notes/pvr-render-pool.md "The replay with the NPOT mip chains")
    sizes terrain's two mipmapped 800x600 targets at 5462 KiB each and
    refract's mipmapped 1600x1200 colour texture at 21846 KiB, the chains of
    their power-of-two extents, as the GL core asks the pool for them
    (`tests/pvr_pool_npot.rs`: level 0 released first, then the chain), with
    the freelists sized per carve-out as `driver/src/pvr_render.c` sizes
    them and the carve-out read from the glmark2 overlay. At 64 MiB, with
    the top region, refract's three blocks (22600 KiB) still fit after
    shadow and terrain at every chain count; refract's own chain does not:
    10056 KiB are free in either arm for a 21846 KiB request (capacity,
    not placement), and terrain's set-up is short from one chain on. At
    the overlay's 256 MiB nothing is refused in either arm, every scene's
    live high-water is under 3/4 of the 255808 KiB pool (refract 139356
    KiB), and the replay matches the e3749a25 board run (end bump 90544
    against 91356 KiB, refract's peak 139356 against 139212). Shrinking the
    overlay's carve-out below what glmark2 needs fails
    `the_glmark2_images_carveout_serves_terrains_and_refracts_chains`. The
    pool's threshold and placement are unchanged.
- The baseline3-am62p investigation moved to
  `docs/notes/am62p-render-pool-glmark2.md`, as the am62p-render-pool note
  asked once the replay cites it; that note's citations were updated with
  it.

## Known defects

- The am62p-render-pool note's "a refused NPOT chain grow leaves the
  texture with no storage" is modelled by the replay's `Regrow` (level 0
  freed, then the chain asked for) and pinned as the real order by
  `tests/pvr_pool_npot.rs`; on the 64 MiB carve-out the replay refuses
  refract's chain in both arms, which is the 1694 `framebuffer 1
  incomplete` lines of cd738b1a.

## Next board session

- AM62P glmark2 (any image on the 64 MiB region, e.g. a gl_test-style
  overlay): expect "render pool exhausted: 21848 KiB requested" at refract's
  first frame, "texture 1600x1200 x1 fmt 0 (11 levels, 22369620 bytes):
  out of GPU memory" and `framebuffer 1 incomplete` each frame; on the
  glmark2 image (256 MiB) expect neither, and the `render pool summary`
  line's bump within 1 MiB of 91356 KiB.
