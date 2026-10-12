<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# i.MX8MP board session: dEQP-GLES2 whole tree from origin/main `c345ab74` (imx8mp-gltest-r2)

Plan #20 item `imx8mp-gltest-r2`, redo of `imx8mp-es2-final` (which ended without
numbers). Roadmap stage 1: zero dEQP-GLES2 failures.

Date: 2026-10-12. Board: Verdin i.MX8M Plus (GC7000), the leased `verdin-imx8mp`
resource, `BOARD_BOOT=uboot-mmc`. The board has no UART in this setup: every result
is the SNVS record `tools/board.sh` reads after each cycle. No board address,
credential or UART path is written here.

## Summary

| what | result |
|---|---|
| Commit | origin/main `c345ab74` (`c345ab7475894e48ab82422a81a6a796febd6c29`, "Merge pull request #177 ... work/issue-9-kickdiff"), built from a detached worktree of that commit |
| Image | `deqp-c345ab74/zephyr/zephyr.bin`, sha256 prefix `c4d409ab8515d0c3`; image build `PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=c345ab74`; Zephyr `v4.4.0-9848-gaf17c0c8aa2a` |
| Build | `tools/cts/imx8mp-deqp-build.sh deqp-c345ab74 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt` rc=0, 29 table rows, `CONFIG_DEQP_REPORT_ES2=y`, conformance-image check passed |
| Run | `tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt <rec>`, all 29 entries + the 4 `DEQP_FAILS` name pages, every row `rc=0 verdict=done`, GPU resets absent/0 |
| Failures | **0** |

TALLY line, verbatim:

```
TALLY (29 entries): begun 17371 passed 17318 failed 0 notsup 34 other 19 noresult 0
```

### Mustpass totals (the 17,165 list)

The tree runs 17,371 names; 206 of them are not in `tools/cts/gles2-mustpass.txt`
(17,371 - 17,165). The board record is a per-case bit record, so only the 19 `other`
names are decoded; the 34 NotSupported results are not named (entries 22 and 28 only
give counts), so how many of the 34 fall outside the list is not known and the
mustpass-list totals are a range, as in `docs/es2-cts-failures.md` (0 to 4 of the
34 NotSupported names may be among the 206 extra names):

| | Pass | Fail | NotSupported | QualityWarning |
|---|---:|---:|---:|---:|
| 17,165 list | 17,112 to 17,116 | 0 | 30 to 34 | 19 |

Both ends sum to 17,165 (17,112 + 0 + 34 + 19, and 17,116 + 0 + 30 + 19). That is the
same range `docs/es2-cts-failures.md` records for the i.MX8MP at `32778b57`. The
whole-run counts are 17,318 pass / 0 fail / 34 NotSupported / 19 QualityWarning.
`QualityWarning` is the `other` column; dEQP counts it as a pass.

## Failing cases

None. No case failed, none crashed, none had no result (`noresult 0`).

## Not Pass

QualityWarning (19, named, decoded from `DEQP_FAILS` pages `e14-p0..p2`, `e27-p0`),
identical to the baseline:

- entry 14: `dEQP-GLES2.functional.texture.mipmap.cube.generate.` + `a8_fastest`,
  `a8_nicest`, `l8_fastest`, `la88_fastest`, `rgb565_fastest`, `rgb565_nicest`,
  `rgb888_fastest`, `rgb888_nicest`, `rgba4444_fastest`, `rgba4444_nicest`,
  `rgba5551_fastest`, `rgba5551_nicest`, `rgba8888_fastest`, `rgba8888_nicest`
- entry 27: `dEQP-GLES2.functional.rasterization.interpolation.` + `basic.line_strip`,
  `basic.line_strip_wide`, `basic.lines`, `projected.lines`, `projected.lines_wide`

NotSupported (34, counts only; the tree does not record their names):

- entry 22 (`negative_api.*`): 9 (237 pass / 9 NotSupported)
- entry 28 (`default_vertex_attrib` ... `multisampled_render_to_texture`): 25
  (235 pass / 25 NotSupported; the `multisample.*` group and friends at 1x)

## Per entry against `tools/cts/imx8mp-full-tree-29460f96.results.tsv`

The tree's `results.tsv` is byte-identical to the baseline file (`diff` empty): every
entry has the baseline's begun / passed / failed / notsup / other / noresult and the
same failing-name column. **No entry changed.** Entries, filters and counts:

