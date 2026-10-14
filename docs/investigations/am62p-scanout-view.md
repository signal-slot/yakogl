<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: a scan-out view for Slint and the cluster?

Plan #2, item am62p-view. Design only; this note changes no code. Line
numbers are of the integration tree at `fdff1147`. The git history of the
tree is one squashed root commit (`afb16335`), so `git log -S` names only that
commit; the evidence below is the tree and the board records.

## Answer

**Yes, the AM62P's DSS can scan out the buffer the PowerVR renders into, and
it already does. No frame is copied on the AM62P today, with or without
`CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`.** There is no copy to remove, so the AM62P
needs no scan-out view for Slint or the cluster.

The opposite claim comes from three lines that use "copy path" to mean "not
a view". On the Pi 4 those two things are the same. On the AM62P they are not:

- `docs/benchmarks.md:88`: "The board layer has no scan-out view, so the view
  row is the copy path again".
- `tools/cts/common-tree.txt:9-13`: "slint: the window copied to the screen";
  "the AM62P's board layer has no view ... there the row is the copy path
  again".
- `tools/cts/RESULTS.md:437` and `:448`: "the copy path, as predicted".

The code and the design say no copy happens:

- `driver/zephyr/lib/board/include/yakogl_board.h:114-118`: "AM62P unchanged,
  because there is no copy to remove: board_show() is a DSS
  application-plane flip by address with the window's own stride ... measured
  board_show at 0.0 ms mean and worst over 5971 frames".
- `driver/docs/one-method.md:186-187`: the same.
- `driver/zephyr/lib/board/am62p/board_am62p.c:69-87`: `board_show()` only
  calls `pvr_sample_show_window` / `pvr_sample_flip_window`. It never calls
  `board_show_copied()`. Only the Pi 4 (`rpi4/board_rpi4.c:312`) and the
  i.MX8MP (`imx8mp/board_imx8mp.c:145`) call it.

So the Slint frame rate on the AM62P (703 frames in 10 s) is the GPU and CPU
rendering time, not a copy. The Pi 4 went from 659 to 1523 because its copy
cost about 8.6 ms of a 15 ms frame. The AM62P has no such cost to remove.

## 1. The Pi 4 view, end to end

