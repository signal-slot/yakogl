<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES2 rows to rerun for the npot change (deqp-npot-rows)

Status: **a recipe; nothing here has run on a board.** It names, per board, the
dEQP tree rows that hold the cases the npot change (`2673197d`, "es 2.0 builds
claim gl_oes_texture_npot") moves, and the commands that rerun only those
rows. The rows and `TREE_ROWS` values come from `tools/cts/rows-for-cases.py`
on the tables at `79c73ea2`. If a table changes, run the tool again rather
than trusting the numbers below.

## The cases

The npot change predicts that these 11 cases move **Pass -> NotSupported** on
every board, because the context now lists `GL_OES_texture_npot`:

- `texture.completeness.2d.{npot_all_repeat, npot_mirrored_repeat, npot_s_repeat, npot_t_repeat, repeat_width_npot, repeat_height_npot}`
- `texture.completeness.cube.{npot_all_repeat, npot_mirrored_repeat, npot_s_repeat, npot_t_repeat}`
- `negative_api.texture.generatemipmap_npot_wdt_hgt`

These 7 may stay NotSupported or pass. Either result is acceptable, and a
failure is not:

- `texture.completeness.2d.npot_size*`: `npot_size`, `npot_size_level_0`, `npot_size_level_1`
- `texture.completeness.cube.npot_size_level_*`: `npot_size_level_0`, `npot_size_level_0_pos_x`, `npot_size_level_1`, `npot_size_level_1_neg_x`

All are `dEQP-GLES2.functional.` names in `tools/cts/gles2-mustpass.txt`. The
list, with its two globs, is
`tools/cts/testdata/rows-for-cases/cases.txt`. Each of the 18 names is in
exactly one row of each board's table (the tool's `--self-test` checks this).

```sh
python3 tools/cts/rows-for-cases.py --cases tools/cts/testdata/rows-for-cases/cases.txt tools/cts/am62p-full-tree.txt
python3 tools/cts/rows-for-cases.py --cases tools/cts/testdata/rows-for-cases/cases.txt tools/cts/pi4-full-tree.txt
python3 tools/cts/rows-for-cases.py --cases tools/cts/testdata/rows-for-cases/cases.txt tools/cts/imx8mp-full-tree.txt
```

"Row" is `TREE_ROWS`'s number: it counts from 0 over the table's non-comment
lines (`tools/board.sh` `tree_rows`/`in_rows`, `am62p-deqp-images.sh`). The
mustpass column counts the mustpass names the whole row selects, which is
what the rerun costs. On the Pi 4 it equals the table's `# image` marker.

## Verdict to look for

In the tally of each rerun (`tools/board.sh <board> tally <rec>`):

- the 11 named cases are **NotSupported**. A Pass means the image does not
  list `GL_OES_texture_npot`; check that it was built from a commit with
  `2673197d`. A Fail is a defect.
