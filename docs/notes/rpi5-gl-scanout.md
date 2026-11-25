<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5: rpi5_gl_scanout, a V3D 7.1 frame in our own HVS6 plane (P11 after P9)

Status: plan, 2026-11-25. Nothing in this note has been built or run.
`rpi5_gl_scanout` renders a 1920x1080 frame with the V3D 7.1 job and shows it
in our own HVS6 plane on the Pi 5. It is the last row of the probe table in
`docs/notes/rpi5-display.md:153`, and it waits for P9 and P10
(`docs/notes/pre-study-rpi5.md:615-616`). This note fixes its design and the
order of its items G1-G5. The display side is stage 2 of `rpi5-display.md`
(`:73-90`, `:152`), not repeated here.

Citation prefixes: `disp:` = `docs/notes/rpi5-display.md`; `facts:` =
`driver/docs/rpi5-v3d71-facts.md`; `lib/` = `driver/zephyr/lib/board/rpi5_scanout/`;
`s32/` = `tools/probes/rpi_5/rpi5_scanout32/`. Other paths are this
repository's. Figures marked *(tile2)* or *(facts)* come from the plan #390
reports glscan-tile2 and glscan-facts. Those reports changed no file, and
their figures were worked out by hand from `driver/src/v3d/v3d_frame.c`, not
by a host run. No board address, host name or UART path is in this file.

## 1. Goal and non-goals

Goal: one Zephyr image for the Pi 5 that
- adopts the firmware's 1920x1080 mode on HDMI0 (`disp:113-129`, Q6 at `disp:243-259`),
- renders full-screen frames with the driver's 7.1 bin and render job (the
  `rpi5_triangle` path, board-proven at 64x64, `facts:666`) straight into one
  of two 32 bpp buffers,
- shows each buffer by swapping channel 0's list pointer at vblank between
  two of our own plane lists at or after word 32 (`disp:73-90`),
- restores the firmware's list at the end, then lets the PM watchdog return
  the board to Linux (`disp:136-139`).

Non-goals:
- No mode set, and no pixel valve, HDMI, PHY or clock writes. The only
  registers written are HVS list words that our own placement check has
  approved, and channel 0's `LPTRS` (`s32/README.md:13-16`). A full mode set
  stays option (ii), scheduled only if Q6 shows the firmware leaves no
  picture (`disp:98-111`, `disp:127-129`).
- No CPU copy or format conversion between the render target and the plane:
  the render target *is* the plane's buffer (section 3).
- No textures unless P10 has merged (section 6).
- No camera: the verdict comes from registers and bytes only (`disp:155-164`).

## 2. Memory layout

| Object | Size | Where it comes from |
|---|---|---|
| Framebuffer A, B (XBGR8888, linear) | 1920 x 1080 x 4 = 8294400 bytes each, stride 7680 bytes; 16588800 for both | `s32/src/main.c:142-145,180-181` (the same two static, 4096-aligned buffers `rpi5_scanout32` already uses; pitch = width x 4 at `:812`) |
| Tile size | 64x64 (one 32 bpp render target, no MSAA) *(tile2)* | 7.1 rule at `driver/src/v3d/v3d_frame.c:69-91` |
| Tile count | 30 x 17 = 510; the last tile row covers only 56 rows (1080 - 16 x 64) *(tile2, facts)* | `v3d_frame.c:92-93` |
| Supertiles | 2x2 tiles, 15 x 9 supertiles *(tile2: the port's output, not checked against Mesa's `v3dx_rcl.c`, which is not vendored)* | `v3d_frame.c:97-116`, `V3D_MAX_SUPERTILES` 256 at `:22` |
| Tile allocation | 110592 bytes for one draw, more for more draws (the pool grows by 510 x 128 / 2 bytes per draw, capped at 512 KiB) *(facts)* | `v3d_frame.c:119-128`, `V3D_TILE_ALLOC_INITIAL_BLOCK` 128 at `:23` |
| Tile state | 510 x 256 = 130560 bytes *(facts)* | `v3d_frame.c:127` |
| Our plane lists (slot A, slot B) | `HVS6W_LIST_WORDS` = 10 words each (a 9-word entry and END) | `lib/hvs6_dlist_write.h:63-64`; placed by `hvs6s_place_io` (`lib/hvs6_dlist_swap.h:1038`), first fit at or after word 32 and clear of the firmware's span `[HEADE, END)` (`lib/hvs6_dlist_swap.h:75,90,249`; `disp:79-86`) |
| The firmware's list | from `LPTRS.HEADE` (word 672 on the board) to its END; entry offset 4 (word 676) is written back by the HVS | `disp:196-210`; `facts:417` onward |

