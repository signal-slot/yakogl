<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5: the display design for the GL image (P11, U3)

Status: design, 2026-11-20. Nothing here was run; the board evidence it uses
is the 2026-11-19 `rpi5_display` session already recorded in
`driver/docs/rpi5-display.md`. This note is item P11 of
`docs/notes/pre-study-rpi5.md` (section 8, table row at `:614`; U3 at `:572`).
It does not repeat the register-level reading of `driver/docs/rpi5-display.md`
(three routes, the Z0-Z4 checks, 574 lines) or the facts file
`driver/docs/rpi5-v3d71-facts.md` section 10 (`:354-416`). It turns them into
one implementation order for a Zephyr image that draws GL on the Pi 5, and
says what only the board can still answer.

Citation prefixes: `facts:` = `driver/docs/rpi5-v3d71-facts.md`; `dd:` =
`driver/docs/rpi5-display.md`; `pre:` = `docs/notes/pre-study-rpi5.md`; `rpi:`
= Linux `rpi-6.12.y` at the commit the facts file names (facts and line
numbers only, GPL: no code, no constant tables are copied); other paths are
this repository's. No board address, host name or UART path is in this file.

## 1. Which Pi 4 pieces carry over, and which differ

The Pi 4 has no HVS, pixel-valve or HDMI code in this repository at all: the
firmware owns the whole chain and the image only asks it for a framebuffer
through the property mailbox (`driver/zephyr/lib/board/rpi4/rpi_mbox.c:165-225,251-294`,
`dd:32-47`). Everything below the buffer is new on the Pi 5, unless the
firmware keeps doing it.

