<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# One method on every board: the runners before, and the acceptance runs of 2026-10-03

Status: finished record, moved here from `driver/docs/one-method.md` (section 1.6, the Step 1 acceptance run and the Step 5 acceptance run) on 2026-10-11. The runners it lists were replaced by `tools/board.sh`; the design and the migration state are `driver/docs/one-method.md`.

## The runners before Step 1 (from `driver/docs/one-method.md` section 1.6)

| Step | Pi 4 tryboot runner | AM62P netboot runner (U-Boot TFTP) | i.MX8MP netboot runner (U-Boot TFTP) | AM62P and i.MX8MP kexec runners (dev) |
|---|---|---|---|---|
| Lock | none; scans `/proc` for UART holders, `who` for other hosts (`:73-90`) | `board_lock.sh` | `board_lock.sh` | none (wrapper-jump checks uptime and processes) |
| Pre-checks | ssh+sudo, `/boot/firmware` free >= 2x image | image header, footprint, `rti_arm: armed` marker, TFTP canary, `/dev/pvrgpu` free, hook installed | header, footprint, `gc_wdog_arm: armed`, TFTP canary + log, no kexec loaded | AM62P: bridge "busy" retry; i.MX8MP: up >= 180 s, M7 offline |
| Clear sink | - | - | zero SNVS (+ selector) | i.MX8MP: zero SNVS (+ selector) |
| Stage | `/boot/firmware/pi4run-<label>.bin` + `tryboot.txt`, read back | TFTP root `yakogl/<token>.bin` | same | `~/kexec` via `kexec_run.sh` |
| Boot | `reboot "0 tryboot"` | `fw_setenv` one-shot, reboot | same | kexec |
| Wait | end-line regex or 420 s, then new `boot_id` | UART phase scan, then ssh | ssh from a new boot (uptime) | `deqp: summary` or 1500 s |
| Read | UART log | UART log | TFTP log + SNVS | UART / SNVS |
| Verdict | rc 0/1/2 | `verdict=` line, rc 0-6 | `verdict=` line | rc 0-6 / record |
| Records | `record_<label>.txt`, `<label>.meta` | `<logdir>`, env backup in `records/am62p/netboot` | `records/imx8mp/netboot` | `<log>` |

**Defects found while reading.** The lines are at `700a4310`. Status was
checked against the branches named.

| Defect | Where | Status |
|---|---|---|
| The lock directory is hard-coded into one old session's scratchpad (of the pre-merge repository path), so the lock is no lock for any other session | `driver/tools/kexec-zephyr/board_lock.sh:75` | fix on branch `tools/board-lock-dir` (not on main) |
| Every `gl_test` run gets rc 4, because the script waits for `deqp: summary` | the AM62P kexec runner, its wait loop | gone with the script in Step 1: `board.sh` ends a run on `gl_test: all:` too |
| The netboot scan reports `rti_armed=absent` for runs that armed, because the marker comes before the boot banner | `tools/am62p_netboot_uart.py` (`scan`) | fixed on `am62p/netboot-scan-fix` `f67f4e74` |
| `verdict=done` for a breadcrumb-only record | the i.MX8MP netboot runner, its verdict | fixed on `imx8mp/cold-entry` `18487926` (`died_after=<stage>`, rc 4) |
| The record is decoded twice | the Python record decoder, the i.MX8MP tree runner | fixed: one decoder, `yakogl-tools record` on `record.rs`; both scripts deleted in Step 1 |

## Step 1 acceptance run, 2026-10-03

The one `slint` sample (`unify/slint` `96a10940`, on the board layer
`db659a88`) ran once per board from the bootloader boot path. Every number
below is read from the record files named in the last column (under
`records/`, which is gitignored; addresses, MACs and serials are redacted in
the copies).