| step | what happens | where |
|---|---|---|
| Kconfig | `YAKOGL_BOARD_SCANOUT_VIEW`, default n; for the Pi 4 and the i.MX8MP only | `driver/zephyr/lib/board/Kconfig:101-116` |
| two screens | `YAKOGL_BOARD_RPI4_FB_VIRTUAL_HEIGHT` asks the firmware for a taller framebuffer (the cluster's double buffer) | `Kconfig:88-99`, `rpi4/board_rpi4.c:136-153` |
| window | with the knob, when the window fits: `phys`/`cpu` = the firmware framebuffer, `stride` = its pitch (5120 for 1280 wide); prints "the WxH window is a view into the framebuffer, pitch P (no copy)" | `rpi4/board_rpi4.c:228-250` |
| else | a static buffer in the image, packed rows | `rpi4/board_rpi4.c:251-271` |
| EGL | takes a window pitch wider than the packed pitch: whole pixels, even, at least `tex_pitch(w)`, at most 16384 | `src/gl/egl.rs:447-473` (`native_window_pitch`) |
| surface | maps the window's pages at a new GPU VA, no pool memory; stores the wide pitch per surface (0 = packed) | `src/gl/egl.rs:1279-1285`, `:1310` |
| show | a part of the view: pan to its row (`SET_VIRTUAL_OFFSET`), no copy; anything else: `board_show_copied()` and a row-by-row `memcpy` | `rpi4/board_rpi4.c:279-319` (copy at `:312-317`) |
| count | the first copy prints "board: board_show copies the WxH window ..."; `board_show_copies()` counts | `common/board.c:48-61` |
| APP bit 16 | `board_finish()` writes kind APP with `YK_RECORD_APP_SHOWN_BY_COPY` (0x10000) only when the run stored no result record of its own | `common/board.c:171-179`, `driver/rust/src/board/record.rs:92-96`, `:294` |
| Slint | hands `stride` to EGL as is; the check walks rows at `stride` | `driver/samples/zephyr/slint/src/main.c:172-214`, `:242` |
| cluster | the knob is on in `prj.conf`; the cluster writes its own APP record with `board_show_copies() != 0` | `driver/samples/zephyr/cluster/prj.conf:16-21`, `cluster/src/main.c:943`, `cluster/rust/src/board.rs:272-298` |

Note on bit 16: Slint stores PROBES as its result, so `board_finish()` writes
no APP record for Slint (`common/board.c:174`). For Slint, the proof of "no
copy" is the console: the "board: board_show copies" line is absent. Only the
cluster carries bit 16 in a record.

## 2. The AM62P display path today

| item | value | where |
|---|---|---|
| panel | 1024x600, 51.2 MHz pixel clock, totals 1349 x 645, so 58.8 Hz (17.0 ms a frame) | `driver/boards/toradex/verdin_am62p/verdin_am62p_am62p54_a53.dts:182-220` |
| route | DSS1 VP2 -> DSI -> SN65DSI83 -> LVDS; layer 0 = VID (the application), layer 1 = VIDL1 (safety, ARGB8888) | `driver/rust/src/soc/am62p/dss.rs:3-31`, `:185-222` |
| window memory | `s_win`, a static `__noinit` buffer, 4096-aligned, `CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB` (4800 KiB) large, in `zephyr,sram` (DDR 0x82000000 + 344 MiB), outside the 64 MiB GPU carve-out at 0x97800000 | `am62p/board_am62p.c:30`, dts `:55-78`, `Kconfig:51-60` |
| CPU view | a Normal non-cacheable alias, so no cache work | `am62p/board_am62p.c:47-54` |
| stride and format | `w * 4`, `GLES_NATIVE_FORMAT_XRGB8888`; no bottom-up flag (the DSS header has no `AM62P_DSS_PLANE_VFLIP`, so `PVR_SAMPLE_CAN_VFLIP` is 0) | `am62p/board_am62p.c:56-65`, `common/sample_glue.c:63-67`, `driver/include/safeui/am62p_dss.h:27-34` |
| first show | `am62p_dss_set_app_buffer` + enable the plane + enable the port | `common/sample_glue.c:85-118` |
| later shows | `am62p_dss_set_app_buffer` only: new BA_0/BA_1, PICTURE_SIZE, ROW_INC; GO; latched at the next vsync | `common/sample_glue.c:161-195`, `driver/rust/src/soc/am62p/dss.rs:245-261` |
| plane rules | width and height 1..4096; address not 0, below 2^48, a multiple of the pixel size; stride at least `w * bpp`; any stride (ROW_INC = 1 + stride - line) | `driver/rust/src/ip/ti_dss7.rs:277-294` |
| copy | none: `board_show_copied()` is never called | `am62p/board_am62p.c:69-87` |

Every window the three benchmarks ask for fits the plane as it is: Slint
1024x600 (`slint/src/main.c:45-46`), the cluster's two 1024x600 halves
(`cluster/boards/verdin_am62p_am62p54_a53.conf:15-19`), glmark2 800x600 (a
plane smaller than the screen, at (0,0)).

## 3. What the PowerVR window needs, and whether the DSS takes it

| need | PowerVR | DSS | match |
|---|---|---|---|
| layout | linear: the PBE writes `ROGUE_PBESTATE_MEMLAYOUT_LINEAR` unless the twiddle option is set for a power-of-two target | linear only | yes (`src/pvr/backend.rs:737-765`) |
| pitch | even pixels, at least `tex_pitch(w)`, at most 16384 (PBE linestride counts pixel pairs) | any pitch of at least `w * 4` | yes (`src/gl/egl.rs:447-473`) |
| format | XRGB8888 (PBE packmode U8U8U8U8) | `AM62P_DSS_FMT_XRGB8888` (0x27) | yes (`src/pvr/backend.rs:795-799`, `common/sample_glue.c:75`) |
| address | `pvr_render_map_phys` refuses a `phys` that is not 4 KiB aligned; maps any DDR page through the GPU MMU | 4-byte aligned, 48-bit bus address; the A53 maps DDR 1:1 | yes (`driver/src/pvr_render.c:120-136`; `dss.rs:106`) |
| contiguity | none needed (page by page through the GPU MMU) | one contiguous range | `s_win` is one static array: yes |
| pool | the window uses a fresh GPU VA, **no render-pool pages** | - | `src/gl/egl.rs:1279-1285`, `src/osal/zephyr.c:447-471` |

Board proof that the DSS shows what the PowerVR wrote, in place:
`tools/cts/RESULTS.md:591` (cluster, 1024x600, "two halves at 0x84d44000
and 0x84f9c000, stride 4096 (no view line: the DSS flips the packed halves)",
15/15 probes, APP `594b0209 00000701 ...` with bit 16 clear) and
`tools/cts/RESULTS.md:16803-16812` (Slint and the cluster at `17605c36`).
Mesa pvr and Linux tidss were not cloned: the tree and these records already
answer the question.

Why the AM62P's SO and SV rows are one image: no source file in the AM62P
build reads `CONFIG_YAKOGL_BOARD_SCANOUT_VIEW`, so both images have the same
sha256 (`docs/benchmarks-baseline/am62p.md:33-34`). The two rows are the same
copy-free run twice.

## 4. What is really open on the AM62P display path: tearing, not copies

These are correctness and smoothness gaps of the present path. None of them
is a copy.

1. **Slint draws into the buffer the DSS is scanning.** Slint has one window
   (`slint/src/main.c:172-214`). `eglSwapBuffers` submits frame N and waits
   only for frame N-1 (`src/gl/egl.rs:2110-2118`), so frame N+1 renders
   into the same buffer during scan-out. A frame can show part old, part
   new. The Pi 4 view has the same single buffer.
2. **The cluster may draw into a half that the DSS still scans.** The
   cluster submits N+1 into the other half right after `board_show(N)`. The
   flip only latches at the next vsync (`cluster/src/main.c:37-48` says so;
   `dss.rs:243-261`, `:326-335`). `plane_init` always sends `seq = 0`
   (`common/sample_glue.c:70-82`), so `am62p_dss_app_seq_shown()` cannot tell
   when a flip has latched.
3. **The swap interval is ignored.** `yakogl_zephyr_wait_vblank` is a weak
   function that returns -ENOTSUP, and no board defines it
   (`src/osal/zephyr.c:602-610`). So EGL's default interval 1
   (`src/gl/egl.rs:276`, `:2122-2134`) does not wait on any board. glmark2
   asks for interval 0 anyway (glmark2 `src/gl-state-egl.cpp:426-427` at
   `22c527cb`). Slint and the cluster never call `eglSwapInterval`, so a
   working vblank wait would limit them to 58.8 frames/s on this panel.

## 5. Change list for a later implementation item

### A. Required: make the docs say what the code does (no code)

| file | change |
|---|---|
| `docs/benchmarks.md:88` | replace "The board layer has no scan-out view, so the view row is the copy path again" with: "no copy on this board: board_show() flips the DSS plane to the window (yakogl_board.h:114-118); SO and SV are the same image" |
| `docs/benchmarks.md:90` (and the gaps list) | keep the Pi 4 numbers; add nothing for the AM62P |
| `tools/cts/common-tree.txt:9-13` | "slint: the window copied to the screen" -> "shown by a copy where the board copies (Pi 4, i.MX8MP); scanned in place on the AM62P"; "there the row is the copy path again" -> "there the row is the same copy-free image as SO" |
| `driver/zephyr/lib/board/Kconfig:101-116` (help) | add: "AM62P: no effect; its window is already scanned in place" |
| `tools/cts/STATUS.md:110` | "クラスタのブランチとビューの板レイヤーの統合" is done: the cluster is on the view (`cluster/prj.conf:21`) and in the common set (`common-tree.txt:28-29`) |
| `tools/cts/RESULTS.md:437`, `:448` | these are history: do not edit them; add one dated line under them, "on the AM62P the SV row is not a copy path: same sha256 as SO, board_show is a flip" |

The `bench-recipes` item owns HANDOFF, README, RESULTS and STATUS sync, so A
fits there.

### B. Recommended: a positive console line on the AM62P (small code)

Slint has no APP record, so today the only "no copy" proof for Slint is a
missing line. Add one line that is there:

- `driver/zephyr/lib/board/am62p/board_am62p.c`, `board_window()`: after
  `s_taken = true`, print once
  `"board: the %ux%u window is scanned in place by the DSS, stride %u (no copy)\n"`.
  Do not reuse the Pi 4 text "is a view into the framebuffer": a tally that
  greps it would then count the AM62P as a view.
- `tools/cts/common-tally.py`: optional, read the line as "in place" for
  am62p.
- No Kconfig change. No change to `src/` and no change to the driver.

### C. Optional, its own item: no tearing on the AM62P

It needs a decision first, because it changes the frame counts.

1. `common/sample_glue.c` `plane_init()` / `flip_app_buffer()`: take a
   `seq` argument and set `am62p_dss_plane_cfg.seq` (today always 0).
2. `am62p/board_am62p.c` `board_show()`: keep a counter. When the address
   changes (a flip), return only after `am62p_dss_app_seq_shown(dss) == seq`.
   Wait with `am62p_dss_wait_vsync(dss, K_MSEC(20))`, at most 3 times. On a
   timeout return -EIO (`yakogl_board.h:155-160` allows it). A re-commit of
   the same address (Slint) does not wait.
   Cost: the cluster becomes vsync-paced, at most 58.8 fps against 91.8 now.
   A third buffer (3 x 2400 KiB = 7200 KiB:
   `CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB=7200` in
   `cluster/boards/verdin_am62p_am62p54_a53.conf`, plus a third surface in
   `cluster/src/main.c`) removes the wait. That changes the shared sample, so
   it needs a Pi 4 and an i.MX8MP run too.
3. Slint: no tearing needs two buffers in `slint/src/main.c`, as the cluster
   does. 1024 x (2 x 600 + 8 guard rows) x 4 = 4,947,968 B, more than
   4800 KiB, so set `CONFIG_YAKOGL_BOARD_WINDOW_MAX_KB=4864` in
   `slint/boards/verdin_am62p_am62p54_a53.conf`. The second half starts at
   600 x 4096 = 0x258000 bytes, a whole number of pages, as
   `pvr_render_map_phys` needs (`driver/src/pvr_render.c:127`). This is a
   change to the common sample on all three boards.
4. Do **not** define `yakogl_zephyr_wait_vblank` on the AM62P only for this.
   It would limit Slint and the cluster to 58.8 Hz (they do not set the
   interval), and it would make the AM62P numbers incomparable to the other
   boards, where no vblank wait exists.

### Risks

- **Tearing:** as in section 4. The present path already tears, and so does
  the Pi 4 Slint view. B does not change it; C does.
- **Buffer count:** the cluster uses 2 halves, which is the whole 4800 KiB
  (`cluster/boards/verdin_am62p_am62p54_a53.conf:15-17`). A third half needs
  7200 KiB in `zephyr,sram`, which has 344 MiB (dts `:62-64`).
- **Pool against the refract fix:** none. The window is in `zephyr,sram`, not
  in the 64 MiB carve-out, and EGL maps it with no pool pages
  (`src/gl/egl.rs:1279-1285`). A bigger `s_win` does not touch the render
  pool, the large-block path of `541c9d3b`, or refract's 7.6 MB depth buffer.
  Only the window's depth/stencil buffer (Slint and the cluster ask for
  stencil 8) is in the pool, and it is the same size with or without B or C.
