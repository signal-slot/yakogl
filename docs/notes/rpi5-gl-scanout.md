<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5: rpi5_gl_scanout, a V3D 7.1 frame in our own HVS6 plane (P11 after P9)

Status: plan, 2026-11-25, current at the tip. Nothing of `rpi5_gl_scanout`
is built or run: it is the row `rpi5_gl_scanout (after the GL probes)` in the
probe table of `docs/notes/rpi5-display.md:153`, part of P11 Display (U3) of
`docs/notes/pre-study-rpi5.md:617` (depends only on P0), and it waits for P9
and P10 (`docs/notes/pre-study-rpi5.md:615-616`, not merged). The display
side is stage 2 of `docs/notes/rpi5-display.md:73-90` and its session rules
(`docs/notes/rpi5-display.md:136-139`); this note does not repeat them. The
first `rpi5_scanout32` board run of 2026-11-25 is in and is part of this
design: it refused its placement and left Q4 unanswered (section 2).

Citations are full repository paths; a bare `:N` names a line of the file
named just before it. Figures marked *(tile2)* or *(facts)* are the plan #390
reports glscan-tile2 and glscan-facts, hand-worked from
`driver/src/v3d/v3d_frame.c`, not host-run; **unverified** marks a point no
file at the tip proves. No board address, host name or UART path is in this
file.

## 1. Goal and non-goals

Goal: one Zephyr image for the Pi 5 that

- keeps the firmware's running scanout: it drives HDMI0 with a
  1920x1080 16 bpp picture and hotplug on that port
  (`docs/notes/rpi5-display.md:296-312`, Q6 answered);
- renders full-screen frames with the driver's 7.1 bin-and-render job — the
  `rpi5_triangle` path, board-proven at 64x64 with `FLDONE`/`FRDONE` and a
  read-back check at tolerance 3 (`driver/docs/rpi5-v3d71-facts.md:675-676`)
  — straight into one of two 32 bpp buffers;
- shows each buffer by swapping channel 0's list pointer `LPTRS` at vblank
  between two of our own plane slots
  (`tools/probes/rpi_5/rpi5_scanout32/README.md:31`);
- restores the firmware's list at the end and stops feeding the watchdog, so
  the board returns to Linux by tryboot
  (`docs/notes/rpi5-display.md:136-139`,
  `tools/probes/rpi_5/rpi5_scanout32/README.md:159-169`; the 2026-11-25
  session came back on its own,
  `tools/probes/rpi_5/rpi5_scanout32/README.md:229-243`).

Non-goals:

- No mode set, no pixel valve, HDMI, PHY or clock writes. The image writes
  only channel 0's `LPTRS` and list words its own placement check approved,
  and writes nothing at all when that check refuses
  (`tools/probes/rpi_5/rpi5_scanout32/README.md:13-16`,
  `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:176-179`). A full
  mode set stays the fallback, scheduled only if a boot leaves no picture
  (`docs/notes/rpi5-display.md:127-129`).
- No CPU copy or format conversion between the render target and the plane:
  the render target *is* the plane's buffer. `docs/notes/rpi5-display.md:153`
  still reads "converts or copies it into the plane"; the byte-order answer
  of section 3 makes the copy unnecessary, and this note plans without one.
- No textures until P10 merges (`docs/notes/pre-study-rpi5.md:616`): a
  textured frame is out of G2-G4.
- No camera: the farm lists none for the Pi 5, and verdicts come from
  registers and bytes (`docs/notes/rpi5-display.md:155-164`).

## 2. What the board already says (2026-11-25)

The stage-2 probe `rpi5_scanout32` ran once on the Pi 5 (tryboot, image from
`458834ea`; record `tools/probes/rpi_5/rpi5_scanout32/README.md:221-294`):

