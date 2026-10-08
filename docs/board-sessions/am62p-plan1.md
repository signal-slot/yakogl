<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P board session: plan #1 on the integration image

**Integration image, not a landing proof.** Source: the ccfarm integration
snapshot, HEAD `b83216d4003b3e0b4764adc067dbbc843083c8f0` (`b83216d4`, the
`ccfarm/integration` tip this session ran; it replaces the earlier label
`198e8a5de6da1b4452f8cc7c9c502b6cd757f1bf`, at which nothing ran — the
session's shell was restricted and no command outside the workspace was
approved). `4e9397c` and `c6aa6ba` are both ancestors of HEAD (checked with
`git merge-base --is-ancestor`). A later origin/main session repeats this
before landing.

## Result: images 1 to 7 ran on verdin-am62p; 10,092 pass, 9 fail

All seven shaders images (table rows 1 to 7, `TREE_ROWS=0-6`) built at
`b83216d4` and ran to `verdict=done` through `tools/board.sh am62p tree`
(boot `uboot-mmc`, serial bridge `usb-serial-1:35933`). Every case in every
image got a result. Six images match the `d60b9d76` baseline exactly. Image 1
has **9 new Fail cases** (`shaders.invariance.*.common_subexpression_1/2/3`
in all three precisions) and one expected flip: `fragdata.invalid_assign_to_1`
moved from NotSupported to **Pass**, confirming the prediction in
`docs/es2-cts-failures.md` ("expected pass, awaiting board"). The 9 failures
are a regression against `d60b9d76` (they passed there) and are not in
`docs/es2-cts-failures.md`.

## How it was run

Inputs on this node (Lepton), all under the ccfarm dependency cache
`/home/tasuku/.local/share/ccfarm-local/deps/yakogl`:

- `ZEPHYR_WS=.../deqp-stage1-rpi_4b/zephyr-ws`,
  `WEST_VENV=.../deqp-stage1-rpi_4b/venv`,
  `ZEPHYR_SDK_INSTALL_DIR=.../deqp-stage1-rpi_4b/sdk/zephyr-sdk-1.0.1`,
  `CTS=.../deqp-stage1-rpi_4b/cts-src` (shared with the Pi 4 stage 1).
- Stage 1 for this board: `WORK=/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-am62p
  BOARD=verdin_am62p/am62p54/a53 tools/cts/cts-build-zephyr.sh` (rc=0).
  It needs a real `cargo`/`rustc` first on PATH (`/home/tasuku/.cargo/bin`);
  the kache cargo shim has no rustc next to it (same finding as baseline2).
- `driver/src/fw/pvr_fw_blob.h` regenerated from the board's
  `/lib/firmware/rgx.fw.36.53.104.796` (fetched with
  `ssh torizon@verdin-am62p cat ... > .inputs/rgx.fw`, then the
  `driver/tools/mkfwblob.sh` format). Never committed.
- Images: `TABLE=/home/tasuku/.local/share/ccfarm-local/deps/yakogl/plan1-am62p-images/am62p-shaders7.txt
  tools/cts/am62p-deqp-images.sh <that table> b83216d4` — the table is rows
  1 to 7 of `tools/cts/am62p-full-tree.txt`. All seven `img<N> rc=0`
  (`build.out`). The script still exits 1 after the last unlabelled row (a
  `set -e` quirk in its final labelled-table test); every image built.
- Board: `BOARD_HOST=torizon@verdin-am62p BOARD_UART=usb-serial-1:35933
  BOARD_BOOT=uboot-mmc TREE_IMAGE_DIR=<the images dir above> TREE_TAG=b83216d4
  TREE_ROWS=0-6 tools/board.sh am62p tree tools/cts/am62p-full-tree.txt
  records/plan1-am62p/full`, then `tools/board.sh am62p tally
  records/plan1-am62p/full`. The tree is resumable: rows with an `INDEX.txt`
  are skipped, so an interrupted row re-runs alone.

## Records and tallies

| image | record | pass | fail | NotSupported | warnings |
|---|---|---|---|---|---|
| 1 | `20261008-0634-img1-b83216d4-f4c169af` | 1503 | **9** | 0 | 0 |
| 2 | `20261008-0636-img2-b83216d4-b7f3c40b` | 994 | 0 | 0 | 0 |
| 3 | `20261008-0638-img3-b83216d4-a09e9b60` | 1172 | 0 | 0 | 0 |
| 4 | `20261008-0641-img4-b83216d4-891bc4bd` | 1100 | 0 | 0 | 0 |
| 5 | `20261008-0650-img5-b83216d4-f442972e` | 2525 | 0 | 0 | 0 |
| 6 | `20261008-0653-img6-b83216d4-6e9f93f0` | 1318 | 0 | 0 | 0 |
| 7 | `20261008-0656-img7-b83216d4-fd54245d` | 1631 | 0 | 0 | 0 |
| 8 to 17, ms4 | none | not run | not run | not run | not run |