| entry | begun | passed | failed | notsup | other | vs baseline |
|---|---:|---:|---:|---:|---:|---|
| 0 info, prerequisite | 9 | 9 | 0 | 0 | 0 | same |
| 1 color_clear | 19 | 19 | 0 | 0 | 0 | same |
| 2 shaders (20 groups) | 1512 | 1512 | 0 | 0 | 0 | same |
| 3 matrix, invalid_implicit_conversions | 994 | 994 | 0 | 0 | 0 | same |
| 4 swizzles, conversions | 1172 | 1172 | 0 | 0 | 0 | same |
| 5 indexing, preprocessor | 1100 | 1100 | 0 | 0 | 0 | same |
| 6 loops | 625 | 625 | 0 | 0 | 0 | same |
| 7 random | 1900 | 1900 | 0 | 0 | 0 | same |
| 8 operator.binary_operator | 1318 | 1318 | 0 | 0 | 0 | same |
| 9 operator (other 10 groups) | 1631 | 1631 | 0 | 0 | 0 | same |
| 10 texture.completeness | 40 | 40 | 0 | 0 | 0 | same |
| 11 texture.filtering.2d | 136 | 136 | 0 | 0 | 0 | same |
| 12 texture.filtering.cube | 112 | 112 | 0 | 0 | 0 | same |
| 13 texture.format | 36 | 36 | 0 | 0 | 0 | same |
| 14 texture.mipmap | 112 | 98 | 0 | 0 | 14 | same |
| 15 texture.size | 80 | 80 | 0 | 0 | 0 | same |
| 16 texture.specification | 192 | 192 | 0 | 0 | 0 | same |
| 17 texture.vertex | 90 | 90 | 0 | 0 | 0 | same |
| 18 texture.wrap | 58 | 58 | 0 | 0 | 0 | same |
| 19 fragment_ops.blend | 1060 | 1060 | 0 | 0 | 0 | same |
| 20 fragment_ops (rest) | 863 | 863 | 0 | 0 | 0 | same |
| 21 fbo | 711 | 711 | 0 | 0 | 0 | same |
| 22 negative_api | 246 | 237 | 0 | 9 | 0 | same |
| 23 state_query | 396 | 396 | 0 | 0 | 0 | same |
| 24 uniform_api | 1267 | 1267 | 0 | 0 | 0 | same |
| 25 clipping | 602 | 602 | 0 | 0 | 0 | same |
| 26 vertex_arrays | 571 | 571 | 0 | 0 | 0 | same |
| 27 draw, attribute_location, rasterization, buffer | 259 | 254 | 0 | 0 | 5 | same |
| 28 default_vertex_attrib ... multisampled_render_to_texture | 260 | 235 | 0 | 25 | 0 | same |

Against the earlier i.MX8MP session `imx8mp-es2-r1` (image `334d4e65`,
`tools/cts/log/board-sessions/imx8mp-es2-r1.md`) the TALLY line is the same string.

## Remaining to reach stage 1 on this board

The target is 0 failures: met (0 failed, 0 no result). What is left under the
conformance rule of `tools/cts/README.md` ("Pass or NotSupported/QualityWarning"):
34 NotSupported (negative_api 9; entry 28: 25) and 19 QualityWarning (cube mipmap
generation, line interpolation). Their causes are in `docs/es2-cts-failures.md`
(sections for the i.MX8MP); this session did not re-diagnose them. The names of the
NotSupported cases are not recoverable from the SNVS record.

## Verdict lines

Every row's line, as `tools/board.sh` printed it (no address in them; `back_after`
and `uptime` are the board's seconds):