| | AM62P | i.MX8MP | Raspberry Pi 4 |
|---|---|---|---|
| boot path | U-Boot netboot, env route (TFTP) | U-Boot netboot, env route (TFTP) | firmware tryboot |
| image sha256 | `a24ec8cff8034403...` | `d526fdd238049b26...` | `4c7ecefef3fb7803...` |
| image size | 18,704,464 B (TFTP, 0.273 s) | 5,491,416 B (TFTP, 0.079 s) | 4,147,624 B |
| window | 1024x600 | not read (no UART) | 1024x600 |
| GL_RENDERER | PowerVR BXS-4-64 (YakoGL) | not read (no UART) | Broadcom V3D 4.2 (YakoGL) |
| 5 points, diff vs reference | 0 / 1 / 0 / 0 / 0 | not read (no UART) | 1 / 1 / 0 / 0 / 0 |
| check | PASS (tolerance 1) | PROBES status 1 | PASS (tolerance 1) |
| window crc32 | `0x922d777a` | not read | `0x5c41b81a` |
| animation | 708 frames / 10011 ms | not read | 651 frames / 10008 ms |
| PROBES record | `594b0601 08420882 00210842 98080883` | `594b0601 08420882 00210842 98080883` (SNVS) | `594b0601 08420862 00210842 c5920883` |
| board setup | holds 60 s, then the reset (`board: holding 60 s, then the reset`) | not read (no UART) | `board: V3D clock 250000000 -> 500000000 Hz (0)`; holds 60 s, then the reset |
| GPU resets | not counted | not counted | 0 |
| back to Linux | 112 s after the arm | 97 s | tryboot at 18:30:53, Linux back 18:32:53 |
| records | `records/am62p/slint-one-20261003-1827/` | `records/imx8mp/slint-one-20261003-1830/` | `records/pi4/slint-one-20261003-1830/` |

Two facts, stated plainly:
- The PROBES record is identical on the two Verdins. On the Pi 4 it differs
  at point 0 only, by one LSB of green (`0882` against `0862`), which is
  inside the tolerance of 1 per channel. The last word (the CRC16 of the five
  points) differs with it.
- The whole-window crc32 differs per GPU (`0x922d777a` against `0x5c41b81a`).
  That is expected: bit-equal anti-aliasing across three rasterisers is not
  the acceptance (see above). It is reported, not judged.

The i.MX8MP has no UART, so its row is only what the SNVS record carries (the
PROBES words and status 1). Its renderer string, window, per-point diffs,
crc32 and frame count are not in the records and are not claimed.

Still open in Step 1:
- the one runner (`tools/board.sh`) is in work; these three runs used each
  board's current runner;
- step 1b, `board.sh` reproducing one proven row per board, has not run;
- the deletions listed above are not done.

## Step 5 acceptance run, 2026-10-03

One `cluster` image per board,
tree `0f0c4240`, booted from the bootloader; every value is read from the
run's console and INDEX (copied from the runner's `build/` into `records/`, gitignored).
"Smooth" is no animation frame over 33.3 ms (two 60 Hz periods), from the
animation line's worst frame; fps is not a performance measure here
(that is glmark2-es2, a separate task). "Scan-out" is the window shown in
place (`scanned in place`, no `board: board_show copies` line).

| board | boot path | image sha256 | window | probes | frames / 60 s | smooth | scan-out | run |
|---|---|---|---|---|---|---|---|---|
| AM62P (PowerVR) | U-Boot netboot (TFTP) | `4dcdf3e13ffffe79...` | 1024x600 XRGB8888 | 15/15 (worst 2) | 6177 / 60015 ms (102.9 fps) | yes (worst 16.8 ms) | yes (DSS flip) | `records/am62p/20261003-2133-cluster-4dcdf3e1/` (console.txt sha256 34a35651..., 2022 lines) |
| i.MX8MP (GC7000) | U-Boot netboot (TFTP) | pending | 1280x800 | pending | pending | pending | pending | pending |
| Pi 4 (V3D) | firmware tryboot | `92f0c68eca508732...` | 1920x720 RGBA8888 (firmware: pixel order RGB) | 34/34 (worst 1) | 8391 / 60014 ms (139.8 fps) | yes (worst 30.6 ms) | yes (pan) | `records/pi4/20261003-2133-cluster-92f0c68e/` (console.txt sha256 b27678be..., 220 lines) |

Both runs PASS (PROBES status 1; the Pi 4 counted 0 GPU resets, the
AM62P's driver does not count them). The AM62P's first frame takes
10.4 s: six FemtoVG fragment shaders (the no_std patch's NO_DISCARD fill
variants) are not in the PowerVR offline table, and each miss prints its
source on the 115200-baud UART (119,700 bytes, about 10.4 s, in the
2026-10-03 console); not yet fixed. The Pi 4 compiles online: 0.49 s.
