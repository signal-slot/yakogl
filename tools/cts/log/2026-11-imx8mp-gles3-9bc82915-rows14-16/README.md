<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP dEQP-GLES3 table image es3-9bc82915, rows 14-16 (entries 13-15)

commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad

Measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL), not the Verdin i.MX8MP.

Run 2026-11-27, measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2 (crash-route kexec): the slice
`2026-11-imx8mp-gles3-9bc82915-rows14-16`, rows 14-16 of `tools/cts/imx8mp-gles3-tree.txt` (entries 13-15, counted from 0 as the
selector does), one crash-route kexec per entry, from the campaign commit above. The Toradex Verdin iMX8M
Plus is off the farm; by the owner's decision of 2026-11-27 these kexec-dev runs (board.sh's `DEV`, "never
a row") are rows of this campaign. The procedure is `docs/es3/board-recipe-imx8mp.md`, "Running a GLES3
slice on the Armadillo-X2".

Records (gitignored): `records/x2/20261127-1410-es3-9bc82915-rows14-16-c7bda171`.

## What ran

Run 2026-11-27, 14:11-14:16 UTC (the board's clock): rows 14-16 of `tools/cts/imx8mp-gles3-tree.txt`
(`TREE_ROWS=13-15`), one boot per entry.

`tools/cts/imx8mp-gles3-artefacts.sh x2/es3-9bc82915` (bytes, sha256, file below `$IMX8MP_OUT`):

```
16451216 c7bda171bd69b5d17b00fd702eae07682ee673ffd968f4b7a77a67448243f1c9 x2/es3-9bc82915/zephyr/zephyr.bin
41880040 a90f67312703621f8f5d0fd79e5834a21012526174d908bd280b1bca24533b24 x2/es3-9bc82915/zephyr/zephyr.elf
38164 5b7a612379058051e8be1d1983829e7c13254e817289282fe6e38f99bbedb82f x2/es3-9bc82915.table.conf
132976 e59424502329afb28d91ddb2614f6aa9a2a1bd572f99e1b43499cc5032e52e75 deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a
12734868 91aaa58e707356abc70452af1f5dbfdca2e3d7a990bf911d99a5aa1ae0015f53 deqp-stage1-gles3/build/modules/gles3/functional/libdeqp-gles3-functional.a
# CONFIG_DEQP_REPORT_ES2 is not set
# CONFIG_YAKOGL_REPORT_ES2 is not set
CONFIG_BOARD="armadillo_x2"
CONFIG_GC_BOARD_WDOG_SELF_ARM=y
Memory region         Used Size  Region Size  %age Used
           FLASH:           0 B          0 B
             RAM:     222576 KB       256 MB     84.91%
```

`tools/cts/imx8mp-gles3-preflight.sh --board x2 --commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad --out $IMX8MP_OUT`: 13 `ok` lines, `preflight: all ok`.

The run, knob names only (values come from the environment; `$REC` is
`records/x2/<YYYYMMDD-HHMM>-es3-9bc82915-rows14-16-<image sha8>`):

```
TREE_ROWS=13-15 TREE_IMAGE=$IMX8MP_OUT/x2/es3-9bc82915/zephyr/zephyr.bin IMX8MP_BOARD=x2 \
  BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt BOARD_HOST=$BOARD_HOST [BOARD_UART=$BOARD_UART] \
  tools/cts/imx8mp-gles3-board.sh tree $REC
IMX8MP_BOARD=x2 tools/cts/imx8mp-gles3-board.sh tally $REC
```

`BOARD_HOST` and `BOARD_UART` are named, never their values.

## Verdict lines

Verbatim from `tools/board.sh x2` (`back_after` is the jump to ssh; `DEV` and "never a row" are board.sh's words, rows here by the owner's decision of 2026-11-27):

```
board.sh x2 e13 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP resets=absent record=594b0844,01ae0014,00000000 back_after=61s (kexec-dev: a development run, never a row)
board.sh x2 e14 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP resets=absent record=594b08b4,00e0001a,00000000 back_after=41s (kexec-dev: a development run, never a row)
board.sh x2 e15 rc=0 verdict=done boot=kexec-dev DEV boot_verdict=complete result=record:DEQP resets=absent record=594b0886,01c20021,00000000 back_after=65s (kexec-dev: a development run, never a row)
```

## Per entry

| entry | rc | verdict | resets | back_after | uptime | pages | begun | pass | fail | n.s. | other | none |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| e13 | 0 | done | absent | 61s | - | 0 | 450 | 20 | 430 | 0 | 0 | - |
| e14 | 0 | done | absent | 41s | - | 0 | 250 | 26 | 224 | 0 | 0 | - |
| e15 | 0 | done | absent | 65s | - | 0 | 483 | 33 | 450 | 0 | 0 | - |

## Not-pass list

The failing names `results.tsv` holds per entry (`imx8mp-gles3-merge.py --notpass` reads them from
there; the other statuses come from the per-case sources beside it):

- e13: none named
- e14: none named
- e15: none named

## Console

Not captured by `tools/board.sh` (`BOARD_UART` unset): the X2's UART2 reached this session only through
connected's serial-console lease of the console resource (target `x2-console`), which holds the bridge's one client
connection, so `board.sh` had nothing to open. That console was read, never written, while the rows ran. It
delivered text, unlike the Verdin's bridge in the three earlier slices: the crash-route jump (`Starting
crashdump kernel...`), the image's stamp `sample: build tree=9bc82915 table=6c5b8a70`, `deqp: selector 13`,
the entry's case filter, and one `deqp: Fail dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb...` line with
`Fail (Image compare failed)` per case read. Only the start of e13 and Linux's boot after e15 were read
(about 16 KB of 550 KB); the rest was not copied out, so there is no `console-scan.txt` and GPU recoveries
were not counted. `resets=absent` is `board.sh`'s; the zero-recovery proof of the recipe is missing for
every row. This is reported, not claimed.

## Session notes

- **Board.** The Armadillo-X2 stands in for the Verdin iMX8M Plus, which has been off the farm since 2026-11-16
  (owner's rule of 2026-11-27). The image is the campaign commit's, built for `armadillo_x2/mimx8ml8/a53`
  (`IMX8MP_BOARD=x2`, `CONFIG_GC_BOARD_WDOG_SELF_ARM=y`) from a clean worktree of `9bc82915`. The runner
  and the loader `kexec_zephyr` and `snvs_lpgpr` come from the current tip. `board.sh` read the image's
  build as `YAKOGL_SAMPLE_TREE=9bc82915`, `YAKOGL_SAMPLE_TABLE=6c5b8a70`, Zephyr `af17c0c8aa2a`, footprint
  227,917,824 bytes (inside the 256 MiB crash window).
- **Runner knobs** (`tools/cts/imx8mp-gles3-board.sh`): `X2_PING=ssh` and `X2_KEEP_KEY=1`, besides the
  `TREE_ROWS=13-15 NAMES=0` above. The host reached the board through an ssh forward only, with no ICMP, so
  `board.sh x2`'s `ping` was `tools/cts/x2-ping/ping`, which answers when the board's sshd does. The host's
  key lived in the board's tmpfs `/root`, so the wrapper persisted the live `authorized_keys` for the session
  and afterwards put the persisted copy back byte for byte. The wrapper printed `X2_KEEP_KEY: the persisted
  /root/.ssh/authorized_keys is back as it was`, and its sha256 is the same as before the session.
- **Boot timeline per row** (`meta.txt`): e13 `ping lost 9s, ssh back 60s`, Linux uptime 22.7 s at the read;
  e14 `9s / 40s`, 17.3 s; e15 `9s / 65s`, 21.6 s. Every row ended with the image's own reset; no watchdog end,
  no row near `board.sh`'s 1500 s bound. LPGPR1..3 were armed with `5345000d`..`5345000f` before each jump
  (LPGPR0, U-Boot's boot count, untouched).
- **No pass 2 (failure names).** The X2 record names four failing cases a page, and `board.sh` reads at most
  40 pages, which is 160 names a row. Each row here fails more: e13 430, e14 224, e15 450. Naming them would
  name only part of each row and cost about 120 more boots (about 2 h). As with e25, e26 and e29 of
  `rows17-34`, the rows keep their counts without names. Name them from a captured console
  (`NAMES=0` with `BOARD_UART`, `tools/cts/deqp-uart-tally.py`) or with a split entry.
- **Case list.** The tally ran with `DEQP_CASELIST` set to the full dEQP-GLES3 case list: 48,306 names,
  `dEQP-GLES3-cases.txt` of a `DEQP_TARGET=null` build of VK-GL-CTS 93bca01. The three entries' filters
  select 450, 250 and 483 cases of it, all in `tools/cts/gles3-mustpass.txt`, so no CRC of this slice can
  resolve outside the mustpass and no `crc-resolved.tsv` is needed. `results.tsv` is byte-identical with and
  without the list.
- **What the counts say.** 1,183 cases begun, exactly the slice's mustpass count, with no shortfall. 79 Pass,
  1,104 Fail (93.3 %), 0 NotSupported, 0 other, 0 without a result. The recipe's host prediction for this
  slice ("nearly all cases Fail by image compare", blend `fbo_srgb`) holds. Every Fail read on the console is
  `Image compare failed`.

`imx8mp-gles3-merge.py --check` reads the verdict lines above; `results.tsv` and `rows.md` of the
same tally stand beside this README.
