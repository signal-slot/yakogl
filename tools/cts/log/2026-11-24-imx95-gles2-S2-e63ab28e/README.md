# Verdin iMX95 dEQP-GLES2, rows T6S2R01-T6S2R14 on Zephyr from e63ab28e

Plan #407, item vi95-s2 (issue #7, Verdin iMX95 item B8 of
`docs/notes/pre-study-imx95.md`): rows T6S2R01 to T6S2R14 of
`tools/cts/mali-gles2-tree.txt` (the NanoPC-T6's `# image` rows, unchanged),
one Zephyr image per row, on the Toradex Verdin iMX95 (Mali-G310,
GPU_ID 0xac740001) on its Mallow carrier, started from the module's own
U-Boot prompt. The run day is 2026-11-27 (host clock, UTC, first `go` 03:14,
last 03:45); the directory keeps the name the plan item gave it.

## What ran, against what

- Images: commit `e63ab28e` (`e63ab28e30ae913f06bb595bc10f972fc1c2ff77`, an
  ancestor of `origin/main`), built from a detached worktree of that commit:
  stage 1 `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`
  (VK-GL-CTS 93bca018, built fresh for this session), the firmware header
  from `driver/tools/mkmalifw.sh` (linux-firmware's `mali_csffw.bin`, sha256
  a27847ea..., not committed), stage 2
  `tools/cts/imx95-deqp-images.sh tools/cts/mali-gles2-tree.txt <img-dir> T6S2R01 ... T6S2R14`.
- Build string: every `.built` line ends `build=e63ab28e`, and every console
  prints `vi95_deqp: build e63ab28e` and `sample: build tree=e63ab28e
  table=6c5b8a70`. The `zephyr.bin` files are 12,768,592 to 12,854,608 bytes
  (AArch64 image_size 0xbf04000, 191 MiB of the 256 MiB window); lifetime
  600 s (`fed until 600 s of uptime` on every console) plus WDOG3's 60 s.

| label | sha256 of zephyr.bin (`.built`) |
|---|---|
| T6S2R01 | f736eab5009255facd7d7bf75cc5fdfd8c6e1fbc62568b6d776a649da630a25d |
| T6S2R02 | 5197f46355f1606d94826e7710f31b7c313a5965b8207e15074002d21e8baa06 |
| T6S2R03 | 10b88f9352249b6ea898f7e3fbb15f1d3ef4e51a7a27438d70509afee1e7d022 |
| T6S2R04 | 83757d257846317f8f3034fb0c904f446d787e73888080e7e31f93bf7eb04cf4 |
| T6S2R05 | a28c9b8883eecac77991dae4b55a1b31b6ab6b791de944cb47d85ea7c6d482b7 |
| T6S2R06 | 7d763cdfac9c71e83a6ad644a46ddd710586bca0704fcb58aa24b36c67fffbd0 |
| T6S2R07 | 84a7c914338e16958506f7e5ddbacd59b581f19a2317b1c4182699655a773d4f |
| T6S2R08 | 870c78dcb83cc2b100a1a42845a548bfe3c4e5713addc8500e1930deb5b3563f |
| T6S2R09 | fe45282744fc006e8b9e0508409a0c85776b366980f8cd402bbf6c32b942ae3c |
| T6S2R10 | d19514bd773f7f982b7a25654b15d96a7070d44d6490239bf14a164d355b8329 |
| T6S2R11 | 59e7dc87efc15e01cd2e9a7f59aa1378cf1c15f65d64d832a0c64aa73ac5a021 |
| T6S2R12 | a13fe3de47a30b991523b10e467db7d55d89d81f48ba497ccdd2446704dd79ce |
| T6S2R13 | de841c1f1377faee6e87575b4fd3c0697e07e8974aa04ac21b59ff266aba39c2 |
| T6S2R14 | ba1c216b9c73667d4da6544541c950d4162a9ff30deeeb88b8ad379f588e01f8 |

## The route, per row

The hand-typed U-Boot route of `tools/cts/README.md` section 5, lines as
`tools/cts/imx95-uboot-row.py --storage 0:1 --stage /boot/ --fs fat` prints
them (the FAT boot-files partition, mmc 0:1, mounted at `/boot`; S1a's route):

1. module's Linux, over ssh: `gzip -dc > /boot/<LABEL>.bin` (the image,
   gzip'd on the host), `md5sum /boot/<LABEL>.bin` equal to the host's;
2. over ssh, `sync; reboot`; on the console, spaces sent right after U-Boot
   proper's `LM Boot reason` line (autoboot stopped, prompt
   `Verdin iMX95 # `), then Ctrl-C;
3. `fatload mmc 0:1 0xd0000000 <LABEL>.bin; md5sum 0xd0000000 0x<size>`:
   the md5 equal to the host's on every row;
4. `dcache off; icache flush; go 0xd0000000`, the capture starting there;
5. the image ends with PSCI SYSTEM_RESET, the board boots Linux by itself
   (nothing typed), then over ssh `sh -s -- read < tools/imx95_bbm_record.sh`
   and `rm -f /boot/<LABEL>.bin`, and the md5 list of `/boot` compared with
   the one taken before the session (equal after every row).

No U-Boot variable was set or saved, nothing but `/boot/<LABEL>.bin` was
written (removed after each row; `/boot` holds its stock files only at the
end), and the eMMC boot areas and the boot command were not touched.

## Files

- `console/<LABEL>.log`: the full serial console of the row, from the
  echoed `go` line to the last byte before the next boot's `U-Boot SPL`
  banner, as the console tool returned it (control bytes written as `\xNN`).
  Nothing was cut. `tools/cts/imx95-redact-check.py --check console` is
  clean on them as captured: no host name, address, MAC, serial number or
  UART path appears between `go` and the reset. Each has a `.license`
  sidecar so that the capture stays byte for byte.
- `rows.tsv` (+ `.license`): `tools/cts/imx95-rows-tsv.py --out rows.tsv
  --lifetime-s 600 --bbm-dir <records>/bbm console/T6S2R*.log`; `--check`
  passes with 14 rows.

## Per row

seconds = the uptime of the `run over` line; lifetime 600 s. host = seconds
on the host from `go` until the module's Linux answered ssh again (not timed
for T6S2R01). Every row ended `done` with `verdin_imx95: board: run over:
PSCI SYSTEM_RESET` and came back to Linux; no hang, no noboot, no lifetime
row, no WDOG3 reset. Every row printed `osal(mali): restarts 0 ... tiler OOMs
0 ... CS stops refused 0` and `yakogl: draws refused 0`.

