<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# 2026-11-25 t6-uboot 65af54b5: session 1 from U-Boot, stopped before the prompt

Plan #408, item uboot-s1 (issues #7 and #4627): start the NanoPC-T6's probe
images from the U-Boot prompt (`driver/tools/uboot-t6/`) instead of kexec.
Run order: `prompt-reads.sh --go`, then `prompt-go.sh --go` for t6_cold,
t6_gpu, t6_fw and t6_submit, stopping at the first failure.

## What ran, against what

- Images built from `origin/main` 65af54b5
  (65af54b50fba26e15c1d5d7d1d20b0b9bab95cb4), `west build -p always -b
  nanopc_t6 ... -DYAKOGL_WERROR=ON` with `ZEPHYR_MODULES` from
  `tools/cts/zephyr-modules.sh`, as `tools/check.sh`'s `zephyr nanopc_t6`
  rows build them. CSF firmware header from `mali_csffw.bin` sha256
  a27847ea11f8efb3136340c3ba8aab413ae25145eeb4a7f64ff5edd829a2405b
  (`driver/tools/mkmalifw.sh`).

  | Image | Bytes | CRC-32 | sha256 |
  |---|---|---|---|
  | t6_cold | 127184 | 2b3654ed | 578017a4fecd4ff7d512261ab0acd185ecc98c8a6b2c4608a8a85233eabd3b79 |
  | t6_gpu | 86224 | 3aca1a4e | 53a8ad3cf4c4171d002661147f408595a2f27632652469f2fa09194f000eab0c |
  | t6_fw | 454752 | b6d60b71 | 9f4b26c55a13d9c40f915cc8906ed07c838af6791cb5b283344377115e110bd0 |
  | t6_submit | 4178016 | 8eae088b | cf6f16bd3d154de274f2c5511b7ed4becb7d5148a9cdd9f95f4cf142d772df50 |

  Each image: linked at 0x10000000, WDT0 TOP 13, lifetime 60 s.

- Environment names in effect: `BOARD_HOST`, `T6_UART`
  (`<rfc2217-uri>`, the network serial adapter on the debug UART),
  `T6_RECORD_DIR`, `T6_HAND` (`--go` only). Values are not recorded.

## Commands and exits (board clock UTC, 2026-11-25)

| Step | Command | Exit |
|---|---|---|
| 1 | `stage.sh stage` t6_cold, t6_gpu, t6_fw, t6_submit | 0, 0, 0, 0 |
| 2 | `prompt-reads.sh` (dry run; self-test 115 checks, 0 failed) | 0 |
| 3 | `prompt-go.sh <image> <probe>` (dry run) for the four images | 0, 0, 0, 0 |
| 4 | `prompt-reads.sh --go` at 05:56:00 | **40** |
| - | `prompt-go.sh --go` t6_cold, t6_gpu, t6_fw, t6_submit | not run (stop at the first failure) |
| 5 | `stage.sh undo` ("stock /boot restored") | 0 |

The dry runs found every staged copy byte-equal to its image (sha256) and
printed the prompt commands with the expected answers (`runs.txt`).

## The failure: the adapter's one client slot was held

Exit 40 is `t6-uart-rfc2217.py`'s "the line could not be set, nothing
done": the adapter answered `busy: another client is connected` and closed
the connection. A raw TCP connection got the same answer. The client
holding the slot was the farm's own serial console server for this lease
(the serial MCP server of `resource:serial-nanopc-t6`): its status read
`connected`, RFC 2217, **115200** baud set by the adapter (the adapter's
stored rate), while the T6's console runs at 1500000 (`stty` on Armbian's
UART2 read 1500000). A 24-character line written to that UART from Armbian
reached the console server as 3 bytes of garbage. So neither route to the
prompt worked in this lease: the runner could not open its own session, and
the console server's session was at the wrong rate (typing into it would
reach the Linux console as framing errors, a break and SysRq, which the
adapter's README warns can reboot the target).

The earlier attempt of this item (2026-11-25, images at cbb55860, not
merged) stopped at the same exit 40 for the same reason.

## Resets and span

No reboot: boot_id unchanged from the first ssh read to the last (session
05:50 to 06:05 UTC). Nothing was written to the UART. `/boot/yakogl` was
made by `stage` and removed by `undo`; `/boot`'s 422 other entries matched
the fingerprint before and after.

## What the next session needs

The adapter's slot free for `t6-uart-rfc2217.py` (a serial lease without the
console server's connection), or the console server opening the adapter at
1500000; and a `T6_HAND` who can cut the board's power, since from `go`
until the image arms WDT0 nothing resets a stopped image.

Full record (gitignored): `records/nanopc-t6/20261125-0600-t6-uboot-s1-65af54b5/`.
