# Raspberry Pi 5: `rpi5_gl_scanout`, the V3D 7.1 job rendering into the HVS6 plane

Design of the probe named in `docs/notes/rpi5-display.md:153` (the
`rpi5_gl_scanout` row): it renders with the V3D 7.1 job (the `rpi5_triangle`
path) into a 1920x1080 buffer that channel 0 of the HVS scans out. It is not
written; it waits for P9/P10 (plan #384: the 7.1 table and `rpi5_tfu`). This
note is the current design only. A point without a `file:line` is marked
**unverified**. `lib/` below is `driver/zephyr/lib/board/rpi5_scanout/`,
`s32/` is `tools/probes/rpi_5/rpi5_scanout32/`, `tri/` is
`tools/probes/rpi_5/rpi5_triangle/`.

## 1. The plane

| Field | Value | Source |
|---|---|---|
| Layout | linear (ADDR_MODE 0 comes from the zeroed field) | `lib/hvs6_dlist_write.h:97` |
| Size | 1920x1080, unscaled, x = y = 0 (the firmware's mode) | `s32/src/main.c:141-143` |
| Depth | 32 bpp | `hvs6w_bpp`, `lib/hvs6_dlist_write.h:82-93` |
| Pitch | 7680 bytes (1920 x 4); a multiple of 4, at most 17 bits | checked at `lib/hvs6_dlist_write.h:113-114` |
| Buffer | 8 294 400 bytes (`FB_BYTES`) | `s32/src/main.c:144` |

**Format.** The V3D 7.1 raster store writes R, G, B, A in memory bytes 0 to 3
(glscan-facts, a host reading; not run on the board). The HVS format with that
memory order is `XBGR8888`: code `RGBA8888` (7) with order `ABGR`
(`lib/hvs6_fmt.h:21-24`; table row `:77`, window format `HVS6F_WIN_RGBA8888`,
R,G,B,A in memory, `:57-58`). `hvs6w_write_plane` cannot write it today:
`enum hvs6w_format` has only XRGB8888, ARGB8888 and RGB565
(`lib/hvs6_dlist_write.h:46-50`) and writes order ARGB for every 32-bit
format (`:121-124`). That is step G1. XRGB8888 would need an R/B swap in the
store, **unverified** and not planned.

Alpha: CTL2 is opaque (`HVS6W_CTL2_OPAQUE`, `lib/hvs6_dlist_write.h:69`,
written at `:127`), so the X byte is ignored.

## 2. The buffers

Two buffers, A and B, of `FB_BYTES` each, page aligned (as
`s32/src/main.c:180-181`), in image RAM below 4 GiB. **Unverified:** that both
land in the `sram0` region below 4 GiB comes from the plan, not from a map
file; the probe must print each buffer's physical address and refuse one at or
above 4 GiB.

Two addresses name one buffer:

- **HVS:** the physical address in the plane's PTR words. The writer takes a
  40-bit address (`lib/hvs6_dlist_write.h:115-116`, words 5 and 6 at
  `:130-131`). IOMMU4 is off on the board, so a list address is an ARM
  physical address (`docs/notes/rpi5-display.md:43`); ARM and display
  addresses are the same, offset 0 (`lib/bus_addr.h:44`, `bcm2712_arm_to_bus`
  at `:144-155`).
- **V3D:** the GPU VA from `gpu_map`, which maps the buffer's physical address
  through the job's own page table (`tri/src/main.c:143-158`; the triangle
  maps its target at `:253`). The buffer is mapped once, writable.

Frame sizes: tile allocation 110 592 bytes for one draw (more for more draws)
and tile state 130 560 bytes are hand-computed by glscan-facts, **not
host-run**. The probe takes the real numbers from `v3d_frame_alloc_sizes`
(`tri/src/main.c:329`) and prints them; the triangle's 64x64 frame gives
16 384 bytes for one draw (`:328`).

## 3. Cache rules

The buffers are cacheable image memory; the V3D and the HVS read RAM
directly. `CONFIG_CACHE_MANAGEMENT=y` is required (without it every
`sys_cache_data_*` call does nothing, `s32/prj.conf:9-10`).

1. **Before the job:** flush and invalidate every range the job reads or
   writes: command lists, shader code, vertices, tile allocation, tile state,
   overflow block and the target (`clean()`, `tri/src/main.c:160-168`, calls
   at `:379-383`). For the target this drops dirty CPU lines, so none is
   written over the GPU's pixels.
2. **After the job:** `v3d_caches_clean` (`tri/src/main.c:416`), so the
   GPU's own caches hold none of the pixels.
3. **Before a CPU read-back:** invalidate the range
   (`sys_cache_data_invd_range`, `tri/src/main.c:424`).
4. **Before the HVS scans a buffer:** the CPU must have no dirty line of it;
   after steps 1 to 3 the CPU does not write the buffer. A CPU write (a test
   pattern) is followed by `sys_cache_data_flush_range`
   (`s32/src/main.c:55`, `:249`).
5. **List words** are device memory in the SoC window, no cache line holds
   them (`s32/src/main.c:240`); the library cleans its own list range
   (`lib/hvs6_dlist_swap.h:100-102`).

## 4. The frame loop

It reuses `hvs6_dlist_swap.h`; the swap and restore mechanics are not
redesigned here.

1. Snapshot the firmware's list and place ours at or above word 32 clear of
   its span (`lib/hvs6_dlist_swap.h:83-98`), once for the whole run.
2. **Render into the hidden buffer** (the one the plane does not name): the
   job of section 3 steps 1 to 3, then the read-back check of section 6.
3. **Swap at vblank:** `hvs6s_swap` with `write_at = HVS6S_AT_VBLANK`, the
   default (`lib/hvs6_dlist_swap.h:166`, `:338`). LPTRS is written just after
   a frame edge found by reads of `STATUS.FRCNT` (`lib/hvs6_vblank.h:9-12`;
   the detector writes no register, `:17-19`). The swap passes only once
   `DL.LACT` reads the new head (`lib/hvs6_dlist_swap.h:105-113`,
   `:151-153`).
4. **Do not reuse a buffer yet:** wait for the LACT pickup and two `FRCNT`
   advances past the write (`lib/hvs6_dlist_swap.h:110-113`; Linux frees an
   old list after one step, `:72-74`). Only then render into that buffer.
5. Alternate A and B for N frames, one `u3 swap` and `dlswap` record per swap
   (`s32/src/main.c:456-476`).
6. **Restore:** the firmware's LPTRS written just after a vblank found by
   reads, `DL.LACT` back on the firmware's head, two `FRCNT` advances, then
   the firmware's words byte-equal except the hardware-written word
   (`lib/hvs6_dlist_swap.h:114-122`, `:124-149`). The watchdog then returns
   the board to Linux through `tools/board.sh pi5`
   (`s32/prj.conf:5-8`).

Whether a vblank swap is glitch-free (Q4) is open until board-u3 has run
`rpi5_scanout32`; G3 starts after that answer.

## 5. Y orientation and edge tiles

- **Y.** With `y0_top = false` GL y = 0 lands at memory row 0, which is the
  case the board proved (`tri/src/main.c:323`). Scanout reads row 0 at the
  top, so the picture needs flipping: `v3d_viewport_gl(..., y0_top = true,
  fb_height = 1080, ...)` (its caller `tools/v3d/pack_seq.h:41`; see also
  `src/v3d/state.rs:288`) or the store's `Height` flip field. **Neither has
  run on the board** (glscan-facts).
- **Edge tiles.** 1080 is not a multiple of the tile height, so the last tile
  row is partial (56 rows, glscan-facts). Whether the store writes only those
  56 rows is **unverified**. Until a board run shows it, give each buffer the
  guard rows the triangle gives its target (`TGT_BYTES` adds two rows,
  `tri/src/main.c:63`) so a full-tile store cannot overrun the next buffer.

## 6. Verification

No camera is available for the Pi 5 (`docs/notes/rpi5-display.md:155-164`).
The proof is registers and bytes: the list names the buffer, `FRCNT` advances
through each swap, and the CPU read-back after the invalidate holds the
rendered pixels, checked inside, outside and at the edge of the triangle with
the triangle probe's tolerance (`tri/src/main.c:71`, `:437`). A monitor
frame is confirmation only. The expected lines are
`rpi5_gl_scanout: <n> px, 0 wrong` and the plane readback line
(`docs/notes/rpi5-display.md:153`; `<n>` for a full frame is **unverified**).

## 7. Ordered items

- **G1: XBGR plane format.** `hvs6w_format` gains `HVS6W_XBGR8888` (code 7,
  order ABGR) and `hvs6w_write_plane` writes it
  (`lib/hvs6_dlist_write.h:46-50`, `:121-124`); the format table has it
  (`lib/hvs6_fmt.h:77`). Plan item g1-xbgr.
- **G2: probe skeleton.** `tools/probes/rpi_5/rpi5_gl_scanout`: buffers,
  `gpu_map`, one render into A, read-back, no swap. Needs P9 (the 7.1 table).
- **G3: frame loop.** Section 4 with the Y flip and edge tiles of section 5.
  After board-u3's Q4 answer.
- **G4: board session.** One tryboot run on `target:rpi5` with
  `target:rpi5-console`: read-back lines, `FRCNT` through each swap,
  `u3 restore: PASS`, the watchdog back to Linux.
- **G5: docs.** The Q answers in `docs/notes/rpi5-display.md`, the probe's
  README, and the RESULTS.md and STATUS.md rows, the last in a commit of
  their own.