- **GPU VA:** each window surface takes new VA from the bump in
  `pvr_render_map_phys` and does not give it back (`pvr_render.c:129-131`;
  `src/gl/egl.rs:5209` keeps the VA range on destroy). That is fine for 2 or 3
  surfaces made once.
- **Alignment:** every surface `phys` must be 4 KiB aligned
  (`pvr_render.c:127`); the DSS needs only 4 bytes.

## 6. Board recipe (AM62P)

This needs the board, `ZEPHYR_WS`, `WEST_VENV`, `SLINT_NOSTD_DIR`,
`NOSTD_PATCH_DIR`, `AGL_SLINT_CLUSTER_DIR`, `NOTO_SANS_DIR`, and
`driver/src/fw/pvr_fw_blob.h` (`tools/cts/common-images.sh:47-59`), and
`BOARD_HOST` for `tools/board.sh` (`tools/board.sh:129`). It was
not run by this item.

Build the three rows from the tree under test:

    tools/cts/common-images.sh am62p tools/cts/common-tree.txt <img> SO SV CL

Host check before any boot: `grep sha256= <img>/SO.built <img>/SV.built`
gives the same hash on the AM62P (the knob is a no-op here). If they differ,
something now reads the knob on the AM62P: stop and find it.

Run (the default boot is `uboot-tftp`; the recent records used `uboot-mmc`):

    BOARD_BOOT=uboot-mmc tools/board.sh am62p run <img>/SV/zephyr/zephyr.bin <rec> slint-view
    BOARD_BOOT=uboot-mmc tools/board.sh am62p run <img>/SO/zephyr/zephyr.bin <rec> slint
    BOARD_BOOT=uboot-mmc tools/board.sh am62p run <img>/CL/zephyr/zephyr.bin <rec> cluster
    tools/board.sh am62p tally <rec>

