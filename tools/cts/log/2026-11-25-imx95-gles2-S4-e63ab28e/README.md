<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX95 dEQP-GLES2 S4, rows 69-76, at e63ab28e

commit e63ab28e30ae913f06bb595bc10f972fc1c2ff77

Board session 1 of B8 (`docs/notes/pre-study-imx95.md` section 8, issue #7):
rows `T6S4R01`-`T6S4R08` of `tools/cts/mali-gles2-tree.txt` (the NanoPC-T6's
rows, unchanged) on the Toradex Verdin iMX95 (i.MX 95, Mali-G310, Mallow
carrier) under Zephyr, one image per row, started from the module's own
U-Boot by hand on the UART_3 console. 2026-11-25, 05:48 to 06:12 UTC.

| | |
|---|---|
| session | S4 |
| rows | 69-76 of `tools/cts/mali-gles2-tree.txt` |
| commit | `e63ab28e30ae913f06bb595bc10f972fc1c2ff77` (image build string `e63ab28e`) |
| images | `tools/cts/imx95-deqp-images.sh` (`deqp-tree-images.sh verdin_imx95/mimx9596/a55`, `-DYAKOGL_WERROR=ON`), stage 1 `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95` |
| firmware | `driver/tools/mkmalifw.sh` on the T6's `mali_csffw.bin` (sha256 `a27847ea…405b`) |
| kernel | none: Zephyr `af17c0c8aa2a` on the A55, started by U-Boot 2024.04-7.8.0 |
| lifetime | 600 s per row (`deqp/verdin_imx95.conf`) + WDOG3's 60 s |
| board record | `records/verdin-imx95/20261125-0548-gles2-S4-e63ab28e/` (gitignored) |

## Result

Every row ran to its end, reported a result for every case it began
(`deqp: summary: N cases begun, N with a result, 0 without`), and reset the
board itself (`verdin_imx95: board: run over: PSCI SYSTEM_RESET at N ms`;
the next U-Boot reports `LM Boot reason: sw`). No hang, no WDOG3 reset, no
GPU restart, no tiler OOM (`osal(mali): restarts 0 ...; tiler OOMs 0 grown,
0 with no chunk`), no refused draw (`yakogl: draws refused 0`). The module
came back to its Linux each time.

The 2,675 mustpass names of S4 (`results.tsv`, the names the T6's S4 slice
holds):

| status | cases |
|---|---|
| Pass | 2107 |
| Fail | 537 |
| NotSupported | 31 |
| QualityWarning | 0 |

567 of the 568 non-Pass cases carry the library's own refusal of a shader or
a program (`refusals.tsv`); the one without is
`shaders.texture_functions.vertex.texturecubelod`, a Fail on the image
compare, as on the T6.

The image's case filter enumerates 146 more names that are not in
`gles2-mustpass.txt` (`extra.tsv`: 93 of `shaders.keywords`, the rest in
functions, preprocessor, linkage, scoping, texture_functions, constants,
declarations and reserved_operators). All 146 pass. They are outside the
tally.

## Rows

`enumerated` is what the image ran: its filter's names, 2,821 in all.
`mustpass` counts the names of `gles2-mustpass.txt` among them: the same
count as the T6's S4 row, and the one to compare with it. Pass, Fail and
NotSupported are over the enumerated names (the 146 extra names are all
Pass). `run` is the image's own uptime at its reset; the lifetime is 600 s.
`back` is when the module's Linux answered ssh again. The 20 s wait and 5 s
poll of that check make it a bound, not a measurement. `md5` is U-Boot's
`md5sum` of the loaded bytes, equal to the host's for every row.

| row | label | enumerated | mustpass | Pass | Fail | NotSupported | run | of lifetime | `go` (UTC) | back | md5 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 69 | T6S4R01 | 395 | 395 | 271 | 124 | 0 | 18.8 s | 3.1 % | 05:48:40 | - | `d5a242b0…` |
| 70 | T6S4R02 | 367 | 365 | 278 | 89 | 0 | 16.4 s | 2.7 % | 05:51:03 | 05:52:01 | `483c736a…` |
| 71 | T6S4R03 | 380 | 378 | 269 | 111 | 0 | 17.6 s | 2.9 % | 05:54:12 | 05:55:08 | `7ae235ed…` |
| 72 | T6S4R04 | 307 | 292 | 229 | 73 | 5 | 13.3 s | 2.2 % | 05:57:20 | 05:58:16 | `1971e74d…` |
| 73 | T6S4R05 | 480 | 387 | 444 | 36 | 0 | 16.8 s | 2.8 % | 06:00:19 | 06:01:15 | `8ca98178…` |
| 74 | T6S4R06 | 404 | 383 | 383 | 21 | 0 | 14.6 s | 2.4 % | 06:03:51 | 06:04:47 | `f83500c9…` |
| 75 | T6S4R07 | 366 | 356 | 345 | 18 | 3 | 14.7 s | 2.5 % | 06:07:50 | 06:08:46 | `dcf6a22a…` |
| 76 | T6S4R08 | 122 | 119 | 34 | 65 | 23 | 7.3 s | 1.2 % | 06:11:26 | 06:12:06 | `326aed15…` |

The images (`zephyr.bin`, sha256 from each `<label>.built`; every image
prints `vi95_deqp: build e63ab28e` and `sample: build tree=e63ab28e`):

| label | sha256 |
|---|---|
| T6S4R01 | `47cc9c7f082b56dcaeadfce8a50fb6c4285f299c0e6e8d9800dc5987d7697ef9` |
| T6S4R02 | `41db5a92d36224dd40f52575f97cd421b37f17d75c907730069c0f06b856fe65` |
| T6S4R03 | `e04532825c5a00d7f4f1c0219bf6263e854f4f4876bcb0a21b911168af3ebfd8` |
| T6S4R04 | `1a5e4ee7eca78eeffd318a2daca9def3d352f1fa0fd7eda9a70f57e9a001e961` |
| T6S4R05 | `76cbf2a6269e92cf8dae6bdc86053228ea287872c4b53c9f7582aed4794e36eb` |
| T6S4R06 | `3025c2ec1e39c762ea0e654a9064fabf57afe1c0c5c2e6783e6118b94b57a499` |
| T6S4R07 | `4e52548c5055e1ac6d46a3c26831f6d815ef70221423e2e3f8f9d8a47887b901` |
| T6S4R08 | `0e587066d31e6815f9b75eb012b53fb87592d16788dabfec110ee65023f8e8a1` |

The table above is the session's own; `rows.tsv` is built by
`tools/cts/imx95-rows-tsv.py` from `console/` in the other sessions' schema
(`row` 1-8 for T6S4R01-R08, `cases`, `status`, `seconds`, `lifetime_s` 600,
`bbm` not-read, `exit_line`; the image's uptime in ms is in `exit_line`).
The tool does not derive the md5, `go` and `back` columns of the earlier
hand-written table, so they are only in the table above. The per-chunk figures
in `tools/cts/mali-deqp-tally.py`'s columns are in `chunks.tsv`. From `go` to the Linux login takes about 56 s per row:
the image, the reset, U-Boot and the module's Linux boot.

## The record that survives the reset (U2)

Each image printed its record on the console, `yk-record: 594b0201 00000002
00000000 00000000`, and wrote it to the BBNSM GPR4..7 over SCMI. It was not
read back. `tools/imx95_bbm_record.sh probe` on the module's Linux answers
`no SCMI raw interface: CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX not in this
kernel` (exit 3). The verdicts above are the console's.

## Against the NanoPC-T6 (Mali-G610)

The baseline is `tools/cts/log/2026-11-24-t6-gles2-S4-8ff01055/`: the same
2,675 names, 1944 Pass, 699 Fail, 32 NotSupported. `diff-t6.tsv` lists each
name whose status differs:

- 163 names differ, all toward Pass on the iMX95: 162 Fail to Pass, 1
  NotSupported to Pass. None is worse on the iMX95. By group: conversions
  111, constant_expressions 26, functions 14, linkage 6, struct 5,
  qualification_order 1.
- Of the 568 names that are non-Pass on both boards, 550 have the same first
  refusal. 18 differ:
  - 8 that the T6 refused in the vertex shader (a fused multiply-add) now
    reach the fragment shader and stop there on the back-end defect
    `an invariant of the pipeline did not hold (IrIndex)`.
  - 6 have no refusal line on the T6 and a mediump fragment-uniform refusal
    here.
  - 2 carry a reworded uniform refusal.
  - 1 moved from the vertex loop to a constant-operation refusal.
  - 1 (texturecubelod) has no refusal on either board.

The two sides differ in board and in commit: 8ff01055 (T6, under panthor on
the board's Armbian) against e63ab28e (iMX95, Zephyr), which is 283 commits
later. Those commits move the online compiler, and every difference above
is a compiler status or a compiler refusal. None of them is a GPU fault, a
hang or a wrong picture that only the G310 shows. S4 shows no G310-only
difference. The G610 constants that `docs/notes/mali-g310-vs-g610.md`
section 5 names (`TIB_BUDGET`, `MEASURED_TILER_LEVELS`, the renderer name)
had no visible effect in these rows: no tiler OOM, no restart, and no
`info.*` case in S4 to print the renderer string. Telling the board apart
from the commit would need the T6's S4 rerun at e63ab28e.

The causes, as `python3 -I tools/cts/mali-deqp-tally.py --tally` prints
them over this slice (count, status, refusal; text cut at 256 bytes by the
library's log line):

| cases | status | stage | cause |
|---|---|---|---|
| 107 | Fail | fragment | arithmetic other than +, -, * and a dot or matrix product is not lowered yet |
| 61 | Fail | fragment | back end defect: an invariant of the pipeline did not hold (IrIndex) (10 of them through the link) |
| 50 | Fail | fragment | an integer or boolean value is not lowered yet (6 through the link) |
| 44 | Fail | vertex | arithmetic other than +, -, * and a dot or matrix product is not lowered yet |
| 33 | Fail | fragment | fp16 arithmetic on gl_FragCoord other than x and y as a pair ... |
| 23 | Fail | vertex | a uniform other than a float, vec2, vec3, vec4 or mat4 |
| 22 | Fail | fragment | a mediump fragment shader uniform other than ... read in its own fp16 lane pairs |
| 20 | Fail | fragment | a constant product in a shape Mesa's algebraic rules move or fold |
| 19 | Fail | vertex | a loop in a vertex shader |
| 17 | Fail | fragment | a gl_FragColor lane other than the varying's own lane or a constant |
| 16 | Fail | fragment | a constant operation on a value that is not a finite normal float |
| 16 | Fail | fragment | a discard other than a highp `if (a < b) discard;` |
| 16 | Fail | vertex | a fused multiply-add other than a matrix product's |
| 15 | Fail | fragment | a fragment shader operand of a form or in a place no reference draws |
| 15 | Fail | vertex | a constant product in a shape Mesa's algebraic rules move or fold |
| 11 | Fail | vertex | a constant operation on a value that is not a finite normal float |
| 9 | Fail | fragment | control flow (if, a loop, a return before the end) |
| 8 | Fail | fragment | a fragment shader that mixes highp and mediump or lowp float values |
| 7 | Fail | fragment | two varying lanes of equal value (through the link) |
| 7 | Fail | vertex | control flow |
| 4 each | Fail | | texture coordinate (fragment), texture look-up form (fragment), non-highp vertex uniform, three or more chained additions (vertex) |
| 3 | Fail | | fp16 varying load form (2), texture result lanes (1) |
| 1 | Fail | fragment | three or more chained additions |
| 1 | Fail | - | no refusal (texturecubelod, image compare) |
| 31 | NotSupported | | refusals of the same kinds (vertex uniform 9, discard 4, kept mediump loop 4, loop limits 3, arithmetic 3, vertex loop 2, integer 2, 4 others) |

Every one is a refusal of the online compiler (issue #11251; the T6's
`docs/es2-cts-failures.md` section M has the same kinds).

## What the logs hold

`console/<label>.log` is the whole console of one row, joined from the
serial captures without a gap. It starts at the U-Boot SPL banner of the
reboot that led to the U-Boot prompt and takes in, in order:

- the U-Boot lines;
- the commands typed (`ext4load ...; md5sum ...`, then `dcache off;
  icache flush; go 0xd0000000`);
- the image's whole output: the build line, the WDOG3 line, every `Test
  case '...'..` line with the backend's per-case lines and refusals, every
  `deqp: <status> <case>` line with dEQP's status line, the summary, the
  OSAL counters, `yk-record` and the reset line;
- the next U-Boot banner, up to its `Hit any key to stop autoboot` line.

From there the module boots its stock Linux. That boot is not part of the
row, and its kernel log names hundreds of device-tree nodes that
`mali-slice-redact.py` reads as a user and host name (a node name, an at sign, its address), so it is in the board record's
raw capture only (`<label>.raw.log`, with the serial tool's JSON windows
beside it). Two redactions: the module's serial number (`Serial#: <serial>`)
and the device-tree path of one U-Boot line (`DRAM:  Delete node <device-tree
path>`). Line ends are LF, and control bytes are written as the serial tool
gives them (`\x1b`, `\x08`). A refusal line longer than 256 bytes was already
cut by the library's log line. The capture still holds it as printed.

`results.tsv`, `refusals.tsv`, `extra.tsv`, `rows.tsv`, `chunks.tsv` and
`diff-t6.tsv` are computed from those console logs.
`python3 -I tools/cts/mali-deqp-tally.py --tally [--per-case]` reads the
slice like a T6 slice. `--check-cover` against the whole mustpass reports the
names of S1-S3 as missing, which is expected: this slice is S4 only.

## How the slice was produced

1. A clean checkout of e63ab28e outside the working tree, with the firmware
   header made by `driver/tools/mkmalifw.sh` (never committed).
2. Stage 1: `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`.
3. The eight images: `tools/cts/imx95-deqp-images.sh
   tools/cts/mali-gles2-tree.txt <img-dir> T6S4R01 ... T6S4R08`, each 12.2 MiB
   with a RAM footprint (`image_size`) of 191.0 MiB of the 256 MiB window.
4. Per row, as in the first session (`tools/cts/RESULTS.md`, "Mali: Verdin
   iMX95 on Zephyr (started by U-Boot)"):
   - the module's Linux stores the image on the eMMC's root file system
     (`/var/yakogl/s4-e63ab28e/<label>.bin`, md5 compared with the host's)
     and reboots;
   - a key on UART_3 stops U-Boot at its prompt;
   - `ext4load mmc 0:2 0xd0000000 /var/yakogl/s4-e63ab28e/<label>.bin;
     md5sum 0xd0000000 ${filesize}`, then `dcache off; icache flush; go
     0xd0000000`.

   The boot partitions, the eMMC boot areas, the boot command and the
   U-Boot environment were not written. The images were removed from
   `/var/yakogl` afterwards.
5. The console stream was read from the serial lease in windows and joined
   into `<label>.raw.log` in the board record. This slice is computed from
   those files.

The slice holds no host name, address, user name or USB serial.