- It **refused its placement and wrote nothing**: the firmware's list at
  `HEADE` 672 reaches no END word within the walked 256-word window (8
  entries, stopped at word 896), and with no END the library puts our list
  only wholly before `HEADE`
  (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:18-22,41-51,389,417`),
  where words 32..671 hold 619 non-zero words, so no 20-word zero span was
  found (`tools/probes/rpi_5/rpi5_scanout32/README.md:245-256`).
- **Q4 is unanswered** (`q4: unanswered (placement refused)`,
  `tools/probes/rpi_5/rpi5_scanout32/README.md:275-276`, judged by
  `tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:121-122`).
  `docs/notes/rpi5-display.md:224-227` expected the answer from this run;
  the answer is instead that the probe first needs a change — the walk finds
  the list's END, or a provably free span before `HEADE` is found — "not a
  board retry" (`tools/probes/rpi_5/rpi5_scanout32/README.md:282-285`).
- The read-only compare of the firmware's 256 words passed with exactly one
  differing word, 676, entry offset 4, the declared `4:skip` word: the HVS
  writes that context word back while scanning
  (`tools/probes/rpi_5/rpi5_scanout32/README.md:264,279-283`; Q2's board
  answer `docs/notes/rpi5-display.md:194-210`; the Linux reading behind it
  `driver/docs/rpi5-v3d71-facts.md:438`). This is already folded into the
  probe: the knob `RPI5_SCANOUT32_HW_WORDS` defaults to `4:skip`
  (`tools/probes/rpi_5/rpi5_scanout32/CMakeLists.txt:34`), the skip applies
  to the firmware's span and to our own slot read-back
  (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:124-149`,
  `tools/probes/rpi_5/rpi5_scanout32/README.md:53-86`), and every differing
  word is named and judged
  (`tools/probes/rpi_5/rpi5_scanout32/README.md:85-99`).

So the frame loop below runs on top of a library whose swap and restore
mechanics are written and host-checked against a channel model
(`tools/v3d/check71.d/hvs6_dlist_swap.sh`, 269 checks,
`tools/probes/rpi_5/rpi5_scanout32/README.md:175-179`) but not yet proven by
a board swap. Everything about a swap that follows is **unverified** on this
firmware until a fixed `rpi5_scanout32` makes one.

## 3. The plane

Linear, unscaled, one plane — the entry `hvs6w_write_plane` writes
(`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:94-138`): a 9-word
entry plus the list's END word, 10 words (`:61-64`); the decoder's ADDR_MODE
0 (linear) comes from the zeroed field (`:97`); no POS1 word, no scaling, no
flip, no tiling (`:31`).

| Field | Value | Source |
|---|---|---|
| Size and position | 1920x1080 at (0,0) — the firmware's mode | `docs/notes/rpi5-display.md:296-312`; the same frame bounds as `rpi5_scanout32` (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:141-145`) |
| Depth | 32 bpp | `hvs6w_bpp` returns 4 for the 8888 formats (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:81-92`) |
| Pitch | 7680 bytes = 1920 x 4; checked as a multiple of 4, at least one row, at most 17 bits | `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:113-114`; `pitch = w * 4` at `tools/probes/rpi_5/rpi5_scanout32/src/main.c:812` |
| Buffer size | 1920 x 1080 x 4 = 8294400 bytes per buffer | `tools/probes/rpi_5/rpi5_scanout32/src/main.c:144-145` (`FB_BYTES`) |
| Fetch address | 40-bit `addr`, carried in PTR0 upper bits and PTR1 | `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:115-116`, `:132-133` |

**Format.** The 7.1 raster store writes R, G, B, A to memory bytes 0-3
*(facts, host reading, never run against a scanout)*. With that memory order
the window format is `RGBA8888` ("R,G,B,A in memory",
`driver/zephyr/lib/board/rpi5_scanout/hvs6_fmt.h:55-58`), and the scanout
entry for it is **XBGR8888**: format code `RGBA8888` (7) with order `ABGR`
(`driver/zephyr/lib/board/rpi5_scanout/hvs6_fmt.h:47-53`, table row `:77`,
inverse map `:108-114`, Linux's use of these two orders `:22-27`). The plane
must therefore be XBGR8888, not the XRGB8888 `rpi5_scanout32` uses today
(`tools/probes/rpi_5/rpi5_scanout32/README.md:6`, the `u3 plane` line at
`tools/probes/rpi_5/rpi5_scanout32/src/main.c:830-832`). The writer cannot
express XBGR8888 yet: `enum hvs6w_format` has XRGB8888, ARGB8888 and RGB565
only (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:46-50`) and
picks order `ARGB` for every 32-bit format (`:118-124`). Adding
`HVS6W_XBGR8888` (code 7, order ABGR) is item G1. Whether the 7.1 store can
swap R and B is **unverified** (glscan-survey found no such setting);
XRGB8888 would need it, so it is not the plan. If the byte-order answer at
G2/G4 turns out reversed, the plane becomes XRGB8888 and G1 is dropped.

