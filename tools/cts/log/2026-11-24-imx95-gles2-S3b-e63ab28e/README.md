# Verdin iMX95 dEQP-GLES2, rows T6S3R16-T6S3R30 on Zephyr from e63ab28e

Plan #407, item vi95-s3b (issue #7, Verdin iMX95 item B8 of
`docs/notes/pre-study-imx95.md`): rows T6S3R16 to T6S3R30 of
`tools/cts/mali-gles2-tree.txt` (the NanoPC-T6's `# image` rows, unchanged),
one Zephyr image per row, on the Toradex Verdin iMX95 (Mali-G310,
GPU_ID 0xac740001) on its Mallow carrier, started from the module's own
U-Boot prompt. The run day is 2026-11-27 (host clock, UTC, first `go` 06:33,
last 07:06); the directory keeps the name the plan item gave it.

## What ran, against what

- Images: commit `e63ab28e` (`e63ab28e30ae913f06bb595bc10f972fc1c2ff77`, an
  ancestor of `origin/main`), built from a detached worktree of that commit:
  stage 1 `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`
  (VK-GL-CTS 93bca018, built fresh for this session), the firmware header
  from `driver/tools/mkmalifw.sh` (linux-firmware's `mali_csffw.bin`, sha256
  a27847ea..., not committed), stage 2
  `tools/cts/imx95-deqp-images.sh tools/cts/mali-gles2-tree.txt <img-dir> T6S3R16 ... T6S3R30`.
- Build string: every `.built` line ends `build=e63ab28e`, and every console
  prints `vi95_deqp: build e63ab28e` and `sample: build tree=e63ab28e
  table=6c5b8a70`. The `zephyr.bin` files are 12,768,592 bytes (T6S3R23:
  12,834,128, T6S3R24: 12,809,552; AArch64 image_size 0xbf04000, 191 MiB of
  the 256 MiB window); lifetime 600 s (`fed until 600 s of uptime` on every
  console) plus WDOG3's 60 s.

| label | sha256 of zephyr.bin (`.built`) |
|---|---|
| T6S3R16 | d7e88f96d5fccba6efdc9c5ceca74e3cd1e2daf05b9d27ef7d9f101677abc6ad |
| T6S3R17 | b6170c82ab003b713f8619e1c1af77bfb06d4efa693fa4790e8ebda2486d4db0 |
| T6S3R18 | 489f8bad5a1aa9f7aca63f227b6fcea2266752d2f8a7139e59aff6315263c348 |
| T6S3R19 | 8c4aeb6027fc8fae0d0799b9caadbbda64a2f92ef81774c1d5a1819188d2df02 |
| T6S3R20 | d73987273cc8e43651bd086841e55b4b8b5870d990bd8f379d7ea24e5d82d1f5 |
| T6S3R21 | 2b1bd36f55b75459ec740f51712cd0e6e4ecae9649a8eff144c4cbdfc53d3989 |
| T6S3R22 | 8fe493c0f2c3dd492d4b7c885f85dc879dbf7a2e108f30023b4ca4f98330b090 |
| T6S3R23 | d4d46d8b346dbb4951fd903bfd6006391b35841bde61923853bf42a932837102 |
| T6S3R24 | e7985a5d74f7d97f1f85252180c4f394c811e64b581c06b95f152adca4498d4f |
| T6S3R25 | 80e56449e5b9d053290607e1ee11b8d7c5a201b90b62ecda5e0ea253c86b693d |
| T6S3R26 | 9801818b51da84de2fcfa577920e73fb50b4306eedbcf6aa112394b045a49d2d |
| T6S3R27 | ad7c190784b318ddd1c0a86801bd6092226fb6777c641593389901c0ee57d687 |
| T6S3R28 | 507dafc38658f69972e6d5aa0b402c9aac5e0b8c1514fea3580993c0983e3ee9 |
| T6S3R29 | fd34af66506e528f065e6776954392edff4278a04eb4285cec145c897848c357 |
| T6S3R30 | f216394f8a39ac9be08065a2e63bace69977d539176327e4d0195a135c3b385c |

## The route, per row

The hand-typed U-Boot route of `tools/cts/README.md` section 5, lines as
`tools/cts/imx95-uboot-row.py --storage 0:1 --stage /boot/ --fs fat` prints
them (the FAT boot-files partition, mmc 0:1, mounted at `/boot`; S1a's and
S2's route):

1. module's Linux, over ssh: `rm -f /boot/<LABEL>.bin`, then
   `gzip -dc > /boot/<LABEL>.bin` (the image, gzip'd on the host),
   `md5sum /boot/<LABEL>.bin` equal to the host's;
2. over ssh, `sync; reboot`; on the console, spaces sent right after U-Boot
   proper's `LM Boot reason` line (autoboot stopped, prompt
   `Verdin iMX95 # `), then Ctrl-C;
3. `fatload mmc 0:1 0xd0000000 <LABEL>.bin; md5sum 0xd0000000 0x<size>`
   (rows T6S3R24 to T6S3R30: `${filesize}`, the size `fatload` had just set,
   for `0x<size>`): the bytes read equal to the host's size and the md5
   equal to the host's on every row;
4. `dcache off; icache flush; go 0xd0000000`, the capture starting there;
5. the image ends with PSCI SYSTEM_RESET, the board boots Linux by itself
   (nothing typed), then over ssh `sh -s -- read < tools/imx95_bbm_record.sh`
   and `rm -f /boot/<LABEL>.bin`, and the md5 list of `/boot` compared with
   the one taken before the session (equal after every row).

No U-Boot variable was set or saved, nothing but `/boot/<LABEL>.bin` was
written (removed after each row; `/boot` holds its stock files only at the
end), and the eMMC boot areas and the boot command were not touched.

What did not go by the list:

- Before the session `/boot` held a `T6S3R16.bin` of an earlier, stopped
  attempt at this item (another build: its md5 was not this session's). It
  was replaced by this session's image in step 1 and removed with row
  T6S3R16's undo line.
- The autoboot window (1 s) was missed twice, before T6S3R16 and before
  T6S3R18: the spaces arrived after `Hit any key`, the board booted its
  Linux with the row's file still staged, and the reboot of step 2 was
  repeated. No image ran in those boots.
- T6S3R23: the first `md5sum` was typed with the size of the other rows
  (0xc2d550) and so covered 65,536 bytes too few; it was typed again with
  the row's size (0xc3d550) and then equalled the host's. The image was
  entered only after that. Both `md5sum` lines are before the `go` line and
  so outside the capture.

## Files

- `console/<LABEL>.log`: the full serial console of the row, from the
  echoed `go` line to the last byte before the next boot's `U-Boot SPL`
  banner, as the console tool returned it (control bytes written as `\xNN`,
  a byte that is no UTF-8 as U+FFFD), with `\r\n` written as `\n`. Nothing
  was cut. `tools/cts/imx95-redact-check.py` was run over them: its only
  rewrite is rule (a), the dEQP version token of the `dEQP Core` line
  (`vulkan-cts-<1.4.6-2>-555-g93bca018...`), one line per file; `--check` was
  clean before and after, since no host name, address, MAC, serial number
  or UART path appears between `go` and the reset. Each has a `.license`
  sidecar so that the capture stays byte for byte.
- `rows.tsv` (+ `.license`): `tools/cts/imx95-rows-tsv.py --out rows.tsv
  --lifetime-s 600 --bbm-dir <records>/bbm console/T6S3R*.log`; `--check`
  passes with 15 rows.

## Per row

seconds = the uptime of the `run over` line; lifetime 600 s. host = seconds
on the host from `go` until the module's Linux answered ssh again. Every row
ended `done` with `verdin_imx95: board: run over: PSCI SYSTEM_RESET` and
came back to Linux (`LM Boot reason: sw` on the next boot); no hang, no
noboot, no lifetime row, no WDOG3 reset. Every row printed `osal(mali):
restarts 0 ... tiler OOMs 0 grown, 0 with no chunk ... CS stops refused 0`
and `yakogl: draws refused 0`, and `deqp: summary: N cases begun, N with a
result, 0 without`.

| row | label | cases | Pass | Fail | NotSupported | QW | verdict | seconds | host |
|---|---|---|---|---|---|---|---|---|---|
| 16 | T6S3R16 | 100 | 33 | 67 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 5 | 27 |
| 17 | T6S3R17 | 300 | 68 | 232 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 14 | 43 |
| 18 | T6S3R18 | 200 | 31 | 169 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 9 | 33 |
| 19 | T6S3R19 | 300 | 126 | 174 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 14 | 42 |
| 20 | T6S3R20 | 100 | 46 | 54 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 5 | 28 |
| 21 | T6S3R21 | 200 | 56 | 144 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 9 | 40 |
| 22 | T6S3R22 | 200 | 19 | 181 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 9 | 36 |
| 23 | T6S3R23 | 400 | 223 | 177 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 22 | 49 |
| 24 | T6S3R24 | 248 | 191 | 57 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 14 | 37 |
| 25 | T6S3R25 | 34 | 18 | 0 | 16 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 | 32 |
| 26 | T6S3R26 | 34 | 18 | 0 | 16 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 | 25 |
| 27 | T6S3R27 | 35 | 24 | 0 | 11 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 | 32 |
| 28 | T6S3R28 | 35 | 24 | 0 | 11 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 | 26 |
| 29 | T6S3R29 | 35 | 23 | 0 | 12 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 | 32 |
| 30 | T6S3R30 | 35 | 23 | 0 | 12 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 | 26 |

Session: 2,256 cases begun, 2,256 with a result (0 without): 923 Pass,
1,255 Fail, 78 NotSupported, 0 QualityWarning. All 1,255 Fail lines are
shader compile or link refusals: 1,021 `Failed to compile shader`
(`shaders.random.*`, rows R16-R22), 201 `expected shaders to compile and
link properly, but failed to compile.` and 33 `... but failed to link.`
(`shaders.swizzles.*`, rows R23-R24). All 78 NotSupported lines are `Loop
type is not supported` (rows R25-R30). The per-case diff against the T6's
tree and the causes are left to the analysis item (vi95-analysis). The
`yk-record:` words are `594b0201 00000001 00000000 00000000` on a passing
row and `594b0201 00000002 00000000 00000000` on a failing one. The longest
row on the G310 was R23 (22 s), under a twenty-fifth of the 600 s lifetime.

## The BBM record (U2)

Not read back on any row: the module's kernel (Toradex 6.6.157-7.8.0) has no
SCMI raw interface, so `tools/imx95_bbm_record.sh read` exits 3 ("no SCMI
raw interface ... the record cannot be read from Linux") and the `bbm`
column is `not-read` throughout. Whether the record survives the reset stays
open (U2 of `docs/notes/pre-study-imx95.md`); the console's `yk-record:`
line is the record of this session.

Raw unredacted records (the serial reads from `go` to the next Linux boot,
the `.built` lines, the BBM read-backs, the host-timed seconds, the md5
list of `/boot`) are in the gitignored
`records/verdin-imx95/20261127-0628-gles2-S3b-e63ab28e/`.
