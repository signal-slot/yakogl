<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP dEQP-GLES3 table image es3-9bc82915, rows 35-51 (entries 34-50)

commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad

Run 2026-11-15, 20:46-21:33 UTC (the board's clock, UTC), on the Toradex Verdin
iMX8M Plus (GC7000UL, Mallow carrier, Torizon, U-Boot one-shot from the eMMC),
item s04 of plan #316 (issue #2; the ES 3.0 record of #1262): rows 35-51 of
`tools/cts/imx8mp-gles3-tree.txt` (`TREE_ROWS=34-50`, table entries 34-50,
counted from 0 as the selector does), one boot per entry, from the one campaign
commit `9bc82915` of `origin/main`. The procedure is
`docs/es3/board-recipe-imx8mp.md`, "Running a GLES3 slice session", steps 1-13.
All 17 entries are `functional.shaders.*` groups (`matrix.mul` ..
`swizzles`).

## What ran

Everything was built on this session's worker from a detached worktree of
`9bc82915` (its own `tools/cts/imx8mp-*.sh`; the table there is
byte-identical to the committed one), with the Zephyr workspace of the
dependency layout (`tools/cts/deps.toml`: zephyr af17c0c8, hal_nxp) and Zephyr
SDK 1.0.1; VK-GL-CTS at the pinned 93bca018 with its external sources:

- stage 1: `tools/cts/imx8mp-deqp-stage1.sh gles3` (`MODULE=gles3`): `rc=0`,
  453 ninja steps, `[453/453] Linking CXX static library
  modules/gles3/libdeqp-gles3-package.a`;
- the table image, console on: `IMX8MP_CONSOLE=1 tools/cts/imx8mp-deqp-build.sh
  --gles3 es3-9bc82915 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt`:
  `imx8mp: 133 table rows -> <out>/es3-9bc82915.table.conf`, `rc=0`,
  `check-deqp-images: the dEQP image recipes build what conformance needs`;
  `.config` has `CONFIG_UART_CONSOLE=y`, `CONFIG_STDOUT_CONSOLE=y`,
  `CONFIG_PRINTK=y`; image build stamp
  `YAKOGL_SAMPLE_TABLE=6c5b8a70 YAKOGL_SAMPLE_TREE=9bc82915`, zephyr
  `af17c0c8aa2a`; footprint 227909632 (the AArch64 header `image_size`
  `board.sh` checks).

`tools/cts/imx8mp-gles3-artefacts.sh es3-9bc82915` (bytes, sha256, file).
`zephyr.bin`, the table fragment and both stage 1 libraries are
byte-identical to slice rows01-13's build on another worker
(`2026-11-imx8mp-gles3-9bc82915-rows01-13/README.md`); only `zephyr.elf`
differs (its debug paths):

```
16443032 f9c80e5207f974eeaefed49decb19766ac3efb87b8d7836d79e89d893d8c4cfc es3-9bc82915/zephyr/zephyr.bin
41869040 2022edb67425d98150bb43153677d4ea8ec0e6edb99a99d96ed0c28863a9d83e es3-9bc82915/zephyr/zephyr.elf
38164 5b7a612379058051e8be1d1983829e7c13254e817289282fe6e38f99bbedb82f es3-9bc82915.table.conf
132944 2fff51840518bd589eb2eaa700bacc120b365448b2ac0ada7bcdccf2bf5bb2e8 deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a
12732236 7b842c954d403450601f75fe245b47581233cc870a778598800065b41a0885cc deqp-stage1-gles3/build/modules/gles3/functional/libdeqp-gles3-functional.a
# CONFIG_DEQP_REPORT_ES2 is not set
# CONFIG_YAKOGL_REPORT_ES2 is not set
Memory region         Used Size  Region Size  %age Used
             RAM:     222568 KB       256 MB     84.90%
```

`tools/cts/imx8mp-gles3-preflight.sh --commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad --out $IMX8MP_OUT`:
nine `ok` lines, `preflight: all ok`. `tools/board.sh imx8mp status` before the
run: lease `none`, `install: ok`, no staged `once-*.bin`, `yako_once` and
`yako_default` empty (the last one-shot named was slice s03's `e26`). The dry
run (`tools/cts/imx8mp-gles3-board.sh --dry-run tree ...` with the same
`TREE_ROWS` and `TREE_IMAGE`) printed the steps of `e34`..`e50`, selectors
34..50, `footprint 227909632`.

The run, with the integration tip's `tools/board.sh` (build 3fecb2f5; the
image is the campaign commit's, the runner the current one, as the campaign
rule says), in two passes over the same output directory:

```
TREE_ROWS=34-50 TREE_IMAGE=$IMX8MP_OUT/es3-9bc82915/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \
  BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt RUN_BOUND=800 BOARD_UART=<local feed bridge> \
  NAMES=0 tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-9bc82915-rows35-51   # pass 1: the 17 rows, 20:46-21:19
  ...  (NAMES unset) tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-9bc82915-rows35-51   # pass 2: the e<NN>-p<page> name pages, 21:20-21:33
tools/cts/imx8mp-gles3-board.sh tally records/imx8mp/es3-9bc82915-rows35-51
```

- `RUN_BOUND=800`: as slice rows01-13 (the imx8mp U-Boot default is 600 s,
  the WDOG1 window itself). No row came near either bound.
- `NAMES=0` on the first pass, then the pages: a page boot runs the whole
  entry again and records six failing-case CRC-16s. 7 page boots (e43 1, e45
  2, e47 4), each `rc=0 verdict=done resets=absent`, 67-90 s from reboot to
  ssh. The names in `results.tsv` come from these pages
  (`yakogl-tools record crc`), not from the console (below).
- `BOARD_UART` named a local feed bridge (`tools/am62p_netboot_uart.py feed`,
  the route for a console the farm hands out as MCP tools): the bytes the
  leased console returned were appended to it; see "Console".
- `BOARD_HOST` and the console's address came from the task's environment
  only.

## Verdict lines

The 17 rows, verbatim from `tools/board.sh imx8mp tally` (every row
`rc=0 verdict=done`, `resets=absent`; `record=` is the SNVS DEQP record
w0..w3; `back_after` is reboot to ssh, `uptime` Linux's at that moment):

```
board.sh imx8mp e34 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087c,000003de,00000000,ffff03de back_after=102s uptime=34s
board.sh imx8mp e35 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0838,0000033c,00000000,ffff033c back_after=91s uptime=26s
board.sh imx8mp e36 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08f8,0000039c,00000000,ffff039c back_after=103s uptime=33s
board.sh imx8mp e37 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b084c,000003c6,00000000,ffff03c6 back_after=96s uptime=21s
board.sh imx8mp e38 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0880,000003e0,00000000,ffff03e0 back_after=105s uptime=28s
board.sh imx8mp e39 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,000003a4,00000000,ffff03a4 back_after=94s uptime=21s
board.sh imx8mp e40 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08ac,00000276,00000000,ffff0276 back_after=90s uptime=37s
board.sh imx8mp e41 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0820,000003b0,00000000,ffff03b0 back_after=95s uptime=24s
board.sh imx8mp e42 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0880,00000160,00000000,ffff0160 back_after=65s uptime=27s
board.sh imx8mp e43 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0804,00010321,00000000,f8380322 back_after=91s uptime=21s
board.sh imx8mp e44 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08b6,0000027b,00000000,ffff027b back_after=64s uptime=24s
board.sh imx8mp e45 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0870,00080250,00000000,bfaf0258 back_after=78s uptime=26s
board.sh imx8mp e46 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a8,000001f4,00000000,ffff01f4 back_after=67s uptime=28s
board.sh imx8mp e47 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0870,00160242,00000000,d8170258 back_after=77s uptime=33s
board.sh imx8mp e48 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0864,00000252,00000000,ffff0252 back_after=76s uptime=30s
board.sh imx8mp e49 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0800,00000220,00000000,ffff0220 back_after=75s uptime=33s
board.sh imx8mp e50 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08d0,00000288,00000000,ffff0288 back_after=76s uptime=30s
```

The 7 page boots (`result=record:DEQP_FAILS`, six CRCs each) are in
`verdicts.txt` with the rows, verbatim from the run logs; every one `rc=0
verdict=done resets=absent`.

## Result

12,295 mustpass cases in the 17 entries, every one begun (the records'
`begun` 12,481 includes 186 cases outside the mustpass list that the
wildcards also select): **12,450 pass, 31 fail, 0 NotSupported, 0 other, 0
without a result** in the records' counts. Of the 31 fails, 27 are named
mustpass cases (below) and 4 are e45 cases outside the list. No row ended at
the watchdog or died: the longest board time was e38's 105 s from reboot to
ssh with Linux's own boot (21-37 s) inside it, so every entry ran in well
under 90 s of the 600 s window (the host estimate of
`imx8mp-gles3-sessions.py` was 106-298 s per entry). Nothing was re-split;
the table is unchanged. Per entry (`merge.py --totals` plus the row's
`back_after`, its `uptime` at the return and its page boots):

| entry | mustpass | begun | pass | fail | n.s. | other | none | back_after | uptime at return | pages |
|---|---|---|---|---|---|---|---|---|---|---|
| e34 shaders.matrix.mul | 990 | 990 | 990 | 0 | 0 | 0 | 0 | 102 s | 34 s | 0 |
| e35 shaders.matrix.negation .. | 828 | 828 | 828 | 0 | 0 | 0 | 0 | 91 s | 26 s | 0 |
| e36 shaders.operator.angle_and_trigonometry .. | 920 | 924 | 924 | 0 | 0 | 0 | 0 | 103 s | 33 s | 0 |
| e37 shaders.operator.binary_operator.bitwise_and_assign_result .. | 966 | 966 | 966 | 0 | 0 | 0 | 0 | 96 s | 21 s | 0 |
| e38 shaders.operator.binary_operator.div_assign_result .. | 992 | 992 | 992 | 0 | 0 | 0 | 0 | 105 s | 28 s | 0 |
| e39 shaders.operator.binary_operator.mod_assign_result .. | 932 | 932 | 932 | 0 | 0 | 0 | 0 | 94 s | 21 s | 0 |
| e40 shaders.operator.binary_operator.right_shift_assign_result .. | 630 | 630 | 630 | 0 | 0 | 0 | 0 | 90 s | 37 s | 0 |
| e41 shaders.operator.common_functions .. | 944 | 944 | 944 | 0 | 0 | 0 | 0 | 95 s | 24 s | 0 |
| e42 shaders.operator.geometric .. | 352 | 352 | 352 | 0 | 0 | 0 | 0 | 65 s | 27 s | 0 |
| e43 shaders.operator.unary_operator .. | 730 | 802 | 801 | 1 | 0 | 0 | 0 | 91 s | 21 s | 1 |
| e44 shaders.preprocessor .. | 625 | 635 | 635 | 0 | 0 | 0 | 0 | 64 s | 24 s | 0 |
| e45 shaders.random.all_features .. | 536 | 600 | 592 | 8 | 0 | 0 | 0 | 78 s | 26 s | 2 |
| e46 shaders.random.conditionals .. | 500 | 500 | 500 | 0 | 0 | 0 | 0 | 67 s | 28 s | 0 |
| e47 shaders.random.scalar_conversion .. | 597 | 600 | 578 | 22 | 0 | 0 | 0 | 77 s | 33 s | 4 |
| e48 shaders.random.trigonometric .. | 561 | 594 | 594 | 0 | 0 | 0 | 0 | 76 s | 30 s | 0 |
| e49 shaders.swizzle_math_operations .. | 544 | 544 | 544 | 0 | 0 | 0 | 0 | 75 s | 33 s | 0 |
| e50 shaders.swizzles .. | 648 | 648 | 648 | 0 | 0 | 0 | 0 | 76 s | 30 s | 0 |
| total | 12295 | 12481 | 12450 | 31 | 0 | 0 | 0 | | | 7 |

"begun" above "mustpass" is the filter matching cases outside the mustpass
list (e36 +4, e43 +72, e44 +10, e45 +64, e47 +3, e48 +33); "none" is
mustpass minus the record's counts, 0 for every row. The second column names
the entry's first group; the full filters are in `results.tsv`.

`tools/cts/imx8mp-gles3-merge.py --totals` on this directory (the rows
0-33 and 51-132 it also lists are not this slice's, `no record: 116 of 133
entries (0-33,51-132), 30515 mustpass cases`):

```
entry  mustpass    begun     pass     fail     n.s.    other     none
e34         990      990      990        0        0        0        0
e35         828      828      828        0        0        0        0
e36         920      924      924        0        0        0        0
e37         966      966      966        0        0        0        0
e38         992      992      992        0        0        0        0
e39         932      932      932        0        0        0        0
e40         630      630      630        0        0        0        0
e41         944      944      944        0        0        0        0
e42         352      352      352        0        0        0        0
e43         730      802      801        1        0        0        0
e44         625      635      635        0        0        0        0
e45         536      600      592        8        0        0        0
e46         500      500      500        0        0        0        0
e47         597      600      578       22        0        0        0
e48         561      594      594        0        0        0        0
e49         544      544      544        0        0        0        0
e50         648      648      648        0        0        0        0
total     42810    12481    12450       31        0        0    30515
```

### The failing names

`results.tsv` holds one entry per recorded CRC: 31 for the 31 fails.

- **e43, 1 fail**: `shaders.precision.int.highp_div_fragment`.
- **e45, 8 fails, 4 named**: `shaders.random.all_features.fragment.10`,
  `.17`, `.42` and `shaders.random.basic_expression.fragment.38`. The other
  four stay as CRCs (`crc:df69`, `crc:fadd`, `crc:c152`, `crc:0ba1`): `df69`
  resolves to no name of the gles2+gles3 mustpass lists at all, and `fadd`,
  `c152` and `0ba1` only to names outside e45's filter (`fragment_out`,
  `shaders.preprocessor`, `fbo.render`, `vertex_arrays` and dEQP-GLES2
  cases). e45's filter begins 600 cases for 536 mustpass ones, so these are
  four of the 64 non-mustpass `shaders.random.*` cases its wildcards also
  select; the 4 named fails are the mustpass ones.
- **e47, 22 fails, a CRC-16 collision in one of them.** 21 are single names,
  all `shaders.random.texture.{fragment,vertex}.*`. CRC `3139` (page e47-p0,
  the row's third fail) resolves to two names inside e47's filter,
  `shaders.random.scalar_conversion.combined.87` and
  `shaders.random.texture.fragment.143`, so `board.sh` prints the pair, and
  `merge.py --notpass` lists both (it then counts 23 names for 22 fails).
  The position resolves it: the pages hold the fails in run order, and this
  one comes after `texture.fragment.1` and `texture.fragment.139` (fails 1
  and 2) and before `texture.fragment.29`, where only
  `texture.fragment.143` fits; no other fail of the row is a
  `scalar_conversion` case. So the 22 fails are 22 `random.texture` cases,
  and `scalar_conversion.combined.87` passes. `results.tsv` is left as
  `board.sh tally` wrote it.

The 7 page records of a row all carry their row's fail count (w0's low byte:
e43 1, e45 8, e47 22 = `0x16`): the failure sets were stable across the
reruns.

## Console: what the bridge delivered

The image was built with the console on and `tools/board.sh` captured
`BOARD_UART` for every boot (`console.raw`, `console.ts`, `console.txt` in
each run directory; `e<NN>.console.txt` here), but the leased console
delivered **no text at all** during the session, as in slice rows01-13: one
NUL byte (`\x00`) at each board reset and nothing else, 24 bytes for the 24
boots. The lease's serial console was `connected` (RFC 2217, 115200 8N1, the
adapter's settings) the whole time, and a bare Enter sent to it before the
run got nothing back.

What follows from it:

- `tools/cts/imx8mp-console-scan.py` (`console-scan.txt`) sees 1 line (the
  NUL), 0 boot banners and 0 rows in every capture, so **GPU recoveries were
  not observable**: `recoveries 0` is not shown for any row. `board.sh` does
  not count recoveries on this board (`resets=absent`), and the SNVS record
  holds none. The rows are complete records (`begun` at or above the
  mustpass count, `verdict=done`), but the recipe's zero-recovery proof is
  missing for all 17; this is reported, not claimed.
- No per-entry board time from the scan: the times above are `back_after`
  and `uptime`, from `meta.txt` (`t_reboot`, `t_back`, `up`).
- No row had NotSupported or other statuses, so the console's missing
  per-case lines lose no non-Fail names here; failing names came from the
  CRC pages, at 7 extra boots.

## Resets and reboots outside the rows

Every boot is one reboot by design (into the image, WDOG1/PSCI reset back
to Linux); `board.sh` accepted each return only from a new boot (`uptime`
below the time since the reboot). Between the boots the board stayed in
Linux: each boot's prechecks passed first time (`who` sessions 0 for all
24), no boot was refused (rc 3), none took a retry and none waited for the
board lease. After the last boot the board's uptime matched the time since
that boot's return (193 s at board clock 1794778524, booted 1794778331;
e47-p3's `t_back` 1794778354 minus its `uptime` 22 = 1794778332), with the
lease `none`, the staging directory empty and `yako_once` empty. The
image's 600 s WDOG1 window was never reached (longest `back_after` 105 s).

## Files

- `results.tsv`, `rows.md`: `tools/board.sh imx8mp tally` of the output
  directory (index, filter, begun, passed, failed, notsup, other, noresult,
  failing names; rows 0-33 and 51-132 are `-`, not run in this slice), with
  their REUSE `.license` sidecars.
- `verdicts.txt`: every `board.sh imx8mp e..` verdict line of the two run
  logs, rows and pages.
- `e34.console.txt` .. `e50.console.txt`: each row's captured console
  (header lines, then the capture: one NUL line each).
- `console-scan.txt`: the scan over the 17 row directories.

Raw captures (`console.raw`, `console.ts`) and the run directories stay
under `records/` (gitignored). Addresses: `board.sh` redacted the copies and
every file here was read again for logins, addresses, ports and device
paths before the commit; `BOARD_HOST` and `BOARD_UART` are named, never
their values.