Alpha is ignored: CTL2 is written opaque (`HVS6W_CTL2_OPAQUE` at
`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:68-69`, written at
`:129`) and the CTL0 alpha-mask field is FIXED (`:65-67`, `:125-127`). The
entry's word 4 is the POS3 context word (the writer puts Linux's filler
`0xc0c0c0c0` there, `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:70-71,131`)
— the one word the HVS writes back
(`driver/docs/rpi5-v3d71-facts.md:438`, confirmed on the board, section 2),
so our own slot keeps the same `4:skip` treatment at its read-back
(`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:135-136,1315-1333`).

## 4. The buffers

Two buffers A and B, `FB_BYTES` each, page-aligned static arrays in the
image (the pattern `rpi5_scanout32` already uses:
`tools/probes/rpi_5/rpi5_scanout32/src/main.c:141-145`, `:180-181`). They
live in the image's `sram0`: every Pi 5 probe overlay sets `sram0` to 32 MiB
from `0x200000`, below 4 GiB
(`tools/probes/rpi_5/rpi5_scanout32/boards/rpi_5.overlay:4,14-15`). 2 x
8294400 = 16588800 bytes fit the 32 MiB region with the kernel, the job's
control lists, tile allocation and tile state (section 7); the image must
link, as `rpi5_scanout32` did with the same two arrays.

One buffer, two addresses:

- **For the HVS**, the ARM physical address, which *is* the bus address:
  the offset is 0, the identity, because the BCM2712 `soc` has no
  dma-ranges (`driver/zephyr/lib/board/rpi5_scanout/bus_addr.h:35-44`), and
  display-path addresses are 40 bits (`:46-60`, refused at
  `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:115-116`).
  `rpi5_scanout32` converts `k_mem_phys_addr` through `bcm2712_arm_to_bus`
  (`driver/zephyr/lib/board/rpi5_scanout/bus_addr.h:144-157`, called at
  `tools/probes/rpi_5/rpi5_scanout32/src/main.c:820-821`), prints both
  values (`:825-832`), and puts the bus value in the plane entry
  (`:850`, `:947`, `:967`). IOMMU4 is off on the board, so a list address is
  an ARM physical address (`docs/notes/rpi5-display.md:43`).
- **For the V3D**, a GPU VA from the probe's own page table: `gpu_map` maps
  the buffer's physical address into the job's VA space, writable
  (`tools/probes/rpi_5/rpi5_triangle/src/main.c:143-157`; the triangle maps
  its target the same way at `:253`, every job object at `:247-253`).

**Open — the GPU VA window.** `rpi5_triangle` maps 16 MiB of GPU VA from
1 MiB with one 4096-entry page table
(`tools/probes/rpi_5/rpi5_triangle/README.md:27`,
`tools/probes/rpi_5/rpi5_triangle/src/main.c:54`), and its board run worked
inside `VA 0x100000..0x12c000` (`driver/docs/rpi5-v3d71-facts.md:675`). The
two framebuffers alone take 16588800 bytes of the 16 MiB window before the
tile allocation, tile state, lists and shaders. G2 must either enlarge the
window (more page-table entries) or map only the buffer being rendered. Not
decided here.

**Guard rows.** 1080 is not a multiple of 64 (section 7), so a full-tile
store could write past the last row; give each buffer the guard rows the
triangle gives its target (`TGT_BYTES` adds two rows and rounds to a page,
`tools/probes/rpi_5/rpi5_triangle/src/main.c:57-63`, sentinel-filled at
`:375-376`; board record "guard rows unchanged",
`driver/docs/rpi5-v3d71-facts.md:676`), and keep the buffers from being
neighbours the store can overrun into.

