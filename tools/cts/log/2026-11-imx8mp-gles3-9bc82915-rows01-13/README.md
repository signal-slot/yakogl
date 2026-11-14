<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP dEQP-GLES3 table image es3-9bc82915, rows 1-13 (entries 0-12)

commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad

Run 2026-11-14, 19:47-22:19 UTC (the board's clock, UTC), on the Toradex Verdin
iMX8M Plus (GC7000UL, Mallow carrier, Torizon 6.6.142, U-Boot one-shot from
the eMMC), item s01 of plan #316 (issue #2; the ES 3.0 record of #1262): the
first 13 rows of `tools/cts/imx8mp-gles3-tree.txt` (`TREE_ROWS=0-12`, table
entries 0-12, counted from 0 as the selector does), one boot per entry, from
the one campaign commit `9bc82915` of `origin/main`. The procedure is
`docs/es3/board-recipe-imx8mp.md`, "Running a GLES3 slice session", steps 1-13.

## What ran

Everything was built from a detached worktree of `9bc82915` (its own
`tools/cts/imx8mp-*.sh`; the table there is byte-identical to the committed
one), with the registered Zephyr workspace (zephyr af17c0c8, hal_nxp) and
Zephyr SDK 1.0.1:

- stage 1: `tools/cts/imx8mp-deqp-stage1.sh gles3` (VK-GL-CTS 93bca018 with
  its external sources, `MODULE=gles3`): `rc=0`, 453 ninja steps, 5 min on
  4 cores;
- the table image, console on: `IMX8MP_CONSOLE=1 tools/cts/imx8mp-deqp-build.sh
  --gles3 es3-9bc82915 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt`:
  `imx8mp: 133 table rows -> <out>/es3-9bc82915.table.conf`, `rc=0`,
  `check-deqp-images: the dEQP image recipes build what conformance needs`;
  `.config` has `CONFIG_UART_CONSOLE=y`, `CONFIG_STDOUT_CONSOLE=y`,
  `CONFIG_PRINTK=y`, `# CONFIG_DEQP_REPORT_ES2 is not set`,
  `# CONFIG_YAKOGL_REPORT_ES2 is not set`; image build stamp
  `YAKOGL_SAMPLE_TABLE=6c5b8a70 YAKOGL_SAMPLE_TREE=9bc82915`, zephyr `af17c0c8aa2a`;
  linker `RAM: 222568 KB 256 MB 84.90%`, footprint 227909632 (the
  AArch64 header `image_size` `board.sh` checks).

`tools/cts/imx8mp-gles3-artefacts.sh es3-9bc82915` (bytes, sha256, file; the
sha256 of `zephyr.bin`/`zephyr.elf` differs per build host, the embedded build
paths, so it is this session's, not a campaign constant):

```
16443032 f9c80e5207f974eeaefed49decb19766ac3efb87b8d7836d79e89d893d8c4cfc es3-9bc82915/zephyr/zephyr.bin
41869168 56aa62f3e89524995ebb83d0bfe96f962276a344604b34568d377eaa1671d9ea es3-9bc82915/zephyr/zephyr.elf
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
`yako_default` empty. The dry run (`tools/cts/imx8mp-gles3-board.sh --dry-run
tree ...` with the same `TREE_ROWS` and `TREE_IMAGE`) printed the steps of
`e00`..`e12`, selectors 0..12, `footprint 227909632`.

The run, with the integration tip's `tools/board.sh` (build 715c586b; the
image is the campaign commit's, the runner the current one, as the campaign
rule says), in two passes over the same output directory:

```
TREE_ROWS=0-12 TREE_IMAGE=$IMX8MP_OUT/es3-9bc82915/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \
  BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt RUN_BOUND=800 BOARD_UART=<local feed bridge> \
  NAMES=0 tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-9bc82915-rows01-13   # pass 1: the 13 rows, 19:47-20:08
  ...  (NAMES unset) tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-9bc82915-rows01-13   # pass 2: the e<NN>-p<page> name pages, 20:09-22:19
tools/cts/imx8mp-gles3-board.sh tally records/imx8mp/es3-9bc82915-rows01-13
```

- `RUN_BOUND=800`: the imx8mp U-Boot default is 600 s, the WDOG1 window
  itself, so a row that ran out its window would have read as rc 5 before the
  board was back (`board.sh` counts the bound from the reboot); 800 is the
  AM62P dEQP rule (600 + 60 + boot). No row came near either bound.
- `NAMES=0` on the first pass, then the pages: a page boot runs the whole
  entry again (`main.cpp` `want &= 0xff`) and records six failing-case
  CRC-16s, so the 13 rows were recorded first and the names fetched after:
  76 page boots (e02 4, e03 2, e04 6, e05 10, e06 20, e07 1, e08 13, e09 19,
  e12 1), each `rc=0 verdict=done resets=absent`, 55-104 s from reboot to
  ssh. The names in `results.tsv` come from these pages (`yakogl-tools record
  crc`), not from the console (below).
- `BOARD_UART` named a local feed bridge (`tools/am62p_netboot_uart.py feed`,
  the documented route for a console the farm hands out as MCP tools): the
  bytes the leased console returned were appended to it; see "Console".
- `BOARD_HOST` and the console's address came from the task's environment
  only.

## Verdict lines

The 13 rows, verbatim from `tools/board.sh imx8mp tally` (every row
`rc=0 verdict=done`, `resets=absent`; `record=` is the SNVS DEQP record
w0..w3; `back_after` is reboot to ssh, `uptime` Linux's at that moment):

```
board.sh imx8mp e00 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0814,0000012a,00000000,ffff012a back_after=51s uptime=25s
board.sh imx8mp e01 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b082e,000002b7,00000000,ffff02b7 back_after=66s uptime=19s
board.sh imx8mp e02 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b087a,001802c5,00000000,6bd902dd back_after=104s uptime=29s
board.sh imx8mp e03 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08e0,0007013d,0000004c,ff380190 back_after=70s uptime=27s
board.sh imx8mp e04 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08cc,00220100,00000064,ce450186 back_after=69s uptime=28s
board.sh imx8mp e05 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08de,003c00a5,0000002e,4d7c010f back_after=64s uptime=27s
board.sh imx8mp e06 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08ce,007400f2,00000021,b0750187 back_after=70s uptime=23s
board.sh imx8mp e07 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b089a,000100ec,00000000,163400ed back_after=53s uptime=29s
board.sh imx8mp e08 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08dc,004b0138,0000000b,a994018e back_after=65s uptime=30s
board.sh imx8mp e09 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08e0,007200dd,00000041,c2020190 back_after=79s uptime=24s
board.sh imx8mp e10 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0848,000001c4,00000000,ffff01c4 back_after=65s uptime=28s
board.sh imx8mp e11 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08b4,000000fa,00000000,ffff00fa back_after=55s uptime=26s
board.sh imx8mp e12 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0848,000201c2,00000000,ef6e01c4 back_after=65s uptime=24s
```

The 76 page boots (`result=record:DEQP_FAILS`, six names each) are in
`verdicts.txt` with the rows, verbatim from the run log; every one `rc=0
verdict=done resets=absent`.

## Result

5,291 mustpass cases in the 13 entries, every one begun (the record's
`begun` 5,367 includes 76 cases outside the mustpass list that the
wildcards also select): **4,603 pass, 433 fail, 331 NotSupported, 0 other,
0 without a result**. No row ended at the watchdog or died: the longest
board time was e02's 104 s from reboot to ssh with Linux's own boot
(19-30 s) inside it, so every entry ran in well under 90 s of the 600 s
window. Nothing was re-split; the table is unchanged. Per entry
(`merge.py --totals` plus the row's `back_after`, its `uptime` at the
return and its page boots):

| entry | mustpass | begun | pass | fail | n.s. | other | none | back_after | uptime at return | pages |
|---|---|---|---|---|---|---|---|---|---|---|
| e0 attribute_location | 274 | 298 | 298 | 0 | 0 | 0 | 0 | 51 s | 25 s | 0 |
| e1 buffer | 695 | 695 | 695 | 0 | 0 | 0 | 0 | 66 s | 19 s | 0 |
| e2 clipping .. dither | 732 | 733 | 709 | 24 | 0 | 0 | 0 | 104 s | 29 s | 4 |
| e3 draw, draw_buffers_indexed | 398 | 400 | 317 | 7 | 76 | 0 | 0 | 70 s | 27 s | 2 |
| e4 fbo.api, fbo.blit.conversion.r* | 390 | 390 | 256 | 34 | 100 | 0 | 0 | 69 s | 28 s | 6 |
| e5 fbo.blit.conversion.rgba*, fbo.blit.* | 240 | 271 | 165 | 60 | 46 | 0 | 0 | 64 s | 27 s | 10 |
| e6 fbo.color, fbo.completeness.{attachment_combinations,layer} | 391 | 391 | 242 | 116 | 33 | 0 | 0 | 70 s | 23 s | 20 |
| e7 fbo.completeness.renderable.renderbuffer | 230 | 237 | 236 | 1 | 0 | 0 | 0 | 53 s | 29 s | 1 |
| e8 fbo.completeness.renderable.texture .. fbo.invalidate | 387 | 398 | 312 | 75 | 11 | 0 | 0 | 65 s | 30 s | 13 |
| e9 fbo.msaa, fbo.multiview, fbo.render, fbo.stencil, fence_sync | 400 | 400 | 221 | 114 | 65 | 0 | 0 | 79 s | 24 s | 19 |
| e10 fragment_ops.blend.default_framebuffer.equation_src_func_dst_func.{max,min,add_*,reverse_*} | 452 | 452 | 452 | 0 | 0 | 0 | 0 | 65 s | 28 s | 0 |
| e11 fragment_ops.blend.default_framebuffer.{equation_src_func_dst_func.subtract_*,rgb_equation_alpha_equation} | 250 | 250 | 250 | 0 | 0 | 0 | 0 | 55 s | 26 s | 0 |
| e12 fragment_ops.blend.default_framebuffer.rgb_func_alpha_func, fbo_srgb.{max,min} | 452 | 452 | 450 | 2 | 0 | 0 | 0 | 65 s | 24 s | 1 |
| total | 5291 | 5367 | 4603 | 433 | 331 | 0 | 0 | | | 76 |

"begun" above "mustpass" is the filter matching cases outside the mustpass
list (e0 +24, e2 +1, e3 +2, e5 +31, e7 +7, e8 +11); "none" is mustpass
minus the record's counts, 0 for every row. The second column names the
entry's first groups; the full filters are in `results.tsv`.

`tools/cts/imx8mp-gles3-merge.py --totals` on this directory (the rows
13-132 it also lists are not this slice's, `no record: 120 of 133 entries
(13-132), 37519 mustpass cases`):

```
entry  mustpass    begun     pass     fail     n.s.    other     none
e0          274      298      298        0        0        0        0
e1          695      695      695        0        0        0        0
e2          732      733      709       24        0        0        0
e3          398      400      317        7       76        0        0
e4          390      390      256       34      100        0        0
e5          240      271      165       60       46        0        0
e6          391      391      242      116       33        0        0
e7          230      237      236        1        0        0        0
e8          387      398      312       75       11        0        0
e9          400      400      221      114       65        0        0
e10         452      452      452        0        0        0        0
e11         250      250      250        0        0        0        0
e12         452      452      450        2        0        0        0
total     42810     5367     4603      433      331        0    37519
```

### The failing names: what the pages could and could not tell

`results.tsv` names 433 failing cases, one per recorded CRC. Two things in
them are the page mechanism's, not the board's:

- **e8, three fails outside the mustpass list.** `crc:118d`, `crc:23fb` and
  `crc:579a` stay as CRCs: `118d` resolves only to names outside the row's
  filter (two `shaders.*` cases), `23fb` and `579a` to no dEQP-GLES3 name at
  all. e8's filter begins 398 cases for 387 mustpass ones, so these are three
  of the 11 non-mustpass `fbo.completeness.*` cases its wildcards also
  select; the 72 named fails are the mustpass ones.
- **e9, a CRC-16 collision between two of its failing cases.** CRC `ec9b`
  resolves to two names inside e9's filter, `fbo.msaa.2_samples.rgb5_a1`
  and `fbo.render.recreate_color.tex2d_rg8ui_depth_stencil_rbo_depth24_stencil8`,
  so `board.sh` prints the pair; and the pair appears twice in the list,
  because two pages recorded that CRC: e09-p1 (fails 6-11 of the run, where
  dEQP's order is still in `fbo.msaa.*`) and e09-p9 (fails 54-59, inside
  `fbo.render.*`). Both cases fail, then: the first occurrence is the
  `fbo.msaa` case, the second the `fbo.render` case, and the 114 names are
  the 114 fails (112 single names plus these two). `merge.py --check`
  reports both names as `case listed twice`; `results.tsv` is left as
  `board.sh tally` wrote it, the position above resolves the pair.

- **e2, one more collision pair, resolved by the pattern.** CRC `f305`
  (page e02-p3) resolves to `default_vertex_attrib.mat4.vertex_attrib_4f`
  and `default_vertex_attrib.mat4x2.vertex_attrib_4fv`, both inside e2's
  filter, so the pair stands in the list once. e2's 24 fails are exactly the
  six non-square matrix types of `default_vertex_attrib` (`mat2x3`, `mat2x4`,
  `mat3x2`, `mat3x4`, `mat4x2`, `mat4x3`) in all four variants (`1f`, `1fv`,
  `4f`, `4fv`), 6 x 4 = 24, with the other 23 named singly: the pair is
  `mat4x2.vertex_attrib_4fv`, and `mat4.vertex_attrib_4f` passes (every
  square type passes).

The other 10 rows' names are one-to-one with their fail counts. The 19
page records of e9 all carry the same fail count (114, w0's low byte), as
do every other row's pages with their row's count: the failure sets were
stable across the 76 reruns.

## Console: what the bridge delivered

The image was built with the console on and `tools/board.sh` captured
`BOARD_UART` for every boot (`console.raw`, `console.ts`, `console.txt` in
each run directory; `e<NN>.console.txt` here), but the leased console
delivered **no text at all** during the session: one NUL byte (`\x00`) at
each board reset and nothing else, 89 bytes for the 89 boots. The lease's
serial console was `connected` (RFC 2217, 115200 8N1, the adapter's
settings) the whole time. Before the run a marker line written to the
board's UART3 device from Linux (as root, three times, then once a second
for 15 s) did not arrive either, and a bare Enter sent to the console got no
login prompt back although `agetty` was active on that UART at 115200. The
earlier probe items of plan #316 (imx8mpprobe3-5) had found this bridge held
by a client outside the farm; whether that client still holds its data, or
the UART_3 wiring is open, cannot be told from here. The board was not
touched beyond the rows for this.

What follows from it:

- `tools/cts/imx8mp-console-scan.py` (`console-scan.txt`) sees 0 lines, 0
  boot banners and 0 rows in every capture, so **GPU recoveries were not
  observable**: `recoveries 0` is not shown for any row. `board.sh` does not
  count recoveries on this board (`resets=absent`), and the SNVS record
  holds none. The rows are complete records (`begun` at or above the
  mustpass count, `verdict=done`), but the recipe's zero-recovery proof is
  missing for all 13; this is reported, not claimed.
- No per-entry board time from the scan: the times above are `back_after`
  and `uptime`, from `meta.txt` (`t_reboot`, `t_back`, `up`).
- The non-Pass statuses other than Fail (331 NotSupported in e3-e6, e8 and
  e9) have no names: a console or a `cases.tsv` would give them, the record
  gives their count only. `merge.py --notpass` lists them as unresolved for
  these rows.
- Failing names came from the CRC pages instead, at 76 extra boots.

## Resets and reboots outside the rows

Every boot is one reboot by design (into the image, WDOG1/PSCI reset back
to Linux); `board.sh` accepted each return only from a new boot (`uptime`
below the time since the reboot). Between the boots the board stayed in
Linux: each boot's prechecks (`systemctl is-system-running`, `sudo -n`,
`who`, no foreign lease, no pending one-shot, no staged image, both kexec
flags 0) passed first time, no boot was refused (rc 3) and none took a
retry, and no boot waited for the board lease. After the last boot the
board's uptime matched the time since that boot's return (63 s at 22:19:09
UTC, booted 1794694686 = e12-p0's `t_back` 1794694710 minus its `uptime`
24), with the lease file gone, the staging directory empty and `yako_once`
empty. The image's `CONFIG_GC_BOARD_WDOG_SECONDS=600` window was never
reached (longest `back_after` 104 s).

## Files

- `results.tsv`, `rows.md`: `tools/board.sh imx8mp tally` of the output
  directory (index, filter, begun, passed, failed, notsup, other, noresult,
  failing names; rows 13-132 are `-`, not run in this slice), with their
  REUSE `.license` sidecars (the files are the tool's output and carry no
  header of their own, as the NanoPC-T6 slices' `results.tsv`).
- `verdicts.txt`: every `board.sh imx8mp e..` verdict line of the run log,
  rows and pages (each printed twice: by the run and by the tally).
- `e00.console.txt` .. `e12.console.txt`: each row's captured console
  (header lines, then the capture: NUL bytes only).
- `console-scan.txt`: the scan over the 13 row directories.

Raw captures (`console.raw`, `console.ts`) and the run directories stay
under `records/` (gitignored). Addresses: `board.sh` redacted the copies and
every file here was read again for logins, addresses, ports and device
paths before the commit; `BOARD_HOST` and `BOARD_UART` are named, never
their values.
