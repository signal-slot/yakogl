<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5 display (U3): three routes and the board checks that choose

ROADMAP stage 5 (issue #7), item P11 of `docs/notes/pre-study-rpi5.md`
section 8. The goal is the Pi 4's screen path on the Pi 5: a picture on HDMI
from a Zephyr image started with `kernel=`, so that `gl_test`, dEQP and
glmark2 runs there have the console screen and the window/scanout view the
Pi 4 has (`docs/notes/pi4-v3d-bringup.md`, "Display: the firmware framebuffer
through the mailbox"; `driver/zephyr/lib/board/rpi4/board_rpi4.c`).

What is known is in `driver/docs/rpi5-v3d71-facts.md` section 10 (the path
HVS, pixel valve, HDMI, every address, interrupt and clock, the HDMI PHY
sequence, all from Linux `rpi-6.12.y`, facts and `file:line` only) and its
section 13 (U3: answered in part). This file does not repeat them; it says
what each way of getting a picture needs and what to measure first.
Everything below about the board's behaviour is **open** until the probe
`tools/probes/rpi5_display` has run: no display register or mailbox answer of
the Pi 5 has been read yet.

## The three routes, cheapest first

### Route 1: adopt the firmware's running scanout

Linux treats the firmware as the owner of a first display: the HVS display
list allocator starts after `HVS_BOOTLOADER_DLIST_END` (32) so that the
bootloader's setup is not overwritten (`vc4_hvs.c:1570-1583`), `vc4_hdmi_bind`
enables the HDMI clocks when `HD_VID_CTL` already has bit 31
(`vc4_hdmi.c:3391-3399`), and `vc4_drm_bind` removes conflicting framebuffers
and stops the firmware's display with `RPI_FIRMWARE_NOTIFY_DISPLAY_DONE`
(`vc4_drv.c:397-400`). So the firmware can leave HVS, pixel valve and HDMI
running with a framebuffer.

- **Needs:** nothing programmed. The image must find (a) that a scanout runs:
  the HVS and pixel valve words, `HD_VID_CTL` bit 31; (b) where the picture
  is: the address and pitch in the HVS display list, or the
  `simple-framebuffer` node the firmware puts in the device tree it passes
  (`reg`, `width`, `height`, `stride`, `format`: Linux's
  `/proc/device-tree/framebuffer@*` shows it when the firmware made one);
  (c) the pixel format (the node's `format` string, or the display list's
  pixel-format field). Then it maps that memory Normal non-cacheable, as
  `rpi_fb_alloc` does for the Pi 4, and draws.
- **Costs:** the least code, but it depends on the firmware: a `kernel=` image
  may find the scanout stopped, or the picture's memory reused. The address
  is whatever the firmware chose, so the image reads it at run time and never
  names it.
- **Risk:** reading a block whose clock is off may hang the bus; every read
  group is announced first, and the watchdog (armed before `main`) resets the
  board.

### Route 2: the mailbox framebuffer

The Pi 4 asks the firmware (property channel 8) for physical and virtual
size, depth, pixel order, `FRAMEBUFFER_ALLOCATE` (0x40001) and pitch in one
request (`rpi_mbox.c: rpi_fb_alloc`) and gets a bus address. The mailbox
itself is already used on the Pi 5 for the V3D clock (`rpi_mbox.c` with
`CONFIG_SOC_BCM2712`: base 0x10_7c013880, no bus alias because the BCM2712's
`dma-ranges` map memory one to one, buffer below 4 GiB).

- **Needs:** that the BCM2712 firmware still answers the framebuffer tags and
  that something scans the buffer out afterwards. Whether the firmware keeps
  its own display driver on the Pi 5 is the question; Linux's
  `firmware_kms()` check (`vc4_drv.c:306-312`) shows the firmware can be the
  display's owner, but which nodes mark that on the Pi 5 was not established
  by the reading, so reading does not decide it.
- **Costs:** small. `driver/zephyr/lib/board/rpi5/rpi5_screen.c` asks the six
  GET tags (0x40003 physical size, 0x40004 virtual size, 0x40005 depth, 0x40006
  pixel order, 0x40008 pitch, 0x40009 virtual offset) one request each, then
  `rpi_fb_alloc` (unchanged) allocates, and writes and reads back a pattern.
  The Pi 4's board layer (`board_rpi4.c`, `fbcon.c`) is reused as it is for
  the console; its decisions (`src/board/rpi4.rs`: pixel order to window
  format, view or copy, pan row) do not depend on the SoC and stay as they are,
  so no Pi 4 output changes. Once the route is chosen on the board, a Pi 5
  board layer (`YAKOGL_BOARD` for `SOC_BCM2712`) takes these decisions
  through the same functions; that is an item after the board answer (below).
- **Risk:** the allocation may be answered and nothing scanned out (a
  framebuffer with no display chain behind it): then route 3.

### Route 3: program HVS6, the pixel valve and HDMI ourselves

The whole chain is `disabled` in the base device tree and is brought up by
the `vc4-kms-v3d-pi5` overlay (facts section 10). Doing it from Zephyr needs:

1. clocks: firmware clocks 4 (`core`) and 16 (`disp`) for the HVS, 13 (`hdmi`)
   and 14 (`bvb`) for HDMI (the HSM minimum is the larger of 120 MHz and 101 %
   of the TMDS character rate; `bvb` 75/150/300 MHz by pixel clock,
   `vc4_hdmi.c:1523-1553`), through the mailbox's clock tags (`rpi_clock_*`
   already exist);
2. the HVS: a display list for one plane in FIFO 0 (output 0 to FIFO 0,
   `vc4_hvs.c:916-932`), starting after the bootloader's 32 words, in
   the BCM2712 ("scaler6") layout, which differs from the Pi 4's HVS5;
3. the pixel valve 0 (0x10_7c410000) timing registers for the mode;
4. the HDMI block (`hdmi0`, nine register regions) and its PHY, in the order
   of `vc6_hdmi_phy_init` (facts section 10: reset, PLL constants, reference
   54 MHz, rate-dependent VCO, lane tables, power-up, PLL reset), plus
   EDID/hot-plug if the mode is not fixed (hot-plug is on `aon_intr`).

- **Costs:** the largest: new HVS6 and PHY code, with the per-rate lane tables
  taken from Linux's source as facts (not copied as text), none of it testable
  on the host beyond the arithmetic. The facts file leaves open that the PHY
  sequence is all a board needs that the firmware has not already set up.
- **When:** only if routes 1 and 2 both fail, or if a later item needs a mode
  the firmware does not set.

## The board checks that decide

All under tryboot with the PM watchdog armed first, one run each, one client
at a time, `get_throttled` recorded with every record (the supply is weak).

| # | Check | Where | Decides |
|---|---|---|---|
| C0 | `tools/probes/rpi5_display/linux_ref.sh` (read-only): the device tree's display nodes and their `status`, a `simple-framebuffer` node (`reg`, size, stride, `format`), DRM connector state, `fb*`, debugfs HVS/display-list lines, `/proc/iomem`, raw HVS, pixel valve 0 and HDMI0 `hd` words from `/dev/mem` when debugfs has none | Linux, over ssh | the reference every later reading is compared with; whether the firmware itself made a framebuffer |
| C1 | `rpi5_display` D1: the six framebuffer GET tags | Zephyr via `kernel=` | does the mailbox still describe a framebuffer (route 2's first condition; a mode with pitch and order also hints at a live scanout) |
| C2 | D2/D4: HVS (32 words), pixel valve 0 and 1 (16 words each) before and after the allocation | same | route 1: is a scanout running (compare with C0); route 2: does the allocation move the display registers (the scanout picked it up) or nothing changes |
| C3 | D3: `rpi_fb_alloc` rc, address, pitch, size, pixel order | same | route 2: granted or refused; the address (below 4 GiB or not, its format) |
| C4 | D5: the pattern written and read back from the mapping | same | the bytes are there (memory, not yet the screen) |
| C5 | the picture: the HDMI capture device if the Pi 5's resource description on the farm names one, else the register readback of C2 and the bytes of C4 (what the Pi 4 did first) | farm | whether the panel shows it, and in which pixel order |

Reading the outcomes:

- C2 shows a running scanout equal to C0 **and** C3 is refused or unchanged:
  route 1, and the next item reads the picture address from the display list
  (and the format from the node or the list) instead of allocating.
- C1 and C3 succeed and C2 moves with the allocation (or C5 shows the pattern):
  route 2; the Pi 5 board layer follows (item below).
- C1 unanswered, C3 refused, C2 reads zeros or the board hangs at a block:
  route 3, with the first HVS read that hung named by the probe's last line.
- A hang or no return is not retried: stop, release the board, report that it
  needs a person.

## The probe

`tools/probes/rpi5_display` (its own directory, to move under
`tools/probes/rpi_5/` when #321 releases that tree) runs D1 to D5 in the
order above and ends with `rpi5_display: PASS (...)` or `rpi5_display: FAIL
(reason)`. D5 is the Pi 4's split screen: the console (`fbcon.c`, scale 2)
in the top 256 rows, the pattern (`rpi5_screen.c`: eight colour bars and a red
ramp, in the firmware's pixel order) below. PASS means the memory check held,
not that a panel shows it.

## Host tests

`driver/zephyr/lib/board/test/rpi5_screen_mock.c` runs `rpi5_screen.c` over
the unchanged `rpi_mbox.c` against a mocked mailbox and a firmware model
(`tools/probes/rpi5_display/screen_mock.sh`, built for the BCM2712 and as a
control the BCM2711, with ASan and UBSan): the six requests and their layout,
the four firmware behaviours (framebuffer, zeros, tags not answered, error),
the allocation's ARM address, the pattern in both pixel orders through a wide
pitch, and a corrupted pixel found by its index.
`tools/probes/rpi5_display/linux_ref_selftest.sh` runs `linux_ref.sh` against a
fake root. Neither is a gate row yet (a row is an item after `tools/check.sh`
is released by its other owners).

## Items after the board answer

- Run C0 (Linux) and C1 to C5 (the probe) in one session after #321's first
  clear has released the board.
- Route 2 chosen: a Pi 5 board layer (`YAKOGL_BOARD` on `SOC_BCM2712`) with
  the Pi 4 decisions through `src/board/rpi4.rs` unchanged, then `gl_test`,
  dEQP and glmark2 screens on the Pi 5.
- Route 1 chosen: read the address, pitch and format from the firmware's
  state, and map it.
- Route 3 chosen: one item for the HVS6 display list and pixel valve, one for
  the HDMI PHY, each with a Linux reference read first.
- `tools/board.sh pi5` may need the capture device or a longer hold; that is a
  small item after #362.
