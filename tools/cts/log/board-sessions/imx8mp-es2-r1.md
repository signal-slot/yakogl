<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (merge `40077223c`, ccfarm `01a11c59-0478-75ab-8f81-a7fccbb0bd69`). Written on 2026-10-10 on the integration branch, not on main; the text is as written. Only file paths are changed to main's layout. A file this record names that is not on main stayed on the integration branch or was removed from main; check it with git before relying on it.

# i.MX8MP board session: ES2 round 1, the whole dEQP-GLES2 tree

**Integration image, not a landing proof.** Source: the ccfarm integration
snapshot, HEAD `334d4e658772f466e2625b8de1d512f0fc6afd2f` (`334d4e65`), built
as it is (no local change to any build input). A later origin/main session
repeats this before landing.

Date: 2026-10-09. Board: Verdin i.MX8M Plus (GC7000), leased resource
`verdin-imx8mp`, `BOARD_BOOT=uboot-mmc`. The board has no UART: every
result below is the SNVS record `tools/board.sh` reads after the cycle.
Plan #4 item `imx8mp-es2-r1`.

## Summary

| what | result |
|---|---|
| dEQP-GLES2, all 29 entries of `tools/cts/imx8mp-full-tree.txt`, one image | **TALLY (29 entries): begun 17371 passed 17318 failed 0 notsup 34 other 19 noresult 0**; every entry identical to the `29460f96` baseline (`tools/cts/imx8mp-full-tree-29460f96.results.tsv`), failing-name column included |
| GL_OES_texture_npot cases (`docs/es2-cts-failures.md` section H) | none moved: entry 10 (`texture.completeness.*`) 40/40 Pass, 0 NotSupported; entry 22 (`negative_api.*`) 237 Pass / 9 NotSupported, the baseline's counts |
| gl_test default image, 3 runs | run 1: 46/48, first failure case 16 (the baseline record, byte for byte); runs 2 and 3: 45/48, first failure case 10 (#9) |

## The dEQP image

```sh
tools/cts/imx8mp-deqp-build.sh deqp-334d4e65 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
```

`rc=0`, `CONFIG_DEQP_REPORT_ES2=y`, `check-deqp-images: the dEQP image
recipes build what conformance needs`. Image `zephyr.bin` sha256
`1ae0300aa931ec29…`, image build `PVR_SAMPLE_TABLE=c95547a1
PVR_SAMPLE_TREE=334d4e65`, Zephyr `v4.4.0-9848-gaf17c0c8aa2a`. Stage 1:
VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`
(`tools/cts/log/board-sessions/imx8mp-deqp-stage1.md`).

```sh
TREE_IMAGE=$IMX8MP_OUT/deqp-334d4e65/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \
  tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt <rec>/dEQP-334d4e65
```

The tree was interrupted once by a worker restart after entry 2 and resumed
with the same command (rows with an `INDEX.txt` are skipped); no row ran
twice. Rows with an `other` result got their `DEQP_FAILS` name pages
(`e14-p0..p2`, `e27-p0`).

## Verdict lines

Every line the tree printed (no board address appears in them):

```
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_CASES resets=0 record=594b0909,00015555,00000000,ffff00ff back_after=34s uptime=22s
board.sh imx8mp e01 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_CASES resets=0 record=594b0913,55555555,00000015,ffff00ff back_after=36s uptime=22s
board.sh imx8mp e02 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0890,000005e8,00000000,ffff05e8 back_after=52s uptime=27s
board.sh imx8mp e03 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0884,000003e2,00000000,ffff03e2 back_after=51s uptime=20s
board.sh imx8mp e04 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08e8,00000494,00000000,ffff0494 back_after=58s uptime=29s
board.sh imx8mp e05 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0858,0000044c,00000000,ffff044c back_after=52s uptime=21s
board.sh imx8mp e06 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a2,00000271,00000000,ffff0271 back_after=58s uptime=31s
board.sh imx8mp e07 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0898,0000076c,00000000,ffff076c back_after=138s uptime=22s
board.sh imx8mp e08 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b080c,00000526,00000000,ffff0526 back_after=73s uptime=26s
board.sh imx8mp e09 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087e,0000065f,00000000,ffff065f back_after=84s uptime=30s
board.sh imx8mp e10 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0810,00000028,00000000,ffff0028 back_after=39s uptime=27s
board.sh imx8mp e11 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d0,00000088,00000000,ffff0088 back_after=36s uptime=22s
board.sh imx8mp e12 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a0,00000070,00000000,ffff0070 back_after=46s uptime=27s
board.sh imx8mp e13 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000024,00000000,ffff0024 back_after=35s uptime=22s
board.sh imx8mp e14 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a0,00000062,000e0000,ffff0070 back_after=45s uptime=28s
board.sh imx8mp e14-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,72fd0234,06028a0f,3247f0f0 back_after=38s uptime=21s
board.sh imx8mp e14-p1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,9f6fbcb7,e44d514b,f0dc92fe back_after=48s uptime=30s
board.sh imx8mp e14-p2 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,545de690,ffffffff,ffffffff back_after=39s uptime=22s
board.sh imx8mp e15 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0860,00000050,00000000,ffff0050 back_after=60s uptime=29s
board.sh imx8mp e16 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0840,000000c0,00000000,ffff00c0 back_after=47s uptime=24s
board.sh imx8mp e17 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0874,0000005a,00000000,ffff005a back_after=60s uptime=27s
board.sh imx8mp e18 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0834,0000003a,00000000,ffff003a back_after=45s uptime=31s
board.sh imx8mp e19 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000424,00000000,ffff0424 back_after=47s uptime=25s
board.sh imx8mp e20 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087e,0000035f,00000000,ffff035f back_after=112s uptime=27s
board.sh imx8mp e21 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b084e,000002c7,00000000,ffff02c7 back_after=51s uptime=21s
board.sh imx8mp e22 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08ac,000000ed,00000009,ffff00f6 back_after=39s uptime=26s
board.sh imx8mp e23 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d8,0000018c,00000000,ffff018c back_after=34s uptime=22s
board.sh imx8mp e24 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a6,000004f3,00000000,ffff04f3 back_after=84s uptime=35s
board.sh imx8mp e25 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0874,0000025a,00000000,ffff025a back_after=47s uptime=25s
board.sh imx8mp e26 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0836,0000023b,00000000,ffff023b back_after=99s uptime=30s
board.sh imx8mp e27 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08c6,000000fe,00050000,ffff0103 back_after=49s uptime=21s
board.sh imx8mp e27-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a05,4b3a5417,db733a1d,ffff9f30 back_after=58s uptime=30s
board.sh imx8mp e28 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08c8,000000eb,00000019,ffff0104 back_after=215s uptime=25s
TALLY (29 entries): begun 17371 passed 17318 failed 0 notsup 34 other 19 noresult 0
```

## Per entry against the 29460f96 baseline

The tree's `results.tsv` is byte-identical to
`tools/cts/imx8mp-full-tree-29460f96.results.tsv` (`diff` empty): every
entry has the baseline's begun/passed/failed/notsup/other/noresult and the
same failing names. **No entry differs.**

| entry | filter (short) | begun | passed | failed | notsup | other | vs 29460f96 |
|---|---|---|---|---|---|---|---|
| 0 | info, prerequisite | 9 | 9 | 0 | 0 | 0 | same |
| 1 | color_clear | 19 | 19 | 0 | 0 | 0 | same |
| 2 | shaders: struct … constants (20 groups, invariance among them) | 1512 | 1512 | 0 | 0 | 0 | same |
| 3 | shaders.matrix, invalid_implicit_conversions | 994 | 994 | 0 | 0 | 0 | same |
| 4 | shaders.swizzles, conversions | 1172 | 1172 | 0 | 0 | 0 | same |
| 5 | shaders.indexing, preprocessor | 1100 | 1100 | 0 | 0 | 0 | same |
| 6 | shaders.loops | 625 | 625 | 0 | 0 | 0 | same |
| 7 | shaders.random | 1900 | 1900 | 0 | 0 | 0 | same |
| 8 | shaders.operator.binary_operator | 1318 | 1318 | 0 | 0 | 0 | same |
| 9 | shaders.operator (the other 10 groups) | 1631 | 1631 | 0 | 0 | 0 | same |
| 10 | texture.completeness | 40 | 40 | 0 | 0 | 0 | same |
| 11 | texture.filtering.2d | 136 | 136 | 0 | 0 | 0 | same |
| 12 | texture.filtering.cube | 112 | 112 | 0 | 0 | 0 | same |
| 13 | texture.format | 36 | 36 | 0 | 0 | 0 | same |
| 14 | texture.mipmap | 112 | 98 | 0 | 0 | 14 | same (the 14 `mipmap.cube.generate.*` QualityWarnings) |
| 15 | texture.size | 80 | 80 | 0 | 0 | 0 | same |
| 16 | texture.specification | 192 | 192 | 0 | 0 | 0 | same |
| 17 | texture.vertex | 90 | 90 | 0 | 0 | 0 | same |
| 18 | texture.wrap | 58 | 58 | 0 | 0 | 0 | same |
| 19 | fragment_ops.blend | 1060 | 1060 | 0 | 0 | 0 | same |
| 20 | fragment_ops (depth, stencil, …, scissor) | 863 | 863 | 0 | 0 | 0 | same |
| 21 | fbo | 711 | 711 | 0 | 0 | 0 | same |
| 22 | negative_api | 246 | 237 | 0 | 9 | 0 | same |
| 23 | state_query | 396 | 396 | 0 | 0 | 0 | same |
| 24 | uniform_api | 1267 | 1267 | 0 | 0 | 0 | same |
| 25 | clipping | 602 | 602 | 0 | 0 | 0 | same |
| 26 | vertex_arrays | 571 | 571 | 0 | 0 | 0 | same |
| 27 | draw, attribute_location, rasterization, buffer | 259 | 254 | 0 | 0 | 5 | same (the 5 `rasterization.interpolation.*lines*`) |
| 28 | default_vertex_attrib … multisampled_render_to_texture | 260 | 235 | 0 | 25 | 0 | same |

The 19 `other` results by name (decoded from the `DEQP_FAILS` pages; the
same names as the baseline):

- entry 14: `dEQP-GLES2.functional.texture.mipmap.cube.generate.` +
  `a8_fastest`, `a8_nicest`, `l8_fastest`, `la88_fastest`, `rgb565_fastest`,
  `rgb565_nicest`, `rgb888_fastest`, `rgb888_nicest`, `rgba4444_fastest`,
  `rgba4444_nicest`, `rgba5551_fastest`, `rgba5551_nicest`,
  `rgba8888_fastest`, `rgba8888_nicest`
- entry 27: `dEQP-GLES2.functional.rasterization.interpolation.` +
  `basic.line_strip`, `basic.line_strip_wide`, `basic.lines`,
  `projected.lines`, `projected.lines_wide`

The 34 NotSupported results (entry 22: 9, entry 28: 25) carry no names:
the tree's failing-name column (and the baseline's) lists only the `other`
and `failed` names. Their counts equal the baseline's.

## GL_OES_texture_npot cases (es2-cts-failures.md section H)

This image claims `GL_OES_texture_npot` (`NPOT_EXTENSION` in
`src/gl/state.rs`, true in every report-es2 build); the `29460f96` baseline
predates it. Section H predicted 18 mustpass cases to move from Pass to
NotSupported (or, for the size cases, possibly stay Pass). On the GC7000
none moved:

| group (section H) | cases | entry | measured |
|---|---|---|---|
| `texture.completeness.2d` wrap | `npot_all_repeat`, `npot_mirrored_repeat`, `npot_s_repeat`, `npot_t_repeat`, `repeat_width_npot`, `repeat_height_npot` | 10 | Pass (entry 10: 40/40 Pass, 0 NotSupported) |
| `texture.completeness.cube` wrap | `npot_all_repeat`, `npot_mirrored_repeat`, `npot_s_repeat`, `npot_t_repeat` | 10 | Pass |
| `texture.completeness.2d` size | `npot_size`, `npot_size_level_0`, `npot_size_level_1` | 10 | Pass |
| `texture.completeness.cube` size | `npot_size_level_0`, `npot_size_level_0_pos_x`, `npot_size_level_1`, `npot_size_level_1_neg_x` | 10 | Pass |
| `negative_api.texture` | `generatemipmap_npot_wdt_hgt` | 22 | Pass by count: entry 22 is 237 Pass / 9 NotSupported, the baseline's numbers; a move to NotSupported would read 236 / 10 |

The 17 completeness cases are Pass by name (their entry has no non-Pass
result). The negative_api case is a count argument: NotSupported names are
not recorded. So the prediction "Pass → NotSupported" does not hold for the
GC7000 at this commit; the wrap cases also did not fail (a REPEAT NPOT
texture sampled where dEQP expects black would have been a Fail).
`tools/cts/log/investigations/es2-npot-cases.md` (es2-npot-research) is where the
CTS source explains why.

## gl_test default image, three runs

GT image from `tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt
<img> GT` (default `prj.conf`, watchdog self-arm): `GT rc=0`, sha256
`b12ac586f4b6edda…`, build stamp `GC_BUILD_STAMP=2026-10-09T00:51:42Z`.

```
board.sh imx8mp gt-334d4e65-1 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,0010ff06,0000002e,00000030 back_after=73s uptime=26s
board.sh imx8mp gt-334d4e65-2 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,000aff06,0000002d,00000030 back_after=72s uptime=25s
board.sh imx8mp gt-334d4e65-3 rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,000aff06,0000002d,00000030 back_after=73s uptime=26s
```

Decoded (`board.sh`'s own decode):

| run | decoded record | reading |
|---|---|---|
| 1 | `CASE (1) seq 14: case 16 check 255 status 6 (set failed) got 0x0000002e want 0x00000030` | 46 of 48, first failure case 16: byte-identical to the `b20e4d11` record (`imx8mp-plan1.md` step 2) and the baseline2-imx8mp record. Case 16 is the 4x MSAA half skipped by name, counted as a failure by the set walker |
| 2 | `CASE (1) seq 14: case 10 check 255 status 6 (set failed) got 0x0000002d want 0x00000030` | 45 of 48, first failure case 10: the #9 flake, before case 16 in the set's order (so 16 and the unnamed later failure make the other two) |
| 3 | `CASE (1) seq 14: case 10 check 255 status 6 (set failed) got 0x0000002d want 0x00000030` | the same as run 2 |

Case 10 (#9) failed in 2 of 3 runs here; it passed in all three at
`b20e4d11`. The gl_test record names only the first failure; case 2 (#25)
runs before case 10 and case 16 in the set's order (as `imx8mp-plan1.md`
step 2 reads it), so it was not the first failure in any of the three runs.
The flake rate belongs to `imx8mp-gltest-r2` (the #9/#25 instrument runs).

## How to repeat

With the variables of `tools/cts/imx8mp-env.sh` exported
(`ZEPHYR_WORKSPACE`, `ZEPHYR_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `IMX8MP_OUT`)
plus `DEQP_STAGE1` (stage 1 at VK-GL-CTS `93bca018`), a real cargo first
on `PATH`, `RUSTUP_TOOLCHAIN=nightly` on the build commands only, and
`BOARD_HOST` / `BOARD_BOOT=uboot-mmc` from the leased resource:

```sh
H=$(git rev-parse --short=8 HEAD)
tools/cts/imx8mp-deqp-build.sh deqp-$H 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
TREE_IMAGE=$IMX8MP_OUT/deqp-$H/zephyr/zephyr.bin tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt <rec>/dEQP-$H
diff tools/cts/imx8mp-full-tree-29460f96.results.tsv <rec>/dEQP-$H/results.tsv
ZEPHYR_WS=$ZEPHYR_WORKSPACE WEST_VENV=$ZEPHYR_VENV tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt <img> GT
tools/board.sh imx8mp run <img>/GT/zephyr/zephyr.bin <rec>/gt gt-$H-$i   # i = 1 2 3
```

Records (`INDEX.txt`, `meta.txt`, `snvs.txt` per row) were kept on the
worker, not committed.