or the whole set: `tools/cts/cts-common.sh am62p run <img> <outdir>`.

Expected (the latest AM62P records, `tools/cts/RESULTS.md:16803-16812`,
tree `17605c36`):

| row | verdict line | PROBES words | frames | no-copy proof |
|---|---|---|---|---|
| SV slint-view | `rc=0 verdict=done ... result=record:PROBES resets=0` | `594b0601,08420862,00210842,c5920883` | `slint: animation: ~700 frames in ~10000 ms` (703 at `17605c36`; 715 and 741 in older trees, RESULTS.md:437, :448). Not 1500: the AM62P is GPU-bound, not copy-bound | console has `slint: window 1024x600 at phys 0x..., stride 4096, ...` and **no** `board: board_show copies` line (count 0). No APP record (PROBES is the result). With B: also the "scanned in place" line once |
| SO slint | the same as SV | the same as SV | the same as SV, within run-to-run noise | the same as SV |
| CL cluster | `rc=0 verdict=done ... result=record:PROBES resets=0` | `594b0601,008510c4,19050085,110c6d5f` | `cluster: throughput: 1024x600: ~5500 frames in ~60000 ms, ~92 fps` (5509 / 60014 ms, 91.8 fps, 56.4 Mpx/s), so about 917 frames per 10 s | the APP record in INDEX.txt: `594b02xx 00000701 <frames> <ms>`, decoded `APP (2) ... stage 7 status 1 (pass)` with **no** "shown by copy": bit 16 (0x10000) of w1 is clear. Console: two halves at `stride 4096`, no `board: board_show copies` line |

If the PROBES words differ from these but match the older AM62P words
(`594b0601 08420882 00210842 98080883`, RESULTS.md:437), the picture changed
with the tree, not with the display path. Compare the FINGERPRINT record
before you call it a display fault. With C landed, the cluster expects at
most 58.8 fps (about 588 frames per 10 s) with two halves, or about 92 fps
again with three.
