<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Verdin iMX95 dEQP-GLES2 S1b, rows T6S1R13-T6S1R24, at e63ab28e

Plan #407 (issue #7, item B8), session S1b: rows T6S1R13 to T6S1R24 of
`tools/cts/mali-gles2-tree.txt` (the NanoPC-T6's rows, unchanged) on the
Toradex Verdin iMX95 (i.MX 95, Mali-G310, Mallow carrier) under Zephyr,
started by hand from the module's own U-Boot prompt. Board session
2026-11-25.

| | |
|---|---|
| commit | `e63ab28e30ae913f06bb595bc10f972fc1c2ff77` (on origin/main) |
| build string | `e63ab28e` (every `.built` line: `build=e63ab28e`; every capture: `vi95_deqp: build e63ab28e`, `sample: build tree=e63ab28e table=6c5b8a70`) |
| Zephyr | `af17c0c8aa2a` (`*** Booting Zephyr OS build af17c0c8aa2a ***`) |
| stage 1 | `tools/cts/deps-env.sh --build DEQP_WORK --board verdin_imx95` (VK-GL-CTS 93bca018) |
| stage 2 | `tools/cts/imx95-deqp-images.sh tools/cts/mali-gles2-tree.txt <img-dir> T6S1R13 ... T6S1R24` |
| CSF firmware | the T6's `mali_csffw.bin`, sha256 a27847ea...a2405b, through `driver/tools/mkmalifw.sh` (not committed) |
| image | `zephyr.bin` 12,768,592 bytes, AArch64 `image_size` 0xbf04000 (191.0 MiB of the 256 MiB window), every row |
| lifetime | 600 s + WDOG3 60 s (`fed until 600 s of uptime` on every console) |

## Route

Per row, as `python3 -I tools/cts/imx95-uboot-row.py` prints it, with the
staging on the boot FAT partition (`--storage 0:1 --stage /boot/`):

1. the module's Linux: `cat > /boot/<LABEL>.bin` over ssh, `md5sum` equal to the host's;
2. reboot to U-Boot (autoboot stopped with a key), then
   `fatload mmc 0:1 0xd0000000 <LABEL>.bin; md5sum 0xd0000000 0xc2d550`
   (equal to the host's md5 on every row; `fatload` instead of the tool's
   `ext4load` because partition 1 is FAT) and
   `dcache off; icache flush; go 0xd0000000`;
3. the image ends by PSCI SYSTEM_RESET; the board boots its Linux again; the
   staged file is removed and the BBM record read back with
   `tools/imx95_bbm_record.sh read`.

At the end `/boot` held the same 31 files with the same md5 as before the
session. No boot partition, boot command, U-Boot variable or other rootfs
file was written.

## Per row

`console/<LABEL>.log` is the full console of the row, from the `fatload`
line typed at the U-Boot prompt to the next boot's `Starting kernel ...`
(the board on its way back to Linux; the Linux boot log after it is not
part of the capture; each log's licence is in its `.license` sidecar).
`rows.tsv` is built from them by `python3 -I tools/cts/imx95-rows-tsv.py
--out rows.tsv --lifetime-s 600 --bbm-dir <records>/bbm console/T6S1R*.log`
(the files, not the directory, which would read the sidecars as captures)
and passes its `--check`.

| row | cases | Pass | Fail | status | run (console) | of the 600 s lifetime | bbm |
|---|---|---|---|---|---|---|---|
| T6S1R13 | 36 | 36 | 0 | done, `deqp: PASS` | 3.0 s | 0.5 % | not-read |
| T6S1R14 | 112 | 108 | 4 | done, `deqp: FAIL rc=-1` | 16.5 s | 2.7 % | not-read |
| T6S1R15 | 80 | 80 | 0 | done, `deqp: PASS` | 14.9 s | 2.5 % | not-read |
| T6S1R16 | 192 | 86 | 106 | done, `deqp: FAIL rc=-1` | 9.8 s | 1.6 % | not-read |
| T6S1R17 | 90 | 87 | 3 | done, `deqp: FAIL rc=-1` | 28.2 s | 4.7 % | not-read |
| T6S1R18 | 58 | 58 | 0 | done, `deqp: PASS` | 4.3 s | 0.7 % | not-read |
| T6S1R19 | 343 | 58 | 285 | done, `deqp: FAIL rc=-1` | 17.5 s | 2.9 % | not-read |
| T6S1R20 | 336 | 68 | 268 | done, `deqp: FAIL rc=-1` | 18.8 s | 3.1 % | not-read |
| T6S1R21 | 294 | 48 | 246 | done, `deqp: FAIL rc=-1` | 15.8 s | 2.6 % | not-read |
| T6S1R22 | 294 | 68 | 226 | done, `deqp: FAIL rc=-1` | 14.9 s | 2.5 % | not-read |
| T6S1R23 | 203 | 203 | 0 | done, `deqp: PASS` | 30.1 s | 5.0 % | not-read |
| T6S1R24 | 374 | 374 | 0 | done, `deqp: PASS` | 32.7 s | 5.5 % | not-read |
| all | 2412 | 1274 | 1138 | 12 done, no hang, no lifetime | | | |

Run (console) is the `run over: PSCI SYSTEM_RESET at N ms` uptime. Timed on
the host, from typing `go` to the Linux kernel's start on the next boot,
the rows took 9 to 40 s (the run plus U-Boot's boot to the kernel). No row
came near the lifetime; none needed WDOG3.

Every case of every row has a result (`deqp: summary: N cases begun, N with
a result, 0 without`). No QualityWarning, NotSupported, crash or GPU reset
appears. The Fail causes visible in the captures are online-compiler
refusals (`glCompileShader(...): online shaderc: the code generator
refused ...`, `Compile failed`), 4 draws refused in each of T6S1R20 and
T6S1R21 (`a vertex shader that reads more than one texture is not built
yet`), and 3 `Image comparison failed` in T6S1R17 (texture.vertex.cube). The
slice conversion, tally and causes are plan #407's analysis item.

The rows `texture.size.*` (T6S1R15) and `texture.wrap.*` (T6S1R18) run 80
and 58 cases; the tree's `# image` line counts the
mustpass names they select (74 and 15).

`dEQP-GLES2.info.renderer` (T6S1R24) reports `Arm Mali-G310v2 (YakoGL)`.

## BBM record (U2)

Not read back on any row: the module's kernel (Toradex BSP 6.6.157) has no
SCMI raw interface, so `tools/imx95_bbm_record.sh read` exits 3 ("no SCMI
raw interface ... the record cannot be read from Linux"). The console's
`yk-record:` line stands for the record: `594b0201 00000001 00000000
00000000` on the PASS rows, `594b0201 00000002 00000000 00000000` on the
FAIL rows.

## The captures

The console was read through the serial-console tool of the lease, which
shows control bytes as `\xNN` text; the captures keep that text (`\x00`
after the reset, `\x08` and `\x1b` in U-Boot's autoboot lines) and drop the
carriage returns. Each capture was checked against the console's byte
offsets: its length, with one CR per line and the `\xNN` and U+FFFD
renderings counted back to one byte, equals the console range from the
`fatload` line to `Starting kernel` on all 12 rows. Every `Test case` line
is followed by the `deqp:` result of the same case, the per-status counts
equal each row's `deqp: done:` line, and the U-Boot `md5sum` line equals
the host image's md5.

Redacted: the module's serial number (`Serial#: <module-serial>`) and the
root partition's PARTUUID (`PARTUUID=<root-partuuid>`). No host name,
address, MAC, user name or host UART path appears in the captures.
