<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP, first OpenGL ES 3.0 run (issues #28 and #24)

Board: Verdin iMX8M Plus (Vivante GC7000UL), 2026-10-31, every run through
the kexec route from Torizon (`BOARD_BOOT=kexec-dev`, so each run is a DEV
run, never a row; the eMMC was not written, no recovery boot). Main
`d1b24389`; the runner `tools/board.sh` carries the changes of the commit
that holds this record (`--set=NAME=VALUE`, an ssh link check when ping
fails, `BOARD_STOP_CONTAINER`, `NAME_PAGES`); no `src/`, `driver/` or `capi/`
file changed.

How the board was made ready: the Torizon container `sarta` holds
`/dev/galcore` and Torizon's compose service starts it again at every boot,
so the kexec loader could not unload `galcore` ("Module galcore is in use")
and the jump failed. `BOARD_STOP_CONTAINER=sarta` stops it before each jump.
At the end `sarta` was running again with its restart policy
(`unless-stopped`) as found.

## 1. gl_test, issue #28

Image: `tools/cts/imx8mp-gltest-build.sh gt196 196` (sweep image, cases 196
to 243, ES 3.0 reporting, `CONFIG_GC_BOARD_WDOG_SELF_ARM=y`, sha256
`7324d854e007c958…`), one case per boot with the selector (`SEL` = case -
196). Each case twice; the verdict is the CASE record. `gl_test.tsv`.

| Case | Name | Run 1 | Run 2 | Checks |
|---:|---|---|---|---:|
| 196 | occl | PASS | PASS | 2 |
| 202 | blitconv | PASS | PASS | 12 |
| 203 | tfcap | PASS | PASS | 9 |
| 204 | srgb | PASS (texture half) | PASS (texture half) | 2 |
| 205 | mappbo | PASS | PASS | 15 |

204: the render half of the case is compiled out for this GPU
(`gl_test.c`: "this GPU refuses an sRGB target by name"), so the pass is the
sRGB texel decode only; the AM62P's 8 checks are not comparable.
A first run of the whole 48-case sweep without a selector ended with an
empty pass-bit field (SWEEP counters record, `00000000`); it was not
analysed, the per-case runs above are the result.

## 2. dEQP-GLES3, issue #24

Stage 1: VK-GL-CTS `93bca01` with the repository's two patches
(`tools/cts/imx8mp-deqp-stage1.sh gles3`). Image: `imx8mp-deqp-build.sh
--gles3 es3-<hash8> 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt`,
sha256 `061fe0b9282335ef…` (`check-deqp-images`: ok), 133 table entries (the
table already in the tree; it covers the 42,810 mustpass cases exactly
once, `check-imx8mp-gles3-tree.py`). Run: `tools/board.sh imx8mp tree`
with `NAMES=0`, one boot per entry, `results.tsv` (committed here).

| | cases |
|---|---:|
| begun | 44,635 |
| Pass | 37,140 |
| Fail | 5,274 |
| NotSupported | 2,197 |
| other (QualityWarning or another status; the SNVS record does not split it) | 24 |
| no result (begun, no status) | 0 |

The 133 entries select 1,825 cases that are not in the mustpass list (the
wildcards match dEQP's whole tree), so the counts are over cases begun, not
over the 42,810 mustpass cases; the split needs the case names, which this
run did not collect for all failures (below).

Hangs and resets: none. All 133 entries returned a complete DEQP record
(begun at least the entry's mustpass count in every one), the longest return
was 232 s of the 600 s watchdog window (entry 26), no entry ended at the
watchdog; the 15 runs whose record carried a reset count read 0.

Entries with a failure, other status or NotSupported are in `results.tsv`
(columns: index, filter, begun, passed, failed, notsup, other, noresult).

Failing case names: **not collected for the whole tree.** The record gives
six failing-case CRCs per extra boot (about 75 s), so the roughly 5,300
failures would need about 880 boots; a first attempt (2 pages per failing
entry) was lost with the machine's dependency directory more than once (the
worker restarted while the run was going), and only entry 13 page 0 was read
(`names.tsv`, six names, all `fragment_ops.blend.fbo_srgb`). The image is
unchanged on the board's home directory (`~/es3/es3-4e6a088e.bin`, not in the
tree) so a later session can run `tools/board.sh --set=NAME_PAGES=<n> ...
tree` for chosen entries (`TREE_ROWS`); with the full 40 pages per entry
the cap is 240 names an entry. Until then the failure list by case name for
the i.MX8MP is open (issue #24).

## 3. Reading

See `tools/cts/RESULTS.md` (i.MX8MP ES 3.0 rows) for the groups by cause.
