<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Board console map

Where each board's serial console is reached. The tools hard-code none of
these values: `tools/board.sh` reads `BOARD_UART` (alias `UART_ADDR` for the
AM62P, `PI4_UART` for the Pi 4) from the environment. For the AM62P it is the
`host:port` of a TCP serial bridge. Set it from this table.

| Board | Node | Tailscale IP | LAN IP | Port | Channel | Status |
|---|---|---|---|---|---|---|
| Toradex Verdin AM62P (main_uart0, Linux ttyS2: U-Boot, getty, Zephyr) | `usb-serial-1` | 100.118.139.121 | 192.168.1.51 (direct, ~75 ms) | **35933** | FT4232H channel 4 | **Verified** |
| Raspberry Pi 4 | `usb1` | 100.124.238.76 | 192.168.1.39 (direct, ~80 ms) | 35931 | Debug Probe interface 1 | **Verified** (boot log to login prompt) |
| i.MX8MP | unknown | unknown | unknown | unknown | unknown | Nothing recorded (board.sh notes the i.MX8MP has no UART capture) |

`BOARD_UART=100.118.139.121:35933` selects the AM62P console.

## Node usb-serial-1 (one FT4232H, four UART channels)

Open TCP ports 35930-35933. Port-to-UART meaning, as far as it is known:

| Port | Meaning | Source |
|---|---|---|
| 35933 | AM62P main_uart0: Linux login prompt | Owner, 2026-10-09 |
| 35932 | AM62P mcu_uart0 (Linux ttyS3); never carries U-Boot, getty or Zephyr output; `tools/cts/RESULTS.md` 1a calls it "dead" | `am62p-gles3-percase.md` (measured earlier; silent again in `am62p-es2-r1.md`) |
| 35930, 35931 | not recorded | n/a |

## How verified

- **usb-serial-1:35933, AM62P.** Owner, 2026-10-09: Enter on 35933 returned
  `verdin-am62p-08641335 login:`. Re-checked read-only the same day from the
  ccfarm worker: TCP connect to 100.118.139.121:35933, send `\r`, received
  `\r\nverdin-am62p-08641335 login: `. (A first attempt with one `\r` and a
  3 s wait read nothing; a second with `\r` twice and 4 s waits did.) Nothing
  was typed after that; no login.
- **usb1:35931, Pi 4.** Owner, 2026-10-09 (morning), on the real board:
  through usb1:35931 the Pi 4's boot log was received in full up to the login
  prompt. The bridge works. This supersedes the earlier note (same day) that
  the port accepted connections but gave no answer to Enter and that power
  and boot were unknown. (Only 35931 is open on usb1.) So a missing Pi 4 run
  record is not caused by the bridge.

## Still unverified (guesses are marked as such)

- `tools/board.sh` addresses the Pi 4 as a serial **device**
  (`[ -c "$BOARD_UART" ]`, `stty`; lines 751 and 822), not `host:port`.
  `usb1:35931` is a TCP bridge, so the Pi 4 path cannot use it as is (it would
  need a local pty, e.g. a socat bridge; not set up). Fact: the Pi 4 tool
  hard-codes no host:port (`BOARD_UART`/`PI4_UART` from the environment, a
  device path; `/dev/PI4_UART` only as a dry-run placeholder, lines 1417/1431),
  so it cannot "match" usb1:35931 as configured. Whatever device was set for
  a Pi 4 run is not recorded in the repository (unknown). Likely gap (guess):
  no bridge-to-pty step existed for the Pi 4.
- Meaning of 35930 and 35931 on usb-serial-1; whether the channel order is
  stable across re-plugging.
- Everything about the i.MX8MP console.
- The bridge baud (115200 per `docs/direct-boot.md`) cannot be checked from
  the host.