| Piece | Pi 4 (BCM2711) in the repo | BCM2712 | Verdict |
|---|---|---|---|
| Mailbox client | base `0xfe00b880`, bus alias `0xc0000000` | base `0x10_7c013880`, no alias, buffer must be below 4 GiB (`rpi_mbox.c:22-52`; node `rpi:bcm2712.dtsi` `mailbox`/`soc`, see `dd:32-47`) | Code carries over, **untried**: every request, even `GET_THROTTLED`, returned `-EIO` on the 2026-11-19 run (`dd:531-538`) |
| Framebuffer tags (`rpi_fb_alloc_virtual`, pan, vsync) | the Pi 4's one-request allocation (`rpi_mbox.c:251-294`, pan `:304`) | the same tags exist for Linux's firmware-fb driver, but whether a `kernel=` image is answered is open (`dd:181-258`, `dd:531-538`) | Carries over *if* the transport is fixed; not the first-frame path |
| 32 bpp buffer, window format by pixel order | `repo:driver/rust/src/board/rpi4.rs:28-91`, `board_rpi4.c:212-322` | The firmware's own plane is **RGB565**, 1920x1080, pitch 3840 (`dd:517-530`); `adopt.h` has no 32 bpp window for RGB565 (`driver/zephyr/lib/board/rpi5_scanout/adopt.h:21,34,94`) | **Differs.** The GL window needs a 32 bpp format, so either a 16 bpp conversion at present time or our own display list (section 2) |
| Console (`fbcon`, 8x16 font) | 32 bpp only (`driver/zephyr/lib/board/rpi4/fbcon.h:15`, `fbcon.c:67-80`) | needs a 16 bpp path while the firmware plane is RGB565 | Differs (small, host-testable) |
| Cache handling of the buffer | Normal non-cacheable mapping (`rpi_mbox.c:288`) | same mapping; the adopted plane was written and read back byte-equal through it (`dd:524-530`) | Carries over; `fb_flush.h` has the range maths if a cacheable mapping is ever chosen (`rpi5_scanout/fb_flush.h:41-63`) |
| HVS | none (firmware) | HVS 6.x, `brcm,bcm2712-hvs` at `0x10_7c580000` size `0x1a000`, display list in its own memory, interrupts 2/9/16 on `disp_intr` (`facts:354-383`, `rpi:bcm2712.dtsi:612-623`) | **New.** Two register layouts: C0 (version 0x53) and D0 (0x54); the board is D0, `VERSION` 0x2454 (`dd:94-99`, `dd:517-530`; decode `hvs6_dlist.h:243`, `hvs6_chan.h:42`) |
| Display-list format | none | 6.x word format; one unscaled plane is written by `hvs6_dlist_write.h:83`, decoded by `hvs6_dlist.h:332` | **Differs** from the Pi 4's list (not used by us there) |
| List memory | firmware's | Linux starts its allocator after `HVS_BOOTLOADER_DLIST_END` (32 words) so as never to overwrite the firmware's list (`facts:354-383`, `rpi:vc4_hvs.c:1570-1583`, `rpi:vc4_regs.h:1056`); first-fit placement is in `dlist_alloc.h:61-133` | New; needed only on the (ii)-style routes |
| Output-to-FIFO map | none | output 0 to FIFO 0, output 2 to FIFO 2, outputs 1, 3, 4 to FIFO 1 (`facts:354-383`, `rpi:vc4_hvs.c:916-932`; `hvs6_chan.h:277`) | **New.** On the board FIFO 0 runs 1920x1080 and FIFO 1 runs 640x480 with no list (`dd:517-530`) |
| Pixel valve | firmware's | `pixelvalve0` `0x10_7c410000`, `pixelvalve1` `0x10_7c411000`, SPI 101/110 (`facts:354-383`, `rpi:bcm2712.dtsi:292-302`); timing words from a CEA mode in `pv_timing.h:43-101` | **New**, and not yet understood: Linux's `PV_HORZB` says 1920, the firmware's says 960 (`dd:559-562`) |
| HDMI controller and PHY | firmware's | nine register regions per port, `vc6_hdmi_phy_init` (`facts:384-416`, `rpi:vc4_hdmi_phy.c:954-1198`); capture-and-replay plan in `hdmi_phy.h:3-30`, plan steps `:220-330` | **New.** The PHY constants are GPL tables and may not be copied; `route3.h` leaves `phy-init` as a named placeholder (`route3.h:78-83`) |
| Clocks | firmware | firmware clocks 4 (core), 16 (disp), 13 (hdmi/HSM), 14 (bvb); DVP gate and resets at `0x10_7c700000` (`dd:49-93`, `facts:354-383`); builders in `fw_clk.h` (no `SET_CLOCK_STATE` yet, `route3.h:78-97`) | New |
| IOMMU4 in front of the HVS | none | disabled (`MMMU_CTRL` 0) on the board, so a list address is an ARM physical address (`dd:517-530`); its registers are at `rpi:bcm2712-iommu.c:24-81` | New; recheck on every session |
| Hand-over from the firmware | n/a | the firmware leaves the scanout running and hands over no `simple-framebuffer` node (`dd:517-530,539-540`); Linux's own handling is `vc4_hvs.c:1570-1583`, `vc4_crtc.c:553-610`, `vc4_drv.c:306-312,397-400` (`facts:354-383`) | New: this is what route (i) uses |

## 2. The first frame: take over the firmware's mode, or do a full mode set

### Evidence

- The firmware leaves HDMI0 scanning out for a `kernel=` image: channel 0 mode
  2, 1920x1080, frame counter advancing 27 to 33 in 100 ms, one unity RGB565
  plane at the top of the first GiB, both pixel valves enabled, IOMMU4 off
  (`dd:517-530`).
- The CPU can write that plane: a pattern over 4147200 bytes read back
  byte-equal twice, the list unchanged afterwards (`dd:524-530`); verdicts
  `u3 scanout: running`, `u3 adopt-readback: PASS` (`dd:495-512`).
- The mailbox transport got no answer at all, so route B is unproven, not
  closed (`dd:531-538`).
- HDMI registers were never read, because the HSM clock query was
  unanswered (`dd:541-543`).
- The route the results pick is A (`dd:566-574`).

### Option (i): take over the firmware's mode

The image never programs the pixel valve, HDMI or PHY. In two stages:

1. **Frame 1, no list change.** Read the channel-0 list (`hvs6_dlist_first_plane`,
   `hvs6_dlist.h:332`), run `rpi5_adopt_check` (`adopt.h:115`), draw RGB565
   into the found plane through the non-cacheable mapping. A 16 bpp console
   and a pattern are enough to call it a frame. This is exactly what
   `rpi5_display` already does for its pattern (`tools/probes/rpi5_display/README.md:5-40`);
   what is missing is the text and a 16 bpp `fbcon`.
