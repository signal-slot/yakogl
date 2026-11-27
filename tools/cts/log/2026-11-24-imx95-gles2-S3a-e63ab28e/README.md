# Verdin iMX95 dEQP-GLES2, rows T6S3R01-T6S3R15 on Zephyr from e63ab28e

Plan #407, item vi95-s3a (issue #7, Verdin iMX95 item B8 of
`docs/notes/pre-study-imx95.md`): rows T6S3R01 to T6S3R15 of
`tools/cts/mali-gles2-tree.txt` (the NanoPC-T6's `# image` rows, unchanged),
one Zephyr image per row, on the Toradex Verdin iMX95 (Mali-G310,
GPU_ID 0xac740001) on its Mallow carrier, started from the module's own
U-Boot prompt. The run day is 2026-11-27 (host clock, UTC, the first `go` at
04:20:49, the last row over at about 04:47); the directory keeps the name
the plan item gave it.

## What ran, against what

- Images: commit `e63ab28e` (`e63ab28e30ae913f06bb595bc10f972fc1c2ff77`, on
  `origin/main`), built from a detached worktree of that commit:
  stage 1 `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`
  (VK-GL-CTS 93bca018), the firmware header from
  `driver/tools/mkmalifw.sh` (linux-firmware's `mali_csffw.bin`, sha256
  a27847ea..., not committed), stage 2
  `tools/cts/imx95-deqp-images.sh tools/cts/mali-gles2-tree.txt <img-dir> T6S3R01 ... T6S3R15`.
- Build string: every `.built` line ends `build=e63ab28e`, and every console
  prints `vi95_deqp: build e63ab28e` and `sample: build tree=e63ab28e
  table=6c5b8a70`. Each image's `zephyr.bin` is 12,768,592 bytes
  (AArch64 image_size 0xbf04000, 191 MiB of the 256 MiB window); lifetime
  600 s (`fed until 600 s of uptime` on every console) plus WDOG3's 60 s.

| label | sha256 of zephyr.bin (`.built`) |
|---|---|
| T6S3R01 | 71c5ea67ab6432f5eedc9e7998e3d0562c4892ad04fe654bd6a1ec568df24c77 |
| T6S3R02 | df71438224cc5fe152be8b42749085d3be8ea94cffdd0553ffe867a3b8ef05e7 |
| T6S3R03 | 898c37fc1623f6ea655488be92d83e65d85c66be35e2c0526d9af52ec4bf82c4 |
| T6S3R04 | 665d7a1b9c14530440544641c7d8729c62bf8d2cc6117f1a3cd631f613c64d65 |
| T6S3R05 | 9bf5c958148228e4203cc05a9e1ec8eafcbf8fe0308da1cde2f82eb5db22ec8d |
| T6S3R06 | 67d9fdc6cc179a108b7f05b731c6dd7e72d9112fbb3142f79e4bb295f7a07f27 |
| T6S3R07 | 7919542c65e0610b4fd0fbba14e4727ce4d28d4e0c455594db1f5c03ac142464 |
| T6S3R08 | bc775cf19458156cdcdabd86c51dc5aca1672a2a6e42ec9a748302d2aeff460c |
| T6S3R09 | 03554d38da7502d859d8a32b8ec9f386291b9d455bb68b07325683cb47979491 |
| T6S3R10 | b304dfcd594d86e4d565d64c4b472992689b40e9cbdb5dfbe934fabcf4904891 |
| T6S3R11 | 0bbf5c075422f410d4f1a47436fbe4c18958f090d02d10e49f5578a29e3ecba1 |
| T6S3R12 | 89eece5ee1ab2e70e8a8236e137722f0782e5b6b9ee0b98ac02541581bd3ee4d |
| T6S3R13 | c00069159a638d0241a38f7925deda32da73fe826697474fb9ec53127b7fc192 |
| T6S3R14 | f93d6497c35d195fc8b6a4a7a5b63c45f0728b411d49cdd12df7d0cafe11cf95 |
| T6S3R15 | 73621a737c81590840d8ca5d0e3502f4fa93a721a9602dc81aa53e9cbd535ba5 |

## The route, per row

The hand-typed U-Boot route of `tools/cts/README.md` section 5, FAT
boot-files partition (session S1a's route), lines as
`tools/cts/imx95-uboot-row.py --storage 0:1 --stage /boot/ --fs fat <img-dir> T6S3R01-T6S3R15`
prints them:

1. module's Linux, over ssh: `rm -f /boot/<LABEL>.bin`, `cat >
   /boot/<LABEL>.bin` (the image), `md5sum /boot/<LABEL>.bin` equal to the
   host's;
2. `sync; reboot` (from the console's root login for rows 1 and 2, over
   ssh after that), spaces typed on the console while U-Boot started
   (autoboot stopped, prompt `Verdin iMX95 # `), Ctrl-C;
3. `fatload mmc 0:1 0xd0000000 <LABEL>.bin; md5sum 0xd0000000 0xc2d550`:
   the md5 equal to the host's on every row;
4. `dcache off; icache flush; go 0xd0000000`, the capture starting there;
5. the image ends with PSCI SYSTEM_RESET, the board boots Linux by itself
   (nothing typed), then over ssh `sh -s -- read < tools/imx95_bbm_record.sh`
   into `bbm/<LABEL>.txt` and `rm -f /boot/<LABEL>.bin`.

Before row 1 the autoboot window (bootdelay 1) was missed twice and the
board booted Linux; before row 4 and row 15 once each. Nothing ran in
those boots; the next try stopped U-Boot. No U-Boot variable was set or
saved, nothing but `/boot/<LABEL>.bin` was written (removed after each row;
at the end the files of `/boot` have the md5 sums they had before the
session), and the eMMC boot areas and the boot command were not touched.

## Files

- `console/<LABEL>.log`: the full serial console of the row, from the
  echoed `go` line to the last byte before the next boot's `U-Boot SPL`
  banner, as the console tool returned it (control bytes written as `\xNN`).
  Nothing was cut. `imx95-redact-check.py --check console` is clean on them
  as captured, so they are committed unchanged: no host name, address, MAC,
  serial number or UART path appears between `go` and the reset. Each has a
  `.license` sidecar so that the capture stays byte for byte.
- `rows.tsv` (+ `.license`): `tools/cts/imx95-rows-tsv.py --out rows.tsv
  --lifetime-s 600 --bbm-dir bbm console/T6S3R*.log`; `--check` passes with
  15 rows.
- `bbm/<LABEL>.txt`: the BBM read-back after each row.

## Per row

seconds = the uptime of the `run over` line; lifetime 600 s. Every row
ended `done` with `verdin_imx95: board: run over: PSCI SYSTEM_RESET` and
came back to Linux; no hang, no noboot, no lifetime row, no WDOG3 reset.
Every row printed `osal(mali): restarts 0 ... tiler OOMs 0 ... CS stops
refused 0` and `yakogl: draws refused 0`, and `deqp: summary: N cases
begun, N with a result, 0 without`.

| row | label | cases | Pass | Fail | NotSupported | QW | verdict | seconds |
|---|---|---|---|---|---|---|---|---|
| 1 | T6S3R01 | 96 | 9 | 17 | 70 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 6 |
| 2 | T6S3R02 | 160 | 40 | 16 | 104 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 9 |
| 3 | T6S3R03 | 32 | 23 | 0 | 9 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 |
| 4 | T6S3R04 | 64 | 2 | 14 | 48 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 5 |
| 5 | T6S3R05 | 216 | 63 | 33 | 120 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 13 |
| 6 | T6S3R06 | 1 | 1 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 0 |
| 7 | T6S3R07 | 68 | 52 | 16 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 5 |
| 8 | T6S3R08 | 208 | 3 | 205 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 11 |
| 9 | T6S3R09 | 70 | 58 | 12 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 5 |
| 10 | T6S3R10 | 70 | 57 | 13 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 5 |
| 11 | T6S3R11 | 270 | 164 | 106 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 16 |
| 12 | T6S3R12 | 324 | 153 | 171 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 18 |
| 13 | T6S3R13 | 144 | 89 | 55 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 9 |
| 14 | T6S3R14 | 200 | 6 | 194 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 36 |
| 15 | T6S3R15 | 300 | 92 | 208 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 15 |

Session: 2,223 cases begun, 2,223 with a result (0 without): 812 Pass,
1,060 Fail, 351 NotSupported, 0 QualityWarning. Row T6S3R14's filter
(`random.all_features.*`) ran 200 cases where the table's header says 198:
two cases of the filter that the mustpass list lacks. The `yk-record:`
words are `594b0201 00000001 00000000 00000000` on a passing row and
`594b0201 00000002 00000000 00000000` on a failing one. The longest row
(T6S3R14) used 36 s of its 600 s.

## The BBM record (U2)

Not read back on any row: the module's kernel (Toradex 6.6.157-7.8.0) has no
SCMI raw interface, so `tools/imx95_bbm_record.sh read` exits 3 ("no SCMI
raw interface ... the record cannot be read from Linux") and the `bbm`
column is `not-read` throughout. Whether the record survives the reset stays
open (U2 of `docs/notes/pre-study-imx95.md`); the console's `yk-record:`
line is the record of this session.

Raw records (the console captures as read, the timing marks with the
console offsets of each `go` and end, the U-Boot md5 lines, the md5 list of
`/boot` before the session) are in the gitignored
`records/verdin-imx95/20261127-0418-gles2-S3a-e63ab28e/`.