Open: the GPU VA window. `rpi5_triangle` maps 16 MiB of GPU VA from 1 MiB
with one 4096-entry page table (`tools/probes/rpi_5/rpi5_triangle/README.md:27`,
`tools/probes/rpi_5/rpi5_triangle/src/main.c:54-55`). The two framebuffers alone
need 16588800 bytes, more than 16 MiB (16777216) leaves once tile
allocation, tile state, the control lists and the shaders are added. G2 must
map a larger window or map only the buffer being rendered. Which of the two
is not decided here.

## 3. The plane format and the Y orientation

Format. The 7.1 raster store writes R, G, B, A to memory bytes 0-3 *(facts)*.
That is the window format `RGBA8888`, whose scanout entry is `XBGR8888`: HVS
code 7 (`RGBA8888`), order `ABGR` (`lib/hvs6_fmt.h:57,77,109-111`). The plane
must therefore be XBGR8888, not the XRGB8888 that `rpi5_scanout32` uses
(`s32/README.md:6`). `hvs6w_write_plane` cannot write it yet: its format enum
has XRGB8888, ARGB8888 and RGB565 only (`lib/hvs6_dlist_write.h:46-50`). Adding
it is G1 (item g1-xbgr). glscan-survey found no swap-R/B setting in the 7.1
store, so XRGB8888 would need one, and it is not planned. Whether the store
can swap R and B is open (section 7).

Y orientation. GL's row 0 is the bottom of the picture, and the HVS fetches
memory row 0 as the top line. The board-proven case is `y0_top=false`
(`facts:666`: "rows not flipped"), which puts GL y=0 at memory row 0, so the
picture would appear upside down. A full-screen frame needs one of two flips
*(facts)*:
- the viewport: `v3d_viewport_gl(.., y0_top=true, fb_height=1080, ..)`
  (`driver/src/v3d/v3d_frame.c:369-386`), with `v3d_wpos_gl(true, 1080, ..)`
  for `gl_FragCoord` (`:388-394`); or
- the store's `Height` flip field.

The plan takes the viewport: it is the st/mesa path, already in the port, and
it keeps the store as on the board. Neither flip has run on the board. The
plane's own v-flip bit (pointer word 0, `facts:439`) is not used, because the
note keeps the HVS entry as `rpi5_scanout32` writes it.

## 4. Frame flow

Per run, after `rpi5_scanout32`'s steps up to the placement
(`s32/README.md:18` onward):

1. **Set up.** Snapshot the firmware's list and registers (`hvs6s_snapshot`,
   `lib/hvs6_dlist_swap.h:875`). Place slots A and B (`hvs6s_place_io`, `:1038`)
   and write one XBGR8888 entry per slot, pointing at framebuffer A or B
   (`hvs6w_write_plane`, `lib/hvs6_dlist_write.h:99`, after G1). Bring the V3D
   up as `rpi5_triangle` does (`tools/probes/rpi_5/rpi5_triangle/README.md:24-30`).
2. **Render** frame *n* into the back buffer (B for odd *n*, A for even):
   one 7.1 job at 1920x1080, with the tiling and sizes of section 2 and the
   Y flip of section 3.
3. **Wait for the job.** Bin and render done (`FLDONE`/`FRDONE`, as `facts:666`)
   within a bound, with no overflow error. A timeout stops the loop before
   any swap (the swap of an unfinished buffer is never done).
4. **Make the bytes visible to the HVS.** The buffers are mapped
   non-cacheable for the CPU as in `rpi5_scanout32`. The GPU writes through
   its own path, so before the swap the image flushes the V3D's caches the
   way `rpi5_triangle` does before its read-back. Which flush is enough for
   a scanout reader is open (section 7).
5. **Swap at vblank.** `hvs6s_swap` (`lib/hvs6_dlist_swap.h:1291`) writes
   `LPTRS` just after a frame edge found by reads (`hvs6s_wait_vblank_reads`,
   `:1187`; the read-only detector `lib/hvs6_vblank.h:124`), and counts the
   swap only once `DL.LACT` reaches the new head. The method and answer of Q4
   are `rpi5_scanout32`'s (board-u3); G3 takes them as they are.
6. Repeat 2-5 for a fixed number of frames (for example 8, alternating A and B).
7. **Restore.** `hvs6s_restore` (`lib/hvs6_dlist_swap.h:1472`) puts the
   firmware's `LPTRS` back just after a vblank and checks the firmware's list
   byte-equal, except the words the hardware writes back (entry offset 4,
   `disp:208-210`).
8. Stop feeding the watchdog. The board returns to Linux by tryboot.

No step programs the pixel valve, HDMI or PHY.

## 5. Verdict lines and how a board run is judged