2. **A 32 bpp plane for GL, our own list after the bootloader's.** Build
   one unscaled XRGB8888 plane with `hvs6w_write_plane` (`hvs6_dlist_write.h:83`),
   place it at or above word 32 with `dlist_place`/`dlist_swap`
   (`dlist_alloc.h:88-133`; floor per `rpi:vc4_hvs.c:1570-1583`) and only
   where it overlaps neither the firmware's list span nor the words below it
   that the probe has not shown unused. The firmware's list does not start at
   word 0: it starts at channel 0's `DISPX_LPTRS.HEADE`, read at run time
   (HEADE = 672 (s32dumprun, read-only skeleton run); field mask
   `hvs6_chan.h:94`). The span is `[HEADE, END word)`; the byte-equal snapshot
   and restore cover exactly that span. The rule is enforced by
   `swap_placement_ok` (`tools/probes/rpi_5/rpi5_scanout32/src/swap_check.h:89-99`:
   no overlap, `word+len <= 4096`, unknown lengths refused) and judged again by
   `verdicts.sh` (`tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:257-258`, the
   snapshot's mandatory `heade=` field at `:137`). Then point
   channel 0's `DISP_DL`/list pointer at it. This edits only the HVS, not the
   wire; the pixel valve and HDMI stay as the firmware left them. Double
   buffering is then a list swap at vertical blank (`dlist_swap`), not a
   mailbox pan.

Cost: S for stage 1 (code exists), M for stage 2 (the list write and swap
exist as pure headers; the MMIO side and a stop-the-firmware question do not).
Risk: the firmware's HVS state, the plane order and whether the firmware
redraws into its buffer afterwards; all three are answered by reading
(`dd:517-530`) and only the third is not yet seen beyond one 100 ms window.

### Option (ii): full mode set

The image resets and programs HVS channel, pixel valve and HDMI0 itself, as
`route3.h` plans (`route3.h:3-100`). It needs, in this order: firmware clock
state (`fw_clk.h` cannot yet switch clocks on), the HVS global init, the DVP
reset pulse, the PHY init, pixel valve timing, HDMI controller phases
(`route3.h:78-97`). The PHY init has no licensable source here: the constants
are the GPL tables of `rpi:vc4_hdmi_phy.c:954-1198` and the project's only
lawful route is to capture the board's own working values (`hdmi_phy.h:3-30`,
`dd:466-468`). Whether the facts file's sequence is *sufficient* on a board the
firmware has not set up is open (`facts:384-416`).