## 5. Cache rules

The buffers are cacheable image RAM, never mapped a second time
(`device_map` of a RAM page is `-EBUSY` and a panic,
`tools/probes/rpi_5/rpi5_scanout32/README.md:48-51`). All of this needs
`CONFIG_CACHE_MANAGEMENT=y`, or every `sys_cache_data_*` call is a no-op
(`tools/probes/rpi_5/rpi5_scanout32/prj.conf:9-12`).

1. **Before the job**, flush and invalidate every range the job reads or
   writes — control lists, shader code, vertices, tile allocation, tile
   state, overflow block and the target — with
   `sys_cache_data_flush_and_invd_range` (the triangle's `clean()`,
   `tools/probes/rpi_5/rpi5_triangle/src/main.c:158-168`, purpose: the GPU
   sees the CPU's bytes and no dirty CPU line later lands over the GPU's;
   calls at `:378-381`). For the target this drops dirty lines so a CPU line
   of the *previous* frame cannot be written back over the new pixels.
2. **After the job**, clean the V3D's own caches with `v3d_caches_clean`
   (`tools/probes/rpi_5/rpi5_triangle/src/main.c:416`): the GPU caches must
   not keep lines of pixels the HVS will fetch from RAM. The Linux-side
   cache steps this mirrors are
   `driver/docs/rpi5-v3d71-facts.md:163-185` (invalidate before each queue
   start at `:169,184-185`; clean after a job at `:172`).
3. **Before a CPU read-back**, invalidate the range
   (`sys_cache_data_invd_range` at
   `tools/probes/rpi_5/rpi5_triangle/src/main.c:424`; the s32 pattern
   read-back cleans and invalidates at
   `tools/probes/rpi_5/rpi5_scanout32/src/main.c:1040-1044`).
4. **A CPU write to a buffer** (a test pattern, not a rendered frame) is
   followed by `sys_cache_data_flush_range`
   (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:818-819`).
5. **List words** are device memory in the SoC window: no cache line holds
   them (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:240`,
   `tools/probes/rpi_5/rpi5_scanout32/README.md:50-51`); the library still
   cleans its list range and barriers before `LPTRS`
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:100-102,1334-1338`),
   a no-op there.

**Open:** which GPU flush makes a render visible to the HVS's fetch is not
proven on a board yet (no probe has combined a job with a scanout read-back
of the same bytes). Step 2 plus G4's pixel samples are the test.

## 6. The frame loop

It reuses `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h`
unchanged; the sequence below is the library's (`:83-123`). **Precondition:**
the placement refusal of section 2 is fixed in the probe or the library
first; until then every swap line is **unverified** on this firmware.

1. **Snapshot** the firmware's `LPTRS` and its list walked entry by entry to
   its END (`hvs6s_snapshot`,
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:875`; rules
   `:84-89`, 256-word window `:254`), hashing the span with the declared
   hardware words left out (`:124-149`).
