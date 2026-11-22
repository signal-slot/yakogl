<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5 probes

Zephyr images for the Pi 5 (`rpi_5`, BCM2712, V3D 7.1), one directory each,
run in the order of the bring-up (`docs/notes/pre-study-rpi5.md`, section 6).
Each ends with a line a run can grep for.

| Probe | Shows | End line | In the tree |
|---|---|---|---|
| `rpi5_wdog` | the BCM2712 PM watchdog resets the board (the first board fact) | `rpi5_wdog: feeding stopped, reset expected within 10 s`, then the console goes quiet and the boot loader follows; `rpi5_wdog: FAIL no reset` if it did not reset | yes (ran on the board and passed, build `8eb2b3cf`) |
| `rpi5_hello` | the smallest image boots from `kernel=` and prints on uart10 | planned | planned |
| `rpi5_dtb` (D1) | the DTB pointer the firmware passes and the V3D, SMS, HVS, clock and `pm` nodes in it; answers U6 | `rpi5_dtb: done` or `rpi5_dtb: FAIL: ...` | yes (ran on the board and passed, build `8eb2b3cf`) |
| `rpi5_ident` (D2) | the V3D hub and core identity and `MMU_DEBUG_INFO` read with no setup, equal to Linux's | `rpi5_ident: done`; no `FAIL` line: a faulted read is ended by the watchdog's reset | yes (ran on the board and passed, build `8eb2b3cf`) |
| `rpi5_clear` (M1 to M4) | power-on, identity (raw, with `MMU_DEBUG_INFO`), MMU, the 7.1 clear-only frame binned and rendered, a TLB clear to a 128x64 raster target read back | `v3d_clear: PASS (...)` or `v3d_clear: FAIL (reason)` | yes (ran on the board and passed, build `8eb2b3cf`) |
| `rpi5_triangle` (M5) | M1 and M2 as `rpi5_clear`, then the driver's 7.1 clear-then-triangle job (`v3d_frame_build_triangle`) with Mesa's shaders out of the board's `triangle.clif`, binned (256 KiB overflow block) and rendered; the 64x64 target read back, inside / outside / edge to the tolerance of 3 | `v3d_triangle: PASS (64x64 read back, 0 of 4096 px wrong: ...; shaders mesa)` or `v3d_triangle: FAIL (reason; shaders mesa)` | yes (ran on the board and passed, build `0c58c457`) |
| `rpi5_triangle_ours` (M5) | `rpi5_triangle` built with our 7.1 compiler's pass-through shaders (no sources of its own) | as `rpi5_triangle`, with `shaders ours` | yes (ran on the board and passed, build `0c58c457`) |
| `rpi5_tfu` (P10) | M1 and M2 as `rpi5_clear` (raw `HUB_IDENT1`, bit 17, and `HUB_IDENT3`), then one 7.1 TFU job (`v3d_tfu71_pack`, IOC `0x00086061` as Mesa's `gl_mipgen`): a 64x64 RGBA8 raster source to Mesa's tiled 7-level layout, levels 1 to 6 generated; level 0 read back exact, the mips to a 2x2 box filter within 2 | `v3d_tfu: PASS (levels 0-6 checked, 0 px wrong: ...)` or `v3d_tfu: FAIL (reason)` | yes (ran on the board and passed, build `5857cc88`) |
| `rpi5_tex` (P10, U8) | M1 and M2 as `rpi5_triangle`, then two 7.1 clear-then-draw frames (`v3d_frame_build_triangle`) with programs of our 7.1 online compiler (`src/shaders_ours.h`, `gen/gen_shaders.sh`) and `v3d_tex71`'s records: T1 a 64x64 RGBA8 UIF texture drawn by a full-screen quad with NEAREST, all 4096 px compared with their texels; C1 a 6-face 4x4 RGBA8 cube map (`gl_cube`'s records) sampled LINEAR at 54 directions (face centres, edges, corners), compared with the seamless and the non-seamless prediction | `v3d_tex: PASS\|FAIL <n> of <m> px wrong`, `v3d_cube: PASS seamless=yes <n> of <m> samples wrong` or `v3d_cube: FAIL seamless=<yes\|no\|neither> <n> of <m> samples wrong`, then `rpi5_tex: done (both parts ran)` or `rpi5_tex: FAIL (<step>)` | yes (not run on the board yet) |
| `rpi5_mbox` (Q1 of `docs/notes/rpi5-display.md`) | why the property mailbox answered -EIO: the raw status and response code of `GET_THROTTLED`, `GET_FIRMWARE_REVISION` and the five clock queries, three ways (display buffer without cache calls, clear buffer with clean and invalidate, the board layer's `rpi_mbox_request_timed`) | `rpi5_mbox: done` or `rpi5_mbox: FAIL ...` | yes (ran on the board, image from `0e7742f0`: `PASS (GET_THROTTLED answered by clear board)`, `done (7 of 7 answered)`, `q1: cache maintenance`; Q1 answered: the missing cache maintenance of rpi5_display's image) |
| `rpi5_scanout16` (frame 1, Q2 of `docs/notes/rpi5-display.md`) | adopts the firmware's 1920x1080 RGB565 plane on HDMI0, draws 16 bpp fbcon text and a pattern, reads it back, holds 240 s, reads back again; FRCNT, PV_CONTROL.EN and PV_V_CONTROL.VIDEN; writes no display register or list word | `rpi5_scanout16: PASS (...)` or `rpi5_scanout16: FAIL (reason)`, then `rpi5_scanout16: feeding stopped, reset expected` | yes (ran on the board, image from `2b88a9f6`: `verdicts: PASS (plane adopted, both read-backs match, frcnt 48 -> 51 advanced, EN and VIDEN set)`, `q2: yes, list changed (1 list words changed in 240012 ms)`; the plane was not redrawn in 240 s, which list word changed is open) |
| `rpi5_scanout32` (stage 2, Q4 of `docs/notes/rpi5-display.md`) | our own XRGB8888 plane in list words at or after word 32 that read zero, clear of the firmware's list walked to its END; channel 0's list pointer swapped at vblank between two buffers (8 swaps), FRCNT through each; then the firmware's list restored byte-equal. The restore compares every word except the hardware-written plane-entry words declared by the CMake knob `RPI5_SCANOUT32_HW_WORDS` (`<off>:skip\|expect`, default empty), each differing word printed with its entry and offset; no pixel valve, HDMI or PHY register is written | `rpi5_scanout32: PASS (...)` or `rpi5_scanout32: FAIL (reason)`, with `u3 restore: PASS byte-equal [except hw words off <offs> (..)]`, then `rpi5_scanout32: feeding stopped, reset expected ...` | yes (host selftests only; its README quotes no board run yet) |
| `rpi5_dispregs` (Q3, Q5 and Q6 of `docs/notes/rpi5-display.md`) | read-only: the HVS version and channels 0 and 1 (CTRL0, STATUS), both pixel valves' registers decoded, the firmware mailbox's clock state, rate, measured and max rate for clocks 4, 13, 14 and 16, GET_NUM_DISPLAYS and GET_DISPLAY_SETTINGS, and HDMI hotplug and HD_VID_CTL when the HSM clock is on | `rpi5_dispregs: done (...)` or `rpi5_dispregs: FAIL (reason)`, then `feeding stopped, reset expected`; `verdicts.sh` prints the `q3:`, `q5:` and `q6:` lines | yes (ran on the board, 2026-11-22, image from `a9ca11e1`: `end: done (pv 2 of 2, clock tags 16 of 16, hdmi read)`; `q3: pv0 runs (both valves run) ...; two pixels per clock: unknown (valve active width 960, HVS FWIDTH 1919)`; `q5: hsm on measured 648006592, bvb off measured 324003296, disp off measured 648009856`; `q6: displays: 1, settings: answered, hdmi: hdmi0 hotplug 1 hd_vid_ctl enable 1, hdmi1 hotplug 0 hd_vid_ctl enable 1; the firmware drives HDMI0`) |

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