Cost: L. Needs a Linux register dump per mode, a mode table, and a place to
put a stable result (U3's "route C is its own plan", `dd:387-411`).

### Decision

**Option (i), in two stages, first.** Arguments:

1. The 2026-11-19 session proved its premises: a running scanout, a writable
   plane, no IOMMU translation, a D0 HVS (`dd:517-530`). Option (ii) rests on
   the opposite premise, that nothing is running, which the board refuted.
2. Option (ii)'s hard parts are exactly the ones the repo cannot yet do
   lawfully: PHY constants (`route3.h:78-83`) and clock enable
   (`route3.h:88-91`). Option (i) needs neither.
3. The sink mode comes from the firmware (the board's sink is pinned by
   `cmdline.txt`, which Zephyr does not read, `dd:442-451`), so taking over
   also gets a mode the display is known to accept.
4. GL needs 32 bpp, which stage 2 provides without a mode change.
5. Option (ii) stays the fallback for a boot where the firmware leaves
   nothing running (e.g. no sink attached, HDMI1 only); it is scheduled only
   if a session shows that case matters.

Not decided here: whether to keep `fbcon` at 16 bpp forever or only until
stage 2; it is a board-layer choice.

## 3. The probe plan for a later board session

Rules of every Pi 5 session apply (`tools/probes/rpi_5/README.md:24-34`): the
PM watchdog is armed in the first init hook, one run goes through
`tools/board.sh pi5 run` and `tryboot`, the PM watchdog brings the board back
to Linux, a run that does not return is reported as needing a person.

`rpi5_display` (U3 probe; lives at `tools/probes/rpi5_display` until #321 releases
`tools/probes/rpi_5/`, `tools/probes/rpi5_display/README.md:5-6`) already
ran. The three probes below are proposals, named as they would sit under
`tools/probes/rpi_5/`; each is a small extension of it, reusing its
`verdicts.sh` pattern (every step prints a line even on the failing path,
`tools/probes/rpi5_display/README.md:36-43`).

| Probe | What it does | What it prints | PASS |
|---|---|---|---|
| `rpi5_mbox` (first, cheapest) | Only the mailbox: `GET_THROTTLED`, `GET_FIRMWARE_REVISION`, then the five clock-rate queries, with raw status and response codes | `rpi5_mbox: tag <id> status <hex> answered <hex> value <hex>` per request | `GET_THROTTLED` is answered. Explains the `-EIO` of `dd:531-538` before anyone judges route B |
| `rpi5_scanout16` (frame 1) | Z0/Z1 snapshot as in `rpi5_display`, adopt the plane, 16 bpp `fbcon` text on the top rows and a pattern below, hold, read back twice, stop feeding | `u3 plane: …`, `u3 adopt-readback: PASS`, `rpi5_scanout16: text rows <n> changed`, frame counter before and after | the plane is adopted, the pattern and text read back byte-equal twice, `STATUS.FRCNT` advances, `PV_CONTROL.EN` and `VIDEN` set (`dd:412-434`) |
| `rpi5_scanout32` (stage 2) | builds the XRGB8888 plane, takes the firmware's list start from `LPTRS.HEADE` and places it at or above word 32 clear of the firmware span (`swap_check.h:89`, `verdicts.sh:257`), swaps channel 0's pointer at vblank, alternates two buffers, then restores the firmware's list | `u3 snapshot: … heade=<n> words=<n> …`, `u3 dlist: placed word <n> len <n>`, `u3 plane: addr=… fmt=XRGB8888`, `FRCNT` per swap, `u3 restore: PASS` | the list names our buffer, `FRCNT` advances through each swap, the firmware's list is restored byte-equal at the end, the watchdog returns the board |
| `rpi5_gl_scanout` (after the GL probes) | renders with the 7.1 job (the `rpi5_triangle` path) into a render target, converts or copies it into the plane, reads it back | `rpi5_gl_scanout: 4096 px, 0 wrong` and the plane readback line | the plane holds the rendered image byte-equal |

Observation. The Pi 5 has **no display-observation device**: the farm lists no
HDMI capture target for it as a resource a session can lease, and the board's
HDMI0 capture device cannot be read by a probe (`dd:412-434`). So no check
needs a camera, and none is claimed. Verification is by registers and bytes:
the list names the buffer, `FRCNT` advances, the pixel valve and HDMI enable
bits are set, and every byte reads back as written through the CPU mapping
(`dd:412-434`). A frame from a capture device or a monitor, if someone with
access takes one, is a confirmation, never the proof (`dd:430-434`). The Linux
side, under ssh before the Zephyr run, supplies the references
(`dd:363-376`).

## 4. Open questions only the board can answer

1. **Why does the mailbox not answer?** Answered 2026-11-20 by `rpi5_mbox`
   (one tryboot run, image from `0e7742f0`): the missing cache maintenance of
   `rpi5_display`'s image. Every request, including `GET_THROTTLED`, returned
   `-EIO` with response code 0 (`dd:531-538`) because that image had no
   `CONFIG_CACHE_MANAGEMENT`, so `rpi_mbox.c`'s clean before the doorbell and
   invalidate after the reply did nothing with the data cache on. In the probe
   the same registers and buffer without the cache calls again got 0 of 7
   answered (status 0, and the buffer read after an invalidate was still
   zeros: the firmware never saw the request), while the clean-and-invalidate
   way and the board layer's `rpi_mbox_request_timed()` answered 7 of 7 with
   status 0x80000000 (firmware revision 0x684bf1ce, core and V3D clocks
   500000000 Hz, disp, HDMI HSM and bvb clocks 0 Hz). The base, channel and a
   `kernel=` image are not the cause. Any image that sends a mailbox request
   builds with `CONFIG_CACHE_MANAGEMENT=y`; record in
   `tools/probes/rpi_5/rpi5_mbox/README.md`.
2. **Does the firmware redraw into its plane after hand-over, and does it
   keep the list?** The plane half and the list half are answered.
   `rpi5_scanout16` ran once on 2026-11-21 (tryboot, image from `2b88a9f6`,
   record in `tools/probes/rpi_5/rpi5_scanout16/README.md`). The verdict was
   `verdicts: PASS (plane adopted, both read-backs match, frcnt 48 -> 51
   advanced, EN and VIDEN set)` and the Q2 line was `q2: yes, list changed (1
   list words changed in 240012 ms)`.
   *Plane: no redraw.* The probe wrote the 1920x1080 RGB565 plane at
   0x3f800000. It hashed the same in all 24 samples of a 240 s hold, and
   readback 2 was byte-equal over all 4147200 bytes. LPTRS (0x2a0) and both
   valves' words did not change.
   *List: answered (2026-11-22).* `rpi5_scanout16` ran again (tryboot, image
   built from `ed71b85b`, 238370 ms hold; README section "Q2's list word
   (2026-11-22)"). Exactly one list word changed: **word 676**, which is
   entry offset 4 of the plane entry at word 672 (`HEADE`), the entry's context
   word (`ctx-off 4`, 32 words in the entry). It was `0x8000005d` in the
   adopted snapshot; the samples during the hold read other values (for
   example `0x80000103`, `0x8000002a`, `0x800003b2`) and the last one `0x800000d2`
   (`u3 q2: word 676 entry-off 4 was 0x8000005d now 0x800000d2 per-frame no`;
   `q2: ... hvs-writeback`). The other 63 words, including the pointer word
   (`0x3f800000`, entry word 6), did not change. Two bursts of 16
   consecutive frames (FRCNT 44-59 at the start, 10-25 at the end) read
   `w676=0x80008000` in every frame, so it is not rewritten each frame:
   the value differs between the hold's samples, not within a burst. It is
   a hardware-written status word, not the firmware editing its list.
   *For U3's restore:* the byte-equal restore of the firmware's list
   (`rpi5_scanout32`) must skip, or expect to differ at, entry offset 4
   (absolute word 676 in the firmware's list); every other word must be equal.
3. **Which pixel valve feeds HDMI0, and does the firmware run it at two
   pixels per clock?** Linux reads 1920 active pixels, the firmware's valve
   960 (`dd:559-562`). Matters only for option (ii) and for any plane resize.
4. **Can channel 0's list pointer be changed under a running scanout without
   a visible glitch, and is the vblank the safe point?** Not read anywhere
   (`facts:354-383` shows Linux's enable path, not a hot swap). Stage 2's gate. The probe now prints `heade=` in its snapshot line.
5. **Do the disp (16) and HSM (13) clocks matter to HVS register reads?** The
   HSM rate was never answered, so HDMI registers were skipped
   (`dd:541-543`); the HVS ones were read with the disp clock unknown
   (`dd:456-458`).
6. **HDMI0 or HDMI1, and a board without a sink.** The firmware drove HDMI0
   with hotplug seen on one port only (`dd:517-530`); with no sink or the other
   port the picture of the firmware may not exist, and option (ii) becomes
   necessary. On 2026-11-21 the boot loader again saw hotplug on HDMI0 and
   none on HDMI1, and logged `FB0 ... 1920x1080 stride 3840 base 0x3f800000`.
   That is the plane `rpi5_scanout16` adopted. The case with no sink, or a sink
   on the other port, is still open.
7. **Is a 32 MiB 3840x2160 buffer ever wanted?** The board layer must take the
   granted size from the answer and refuse by name a window that does not fit
   the image's `sram0` (`dd:442-451`).
8. **Pan and vsync cost on the Pi 5**, if the mailbox is ever fixed
   (`dd:462-465`).
9. **Is the sequence of `facts:384-416` enough for the PHY** on a board where
   the firmware did not set it up, and may a captured value be reused for a
   second mode (`dd:466-468`)?
10. **P12/P13 interaction.** Whether the CTS stages need the screen at all:
    a headless `gl_test` does not, so P11 is not on the critical path of
    `board71` (`pre:614`: "may be a separate item after the CTS").