2. **Place** two slots, 20 words (10 each — `HVS6W_LIST_WORDS` at
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:63-64`, the pair
   span at `tools/probes/rpi_5/rpi5_scanout32/src/main.c:153`): the first
   fit at or after word 32
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:75-78,250,256`)
   whose words all read zero, clear of the firmware's span and of every
   channel's `HEADE`/`LACT` (`:90-98`,
   `tools/probes/rpi_5/rpi5_scanout32/src/main.c:748-760`). A refusal names
   the reason and writes nothing
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:402-418`,
   `tools/probes/rpi_5/rpi5_scanout32/src/main.c:801`).
3. **Render frame n into the hidden buffer** — the one the channel does not
   scan: buffer B on odd n, A on even (the alternation `rpi5_scanout32`
   already swaps, `tools/probes/rpi_5/rpi5_scanout32/README.md:31`). One 7.1
   job at 1920x1080 with the tiling and sizes of section 7 and the Y flip of
   section 7, cache steps 1-3 of section 5.
4. **Wait for the job**: bin and render done (`FLDONE`/`FRDONE` core
   interrupts, `driver/docs/rpi5-v3d71-facts.md:97-98`, within bounded polls
   as on the board, `driver/docs/rpi5-v3d71-facts.md:675-676`), no overflow
   block. A timeout or `OUTOMEM` stops the loop before any swap: a buffer
   the job did not finish is never shown. The poll bound is **unverified**
   (`rpi5_scanout32` uses `MAX_POLLS` 200, about 12 frames,
   `tools/probes/rpi_5/rpi5_scanout32/src/main.c:150`).
5. **Swap at vblank**: `hvs6s_swap`
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:1291`) builds
   the slot's entry from `cfg.plane` — the bus address of the buffer just
   rendered, as `rpi5_scanout32` sets it per swap
   (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:947`, `:967`) — writes the
   10 words and reads every one back with the `4:skip` tolerance
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:1303-1333`),
   cleans and barriers (`:1334-1338`), waits for a frame edge found by reads
   only: channel 0's `STATUS.FRCNT` (bits 21:16,
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_vblank.h:54-57`) read in a
   tight loop until it steps (`hvs6s_wait_vblank_reads` at
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:1187`; the
   read-only detector `hvs6v_wait_frame` at
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_vblank.h:124`, rule `:9-11`;
   the same loop carried in the library as `hvs6s_find_step`,
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:1158`); then
   writes `LPTRS` as one write
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:1356`; Linux
   updates a live channel the same way, `:62-64`). Taking the FRCNT step as
   the start of the vertical blank is an assumption no source states
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:170-173`), and
   `PV_INTSTAT` is not touched
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_vblank.h:17-19`).
6. **The swap counts only when `DL.LACT` picks the new list up**: a tight
   loop reads `DL` then `STATUS` until `DL.LACT` (bits 11:0,
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:260-263`) equals
   the head just written
   (`hvs6s_tight_loop` `:1239`, `:1361-1391`; the bounded criterion `:104-109`,
   error `HVS6S_ERR_LACT_PICKUP` `:391,419`; Linux completes a flip the same
   way, `:66-71`, `driver/docs/rpi5-v3d71-facts.md:507-512`). The s32 bounds
   apply: `LACT_FRAMES` 4 frames past the write
   (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:161`).
7. **Do not reuse the other buffer yet**: the swap returns only after
   `FRCNT` advanced twice past the write (`HVS6S_SETTLE_FRAMES` 2,
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:277`, enforced
   `:111-113,1437-1455`; Linux ages a list
   one frame past the swap and the library uses two to be safe, `:72-74`).
   Only then may the next frame render into the buffer the channel has left;
   `rpi5_scanout32` additionally holds each buffer on screen 1 s
   (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:148`, `k_msleep` at
   `:882`, `:982`).
8. **Alternate for a fixed number of frames** (8, as in S8,
   `tools/probes/rpi_5/rpi5_scanout32/README.md:31`), printing one `u3 swap`
   and one `u3 vblank` record per swap
   (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:441-483`; judged by
   `tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:90-109`).
