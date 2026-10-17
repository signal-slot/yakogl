<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pvr-pool-npot: the AM62P pool replay with the NPOT mip chains, 2026-10-17, item pvr-pool-npot

## Open work

- Under item 1, the AM62P glmark2 bullets: add
  - **AM62P: the 64 MiB carveout cannot hold refract since pvr-npot-mips.**
    The host replay (`driver/rust/src/gpu/pvr/pool/tests/replay.rs`,
    docs/notes/pvr-render-pool.md "The replay with the NPOT mip chains")
    sizes terrain's two mipmapped 800x600 targets at 5462 KiB each and
    refract's mipmapped 1600x1200 colour texture at 21846 KiB (the chains of
    their power-of-two extents, what the GL core asks the pool for:
    `tests/pvr_pool_npot.rs`). With the top region, refract's three blocks
    (22600 KiB) still fit after shadow and terrain at every chain count;
    refract's own chain does not: 10056 KiB are free in either arm for a
    21846 KiB request (capacity, not placement), and terrain's set-up is
    short from one chain on. On the glmark2 image's 256 MiB carveout
    (504d4fae, the image every AM62P row has run since) everything is
    served in both arms. So the 256 MiB overlay is a requirement of the
    glmark2 image, not a fallback; an image on the board's 64 MiB region
    runs terrain and refract with GL_OUT_OF_MEMORY from `glGenerateMipmap`
    (the texture stays one level under a mipmap filter: sampled black).
    The pool's threshold and placement are unchanged.

## Next board session

- AM62P glmark2 (any image on the 64 MiB region, e.g. a gl_test-style
  overlay): expect "render pool exhausted: 21848 KiB requested" at refract's
  first frame and "texture 1600x1200 x1 fmt 0 (11 levels, 22369620 bytes):
  out of GPU memory" each frame; on the glmark2 image (256 MiB) expect
  neither. The replay's high-water with the region is at least 22600 +
  21846 + 4096 KiB at refract.
