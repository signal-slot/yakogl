<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP (GC7000UL): first ES 3.0 run, gl_test 196 and 202-205, whole dEQP-GLES3 tree

Measurement only (issues #28 and #24). Board: the Verdin iMX8M Plus on the
Mallow carrier, Torizon, reached through `BOARD_HOST` (environment only).

## What ran

- origin/main commit the images are built from:
  `cd9ebb5c341494dffe3acfdaa57ad964153880ec`, from a detached worktree of that
  commit (`YAKOGL_SAMPLE_TREE=cd9ebb5c` in the dEQP image's build line).
- Runner: `tools/board.sh` of this commit (`board.sh: the imx8mp kexec route
  checks galcore's holders ...`), which differs from main only in the crash
  route's pre-check and wait; nothing in the images comes from it.
- gl_test image: `tools/cts/imx8mp-gltest-build.sh gt196-cd9ebb5c 196` (the
  sweep configuration from case 196, `CONFIG_GC_BOARD_WDOG_SELF_ARM=y`,
  `CONFIG_YAKOGL_REPORT_ES2` not set). Session 1 sha256 `f1fe9994...`, session 2
  `df31a7802fba791f5107d3a57507fa8fb8058e2dfecde7ccb449531a5f3373f8` (the same
  source; the build stamp differs). Each case runs alone: `SEL=<case - 196>`.
- dEQP-GLES3 table image: gles3 stage 1 (`tools/cts/imx8mp-deqp-stage1.sh
  gles3`, VK-GL-CTS `93bca01`), then `tools/cts/imx8mp-deqp-build.sh --gles3
  es3-cd9ebb5c 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt`
  (133 entries, no `IMX8MP_CONSOLE`: this lease has no console). `zephyr.bin`
  16,447,128 bytes, sha256
  `2a2565966766069bad2ee808a8d8ee1d2a916849bf4de1bc33db89ba91aa157e`, built
  twice (one per session) to the same bytes; `zephyr.elf` sha256 `a6422a1e...`;
  `es3-cd9ebb5c.table.conf` sha256 `5b7a6123...`; `CONFIG_DEQP_REPORT_ES2` and
  `CONFIG_YAKOGL_REPORT_ES2` not set; `check-deqp-images: the dEQP image recipes
  build what conformance needs`.
- Route: `BOARD_BOOT=kexec-dev` (the crash-route kexec from Torizon; the eMMC,
  `PARTITION_CONFIG` and the boot loader are not touched). Every verdict line
  therefore ends `(kexec-dev: a development run, never a row)`. The board's
  demo container holds `/dev/galcore` from every boot (docker-compose starts
  it), and the board's `kexec-zephyr` wrapper stops at `rmmod galcore`: before
  each boot the container was stopped (`docker stop`, never removed; the next
  boot starts it again). The wait reads the board through ssh and its
  `boot_id` (`reach=ssh` in `meta.txt`): the host reaches the board only
  through an ssh forward.
- Tree rows: `TREE_ROWS=<i> NAMES=0 BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt
  TREE_IMAGE=<es3 image> tools/board.sh imx8mp tree
  tools/cts/imx8mp-gles3-tree.txt <dir>`, one row per call; the rows the host
  pre-triage expected to hang (13-15, 57, 95, 122-127) or reach the watchdog
  (58-60) last. Failing names: DEQP_FAILS pages, `SEL=0x8000 | page << 8 | i`
  (`tools/board.sh imx8mp run`), six name checks a boot. Unusual statuses:
  `SEL=0x2000 | i` (DEQP_STATUS).

## Session 1 (2026-11-07 15:42 to 2026-11-08 04:10, board clock, UTC)

- gl_test: 10 boots, every one `rc=0 verdict=done`; the same CASE records as
  session 2 (below).
- The tree: 133 of 133 entries, 133 boots, every one `rc=0 verdict=done`
  (`resets=absent`; `resets=0` on e58 and e60, whose 18 cases give a
  DEQP_CASES record), back after 31 to 231 s (median 41 s; e25 230 s, e26 231
  s, e117 and e118 160 s). No row hung and none reached the 600 s WDOG1 bound,
  the pre-triaged ones included. GPU recoveries are not counted (no console).
- Totals (the runner's `TALLY` line): **begun 44,439: pass 37,555 / fail
  4,660 / NotSupported 2,197 / other 27**; mustpass 42,810, of which 178
  never began (e26 123, e105 55). Per entry: `entries.tsv`.
- The 27 others (`SEL=0x2000 | i`, the first one of each entry): e24 (7)
  `QualityWarning`, e27 (4) `QualityWarning` (at its case index 144), e84 (14)
  `QualityWarning` (index 80), e26 (1) `ResourceError`, e105 (1)
  `ResourceError` at index 92, its last begun case (93 begun). So e26 and e105
  stop at a `ResourceError`, and their 123 + 55 mustpass cases have no result.
- Name pages: 6 pages a failing entry (12 for the 16 entries with more than 36
  Fails, e25 and e26 kept at 6: 230 s a boot), about 370 boots, every one `rc=0
  verdict=done`. They named 1,439 Fails by the time the machine running the
  session was replaced (2026-11-08 04:10, during the second set of pages).
  The run directories (`records/`, gitignored) went with that machine. What is
  kept: the per-entry counts (`entries.tsv`, from the per-row tally lines the
  runner printed, which sum to its `TALLY` line), and the case names the
  session printed (`docs/es3/imx8mp-gles3-fails.tsv`, 268 of them named one by
  one, and the texture.format list).
- A page from 32 on is not a page: e26's page 46 (`SEL=0xae1a`) came back as a
  DEQP_STATUS record, because `0x8000 | page << 8` sets bit `0x2000`, the
  status selector, which the image tests first (`deqp_snvs.c`). `board.sh`'s
  `name_pages` goes to page 39 (pages 32-39 cannot name anything, and from 64
  the probe bit `0x4000` is set too).

## Session 2 (2026-11-08, from 07:20, board clock, UTC)

The redo on a new machine. `runs.txt` has every run's verdict line and
decoded records, verbatim.

- gl_test, each case twice, every boot `rc=0 verdict=done`, back after 26 to
  34 s:

  | case | run 1 | run 2 |
  |---|---|---|
  | 196 `occl` | PASS (`594b010e,00c4ff01,00000002,00000000`) | PASS (same) |
  | 202 `blitconv` | PASS (`594b010e,00caff01,0000000c,00000000`) | PASS (same) |
  | 203 `tfcap` | PASS (`594b010e,00cbff01,00000009,00000000`) | PASS (same) |
  | 204 `srgb` | PASS (`594b010e,00ccff01,00000002,00000000`) | PASS (same) |
  | 205 `mappbo` | PASS (`594b010e,00cdff01,0000000f,00000000`) | PASS (same) |

  A CASE record with check 255 and status 1 is the case's pass; `got` is its
  count of checks.
- The tree again: e00 to e03 gave session 1's records word for word. e04
  (`fbo.api.*`, `fbo.blit.conversion.r11f_*` ...; 54 s in session 1) jumped at
  07:42 and the board did not come back: `board.sh imx8mp e04 rc=5
  verdict=not_back`. No ssh answer from 07:42 to at least 08:29 (WDOG1 should
  have reset the SoC by 07:52). The session stopped there; the board needs a
  power cycle by a person.

## Largest failure groups by cause

Counts are per table entry (`entries.tsv`); the cause numbers are those of
`docs/es3-cts-failures.md`.

| Group (entries) | Fail | Cause |
|---|---:|---|
| `shaders` (e25-e29, e43-e51) | 1,535 | RC13: `builtin_functions.common` integer forms (`abs.ivec3_*`), `precision` (`exp.*` vec3/vec4; e26 stops at a `ResourceError`, like the AM62P's image 7, RC16), `texture_functions` (`texelfetchoffset` integer samplers, `texturegrad`), `derivate`, `random.texture` |
| `fragment_ops.blend.fbo_srgb` (e12-e15) | about 1,106 | RC7: blending into an sRGB target; the GC7000UL renders sRGB on the host only (e15 also holds `fragment_ops.depth` and `depth_stencil.random`) |
| `texture.shadow` (e85-e90) | 432 of 432 | RC5: no shadow look-up on the GC7000UL (the host refuses all 432 at compile) |
| `fbo` (e4-e9) | 400 | RC1, RC8, RC17: `blit.conversion` and `blit.default_framebuffer`, `color.repeated_clear` (integer and sRGB targets), `completeness.renderable`, `msaa` 2 and 4 samples |
| `texture.format` + `texture.specification` (e70-e80, e94-e103) | 88 + 217 | RC2: the 32-bit formats, and `texstorage2d.base_level` on large RGBA8 levels; see below |
| `fragment_out` (e18-e20) | 215 | RC1, RC14: `array.float/fixed/uint`, `basic.int/uint`, `random` |
| `texture.filtering`, `texture.vertex`, `texture.mipmap` (e58-e67, e83-e84, e106) | 315 | RC18: wrap R and cube; mirror and repeat combinations of 3D and cube (`filtering.cube` 171, `filtering.3d` 72, `mipmap.cube`/`3d` 44, `vertex.3d`/`cube` 28) |
| `texture.swizzle`, `texture.units` (e104, e105) | 177 | RC18 (swizzle); `units` stops at a `ResourceError` |
| `transform_feedback` (e117, e118, e121) | 71 | `array.*.points` with `mat4` outputs: not classified |
| small groups | 104 | `default_vertex_attrib` non-square matrices 24, `rasterization` and `read_pixels` int/uint 26, `state_query` 19 (RC10), e23 17 (`negative_api` 8, `pbo` RGBA32I/UI 4, `multisample.fbo_max_samples` 4), `draw.random` 7, `instanced` 6, `uniform_api.random` 3, `lifetime` 2 |

NotSupported 2,197: ASTC 1,274 (`texture.compressed.astc` 266, `texture.wrap.astc_*`
1,008), the other compressed wrap formats 288, `fbo` 255, `fragment_out` 115,
`draw_buffers_indexed` 76, `multisample`/`negative_api`/`pbo`/`polygon_offset` 58,
`texture.format` 48, `texture.filtering` 48 (`EXT_texture_sRGB_R8`/`RG8`, 3D sRGB),
`rasterization`/`read_pixels`/`samplers`/`shader_api` 26, `state_query` 9.

**texture.format and `541975a2`.** The host claimed that the GC7000UL now samples
eight more classes (upload refusals 200 -> 64). The board agrees: in
`texture.format`, 88 cases fail and 48 are NotSupported (the host's 48 too),
every other case passes, and not one of the eight classes (R11F_G11F_B10F, RGB9_E5, RGB10_A2UI,
RGBA8_SNORM, RGBA8UI/I, RGBA16UI/I) fails. The 88 Fails are the 64 the host
still refuses (R/RG/RGB/RGBA 32I/UI on 2D, 2D array, 3D and cube, POT and NPOT)
and 24 the host passes on to an image check: RG32F, RGB32F and RGBA32F on the
four targets. `texture.specification` fails the same 32-bit formats.
