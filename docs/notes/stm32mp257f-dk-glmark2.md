<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK: glmark2-es2 on the HDMI panel (design and blockers, 2026-11-21)

Status: **design, written on the host from the tree at 226cd9a2. Nothing here
has run on the board.** It is roadmap item 3 after the U-Boot boot plan
(#357): glmark2-es2 on the DK's HDMI panel, started from ST's U-Boot by the
one-shot of `stm32mp257f-dk-uboot-boot.md`. Issues #7 and #1281.

The headline: **0xb0000000 is claimed twice.** The glmark2 sample's DK
overlay puts a 240 MiB GPU carve-out there; every display overlay puts the
32 MiB LTDC frame buffer `ltdc_fb` there. Two images that combine both
(which glmark2 on the panel must) would overlap by 32 MiB. Section 2 has the
layout that removes it.

Citations are `file:line` of the tree at 226cd9a2.

## 1. The memory map above 0xa0000000, region by region

Region = a range somebody claims or that is fixed by the board. "Claimed by"
is the file that says so. The go window is 0xa0000000..0xb0000000
(`docs/notes/stm32mp257f-dk-uboot-boot.md:272`, `LOAD_END=0xb0000000` in
`tools/board-stm32mp2.sh:297`; the runner rejects an image that does not fit,
`:750-756`).

| range | size | what | claimed by (file:line) |
| --- | --- | --- | --- |
| 0xa0000000 - 0xafffffff | 256 MiB | the image: `zephyr,sram`, link and load address, its 96 MiB heap inside it | board DTS `driver/boards/st/stm32mp257f_dk/stm32mp257f_dk_stm32mp257fxx_a35.dts:22`, `:40`, `:50-52`; heap `driver/samples/zephyr/glmark2/prj.conf:42` |
| 0xb0000000 - 0xb1ffffff | 32 MiB | `ltdc_fb`, no-map, the LTDC's frame buffer (`yakogl,ltdc-fb`) | `tools/probes/stm32mp257f_dk/mp2_display/boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay:41-44` (chosen `:37`); the same in `.../mp2_gpu_scanout/boards/...overlay:49-52`; and `driver/samples/zephyr/gl_test/boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay:27-30` (chosen `:23`) |
| **0xb0000000 - 0xbeffffff** | 240 MiB | `gpu_carveout_glmark2`, no-map, the GPU's whole pool for glmark2 (`&gpu` memory-region) | `driver/samples/zephyr/glmark2/boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay:26-28`, `:33-35` |
| 0xb2000000 - 0xfa7fffff | about 1.2 GiB | nothing declared (System RAM, RISAF4 region 9 non-secure, CID0/1 read/write) | board DTS `:44-46` (RAM 0x84000000 + 0x76800000); RISAF4 row in `docs/notes/stm32mp257f-dk-display-bringup.md:606`, `:662` |
| 0xfa800000 - 0xfe7fffff | 64 MiB | `gpu_reserved`, no-map, the board's GPU carve-out (what gc_fence, gc_clear and gl_test use; `app.overlay` also names it) | board DTS `:64-67`, `:76-78`; `driver/zephyr/lib/board/stm32mp2/app.overlay:6-8`; RISAF4 region 10, display-bringup `:607` |
| 0xfe800000 - 0xfeffffff | 8 MiB | LTDC secure layer, RISAF4 region 13, not used | `docs/notes/stm32mp257f-dk-display-bringup.md:608`, `:664`; `docs/notes/stm32mp257f-dk-display.md:388` |
| 0xff000000 - 0xffffffff | 16 MiB | LTDC rotation buffer, region 14, not used | display-bringup `:609`; display.md `:389` |

Other claims the layout has to respect:

- **U-Boot and the secure side** keep nothing in the go window; U-Boot loads
  below 0x90500000 and relocates to the top of usable RAM (board DTS `:13-24`;
  boot note section 3, `:294-299`, `:321`). Nothing of ST's chain uses 0xb0000000
  up to 0xfa7fffff, which is the base of the proposal below.
- **The runner's `LOAD_END`** limits the *image file*, not the carve-out:
  the carve-out is a run-time mapping, not part of the loaded file.
- **Virtual space.** The glmark2 conf raises `CONFIG_KERNEL_VM_SIZE` to
  0x20000000 because the image, its heap and the carve-out mapping
  (`gc_render.c`, `k_mem_map_phys_bare`) no longer fit the default 256 MiB
  (`driver/samples/zephyr/glmark2/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf:50-54`).
  A 32 MiB `ltdc_fb` mapping comes on top (below).
- **GPU bus-master access.** RISAF4 region 10 (0xfa800000) is the GPU's
  known-open memory; the glmark2 overlay itself says region 9 at 0xb0000000
  "is not known to be open to the GPU's bus master: the first run on the DK
  confirms it" (`...glmark2/boards/...overlay:18-21`). The LTDC master's RIF
  entry is open question 2 of display.md (`:384-398`).

## 2. The double claim and a layout without it

**The overlap.** glmark2's carve-out is 0xb0000000 + 0x0f000000, that is
0xb0000000-0xbeffffff (`glmark2 overlay:27`). `ltdc_fb` is 0xb0000000-0xb1ffffff
(`gl_test overlay:28`, probes' overlays above). The first 32 MiB are in both.
Today the two never meet in one build, because the glmark2 overlay includes
only `app.overlay` (`:6`: the GPU node) and no display node, and
`board_window()` still refuses on the DK
(`driver/zephyr/lib/board/stm32mp2/board_stm32mp2.c:26-32`). They meet when
glmark2 gets the display chain, which is exactly this item. Devicetree does
not reject overlapping `reserved-memory` children; the failure would be silent:
the GPU's pool allocator would hand the scan-out memory out as a vertex
buffer (or the LTDC would scan out a texture).

(Also: the file comment in `gl_test`'s DK overlay says its window "scans out
of ltdc_fb" (`:10-11`), but `board_window()` refuses at this tip
(`board_stm32mp2.c:7-8`, `:26`). The window is the display plan #353's to
land; the comment is ahead of the code.)

**Proposed layout** (the glmark2 image with the display chain):

| range | size | what |
| --- | --- | --- |
| 0xa0000000 - 0xafffffff | 256 MiB | image, unchanged |
| 0xb0000000 - 0xb1ffffff | 32 MiB | `ltdc_fb`, unchanged (the value every display overlay and the bring-up note already use) |
| **0xb2000000 - 0xbfffffff** | **224 MiB** | `gpu_carveout_glmark2` moved up by 32 MiB and cut to 0x0e000000 |

- 0xb2000000 is 4 MiB aligned (the GC MMU's need, `glmark2 overlay:21-23`).
- 224 MiB is 56 of the 64 slave tables (4 MiB each; `gc_internal.h`
  `GC_STLB_COUNT`, as the overlay cites): within the table limit.
- 224 MiB is 16 MiB under the 240 MiB the overlay chose for the 51 MB terrain
  peak and the "frees held until the kick completes" policy (`overlay:8-13`).
  The peak is a host figure, 51 MB, so 224 MiB keeps about 4 times the margin; if
  a scene still runs the pool out, the next step is the large free area
  0xc0000000-0xfa7fffff (1 GiB, section 1), not the gpu_reserved window.
- The end, 0xc0000000, keeps the whole thing below 3 GiB and 4 GiB, as the
  GC MMU needs (`overlay:21`).
- `CONFIG_KERNEL_VM_SIZE` needs a recount: the mapping now holds 224 MiB of
  carve-out and 32 MiB of `ltdc_fb` next to the image, 256 + 256 = 0x20000000
  exactly. Treat 0x20000000 as the floor and take 0x28000000 unless a build
  proves it enough (unverified, not measured).
- Alternative that needs no `ltdc_fb` at all: render into the carve-out and
  point `L1CFBAR` at the render target there, as display-bringup proposes for
  the GPU frame (`display-bringup.md:482-` "Frame buffer placement" item 2,
  and `mp2_gpu_scanout`'s header `:13-14`). glmark2 allocates its window
  through `board_window()` (`main.cpp:1525-1553`), so this is a choice for that
  function and is a follow-up (below), not a prerequisite. The layout above is
  correct either way, since it only removes the overlap.

An additional rule for whoever edits the overlays: **a single check**, a
build-time `BUILD_ASSERT` or a devicetree-walking test over `reserved-memory`
nodes that fails when two `reg` ranges overlap, would have caught this one and
the next. It is not in the tree today.

## 3. How glmark2's scenes fit the run

**The run is short.** The layer resets the board `CONFIG_YAKOGL_STM32MP2_LIFETIME_S`
seconds after boot, default 20 (`driver/zephyr/lib/board/stm32mp2/Kconfig:19-23`);
OP-TEE's watchdog, armed by the image, bounds a hang at about 32 s
(boot note section 9, `:555-565`, `:625-632`). The glmark2 conf sets 900 s
(`...glmark2/boards/...conf:49`) and says the runner's `RUN_BOUND` of 120 s
has to be raised; both were written for a run the image is allowed to hold.

**The default list does not fit:** 33 scenes of 10 s each
(`driver/samples/zephyr/glmark2/Kconfig:23-25`, `prj.conf:44-47`), about 6-7
minutes, against a 20 s (and a hard 32 s) life. The U-Boot one-shot is one
`go` per boot, and the board returns to Linux after it; there is no run that
holds a minute without raising the lifetime above the watchdog's period, which
the image's own feed does not cover at all (`prj.conf:46-48`: "the watchdog is
fed until 15 minutes after boot at most", for boards with a fed one; the DK's
is OP-TEE's and the lifetime reset is the bound).

So the choices are:

1. **One scene per run (proposed for the first run and for the sweep).**
   Build with `CONFIG_GLMARK2_ARGS="-b <scene>"` (`Kconfig:23-31`), one scene
   per image, or pick the scene at run time (below). Set the scene's duration
   so that boot, the shader compiles (about 0.2 s each on an A53, `prj.conf:44-46`),
   the scene and the HDMI hold fit the lifetime: with the default 20 s, a
   scene of about 8 s leaves the compile and the hold; with a lifetime of 28 s the
   scene can keep its 10 s. Do not set the lifetime above about 28 s, the
   watchdog being 32 s. glmark2's per-scene `duration` option is upstream's
   (`-b name:duration=N`); no use of it is in the tree yet, so the exact syntax
   is a check for the first build.
2. **A subset in one run** (for example `terrain` and the cheap shading
   scenes at 3 s each): possible inside 28 s, but the first scene's compile
   cost and the cold display bring-up make it the second step, not the first.
   Do it only after the one-scene run is proven.
3. **All 33, one per run: 33 boots**, each about as long as the one-shot's
   cycle (boot note section 6, `:442-`, which includes the return to Linux).
   That is the route to the full list; the sweep is a loop in the runner, not
   a longer image.

**Picking the scene per run.** The i.MX8MP's pages use a record selector
(`main.cpp:78-90`, `board_selector()`), which returns -1 on the DK
(`board_stm32mp2.c:44-47`), so a DK image runs the whole default list from
the first scene on and is cut by the lifetime. Two ways to give it a scene
without 33 builds: let `board_selector()` return a value that the one-shot
puts where the image can read it (the boot note says `go` passes `x0 = 1` and
`x1` an argv-like array in U-Boot's memory, `:230-233`, and that `x2`, `x3`
are "whatever U-Boot left"; so passing an argument needs a header word of the
image, not a register); or build one image per scene with `GLMARK2_ARGS`. The
second needs nothing new and is the proposal; the first is a follow-up.

**What a score means.** The 10 s scene is glmark2's measurement window;
shortening it changes the score's noise, not its meaning, and a shortened
run is not comparable with the i.MX8MP's published rows. The DK's first
result is pictures and frame counts per scene (`glmark2-frame:` lines and the
pictures-by-eye procedure of `imx8mp-glmark2-pictures-by-eye.md`), not a score.

## 4. The display path it scans out through

HDMI on the DK is LTDC -> DSI -> ADV7535 -> HDMI, four DSI lanes, not LVDS
(`docs/notes/stm32mp257f-dk-display.md:47`). The yakogl devices are
`yakogl,stm32-ltdc` (0x48010000), `yakogl,stm32-dsi` (0x48000000) and
`yakogl,adv7535` on `&i2c2` (0x40130000)
(`mp2_display` overlay `:47-92`). The mode is the largest the driver may choose:
1920x1080@60 when the EDID prefers it, else 1280x720@60; 1080p60 is the
largest the bridge accepts (display.md `:262-267`; overlay `:79-81`). One frame
of 1080p XRGB8888 is pitch 7680, 8 294 400 B (display-bringup `:675-677`);
`fb-size` 0x800000 in the overlay is 8 MiB (`mp2_display overlay:88`), which holds one
1080p frame with 94 208 B to spare (display-bringup `:677`); the 32 MiB `ltdc_fb`
holds four such steps.

The glmark2 side:

- `NativeStateBoard` makes the window from `board_window(W, H)`
  (`main.cpp:512-545`, `:1525-1553`) with `W x H` from `CONFIG_GLMARK2_SIZE`,
  default 800x600 (`Kconfig:9-16`). **800x600 is not a mode of this panel
  path.** Either the display plan lands a window smaller than the mode
  (the LTDC layer window and a black border, centred) or the DK's glmark2
  conf sets `CONFIG_GLMARK2_SIZE` to the mode (1280x720). The first is the one
  that keeps results comparable with the other boards; it is the display
  plan's decision, recorded here as open.
- The GPU renders to the window as the board gives it
  (`gles_native_window`, `main.cpp:600`); `board_show()` puts it on the
  screen after each swap (`:562-590`). On the DK it is `-ENOTSUP` today
  (`board_stm32mp2.c:34-38`).
- The scan-out buffer's home is the open question of
  display-bringup `:802`: render in the carve-out and point the LTDC at it
  (no copy; the GPU's output format is `X8R8G8B8`, display-bringup
  `:508-`), or copy into `ltdc_fb`. Cache rules (non-cacheable mapping of
  both) are display-bringup's, `:696-`.
- Open until a board run: whether the LTDC master reaches RISAF4 region 9 and
  10 (display.md `:384-398`), and whether the GPU master reaches region 9
  (`glmark2 overlay:18-21`).

## 5. Follow-ups, in order, and whose file each is

Every file below is another plan's at this date. This note edits none.

| # | what | files | owner |
| --- | --- | --- | --- |
| 1 | **Fix the double claim**: move `gpu_carveout_glmark2` to 0xb2000000 / 0x0e000000 and recount `CONFIG_KERNEL_VM_SIZE`; include the display overlay in the glmark2 DK overlay (it would otherwise need the `app.overlay` + display nodes of `gl_test`'s overlay) | `driver/samples/zephyr/glmark2/boards/stm32mp257f_dk_stm32mp257fxx_a35.{overlay,conf}` | #349 (`driver/samples/zephyr/glmark2/**`) |
| 2 | One shared display overlay for gl_test, gc_triangle and glmark2 instead of three copies of the same nodes (the `ltdc_fb` and display nodes are copied in the two probes' overlays and in `gl_test`'s) | `driver/zephyr/lib/board/stm32mp2/**` (a shared overlay beside `app.overlay`), the three consumers | #353 for the display nodes, #349 for the samples |
| 3 | A check that fails on overlapping `reserved-memory` nodes (build-time or a host test) | `driver/zephyr/lib/board/stm32mp2/**` or a test | #353 / #349, a new file |
| 4 | `board_window()` / `board_show()` over the LTDC (the window size decision of section 4, 800x600 against the mode) | `driver/zephyr/lib/board/stm32mp2/board_stm32mp2.c`, `driver/drivers/display/**` | #353 (`driver/zephyr/lib/board/stm32mp2/**`, `driver/drivers/{display,...}/**`) |
| 5 | The GPU-master and LTDC-master RISAF reads for 0xb0000000..0xc0000000 (the `md.l`-style check the display probe has, `display-bringup.md:729`) | `tools/probes/stm32mp257f_dk/mp2_display*/**` | #353 |
| 6 | glmark2 DK conf for the U-Boot run: lifetime 28 s or less, `GLMARK2_ARGS` one scene with a duration, `RUN_BOUND` | `.../glmark2/boards/*.conf`, `tools/board-stm32mp2.sh`, `tools/board.sh` | #349 (conf and runner) |
| 7 | The first one-scene run from ST's U-Boot, after #349's triangle and #353's window are proven; then the 33-boot sweep and the scene-per-run selector (section 3) | board session; `tools/board.sh`, `tools/cts/RESULTS.md` | #349 and the board plans; RESULTS.md is #336's |

The `docs/README.md` index line waits for #327.