| row | label | cases | Pass | Fail | NotSupported | QW | verdict | seconds | host |
|---|---|---|---|---|---|---|---|---|---|
| 1 | T6S2R01 | 400 | 400 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 29 | - |
| 2 | T6S2R02 | 239 | 239 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 17 | 49 |
| 3 | T6S2R03 | 196 | 196 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 22 | 50 |
| 4 | T6S2R04 | 400 | 400 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 48 | 82 |
| 5 | T6S2R05 | 400 | 400 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 46 | 80 |
| 6 | T6S2R06 | 288 | 215 | 73 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 56 | 83 |
| 7 | T6S2R07 | 372 | 165 | 207 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 21 | 50 |
| 8 | T6S2R08 | 372 | 283 | 89 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 23 | 48 |
| 9 | T6S2R09 | 398 | 307 | 91 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 22 | 51 |
| 10 | T6S2R10 | 374 | 273 | 101 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 21 | 50 |
| 11 | T6S2R11 | 400 | 133 | 267 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 22 | 50 |
| 12 | T6S2R12 | 336 | 177 | 159 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 19 | 49 |
| 13 | T6S2R13 | 361 | 262 | 99 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 20 | 48 |
| 14 | T6S2R14 | 336 | 234 | 102 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 19 | 49 |

Session: 4,872 cases begun, 4,872 with a result (0 without): 3,684 Pass,
1,188 Fail, 0 NotSupported, 0 QualityWarning. All 1,188 Fail lines are
shader compile refusals: 1,115 `Failed to compile shader program at
glsShaderRenderCase.cpp` (`shaders.operator.*`, rows R07-R14) and 73
`Compile failed at sglrGLContext.cpp` (`fragment_ops.interaction.*`, row
R06). The per-case diff against the T6's tree and the causes are left to
the analysis item (vi95-analysis). The `yk-record:` words are `594b0201
00000001 00000000 00000000` on a passing row and `594b0201 00000002
00000000 00000000` on a failing one. The longest rows on the G310 were R06
(56 s), R04 (48 s) and R05 (46 s), all under a tenth of the 600 s lifetime.

## The BBM record (U2)

Not read back on any row: the module's kernel (Toradex 6.6.157-7.8.0) has no
SCMI raw interface, so `tools/imx95_bbm_record.sh read` exits 3 ("no SCMI
raw interface ... the record cannot be read from Linux") and the `bbm`
column is `not-read` throughout. Whether the record survives the reset stays
open (U2 of `docs/notes/pre-study-imx95.md`); the console's `yk-record:`
line is the record of this session.

Raw unredacted records (the serial reads from `go` to the next Linux login,
the `.built` lines, the BBM read-backs, the md5 list of `/boot`) are in the
gitignored `records/verdin-imx95/20261127-0310-gles2-S2-e63ab28e/`.
