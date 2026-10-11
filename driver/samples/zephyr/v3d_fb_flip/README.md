<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# v3d_fb_flip

The display half of direct rendering on the Raspberry Pi 4, with no GPU.

When this sample was written, the cluster demo copied each rendered window
into the firmware framebuffer (113-155 ms of a 167-235 ms frame). The
alternative is to render into one half of a 1920x1440 virtual framebuffer
while the other half is shown, and to switch halves with the firmware's
`SET_VIRTUAL_OFFSET`. This sample answers two questions before any renderer
depends on it:

1. Does a pan tear (switch halves in the middle of a scanout)?
2. What does one pan cost?

**The answers** (run FBFLIP5, `tools/cts/RESULTS.md`): no tear was seen in 34
captured frames over arms A to C, and a pan costs one `SET_VIRTUAL_OFFSET`,
about 0.1 ms. The firmware's `SET_VSYNC` wait averages 31 ms, about two 60 Hz
periods, so a 60 fps present pans and does not wait on it. Arm D (the whole
property list) does not pan. The cluster sample now pans between the two
halves on the Pi 4 (`driver/docs/one-method.md`, Step 5).

## What it does

- The display timing of every display the firmware drives
  (`GET_DISPLAY_TIMING` 0x00040017, `struct set_timings` in
  `vc4_firmware_kms.c:126-160`): size, pixel clock, totals, and the refresh
  computed from them. This is what tells a 30 Hz mode from a 60 Hz one.
- `GET_VIRTUAL_WIDTH_HEIGHT` and `TEST_VIRTUAL_WIDTH_HEIGHT` 1920x1440, before
  and after the allocation. The first run's test answered "2x2" and the
  allocation was still granted, so these lines show whether the test tag
  reports the current framebuffer rather than what it could offer.
- One tag list: physical 1920x720, virtual 1920x1440, depth 32, pixel order
  RGB, virtual offset 0,0, allocate (4096-byte alignment), pitch. The firmware
  may answer a set tag with other values (firmware wiki, "Mailbox property
  interface"), so every answer is checked. A mismatch prints
  `REFUSED: <what>` and the fallbacks below, and the image stops.
- The CPU fills rows 0-719 black and rows 720-1439 white. Each half has:
  - a 64-pixel checker border of 16-pixel squares, so the capture's
    rectifier finds the quad (the first run's flat halves read as "no
    signal");
  - a 16-pixel grey bar just inside the border (0x40 top, 0xc0 bottom) that
    names the half;
  - a diagonal from the inner top-left corner to the inner bottom-right one,
    in the other colour.
- Four arms. Before each, the top half and then the bottom half are held for
  1 s each (`ARM X HOLD ...` lines), so a capture proves the card sees the
  signal. Then 10 s of pans between offset 0 and offset 720.

| Arm | Per pan | What it proves |
|-----|---------|----------------|
| A | `SET_VIRTUAL_OFFSET` alone | Whether a bare pan tears, and its cost |
| B | `SET_VSYNC`, then `SET_VIRTUAL_OFFSET` | The control: a pan right after the vsync. Linux's `FBIO_WAITFORVSYNC` sends `SET_VSYNC` (`bcm2708_fb.c:719-724`). If B tears, the capture or the scaler is at fault, not the firmware |
| C | `SET_VIRTUAL_OFFSET`, then `SET_VSYNC` | Whether the new offset is latched at the next vsync, and what the wait costs |
| D | Linux's pan list without `ALLOCATE` (physical, virtual, depth, offset, pitch; `bcm2708_fb_set_par`, `bcm2708_fb.c:348-370`) | Whether the firmware treats the offset differently inside the full list, and what that costs |

A, C and D are paced at 23 ms plus 0-16 ms of jitter. That period is not a
multiple of the refresh period, so the pans land all over the scan. B paces
itself on the vsync.

## What it prints (UART only; no fbcon)

- The build, the watchdog, the cycle counter's frequency.
- `test virtual 1920x1440: ...` and `fb: ...` with every answer.
- `ARM X START t=<ms>` and `ARM X END t=<ms>` on the uptime clock. The HDMI
  capture's bursts are mapped to the arms with these.
- A line every 50 pans. It has the transaction time in microseconds
  (min / mean / max) split into three parts:
  - `write`: the buffer flush and the mailbox write;
  - `poll`: the firmware's own time until it answers;
  - `inval`: the buffer invalidate.

  For B and C it adds the vsync wait. It also counts the answers that did
  not echo the requested offset ("not echoed"), the pans after which
  `GET_VIRTUAL_OFFSET` (0x00040009, read untimed right after the pan) holds
  another offset ("read back elsewhere": the pan did not move it), and the
  errors. The first run's arm D was answered in 11 us with every pan to row
  720 not echoed: the size tags seem to reset the offset, and the read-back
  proves or refutes that.

`rpi_mbox_request_timed()` in `driver/zephyr/lib/board/rpi4/rpi_mbox.c` does the timing,
on `k_cycle_get_64`.

## Reading a capture

The HDMI output goes to the capture card, so each captured image is one whole
scanout. A capture that shows one half whole (its bar and an unbroken
diagonal) is clean. A capture whose diagonal breaks at a row, with black above
and white below or the reverse, is a tear; record the row.

## Fallbacks, if the firmware refuses 1920x1440

1. `TEST_VIRTUAL_WIDTH_HEIGHT`'s answer shows what it offers instead.
2. `max_framebuffer_height=1440` and `max_framebuffer_width=1920` in
   `tryboot.txt`, then this image again. The config.txt documentation calls
   these "the maximum dimensions of the internal frame buffer" and gives no
   default.
3. `ALLOCATE` with our own base address, and a flip that resends the list
   with the other base, as Linux passes its own buffer
   (`bcm2708_fb.c:390-420`).
4. `SET_PLANE` (0x00048015), firmware KMS's per-plane buffer address
   (`vc4_firmware_kms.c:64-110`), with the vblank as an SMI interrupt.
5. Direct rendering single-buffered into the visible framebuffer, with the
   tearing measured and accepted.

Sources: Raspberry Pi linux `rpi-6.12.y` at `a553c4648f68`
(`include/soc/bcm2835/raspberrypi-firmware.h`,
`drivers/video/fbdev/bcm2708_fb.c`, `drivers/gpu/drm/vc4/vc4_firmware_kms.c`);
the firmware wiki's "Mailbox property interface"; the Raspberry Pi
documentation's legacy config.txt options.

## Build

```sh
west build -b rpi_4b -d build/v3d_fb_flip driver/samples/zephyr/v3d_fb_flip
```

The watchdog holds 70 s (setup, then about 50 s of holds and arms).