9. **Restore**: `hvs6s_restore`
   (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_swap.h:1472`) finds a
   frame edge by reads again (`:1500-1506`), writes the firmware's `LPTRS`
   back —
   always, even when no edge was found (`:1511`, safety `:213-216`) — waits
   two `FRCNT` advances and `DL.LACT` back on the firmware's head
   (`:114-123`, `dlswap restore-lact`), then compares the firmware's span
   byte-equal against the snapshot **except the declared hardware-written
   word** (`tools/probes/rpi_5/rpi5_scanout32/README.md:33,53-86`; the
   default `4:skip` at `tools/probes/rpi_5/rpi5_scanout32/CMakeLists.txt:34`;
   the fold of Q2's answer into the probe, every differing word named at
   `README.md:85-99` and judged by
   `tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:46` with its selftest).
   The
   restore runs whatever failed after the first write
   (`README.md:37`). Our own slot words go back to zero only when the
   restore has passed (`README.md:33`).
10. **Stop feeding the watchdog**: the board returns to Linux by tryboot
    through `tools/board.sh pi5`
    (`tools/probes/rpi_5/rpi5_scanout32/README.md:159-169`,
    `docs/notes/rpi5-display.md:136-139`), as the 2026-11-25 run did even in
    its failure (`README.md:229-243`).

No step programs the pixel valve, HDMI or PHY. Whether the swap is
glitch-free on screen (Q4, `docs/notes/rpi5-display.md:224-227`) stays what
only a landed swap plus registers can say, with the limits spelled out at
`docs/notes/rpi5-display.md:272-279`: `landed=yes` means "no register shows
a problem" (the landing rules, `tools/probes/rpi_5/rpi5_scanout32/README.md:101-146`).

## 7. Frame sizes, Y orientation, edge tiles

**Tiling.** With one 32 bpp render target and no MSAA, the 7.1 branch takes
the first table size whose colour and depth fit the TLB — 64x64
(`driver/src/v3d/v3d_frame.c:57-62,77-93`; the board jobs used 64x64,
`driver/docs/rpi5-v3d71-facts.md:675-676`). Tiles: 1920/64 = 30 by
ceil(1080/64) = 17, 510 tiles, the last tile row covering only
1080 − 16 x 64 = 56 rows *(tile2)* (`driver/src/v3d/v3d_frame.c:94-95`).
Supertiles 2x2, 15 x 9 = 135 ≤ 256 (`driver/src/v3d/v3d_frame.c:97-115`,
`V3D_MAX_SUPERTILES` at `:22`; the port's rule, not checked against Mesa's
`v3dx_rcl.c`, which is not vendored). Framebuffer width 1920 sits under the
7.1 limit 7680 (`driver/docs/rpi5-v3d71-facts.md:666`).

**Sizes.** From `v3d_frame_alloc_sizes` (`driver/src/v3d/v3d_frame.c:118-128`):
tile state 510 x 256 = 130560 bytes (`:127`); tile allocation grows by
510 x 128 / 2 bytes per draw, capped at 512 KiB (`:124-125`). The report's
figure 110592 for one draw *(tile2)* does not match a hand read of
`:122-126`, which gives 106496 — **unverified**, G2 recomputes it. The
triangle proves the function's own numbers (16384 for one draw at 64x64,
`tools/probes/rpi_5/rpi5_triangle/src/main.c:328-335`, board-confirmed
`driver/docs/rpi5-v3d71-facts.md:676`): the probe computes with
`v3d_frame_tiling_gen`/`v3d_frame_alloc_sizes`, prints the sizes before
using them, and refuses when a static array is too small, as
`rpi5_triangle` does (`tools/probes/rpi_5/rpi5_triangle/src/main.c:329-335`).

**Edge tiles.** Whether the store writes only the 56 rows of the partial
last tile row, or a full 64, is **unverified**; the guard rows of section 4
cover both. G4's read-back samples in the last tile row answer it.

**Y.** GL's row 0 is the bottom of the picture; the HVS fetches memory row 0
as the top line (POS0 y and POS2 lines at
`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:128,130`). The
board-proven case is *not* flipped: `y0_top = false` puts GL y=0 at memory
row 0, and the triangle's read-back matched unflipped
(`tools/probes/rpi_5/rpi5_triangle/src/main.c:322-323`; "rows not flipped",
with a flipped re-check giving more wrong pixels at `:477-486`,
`driver/docs/rpi5-v3d71-facts.md:676`). A full-screen frame therefore needs
one flip, not two. The plan takes the viewport path, Mesa's:
`v3d_viewport_gl(.., y0_top = true, fb_height = 1080, ..)`
(`driver/src/v3d/v3d_frame.c:369-386`) with `v3d_wpos_gl(true, 1080, ..)`
for `gl_FragCoord` (`:388-394`). The store's `Height` flip field is the
alternative, **unverified**. The plane's own v-flip bit in PTR0
(`driver/docs/rpi5-v3d71-facts.md:439`) is not used: the note keeps the
entry as `hvs6w_write_plane` writes it (no flip,
`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:31`). Neither flip
has run at 1920x1080; G4's orientation line (section 8) is where that gets
proven.

## 8. Verification: registers and bytes only

The Pi 5 has no camera the farm can lease, and nothing is judged by a
picture (`docs/notes/rpi5-display.md:155-164`; the s32 statement,
`tools/probes/rpi_5/rpi5_scanout32/README.md:214-219`). A monitor frame, if
someone takes one, confirms and never proves
(`docs/notes/rpi5-display.md:160-162`).

The proof is: the placement lines; the per-swap `u3 swap` and `u3 vblank`
lines with `FRCNT` advancing through each swap and `DL.LACT` picking the new
head (`tools/probes/rpi_5/rpi5_scanout32/src/main.c:441-483`, judged by
`tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:90-109`); the
`u3 restore: PASS byte-equal except hw words` line
(`tools/probes/rpi_5/rpi5_scanout32/README.md:33`); and a CPU read-back of
each buffer after an invalidate (section 5 step 3), checking inside, outside
and at the edge of the rendered shape with the triangle's tolerance 3
(`tools/probes/rpi_5/rpi5_triangle/src/main.c:71`, `:437`, `:498`). The slot
entry read back from the list must name the buffer just rendered, as s32's
sequence judge requires of every swap
(`tools/probes/rpi_5/rpi5_scanout32/README.md:31`).

`docs/notes/rpi5-display.md:153` proposed the line `rpi5_gl_scanout: 4096 px,
0 wrong` (the 64x64-triangle figure). The per-frame lines below are the plan
at the tip; the exact names are proposals, **unverified**, and every one
follows the "every step prints a line even on the failing path" rule
(`tools/probes/rpi_5/rpi5_scanout32/README.md:20`):

| Line | Meaning |
|---|---|
| `rpi5_gl_scanout: frame <n> buf <A\|B> job done\|timeout\|error <hex>` | step 4 per frame |
| `rpi5_gl_scanout: frame <n> readback <n> px, <n> wrong` | step 3's read-back of frame n against the expected render |
| `rpi5_gl_scanout: orientation <ok\|flipped>` | the top screen row holds the shape's top (section 7) |
| `rpi5_gl_scanout: PASS (...)` / `FAIL (reason)`, then `feeding stopped, reset expected ...` | the end line, as s32's (`tools/probes/rpi_5/rpi5_scanout32/README.md:35`) |

The judge is a `verdicts.sh` with a host selftest, reusing s32's rules: PASS
needs the placement valid, every swap counted with its pickup, every swap
landed, and the restore byte-equal but for declared words
(`tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:113-125`); a refusal is
`FAIL (placement refused: ...)` with `q4: unanswered` (`:121-122`); a
missing line is an incomplete log, never a PASS (`:333-334`). The plane line
must name `fmt=XBGR8888` after G1 (today's prints `fmt=XRGB8888`,
`tools/probes/rpi_5/rpi5_scanout32/src/main.c:830`).

## 9. The ordered items

- **G1 — XBGR plane format.** `hvs6w_format` gains `HVS6W_XBGR8888` (code
  7, order ABGR) and `hvs6w_write_plane` writes it
  (`driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:46-50`,
  `:118-124`); the format table already knows the entry
  (`driver/zephyr/lib/board/rpi5_scanout/hvs6_fmt.h:77`). Its host mock and
  check script exist and get the case
  (`driver/zephyr/lib/board/test/hvs6_dlist_write_mock.c`,
  `tools/v3d/check71.d/hvs6_dlist_write.sh`). Depends on nothing, not even
  P9. Plan item g1-xbgr.
- **G2 — probe skeleton (needs P9).** New `tools/probes/rpi_5/rpi5_gl_scanout/`
  (main.c; prj.conf with the watchdog and `CONFIG_CACHE_MANAGEMENT` as
  `tools/probes/rpi_5/rpi5_scanout32/prj.conf:6-12`; CMakeLists; the s32
  overlay): the two buffers with guard rows, `gpu_map`, the sizes from
  `v3d_frame_alloc_sizes`, the `y0_top = true` viewport, one 7.1 render into
  buffer A, the read-back lines — no swap yet. It also settles the GPU VA
  window of section 4 and rechecks the size figures of section 7 on the
  host. Needs P9 merged (`docs/notes/pre-study-rpi5.md:615`).
- **G3 — frame loop.** Section 6 on top of a *fixed* placement: the walk
  finds the firmware list's END, or a provably free span before `HEADE` is
  found (`tools/probes/rpi_5/rpi5_scanout32/README.md:282-285`) — that
  change lands first, in the library or in s32, with its host mock checks
  (`tools/v3d/check71.d/hvs6_dlist_swap.sh`). The board answer to Q4 arrives
  with s32's next run (`tools/probes/rpi_5/rpi5_scanout32/verdicts.sh:90-109`
  already judges it); G3 takes it as it is. Needs G1, G2.
- **G4 — board session.** The probe's `verdicts.sh` and selftest first (the
  s32 pattern, `tools/probes/rpi_5/rpi5_scanout32/README.md:175-179`), then
  one run on `target:rpi5` with the exclusive `target:rpi5-console`, through
  `tools/board.sh pi5` with `BOARD_END_RE` and tryboot
  (`tools/probes/rpi_5/rpi5_scanout32/README.md:159-169`): read-back lines,
  `FRCNT` through each swap, every swap landed, `u3 restore: PASS`, the
  board back in Linux by the watchdog. Session rules:
  `docs/notes/rpi5-display.md:136-139`, `tools/probes/rpi_5/README.md:24-34`.
- **G5 — records.** The probe's README with the session record; the U3 row
  of `docs/notes/pre-study-rpi5.md:575`; the index line in
  `tools/probes/rpi_5/README.md`; and the `V3D 7.1: Raspberry Pi 5 on
  Zephyr` table of `tools/cts/RESULTS.md` plus `tools/cts/STATUS.md` — the
  latter in a commit of their own, because plans #388 and #385 edit those
  files too (`CLAUDE.md`, "Board results replace their row").

## 10. Open questions

1. The placement fix that G3 needs (section 2): where is this firmware's
   list END, and is there provably free room for our slots? Probe/library
   work, not a board retry
   (`tools/probes/rpi_5/rpi5_scanout32/README.md:282-285`).
2. Q4 itself (`docs/notes/rpi5-display.md:224-227`): unanswered as of
   2026-11-25; it lands with the fixed probe's swaps, with the proof limits
   of `docs/notes/rpi5-display.md:272-279`.
3. The byte-order answer: does the 7.1 store write R,G,B,A, as glscan-facts
   read it? G2 (host: the store's words) and G4 (the `fmt=XBGR8888`
   read-back) prove it; G1 is dropped if reversed.
4. The partial last tile row (56 of 64 rows) and the `y0_top = true`
   viewport at full size: neither has run on the board *(tile2)*; G4's
   samples in the last tile row and the orientation line answer them.
5. Which GPU cache flush makes a render visible to the HVS's fetch
   (section 5, **open**): start from
   `driver/docs/rpi5-v3d71-facts.md:163-185`.
6. The GPU VA window: the two 1920x1080 buffers do not fit the 16 MiB
   window `rpi5_triangle` maps (section 4). G2 decides: a bigger window, or
   mapping per buffer.
7. Tile allocation for 510 tiles: tile state 130560 is exact from
   `driver/src/v3d/v3d_frame.c:127`; the allocation figure is
   **unverified** (section 7).
8. Whether a raw RGB plane needs the UPM fields in PTR0 that Linux sets:
   the board question s32 left open
   (`tools/probes/rpi_5/rpi5_scanout32/README.md:217-219`,
   `driver/zephyr/lib/board/rpi5_scanout/hvs6_dlist_write.h:33-36`). A gl
   frame that renders and reads back proves the fetch path with or without
   them.
9. Frame time: whether one 1920x1080 job plus its cache steps fits a
   16.7 ms frame (`driver/zephyr/lib/board/rpi5_scanout/hvs6_vblank.h:46`).
   Not needed for a PASS (the swap waits for the job, then for vblank);
   measured by G4 if the image prints job times.
10. Whether the hardware writes words other than entry offset 4 in the
    firmware's 32-word entry (list words 680-703,
    `driver/docs/rpi5-v3d71-facts.md:534-535`): the 2026-11-25 compare saw
    only word 676 (`tools/probes/rpi_5/rpi5_scanout32/README.md:279-283`);
    s32's knob takes up to four offsets if a gl run shows more.
