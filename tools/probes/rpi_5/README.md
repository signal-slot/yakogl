<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5 probes

Zephyr images for the Pi 5 (`rpi_5`, BCM2712, V3D 7.1), one directory each,
run in the order of the bring-up (`docs/notes/pre-study-rpi5.md`, section 6).
Each ends with a line a run can grep for.

| Probe | Shows | End line | In the tree |
|---|---|---|---|
| `rpi5_wdog` | the BCM2712 PM watchdog resets the board (the first board fact) | `rpi5_wdog: feeding stopped, reset expected within 10 s`, then the console goes quiet and the boot loader follows; `rpi5_wdog: FAIL no reset` if it did not reset | yes |
| `rpi5_hello` | the smallest image boots from `kernel=` and prints on uart10 | planned | planned |
| `rpi5_dtb` (D1) | the DTB pointer the firmware passes and the V3D, SMS, HVS, clock and `pm` nodes in it; answers U6 | planned | planned |
| `rpi5_ident` (D2) | the V3D hub and core identity and `MMU_DEBUG_INFO` read with no setup, equal to Linux's | planned | planned |
| `rpi5_clear` (M1 to M4) | power-on, identity (raw, with `MMU_DEBUG_INFO`), MMU, the 7.1 clear-only frame binned and rendered, a TLB clear to a 128x64 raster target read back | `v3d_clear: PASS (...)` or `v3d_clear: FAIL (reason)` | yes (built, not run) |

`common/boot` holds the shared entry-state stub (`boot_x0`) that keeps the
firmware's x0; see its README. Details of each probe are in its own README.

## Rules every Pi 5 image follows

- **Arm the PM watchdog first.** The board has no remote power or reset; an
  image that hangs holds it until a person power-cycles it. The board layer
  (`driver/zephyr/lib/board/rpi4/rpi_wdog.c`, SoC a parameter) arms it in the
  first init hook, so tryboot brings Linux back after a hang.
- **Build** from a west workspace with `-DYAKOGL_WERROR=ON`:

  ```sh
  west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/<probe> yakogl/tools/probes/rpi_5/<probe>
  ```

- **Run** through `tools/board.sh pi5` (tryboot: `kernel=` names the staged
  `zephyr.bin`, config.txt is restored and the staged files removed after the
  run). `BOARD_HOST` and `BOARD_UART` come from the environment (the leases),
  never from a file; `BOARD_UART` is a character device or `host:port`, and
  is only read. Name no address, host or UART path in a file or a commit.
- **get_throttled**: the supply is weak, so `vcgencmd get_throttled` is
  recorded in `meta.txt` with every run. A GPU result that looks like a
  hardware fault is first checked against it.
- **If the board does not come back** within the runner's limit: stop. Do not
  retry and do not take the board for another run; report that the board needs
  a person to power-cycle it.