- the 7 `npot_size*` cases are NotSupported or Pass, never Fail.
- every other case of the rerun rows keeps its last result (RESULTS.md, the
  board's last full tree). The NotSupported count of the rows goes up by 11
  and their Pass count goes down by 11, give or take the 7 optional cases.

## AM62P (Verdin AM62P, PowerVR): `TREE_ROWS=7,15`

`tools/cts/am62p-full-tree.txt` has 17 rows, one image per row, named
`img<row+1>-$TREE_TAG`.

| Row | Image | Filter | npot cases | Mustpass in row |
|-----|-------|--------|------------|-----------------|
| 7 | `img8-$HEAD8` | `texture.completeness.*`, `texture.filtering.*`, `texture.format.*` | 17 (10 named + 7 `npot_size*`) | 324 |
| 15 | `img16-$HEAD8` | `state_query.*`, `negative_api.*`, `draw.*` | 1 (`generatemipmap_npot_wdt_hgt`) | 743 |

With the environment of the common block in `docs/HANDOFF.md` (`SW`,
`HEAD8`, `ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`, `BOARD_HOST`,
`BOARD_UART`) and stage 1 as `DEQP_WORK`:

```sh
export DEQP_WORK=<stage 1>
mkdir -p <img>
TREE_ROWS=7,15 $SW/tools/cts/am62p-deqp-images.sh $SW/tools/cts/am62p-full-tree.txt <img> $HEAD8 > <img>/build.out   # images 8 and 16 only
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 TREE_ROWS=7,15 $SW/tools/board.sh --dry-run am62p tree $SW/tools/cts/am62p-full-tree.txt <rec>/npot   # first
TREE_IMAGE_DIR=<img> TREE_TAG=$HEAD8 TREE_ROWS=7,15 $SW/tools/board.sh am62p tree $SW/tools/cts/am62p-full-tree.txt <rec>/npot
$SW/tools/board.sh am62p tally <rec>/npot
```

`am62p-deqp-images.sh` builds only the `TREE_ROWS` rows, and `board.sh`
waits for each one's `img<N> rc=` line in `build.out`. Time: two images, at
most about 11 min each (the 600 s watchdog bound plus the boot).

## Raspberry Pi 4 (V3D): `TREE_ROWS=10,19`

`tools/cts/pi4-full-tree.txt` has 29 rows, each labelled by the
`# image <LABEL>:` line above it.

| Row | Image | Filter | npot cases | Mustpass in row |
|-----|-------|--------|------------|-----------------|
| 10 | `NS` | `negative_api.*`, `state_query.*` | 1 (`generatemipmap_npot_wdt_hgt`) | 642 |
| 19 | `T1` | `texture.completeness.*`, `texture.format.*`, `texture.wrap.*`, `texture.vertex.*` | 17 (10 named + 7 `npot_size*`) | 181 |

Same environment, plus `RUSTUP_TOOLCHAIN=nightly`, stage 1 for `rpi_4b` as
`DEQP_WORK`, `BOARD_UART` and `BOARD_HDMI_TIMINGS` as the lab's:

```sh
export DEQP_WORK=<stage 1 for rpi_4b> RUSTUP_TOOLCHAIN=nightly
$SW/tools/cts/pi4-deqp-images.sh $SW/tools/cts/pi4-full-tree.txt <img> NS T1   # <img> must not exist yet; builds only NS and T1
TREE_IMAGE_DIR=<img> TREE_ROWS=10,19 $SW/tools/board.sh --dry-run pi4 tree $SW/tools/cts/pi4-full-tree.txt <rec>/npot   # first
TREE_IMAGE_DIR=<img> TREE_ROWS=10,19 $SW/tools/board.sh pi4 tree $SW/tools/cts/pi4-full-tree.txt <rec>/npot
$SW/tools/board.sh pi4 tally <rec>/npot
```

Neither `NS` nor `T1` needs `texture-2048.conf`, which only `T5` does. Pass
`TREE_ROWS` to `board.sh` as well as the labels to the builder: without it
the tree waits for `<LABEL>.built` files of rows that are not being built.

## i.MX8MP (Verdin i.MX8M Plus, GC7000UL): `TREE_ROWS=10,22`

`tools/cts/imx8mp-full-tree.txt` has 29 rows. They are all in one image, as
the entries of `CONFIG_DEQP_CASE_TABLE`. Row *i* is run by selector *i* and
named `e<ii>`.

| Row (selector) | Run | Filter | npot cases | Mustpass in row |
|-----|-------|--------|------------|-----------------|
| 10 | `e10` | `texture.completeness.*` | 17 (10 named + 7 `npot_size*`) | 40 |
| 22 | `e22` | `negative_api.*` | 1 (`generatemipmap_npot_wdt_hgt`) | 246 |

With the environment of `tools/cts/README.md` section 2d (`ZEPHYR_WORKSPACE`,
`ZEPHYR_VENV`, `IMX8MP_OUT`, `DEQP_STAGE1`) and `BOARD_BOOT=uboot-tftp` or
`uboot-mmc`:

```sh
$SW/tools/cts/imx8mp-deqp-build.sh wt-$HEAD8 'dEQP-GLES2.info.*' --table $SW/tools/cts/imx8mp-full-tree.txt   # the whole table: the selector picks the row
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin TREE_ROWS=10,22 $SW/tools/board.sh --dry-run imx8mp tree $SW/tools/cts/imx8mp-full-tree.txt <rec>/npot   # first
TREE_IMAGE=$IMX8MP_OUT/wt-$HEAD8/zephyr/zephyr.bin TREE_ROWS=10,22 $SW/tools/board.sh imx8mp tree $SW/tools/cts/imx8mp-full-tree.txt <rec>/npot
$SW/tools/board.sh imx8mp tally <rec>/npot                       # results.tsv, rows.md
```

The image always carries every row, so an existing `wt-$HEAD8` image of the
right commit is reused as it is. Only `TREE_ROWS` limits the boots: two
selector boots, plus `DEQP_FAILS` pages for a row with a failure or a case
without a result. A NotSupported case does not cause a page. The i.MX8MP
has no UART, so the tally is `results.tsv` per entry (begun, passed, failed,
notsup). In the 10/22 rows, notsup is the column that should move by 11.

## All three in one place

| Board | Table | `TREE_ROWS` | Rows (label) |
|-------|-------|-------------|--------------|
| AM62P | `tools/cts/am62p-full-tree.txt` | `7,15` | 7 (`img8`), 15 (`img16`) |
| Pi 4 | `tools/cts/pi4-full-tree.txt` | `10,19` | 10 (`NS`), 19 (`T1`) |
| i.MX8MP | `tools/cts/imx8mp-full-tree.txt` | `10,22` | 10 (`e10`), 22 (`e22`) |
