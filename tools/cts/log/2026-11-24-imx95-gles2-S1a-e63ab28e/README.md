# Verdin iMX95 dEQP-GLES2, rows T6S1R01-T6S1R12 on Zephyr from e63ab28e

Plan #407, item vi95-s1a (issue #7, Verdin iMX95 item B8 of
`docs/notes/pre-study-imx95.md`): rows T6S1R01 to T6S1R12 of
`tools/cts/mali-gles2-tree.txt` (the NanoPC-T6's `# image` rows, unchanged),
one Zephyr image per row, on the Toradex Verdin iMX95 (Mali-G310,
GPU_ID 0xac740001) on its Mallow carrier, started from the module's own
U-Boot prompt. The run day is 2026-11-25 (board clock, UTC, 10:35 to 10:58);
the directory keeps the name the plan item gave it.

## What ran, against what

- Images: commit `e63ab28e` (`e63ab28e30ae913f06bb595bc10f972fc1c2ff77`, on
  `origin/main`), built from a detached worktree of that commit:
  stage 1 `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95`
  (VK-GL-CTS 93bca018), the firmware header from
  `driver/tools/mkmalifw.sh` (linux-firmware's `mali_csffw.bin`, sha256
  a27847ea..., not committed), stage 2
  `tools/cts/imx95-deqp-images.sh tools/cts/mali-gles2-tree.txt <img-dir> T6S1R01 ... T6S1R12`.
- Build string: every `.built` line ends `build=e63ab28e`, and every console
  prints `vi95_deqp: build e63ab28e` and `sample: build tree=e63ab28e
  table=6c5b8a70`. Each image's `zephyr.bin` is 12,768,592 bytes
  (AArch64 image_size 0xbf04000, 191 MiB of the 256 MiB window); lifetime
  600 s (`fed until 600 s of uptime` on every console) plus WDOG3's 60 s.

| label | sha256 of zephyr.bin (`.built`) |
|---|---|
| T6S1R01 | f783633cd96da68b8e21655a094e935e191951731176a56a9d7d8bf619a2df47 |
| T6S1R02 | e9b3b71e4427fe7249696fd5e608a9d1808200deb16f4f6aab4d8b8612f20df0 |
| T6S1R03 | 45b63175a454f383c6c8742ea9ef61b115d49eb08d62573b349efd2dd4ec63e5 |
| T6S1R04 | 24c6404c6294118050f617c2685efb47184d334760b79975e764dce897f81688 |
| T6S1R05 | 4bf0e352bdec3d54981bfc9b123fee9e354774cc33398f87657d193963db7988 |
| T6S1R06 | db61e07f7719ce00eaba39c358b50e476a3892d14c0cbd8046c714dfbc22c1f2 |
| T6S1R07 | 7c6badea5facdf0a26dc70a06d153cb7062a80ee16b02f889cb9574efc7b894a |
| T6S1R08 | ad6acc8200c2c71be94d63d6ba815ac4ac2e940c8968b1fb7a428815df49a36e |
| T6S1R09 | 8421785ab9905e2544237c85593edac0ecaac0458611b3288ffcf6034e4f4be8 |
| T6S1R10 | 4a0f01d8fced98d07f6b7cfac1580ed5bab183863af869e558df3d42f3d38fe3 |
| T6S1R11 | 7176496ca85c623de385df7f136f4e5c07e1e7a3c5ed419d42d9a5cd278f9962 |
| T6S1R12 | 07da7e92ebf763dd8b8ad0deb32ae215add43e447ccbef9d4bcce8b3c960f84c |

## The route, per row

The hand-typed U-Boot route of `tools/cts/README.md` section 5, lines as
`tools/cts/imx95-uboot-row.py --storage 0:1 --stage /boot/` prints them,
with one change: the staged file lives on the FAT boot-files partition
(mmc 0:1, mounted at `/boot`), so U-Boot reads it with `fatload` and the
file name relative to that partition, not `ext4load`:

1. module's Linux, over ssh: `gzip -dc > /boot/<LABEL>.bin` (the image,
   gzip'd on the host), `md5sum /boot/<LABEL>.bin` equal to the host's;
2. console: log in as root, `sync; reboot`, a space as U-Boot starts
   (autoboot stopped, prompt `Verdin iMX95 # `), Ctrl-C;
3. `fatload mmc 0:1 0xd0000000 <LABEL>.bin; md5sum 0xd0000000 0xc2d550`:
   the md5 equal to the host's on every row;
4. `dcache off; icache flush; go 0xd0000000`, the capture starting there;
5. the image ends with PSCI SYSTEM_RESET, the board boots Linux by itself
   (nothing typed), then over ssh `sh -s -- read < tools/imx95_bbm_record.sh`
   into `bbm/<LABEL>.txt` and `rm -f /boot/<LABEL>.bin`.

No U-Boot variable was set or saved, nothing but `/boot/<LABEL>.bin` was
written (removed after each row; `/boot` holds its stock files only at the
end), and the eMMC boot areas and the boot command were not touched.

## Files

- `console/<LABEL>.log`: the full serial console of the row, from the
  echoed `go` line to the last byte before the next boot's `U-Boot SPL`
  banner, as the console tool returned it (control bytes written as `\xNN`).
  Nothing was cut; nothing in them needed redaction (no host name, address,
  MAC, serial number or UART path appears between `go` and the reset).
  Each has a `.license` sidecar so that the capture stays byte for byte.
- `rows.tsv` (+ `.license`): `tools/cts/imx95-rows-tsv.py --out rows.tsv
  --lifetime-s 600 --bbm-dir bbm console/T6S1R*.log`; `--check` passes with
  12 rows.
- `bbm/<LABEL>.txt`: the BBM read-back after each row.

## Per row

seconds = the uptime of the `run over` line; lifetime 600 s. Every row
ended `done` with `verdin_imx95: board: run over: PSCI SYSTEM_RESET` and
came back to Linux; no hang, no noboot, no lifetime row, no WDOG3 reset.
Every row printed `osal(mali): restarts 0 ... tiler OOMs 0 ... CS stops
refused 0` and `yakogl: draws refused 0`.

| row | label | cases | Pass | Fail | NotSupported | QW | verdict | seconds |
|---|---|---|---|---|---|---|---|---|
| 1 | T6S1R01 | 224 | 199 | 17 | 8 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 13 |
| 2 | T6S1R02 | 264 | 264 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 14 |
| 3 | T6S1R03 | 349 | 333 | 16 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 24 |
| 4 | T6S1R04 | 101 | 101 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 6 |
| 5 | T6S1R05 | 392 | 392 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 8 |
| 6 | T6S1R06 | 319 | 319 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 23 |
| 7 | T6S1R07 | 89 | 52 | 21 | 16 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 3 |
| 8 | T6S1R08 | 1 | 0 | 0 | 1 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 0 |
| 9 | T6S1R09 | 361 | 349 | 0 | 9 | 3 | `deqp: OK (rc=0)`, `deqp: PASS` | 13 |
| 10 | T6S1R10 | 396 | 393 | 3 | 0 | 0 | `deqp: FAILED (rc=-1)`, `deqp: FAIL rc=-1` | 8 |
| 11 | T6S1R11 | 40 | 40 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 3 |
| 12 | T6S1R12 | 248 | 248 | 0 | 0 | 0 | `deqp: OK (rc=0)`, `deqp: PASS` | 42 |

Session: 2,784 cases begun, 2,784 with a result (0 without): 2,690 Pass,
57 Fail, 34 NotSupported, 3 QualityWarning (dEQP's `warnings`). The
`yk-record:` words are `594b0201 00000001 00000000 00000000` on a passing
row and `594b0201 00000002 00000000 00000000` on a failing one.

## The BBM record (U2)

Not read back on any row: the module's kernel (Toradex 6.6.157-7.8.0) has no
SCMI raw interface, so `tools/imx95_bbm_record.sh read` exits 3 ("no SCMI
raw interface ... the record cannot be read from Linux") and the `bbm`
column is `not-read` throughout. Whether the record survives the reset stays
open (U2 of `docs/notes/pre-study-imx95.md`); the console's `yk-record:`
line is the record of this session.

Raw unredacted records (the console from `go` to the next Linux login,
the timing marks, the `.built` lines) are in the gitignored
`records/verdin-imx95/20261125-1035-gles2-S1a-e63ab28e/`.