The image prints every `rpi5_scanout32` line it reuses unchanged (`u3
snapshot`, `u3 dlist: placed ...`, the per-swap FRCNT and vblank lines,
`u3 restore: PASS|FAIL`, `s32/verdicts.sh:46,125,189-201`), with the plane line
naming `fmt=XBGR8888`. Lines of its own (proposed names):

| Line | Meaning |
|---|---|
| `rpi5_gl_scanout: frame <n> buf <A|B> job done|timeout|error <hex>` | step 3 per frame |
| `rpi5_gl_scanout: frame <n> sample <k> px, <w> wrong` | a CPU read-back of a fixed sample of pixels after step 4 against the expected render (clear colour plus shape, with the tolerance `rpi5_triangle` uses, `facts:666`) |
| `rpi5_gl_scanout: orientation top-row <ok|flipped>` | a check that the frame's top screen row holds the shape's top (section 3) |
| `rpi5_gl_scanout: verdict PASS|FAIL <reason>` | the end line |

PASS needs all of these: every job done, every sample 0 wrong, the
orientation `ok`, every swap counted by `hvs6s_swap` with FRCNT advancing
through it, `u3 restore: PASS`, and the board back in Linux through
`tools/board.sh pi5`. Any missing line is FAIL or incomplete, never PASS, as
in `s32/verdicts.sh:334,347`. The judge is a `verdicts.sh` with its host
selftest, as for every Pi 5 probe. Bytes and registers prove the result. A
picture on a monitor, if someone sees one, only confirms it (`disp:160-162`).

## 6. G1-G5

| Item | What | Depends on | Files it touches |
|---|---|---|---|
| G1 (g1-xbgr) | `hvs6w_write_plane` gains `HVS6W_XBGR8888` (code 7, order ABGR), with its decode and mock case | nothing: no P9 | `lib/hvs6_dlist_write.h:46-50,82-99`, its host mock `driver/zephyr/lib/board/test/hvs6_dlist_write_mock.c` |
| G2 | Host check of the 1920x1080 7.1 job: tiling and allocation sizes from `v3d_frame_tiling_gen`/`v3d_frame_alloc_sizes` (`driver/src/v3d/v3d_frame.c:64-128`) checked against section 2's figures, the `y0_top=true` viewport, and the GPU VA window of section 2 | P9 merged | a new host test next to `tools/v3d/frame_check.c`; no `src/v3d` change unless P9 already owns one |
| G3 | The probe `tools/probes/rpi_5/rpi5_gl_scanout/` (main.c, prj.conf, CMakeLists.txt): sections 4 and 5, reusing `hvs6_dlist_swap.h` as it is | G1, G2, P9; board-u3's Q4 answer (and restore-vblank, merged) so that the swap and restore are the board-proven ones; P10 only if a textured frame is drawn | `tools/probes/rpi_5/rpi5_gl_scanout/**` |
| G4 | Its `verdicts.sh` and host selftest, then the board session on `target:rpi5` with `target:rpi5-console` (exclusive console, the tryboot restore of safety-c) | G3; safety-c | `tools/probes/rpi_5/rpi5_gl_scanout/verdicts*.sh`, its README; `tools/board.sh` only if its end line is not recognised (as board-sh-dispregs did) |
| G5 | Records: `disp:153` row result, the U3 row of `docs/notes/pre-study-rpi5.md:575`, the probes index `tools/probes/rpi_5/README.md`, and (in its own commit, since plans #388 and #385 edit them too) `tools/cts/RESULTS.md` and `tools/cts/STATUS.md` | G4's board run | the files named |

## 7. Open questions

1. The store of the partial last tile row (56 of 64 rows), and the
   `y0_top=true` viewport at full size: neither has run on the board *(facts)*.
   G4's orientation line and samples in the last tile row answer them.
2. The supertile figures (2x2, 15x9) are the port's, not checked against
   Mesa's `v3dx_rcl.c` *(tile2)*. They matter only if the job fails at full
   size.
3. The GPU VA window: two buffers do not fit the 16 MiB `rpi5_triangle`
   maps (section 2). G2 decides.
4. Which V3D cache flush makes a render visible to the HVS fetch (section 4,
   step 4). Open; reading `facts:163` (caches and reset) is the first step.
5. Whether the 7.1 store can swap R and B. Not needed with XBGR8888, so it
   stays open.
6. Q4 itself (`disp:224-226`): can the pointer be swapped without a glitch.
   That is board-u3's answer. G3 does not start before it.
7. Frame time: whether one 1920x1080 job plus its flush fits one 16.7 ms
   frame. Not needed for a PASS (the swap waits for the job). Measured by
   G4 if the image prints the job time. Open.