Image sha256 prefixes (from `build.out` and the board lines): 1
`f4c169aff5c1bbb4`, 2 `b7f3c40bd8bdc0f9`, 3 `a09e9b609e4529bf`, 4
`891bc4bdc9c3092c`, 5 `f442972e497857e8`, 6 `6e9f93f0fe482aff`, 7
`fd54245d5b6f21ce`. Every console shows the provenance line
`PVR_SAMPLE_TABLE=c95547a1 PVR_SAMPLE_TREE=b83216d4; zephyr: b8df2f8d3d4e`.

Image 5 ran twice: the first capture (`20261008-0643-img5-...`) was cut when
the worker's command limit ended the capture mid-run (the tally lists it as
`rc=4 verdict=no_reset`); the re-run above holds the result. Same situation
RESULTS.md §1z2 notes for `d60b9d76` image 5.

Verdict lines, as printed by `tools/board.sh`:

```
board.sh am62p img1-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=136s
board.sh am62p img2-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=117s
board.sh am62p img3-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=110s
board.sh am62p img4-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=123s
board.sh am62p img5-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=179s
board.sh am62p img6-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=137s
board.sh am62p img7-b83216d4 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:deqp_summary resets=0 record=absent back_after=140s
```

The `tally` mustpass section: 10,101 mustpass cases fall in these seven
images' filters (7,064 mustpass cases are in rows 8 to 17, not run); of them
**10,092 pass / 9 fail / 0 NotSupported / 0 warnings**, no case without a
result. No GPU reset lines in any record.

## Comparison with d60b9d76 (17,141 pass / 0 fail / 19 NotSupported / 5 warnings)

Rows 2 to 7 are identical to the §1z2 per-image counts (994, 1172, 1100,
2525, 1318, 1631, all 0 fail). Row 1 changed in two ways:

- `dEQP-GLES2.functional.shaders.fragdata.invalid_assign_to_1`: NotSupported
  at `d60b9d76`, **Pass** here (`deqp: Pass`, console line 2957). All seven
  `fragdata.*` cases pass. The `4e9397c` ES 2.0 reporting fix works on the
  board.
- 9 cases moved from Pass to Fail — a regression of the compiler changes
  since `d60b9d76`, exactly the case HANDOFF warned about:

```
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_1   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_2   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.highp.common_subexpression_3   Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_1    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_2    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.lowp.common_subexpression_3    Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_1 Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_2 Fail (Detected variance between two invariant values)
dEQP-GLES2.functional.shaders.invariance.mediump.common_subexpression_3 Fail (Detected variance between two invariant values)
```

`common_subexpression_0` passes in all three precisions; the failure message
carries no numeric detail. These 9 names are not in `docs/es2-cts-failures.md`
(no `invariance` entry anywhere in it): a new AM62P failure. The test computes
one common subexpression twice and requires bit-identical results, so a
compiler that fuses or reassociates one occurrence and not the other produces
exactly this. The prime suspects are the AM62P fusion/predication commits
listed in `docs/es2-cts-failures.md` section D with risk H — `6f96104` +
`18972dc` (fmad fusion), `39bd698` (varying product fused with its sum),
`81c53c6`, `afa12e9`, `ecd341d`, `f4c41d0`, `97ba03d`/`5f4d367` — plus
`c6aa6ba` (vertex attribute registers). A bisect of image 1 is the next step;
the host `compiler-regression-host` corpus did not catch this.

## Not run

Rows 8 to 17 (fbo, texture, vertex attribute, …) and the `ms4` multisample
image were not built or run: this item covers images 1 to 7 and the fragdata
row only. The whole-tree comparison against 17,141 / 0 / 19 / 5 needs those
rows too.

## Handoff:

- The 9 `invariance.common_subexpression_1/2/3` failures (all precisions) are
  the item's main finding: a board-proven regression of the pvr-online
  compiler since `d60b9d76`, first error line "Detected variance between two
  invariant values", records under
  `records/plan1-am62p/full/20261008-0634-img1-b83216d4-f4c169af/`. Bisect
  image 1 across the section-D H-risk commits; a dEQP-shaped host repro of
  "same subexpression, two occurrences, one fused" would be cheaper than
  board bisection.
- `fragdata.invalid_assign_to_1` Pass is board-confirmed; `es2-cts-failures.md`
  (owned by bench-recipes) can move that row from "awaiting board" to done.
- Stage 1, the seven images, and the records are on this node:
  `DEQP_WORK=/home/tasuku/.local/share/ccfarm-local/deps/yakogl/deqp-stage1-am62p`,
  images in `.../deps/yakogl/plan1-am62p-images/img1..7-b83216d4` (labels
  `img<N>-b83216d4`, `build.out` present). A re-run of rows 8 to 17 needs no
  rebuild of rows 1 to 7: the tree skips rows with `INDEX.txt`.
- `am62p-deqp-images.sh` exits 1 after the last unlabelled row even when all
  images built (`set -e` on its final `[ "$labelled" = 1 ]` test) — worth a
  one-line fix by the tooling owner.
