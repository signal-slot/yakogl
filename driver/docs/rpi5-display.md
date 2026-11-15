<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5 display (U3): three routes and the board checks that pick one

Status: design, host side only. Nothing here was measured on a board. The
facts come from `driver/docs/rpi5-v3d71-facts.md` section 10 (read there from
Linux rpi-6.12.y, file and line only) and `rpi_mbox.c`'s own tests; every
"open" below is a board check, listed in the last section.

The goal is the Pi 4's screen path on the Pi 5: a picture on HDMI from a
Zephyr image started with `kernel=`, text on the top half and a pattern below
(`docs/notes/pi4-v3d-bringup.md`, "Display: the firmware framebuffer through
the mailbox"), so that gl_test, dEQP and glmark2 runs have the console and the
window/scanout view the Pi 4 has.

Chain on the BCM2712: HVS6 (display list, "scaler6") to pixel valve 0 or 1 to
HDMI0 (controller, PHY). Addresses, interrupts and clocks are in the facts
file's section 10 table; none is repeated here except where a route needs it.
`driver/zephyr/lib/board/rpi5_scanout/pv_hdmi.h` decodes the pixel valve and
HDMI0 registers a firmware scanout leaves (pure functions over values the
caller read; host test `tools/v3d/check71.d/pv_hdmi.sh`).

## The routes, cheapest first

### A. Adopt the scanout the firmware left running

Needs: the firmware starts a scanout before it jumps to `kernel=`, and that
scanout is still running when Zephyr starts. Linux has code for exactly this
state: `vc4_hdmi_bind` enables the clocks when `HDMI_VID_CTL` already has bit
31 (facts 10, `vc4_hdmi.c:3391-3399`), and the display-list allocator never
overwrites the bootloader's list (`vc4_hvs.c:1570-1583`, first 32 words).

What the image does: read the pixel valve and HDMI state with `pv_hdmi.h`,
walk the HVS display list for the plane the firmware set up, take its
framebuffer address, pitch and pixel format, and draw into that memory. It
programs nothing, so there is no PHY, clock or timing work.

Cost: lowest (reads only). Risk: the firmware may not enable the pipe for a
`kernel=` boot (the display nodes are `disabled` in the base device tree, and
a framebuffer is only set up with `framebuffer_*` config or a display attached
at boot); the framebuffer may sit where Zephyr's memory map or MMU setup does
not map it; the format may not be what the code expects.

### B. A mailbox framebuffer (the Pi 4's route)

Needs: the BCM2712 property mailbox still answers `FRAMEBUFFER_ALLOCATE`
(0x40001) and its tags (physical and virtual size, depth, pixel order, pitch).
`driver/zephyr/lib/board/rpi4/rpi_mbox.c` already builds for the BCM2712
(`CONFIG_SOC_BCM2712`: its own mailbox base and the bus view without the
`0xc0000000` alias; `tools/v3d/check71.d/mbox_mock.sh` runs 146 checks over
both SoCs), and it returns `-ERANGE` above 4 GiB, which a Pi 5 address can be.
The Pi 4's decisions in `driver/rust/src/board/rpi4.rs` and `fbcon.c` (split
screen, cache attributes, byte order) apply as they are.

What the image does: the same as the Pi 4's. A Pi 5 board-layer screen would
take the board (generation) as a parameter; the Pi 4's output and behaviour
must not change.

Cost: low if the firmware answers; if it answers `0` or an error, the route is
closed and nothing is lost. Risk: on the Pi 5 the firmware's display driver may
be the KMS one (`firmware_kms()` in `vc4_drv.c:306-312`), and the legacy
framebuffer tags may be unimplemented.

### C. Program HVS6, the pixel valve and HDMI ourselves

Needs: everything Linux does, in order: firmware clocks (`core`, `disp`, `hdmi`
= 13, `bvb` = 14, facts 10), the HDMI PHY sequence (facts 10, eleven steps, its
per-rate lane tables are in `vc4_hdmi_phy.c` and are not copied), HVS6 output
to FIFO mapping (`vc4_hvs.c:916-932`), a display list, the pixel valve timing
registers and the HDMI controller (infoframes, scrambling above 340 MHz).
Whether the PHY steps are enough on a board the firmware has not set up is
open (facts 10, last paragraph).

Cost: highest (days; every register needs a board check and a wrong write can
hang the bus). It is the route for a Zephyr image that must not depend on the
firmware's scanout; it is not needed if A or B works.

## Order and what each board check decides

Every image arms the BCM2712 PM watchdog first (`rpi_wdog.c`) and returns to
Linux by itself. Board items take `target:rpi5` and `target:rpi5-console`
together, one client at a time, after #321's "Board session 2".

1. Under Linux, over ssh, read the firmware's state as the reference: the HVS
   display list and control registers, the pixel valve and HDMI0 registers
   (decode with `pv_hdmi.h`), the firmware's framebuffer, `get_throttled`.
2. From Zephyr via `kernel=` through tryboot, read the same registers: **if the
   pixel valve is enabled and `HD_VID_CTL` bit 31 is set, route A is open**;
   record the framebuffer address and format and check that the address maps.
3. Call `FRAMEBUFFER_ALLOCATE` (0x40001): **an address back is route B**; an
   error or zero is not an answer to "no scanout", it only closes B.
4. Write a known pattern to whichever buffer the check found and read it back
   through the scanout registers (the Pi 4's own check: the bytes read back
   and the registers, plus the capture device on the HDMI0 farm node when the
   farm lists it as a resource).
5. Only when neither A nor B gives a picture: route C, as separate items.

Decision table:

| 2: scanout running | 3: mailbox allocates | Route |
|---|---|---|
| yes | any | A (read-only; B is the fallback) |
| no | yes | B |
| no | no | C |

## Open items for the owner of the facts file and the pre-study

- Facts file section 13, U3 row: answered by the checks above once run; until
  then it stays "answered in part".
- Pre-study section 8, row P11: depends on the three checks; the order above
  is the proposal.
- Board items (steps 1 to 4) are not done; they are ordered after #321's board
  session 2 and need a new probe directory under `tools/probes/rpi_5/` that
  `tools/board.sh pi5` can boot (a change to `board.sh` is a separate small
  item after #362).
