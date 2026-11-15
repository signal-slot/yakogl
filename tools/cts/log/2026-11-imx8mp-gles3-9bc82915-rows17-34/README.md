<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP dEQP-GLES3 table image es3-9bc82915, rows 17-34 (entries 16-33)

commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad

Run 2026-11-15, 10:10-14:45 UTC (the board's clock, UTC), on the Toradex Verdin
iMX8M Plus (GC7000UL, Mallow carrier, Torizon 6.6.142, U-Boot one-shot from
the eMMC), item s03 of plan #316 (issue #2; the ES 3.0 record of #1262): rows
17-34 of `tools/cts/imx8mp-gles3-tree.txt` (`TREE_ROWS=16-33`, table entries
16-33, counted from 0 as the selector does), one boot per entry, from the one
campaign commit `9bc82915` of `origin/main`. The procedure is
`docs/es3/board-recipe-imx8mp.md`, "Running a GLES3 slice session", steps 1-13,
run the way slice `2026-11-imx8mp-gles3-9bc82915-rows01-13` (s01) ran it.

## What ran

Everything was built on this session's host from a detached worktree of
`9bc82915` (its own `tools/cts/imx8mp-*.sh`; the table there is byte-identical
to the committed one), with the registered Zephyr workspace (zephyr af17c0c8,
hal_nxp) and Zephyr SDK 1.0.1:

- stage 1: `tools/cts/imx8mp-deqp-stage1.sh gles3` (VK-GL-CTS 93bca018 with
  its external sources, `MODULE=gles3`): `rc=0`, 453 ninja steps;
- the table image, console on: `IMX8MP_CONSOLE=1 tools/cts/imx8mp-deqp-build.sh
  --gles3 es3-9bc82915 'dEQP-GLES3.info.*' --table tools/cts/imx8mp-gles3-tree.txt`:
  `imx8mp: 133 table rows -> <out>/es3-9bc82915.table.conf`, `rc=0`,
  `check-deqp-images: the dEQP image recipes build what conformance needs`;
  `.config` has `CONFIG_UART_CONSOLE=y`, `CONFIG_STDOUT_CONSOLE=y`,
  `CONFIG_PRINTK=y`; image build stamp
  `YAKOGL_SAMPLE_TABLE=6c5b8a70 YAKOGL_SAMPLE_TREE=9bc82915`, zephyr
  `af17c0c8aa2a`, footprint 227909632 (the AArch64 header `image_size`
  `board.sh` checks).

`tools/cts/imx8mp-gles3-artefacts.sh es3-9bc82915` (bytes, sha256, file).
`zephyr.bin` is byte-identical to the one s01 built on another host;
`zephyr.elf` differs (embedded build paths). The script printed the linker's
`Memory region` header without a `RAM:` line on this host:

```
16443032 f9c80e5207f974eeaefed49decb19766ac3efb87b8d7836d79e89d893d8c4cfc es3-9bc82915/zephyr/zephyr.bin
41868592 f43bdb19998f2528569d9ec447901874379275a5af65e7f081c59362ed357b43 es3-9bc82915/zephyr/zephyr.elf
38164 5b7a612379058051e8be1d1983829e7c13254e817289282fe6e38f99bbedb82f es3-9bc82915.table.conf
132944 2fff51840518bd589eb2eaa700bacc120b365448b2ac0ada7bcdccf2bf5bb2e8 deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a
12732236 7b842c954d403450601f75fe245b47581233cc870a778598800065b41a0885cc deqp-stage1-gles3/build/modules/gles3/functional/libdeqp-gles3-functional.a
# CONFIG_DEQP_REPORT_ES2 is not set
# CONFIG_YAKOGL_REPORT_ES2 is not set
Memory region         Used Size  Region Size  %age Used
```

`tools/cts/imx8mp-gles3-preflight.sh --commit 9bc82915e06367b46e3a62b644dbf2bb659d72ad --out $IMX8MP_OUT`:
nine `ok` lines, `preflight: all ok`. `tools/board.sh imx8mp status` before the
run: lease `none`, `install: ok`, no staged `once-*.bin`, `yako_once` and
`yako_default` empty. The dry run (`tools/cts/imx8mp-gles3-board.sh --dry-run
tree ...` with the same `TREE_ROWS` and `TREE_IMAGE`) printed the steps of
`e16`..`e33`, selectors 16..33, `footprint 227909632`.

The run, with the integration tip's `tools/board.sh` (build 1a5cd315; the
image is the campaign commit's, the runner the current one), over one output
directory:

```
TREE_ROWS=16-33 TREE_IMAGE=$IMX8MP_OUT/es3-9bc82915/zephyr/zephyr.bin BOARD_BOOT=uboot-mmc \
  BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt RUN_BOUND=800 BOARD_UART=<local feed bridge> \
  NAMES=0 tools/cts/imx8mp-gles3-board.sh tree records/imx8mp/es3-9bc82915-rows17-34   # pass 1: the 18 rows, 10:10-10:58
  ...  (NAMES unset) TREE_ROWS=16-33 ... tree ...      # pass 2: name pages e18-p0 .. e22-p0, 10:58-12:10
  ...  (NAMES unset) TREE_ROWS=23,24,27 ... tree ...   # pass 2b: e23-p0 .. e27-p11, 12:13-13:24
  ...  (NAMES unset) TREE_ROWS=27 ... tree ...         # pass 2c: e27-p12 .. e27-p25, 13:32-14:30
tools/cts/imx8mp-gles3-board.sh tally records/imx8mp/es3-9bc82915-rows17-34
NAMES=0 TREE_ROWS=26 ... tree records/imx8mp/es3-9bc82915-e26-rerun                  # e26 once more, 14:39-14:45
```

- `RUN_BOUND=800` and `NAMES=0` first, then the pages, as in s01. A page boot
  runs the whole entry again and records six failing-case CRC-16s; `board.sh`
  reads at most 40 pages (240 names) of a row.
- **Pages run: 74**, for the rows whose failures fit in 40 pages: e18 12, e19
  9, e20 16, e21 1, e22 1, e23 3, e24 6, e27 26. Every one `rc=0
  verdict=done resets=absent`, 44-299 s from reboot to ssh.
- **Pages not run: e25 (364 fails), e26 (277 fails + 1 other) and e29 (328
  fails).** Each has more than the 240 names `board.sh` pages, so the pages
  would name only part of the row, and they would have taken about 8 hours of
  board time (40 boots each, e25 and e26 at 273-338 s an entry). These three
  rows have counts and no names. A console, or a page limit raised above 40,
  would name them.
- The passes after the first were split so that each one stays inside the
  agent host's two-hour limit on a command. Pass 2 was ended by that limit
  while e23-p0 was being staged: `board.sh` removed the staged image (`removed
  /var/yakogl/once-e23-p0-... (no reboot)`), and the board was not rebooted for
  it. Pass 2b was ended on purpose at e27-p12's header, before that boot was
  armed. Both unfinished run directories held only `meta.txt` (and an empty
  capture), no `INDEX.txt`. They were moved out of the output directory before
  the next pass, which ran those pages from the start. After each stop
  `tools/board.sh imx8mp status` showed lease `none`, `yako_once` empty and an
  empty staging directory.
- `BOARD_UART` named a local feed bridge (`tools/am62p_netboot_uart.py feed`,
  the route for a console the farm hands out as MCP tools only). The bytes the
  leased console returned were appended to it (see "Console").
- `BOARD_HOST` and the console's address came from the task's environment
  only.

## Verdict lines

The 18 rows, verbatim from the run (`tools/board.sh imx8mp tally` prints the
same lines). Every row is `rc=0 verdict=done` with `resets=absent`.
`record=` is the SNVS DEQP record w0..w3, `back_after` the time from reboot
to ssh, `uptime` Linux's uptime at that moment:

```
board.sh imx8mp e16 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0862,000001d1,00000000,ffff01d1 back_after=93s uptime=25s
board.sh imx8mp e17 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08ac,00000176,00000000,ffff0176 back_after=94s uptime=36s
board.sh imx8mp e18 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0818,004800b4,00000030,01e9012c back_after=54s uptime=25s
board.sh imx8mp e19 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0830,003300d5,00000030,4b710138 back_after=52s uptime=23s
board.sh imx8mp e20 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08b2,005c010a,00000013,32ff0179 back_after=69s uptime=38s
board.sh imx8mp e21 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b081a,00060027,00000000,70cc002d back_after=45s uptime=21s
board.sh imx8mp e22 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0834,00020038,00000000,3ae1003a back_after=51s uptime=32s
board.sh imx8mp e23 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08de,001101c4,0000003a,582d020f back_after=81s uptime=34s
board.sh imx8mp e24 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b080a,001a02ea,0007001a,532c0325 back_after=102s uptime=29s
board.sh imx8mp e25 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0830,016c02cc,00000000,c2460438 back_after=273s uptime=27s
board.sh imx8mp e26 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a8,011500de,00010000,19e301f4 back_after=338s uptime=29s
board.sh imx8mp e27 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0882,009804c5,00040000,dae50561 back_after=194s uptime=22s
board.sh imx8mp e28 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0834,0000033a,00000000,ffff033a back_after=80s uptime=26s
board.sh imx8mp e29 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0850,01480380,00000000,7c3304c8 back_after=91s uptime=25s
board.sh imx8mp e30 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a8,000002f4,00000000,ffff02f4 back_after=91s uptime=28s
board.sh imx8mp e31 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b0808,00000324,00000000,ffff0324 back_after=78s uptime=25s
board.sh imx8mp e32 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b081c,0000042e,00000000,ffff042e back_after=91s uptime=25s
board.sh imx8mp e33 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b085c,0000034e,00000000,ffff034e back_after=89s uptime=28s
```

The rerun of e26 (its own directory, not part of `results.tsv`) printed the
same record:

```
board.sh imx8mp e26 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:DEQP resets=absent record=594b08a8,011500de,00010000,19e301f4 back_after=338s uptime=28s
```

The 74 page boots (`result=record:DEQP_FAILS`, six names each) are in
`verdicts.txt` with the rows, verbatim from the run logs. Every one is `rc=0
verdict=done resets=absent`.

## Result

10,873 mustpass cases in the 18 entries, 10,750 of them begun (the record's
`begun` of 11,746 also counts cases outside the mustpass list that the
wildcards select): **10,148 pass, 1,387 fail, 199 NotSupported, 12 other,
123 without a result** (all 123 in e26, below). No row ended at the watchdog
or died. The longest board time was e26's 338 s from reboot to ssh, with
Linux's own boot (21-38 s) inside it. Nothing was re-split and the table is
unchanged. Per entry (`merge.py --totals`, plus the row's `back_after`, its
`uptime` at the return and its page boots):

| entry | first group | mustpass | begun | pass | fail | n.s. | other | none | back_after | uptime | pages |
|---|---|---|---|---|---|---|---|---|---|---|---|
| e16 | fragment_ops.depth_stencil.stencil_depth_funcs .. | 465 | 465 | 465 | 0 | 0 | 0 | 0 | 93 s | 25 s | 0 |
| e17 | fragment_ops.depth_stencil.stencil_ops.replace_* .. | 368 | 374 | 374 | 0 | 0 | 0 | 0 | 94 s | 36 s | 0 |
| e18 | fragment_out.array.fixed .. | 300 | 300 | 180 | 72 | 48 | 0 | 0 | 54 s | 25 s | 12 |
| e19 | fragment_out.array.uint .. | 312 | 312 | 213 | 51 | 48 | 0 | 0 | 52 s | 23 s | 9 |
| e20 | fragment_out.basic.int .. | 377 | 377 | 266 | 92 | 19 | 0 | 0 | 69 s | 38 s | 16 |
| e21 | instanced | 45 | 45 | 39 | 6 | 0 | 0 | 0 | 45 s | 21 s | 1 |
| e22 | lifetime | 56 | 58 | 56 | 2 | 0 | 0 | 0 | 51 s | 32 s | 1 |
| e23 | multisample .. | 510 | 527 | 452 | 17 | 58 | 0 | 0 | 81 s | 34 s | 3 |
| e24 | primitive_restart .. | 797 | 805 | 746 | 26 | 26 | 7 | 0 | 102 s | 29 s | 6 |
| e25 | shaders.arrays .. | 1000 | 1080 | 716 | 364 | 0 | 0 | 0 | 273 s | 27 s | 0 (not run) |
| e26 | shaders.builtin_functions.precision.dot .. sin | 623 | 500 | 222 | 277 | 0 | 1 | 123 | 338 s | 29 s | 0 (not run) |
| e27 | shaders.builtin_functions.precision.sinh .. | 915 | 1377 | 1221 | 152 | 0 | 4 | 0 | 194 s | 22 s | 26 |
| e28 | shaders.conversions.matrix_to_matrix .. | 826 | 826 | 826 | 0 | 0 | 0 | 0 | 80 s | 26 s | 0 |
| e29 | shaders.conversions.vector_to_vector .. | 882 | 1224 | 896 | 328 | 0 | 0 | 0 | 91 s | 25 s | 0 (not run) |
| e30 | shaders.indexing | 756 | 756 | 756 | 0 | 0 | 0 | 0 | 91 s | 28 s | 0 |
| e31 | shaders.invalid_implicit_conversions .. | 804 | 804 | 804 | 0 | 0 | 0 | 0 | 78 s | 25 s | 0 |
| e32 | shaders.keywords .. | 991 | 1070 | 1070 | 0 | 0 | 0 | 0 | 91 s | 25 s | 0 |
| e33 | shaders.matrix.add .. | 846 | 846 | 846 | 0 | 0 | 0 | 0 | 89 s | 28 s | 0 |
| total | | 10873 | 11746 | 10148 | 1387 | 199 | 12 | 123 | | | 74 |

The full filters are in `results.tsv` (column 2), and
`check-imx8mp-gles3-tree.py` prints each entry's first pattern.
"begun" above "mustpass" means the filter also matched cases outside the
mustpass list.

`tools/cts/imx8mp-gles3-merge.py --totals` on this directory (only the rows
of this slice are shown here; it also lists rows 0-15 and 34-132 as not run,
`no record: 115 of 133 entries (0-15,34-132), 31937 mustpass cases`):

```
entry  mustpass    begun     pass     fail     n.s.    other     none
e16         465      465      465        0        0        0        0
e17         368      374      374        0        0        0        0
e18         300      300      180       72       48        0        0
e19         312      312      213       51       48        0        0
e20         377      377      266       92       19        0        0
e21          45       45       39        6        0        0        0
e22          56       58       56        2        0        0        0
e23         510      527      452       17       58        0        0
e24         797      805      746       26       26        7        0
e25        1000     1080      716      364        0        0        0
e26         623      500      222      277        0        1      123
e27         915     1377     1221      152        0        4        0
e28         826      826      826        0        0        0        0
e29         882     1224      896      328        0        0        0
e30         756      756      756        0        0        0        0
e31         804      804      804        0        0        0        0
e32         991     1070     1070        0        0        0        0
e33         846      846      846        0        0        0        0
total     42810    11746    10148     1387      199       12    32060
```

### e26 ends early, at the first precision.reflect case, every time

e26 began 500 of its 623 mustpass cases and then ended at 338 s, well inside
the 600 s window, with `verdict=done` and a complete record (`begun` 500 =
222 pass + 277 fail + 1 other). The rerun gave the same record word for word
(the same first-failure CRC `19e3`, the same 338 s). The entry's groups run in
order (`precision.dot` .. `precision.radians` hold 499 mustpass cases). Case
500 is therefore the first `precision.reflect` case, and the 123 cases never
begun are exactly the rest: reflect 11, refract 24, round 24, roundeven 24,
sign 24, sin 16. That one case ended with a status other than
Pass/Fail/NotSupported (the record's `other` is 1), and the image did not go
on to the next case. Which status it was (Crash, InternalError,
ResourceError, ...) needs a console or a page past the 40-page limit. This is
not a watchdog end and not a GPU recovery `board.sh` saw. Splitting the entry
would not run the 123 cases either, because the stop is a property of the
reflect case. It goes to the triage, and the table is unchanged.

### The failing names

`results.tsv` names the fails and other statuses of the rows that were paged,
one entry per recorded CRC (e18 72, e19 51, e20 92, e21 6, e22 2, e23 17, e24
33 = 26 fail + 7 other, e27 156 = 152 fail + 4 other). Two things in them come
from the page mechanism, not from the board:

- **CRCs left unresolved** (`crc:<hex>`): e22 1, e23 1, e24 2, e27 14. Each
  resolves to no dEQP-GLES3 name inside the row's own filter. They are among
  the cases outside the mustpass list that the wildcards also select (e22
  +2, e23 +17, e24 +8, e27 +462 begun over mustpass).
- **CRC-16 collision pairs** (two names joined by `,`): e19 1, e27 2. Both
  names fall inside the row's filter. `results.tsv` is left as `board.sh
  tally` wrote it; `merge.py --check --partial` passes on this directory.

e25, e26 and e29 carry `-` in the names column (no pages, above). Their
NotSupported cases, like all NotSupported cases of this slice (e18, e19, e20,
e23, e24), have counts but no names.

## Console: what the bridge delivered

The image was built with the console on and `tools/board.sh` captured
`BOARD_UART` for every boot (`console.raw`, `console.ts`, `console.txt` in
each run directory; `e<NN>.console.txt` here). As in s01, the leased console
delivered **no text at all**: one NUL byte (`\x00`) per board reset and
nothing else, 78 bytes for the first 78 boots (18 rows + 60 pages), then one
per boot after that. The lease's serial console was `connected` the whole time
(RFC 2217, 115200 8N1, the adapter's settings). Before the run, an Enter sent
to the console got no login prompt back, although `serial-getty` was active
on that UART at 115200. A marker line written to the board's UART3 device
from Linux did not arrive either. The relay forwarded each row's NUL as it
came during pass 1. During the pages the console was read in batches, so
which page's capture holds which NUL is not exact. That does not matter for
NULs.

What follows from it:

- `tools/cts/imx8mp-console-scan.py` (`console-scan.txt`) sees 1 line (the
  time stamp of the NUL), 0 boot banners and 0 rows in every capture, exit 0.
  **GPU recoveries were therefore not observable**: `recoveries 0` is not
  shown for any row. `board.sh` does not count recoveries on this board
  (`resets=absent`), and the SNVS record holds none. The rows are complete
  records (`verdict=done`; `begun` at or above the mustpass count everywhere
  but e26, above), but the recipe's zero-recovery proof is missing for all 18
  rows. This is reported, not claimed.
- No per-entry board time comes from the scan. The times above are
  `back_after` and `uptime`, from `meta.txt` (`t_reboot`, `t_back`, `up`).
- Failing names came from the CRC pages instead.

## Resets and reboots outside the rows

Every boot is one reboot by design (into the image, then the WDOG1/PSCI reset
back to Linux). `board.sh` accepted each return only from a new boot (`uptime`
below the time since the reboot). Between the boots the board stayed in Linux.
Each boot's prechecks passed the first time, no boot was refused (rc 3) or
retried, and no boot waited for the board lease. One page, e27-p23, came back
at 299 s with `uptime=129s` (e27's other boots: 194-198 s, uptime 22-30 s).
The image ran its usual time; Linux then took about 100 s longer before ssh
answered, which is on the Linux side, not the image's. Twice the board's uptime
matched the time since the last boot's return. After pass 2c it was 369 s at
1794753376, so the board booted at 1794753007, which is e27-p25's `t_back`
1794753031 minus its `uptime` 24. After the e26 rerun it was 68 s at
1794753953, booted 1794753885, the rerun's `t_back` 1794753913 minus 28. Each
time the lease file was gone, the staging directory empty and `yako_once`
empty. The image's 600 s `CONFIG_GC_BOARD_WDOG_SECONDS` window was never
reached (longest row 338 s, longest page 299 s with the Linux delay above).

## Files

- `results.tsv`, `rows.md`: `tools/board.sh imx8mp tally` of the output
  directory (index, filter, begun, passed, failed, notsup, other, noresult,
  failing names; rows 0-15 and 34-132 are `-`, not run in this slice), with
  their REUSE `.license` sidecars. Its `noresult` is begun minus results, 0
  for e26 too: the 123 cases e26 never began appear only in `merge.py`'s
  `none`.
- `verdicts.txt`: every `board.sh imx8mp e..` verdict line of the run logs,
  rows and pages, each once (the e26 rerun is quoted above, not there).
- `e16.console.txt` .. `e33.console.txt`: each row's captured console
  (header lines, then the capture: one time-stamped empty line, the NUL).
- `console-scan.txt`: the scan over the 18 row directories.

Raw captures (`console.raw`, `console.ts`) and the run directories stay
under `records/` (gitignored). Addresses: `board.sh` redacted the copies, and
every file here was read again for logins, addresses, ports and device paths
before the commit. `BOARD_HOST` and `BOARD_UART` are named here, never
their values.