```
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_CASES resets=0 record=594b0909,00015555,00000000,ffff00ff back_after=34s uptime=22s
board.sh imx8mp e01 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_CASES resets=0 record=594b0913,55555555,00000015,ffff00ff back_after=36s uptime=22s
board.sh imx8mp e02 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0890,000005e8,00000000,ffff05e8
board.sh imx8mp e03 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0884,000003e2,00000000,ffff03e2
board.sh imx8mp e04 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08e8,00000494,00000000,ffff0494
board.sh imx8mp e05 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0858,0000044c,00000000,ffff044c
board.sh imx8mp e06 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a2,00000271,00000000,ffff0271
board.sh imx8mp e07 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0898,0000076c,00000000,ffff076c
board.sh imx8mp e08 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b080c,00000526,00000000,ffff0526
board.sh imx8mp e09 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087e,0000065f,00000000,ffff065f
board.sh imx8mp e10 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0810,00000028,00000000,ffff0028
board.sh imx8mp e11 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d0,00000088,00000000,ffff0088
board.sh imx8mp e12 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a0,00000070,00000000,ffff0070
board.sh imx8mp e13 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000024,00000000,ffff0024
board.sh imx8mp e14 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a0,00000062,000e0000,ffff0070
board.sh imx8mp e14-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,72fd0234,06028a0f,3247f0f0
board.sh imx8mp e14-p1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,9f6fbcb7,e44d514b,f0dc92fe
board.sh imx8mp e14-p2 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a0e,545de690,ffffffff,ffffffff
board.sh imx8mp e15 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0860,00000050,00000000,ffff0050
board.sh imx8mp e16 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0840,000000c0,00000000,ffff00c0
board.sh imx8mp e17 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0874,0000005a,00000000,ffff005a
board.sh imx8mp e18 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0834,0000003a,00000000,ffff003a
board.sh imx8mp e19 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000424,00000000,ffff0424
board.sh imx8mp e20 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087e,0000035f,00000000,ffff035f
board.sh imx8mp e21 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b084e,000002c7,00000000,ffff02c7
board.sh imx8mp e22 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08ac,000000ed,00000009,ffff00f6
board.sh imx8mp e23 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d8,0000018c,00000000,ffff018c
board.sh imx8mp e24 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a6,000004f3,00000000,ffff04f3
board.sh imx8mp e25 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0874,0000025a,00000000,ffff025a
board.sh imx8mp e26 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0836,0000023b,00000000,ffff023b
board.sh imx8mp e27 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08c6,000000fe,00050000,ffff0103
board.sh imx8mp e27-p0 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP_FAILS resets=absent record=594b0a05,4b3a5417,db733a1d,ffff9f30
board.sh imx8mp e28 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08c8,000000eb,00000019,ffff0104
TALLY (29 entries): begun 17371 passed 17318 failed 0 notsup 34 other 19 noresult 0
```

(For e02 to e28 the `back_after` / `uptime` fields, 85 to 261 s and 21 to 33 s, are left
out of the table above only to keep the lines short.)

## What went wrong on the way, and what was changed outside the repository

1. **First build failed**: the farm environment's `DEQP_STAGE1` points at the ES3
   stage-1 tree (`<IMX8MP_OUT>/deqp-stage1-gles3`), which has no
   `libdeqp-gles2-package.a` (`ninja: ... missing and no known rule to make it`).
   The image was built with `DEQP_STAGE1` pointing at `deqp-stage1-imx8mp` in the
   shared deps directory (the GLES2 stage-1 libraries). `DEQP_STAGE1` for GLES2
   image builds should be set that way in the farm environment.
2. **A stale staged image blocked the run**: for the first ~20 minutes `board.sh`
   refused every try (rc=3, "try N of 60") because `/var/yakogl` on the board held
   `once-kd-24-<id>.bin`, a one-shot image from a run of 2026-10-11 that did not clean
   up. `yako_once` was empty (nothing armed), so the file was inert. It was removed on
   the leased board (what `board.sh imx8mp disarm` does for `uboot-mmc`, by hand over
   ssh); the next try ran. The refused tries made no cycle and wrote no result. The
   scratch record directory therefore holds some empty `e00` directories from them.
3. The records (`INDEX.txt`, `meta.txt`, `snvs.txt` per row, `results.tsv`, `rows.md`)
   were kept on the worker in the deps directory, not committed.

## How to repeat

```sh
H=c345ab74
DEQP_STAGE1=<deps>/deqp-stage1-imx8mp \
  tools/cts/imx8mp-deqp-build.sh deqp-$H 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
BOARD_HOST=<user>@<host> BOARD_BOOT=uboot-mmc TREE_IMAGE=$IMX8MP_OUT/deqp-$H/zephyr/zephyr.bin \
  tools/board.sh imx8mp tree tools/cts/imx8mp-full-tree.txt <rec>
diff tools/cts/imx8mp-full-tree-29460f96.results.tsv <rec>/results.tsv
```
