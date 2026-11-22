<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5: what P13 (dEQP-GLES2 and glmark2) needs

Status: plan, read on the tip `24995f2e` (2026-11-22). Nothing here was run
on a board, and no code is changed by this note. It is item P13 of
`docs/notes/pre-study-rpi5.md` (section 8, row at `:619`: "dEQP-GLES2 and
glmark2 on the board", size L, depends on P8 to P10, done when "the Pi 4's
counts, or the refusals named"). It reads how the Pi 4 does the same two
things, what the Pi 5 images lack, what the 7.1 table will have after
P9/P10 (`docs/notes/v3d71-gl-table.md`), and ends with an ordered item list.
Every statement about the tree carries `file:line`; a figure that is only
expected, not measured, says so.

Read first: `docs/notes/v3d71-gl-table.md` (P9 figures, section 4 build
selection), `docs/notes/rpi5-display.md` (P11), `docs/notes/pi4-v3d-bringup.md`
("gl_test on the V3D", `:322`), `docs/online-shaderc/v3d-backend.md` "M8"
(`:3366`) and `driver/docs/glmark2.md`.

## 1. How the Pi 4 does it

### 1.1 The stage and the runner

`tools/cts/board-stage.sh <board> common|deqp` is the one entry the farm
calls (header usage `:10-17`, `usage()` `:175`). The Pi 4 is `pi4)
layout=rpi_4b key=RPI4` (`:182`): the layout names the host dependencies
(`needed_deps`, `:138-150`: the west workspace and SDK for every image, the
glmark2 source `GLMARK2_SRC` for `common` `:141`, the board's stage-1 tree
`DEQP_WORK` for `deqp`). The board comes from `BOARD_HOST`/`RESOURCE_RPI4_SSH`
and `BOARD_UART`/`RESOURCE_RPI4_CONSOLE_*` (`:285-296`; the Pi 4 also needs
`BOARD_HDMI_TIMINGS`, `:298-300`, and boots by tryboot, so it has no stale
one-shot images to clear, `:301`).

| Set | What the stage does | Where |
|---|---|---|
| `common` | `cts-common.sh pi4 build` (two images, labels GM glmark2 and GT gl_test, `tools/cts/common-tree.txt:14-17`) then `run` | `board-stage.sh:363-364`; `tools/cts/cts-common.sh:56-79`; images by `tools/cts/common-images.sh` (board table `:86-100`: `pi4) ZBOARD=rpi_4b`, no watchdog arm, marker `board: PM watchdog resets`; one `west build` per row `:212`) |
| `deqp` | `pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-<sha8>` builds 29 images, then `board.sh pi4 tree` runs them | `board-stage.sh:403-405`; `tools/cts/pi4-deqp-images.sh:11` (one line: `deqp-tree-images.sh rpi_4b`); the builder `tools/cts/deqp-tree-images.sh:142-144` (cmake + ninja of `driver/samples/zephyr/deqp` with `-DDEQP_SRC`/`-DDEQP_BUILD` from the stage 1 and one `-DCONFIG_DEQP_CASE=<filter>` per image) |

The table `tools/cts/pi4-full-tree.txt` has 29 `# image <LABEL>:` blocks
(`:15-71`), each case of the 17,165 mustpass names of
`tools/cts/gles2-mustpass.txt` in exactly one block (header `:7-9`). The one
image that needs more memory carries the marker `(with texture-2048.conf)`
(`:61`), which `deqp-tree-images.sh:85,102` turns into
`-DEXTRA_CONF_FILE=texture-2048.conf` (`driver/samples/zephyr/deqp/texture-2048.conf:15-17`:
64 MiB carve-out, 24 MiB of tiled copies). After each build the resolved
`.config` is checked to still be a conformance image
(`deqp-tree-images.sh:148-150`, `tools/cts/check-deqp-images.py`).

The runner is `tools/board.sh <board> tree <table> <outdir>`
(`TREE_IMAGE_DIR=<dir>` makes `<dir>/<label>/zephyr/zephyr.bin` the image of
a row, `tools/board.sh:166`; `tree_rows` `:2670`; `cmd_tree` is dispatched at
`:3066`). `board.sh pi5` already exists for the probes: tryboot, the BCM2712
PM watchdog as the way back to Linux (`:320-343`), a 256 MiB load window
(`YAKO_ADDR=0x200000; LOAD_END=0x10200000`, `:331`), a run bound of 420 s
and a return bound of 420 s (`:238`, `:357`), and an end-line regex that
**already contains the dEQP end lines** `deqp: summary: ` and
`v3d_deqp: build <sha> (DONE|INCOMPLETE|STOPPED)` and gl_test's
`v3d_gl_test RECORD end` (`:343`). So a Pi 5 `tree` run needs no new end
rule; it needs images that print those lines on the UART (section 3.2).

### 1.2 The images and their memory

| Image | Kconfig/board files on the Pi 4 | Figures |
|---|---|---|
| dEQP (`driver/samples/zephyr/deqp`) | `boards/rpi_4b.conf`, `boards/rpi_4b.overlay` | `CONFIG_YAKOGL_GPU_V3D=y` `:8`; carve-out 48 MiB `:9`; tiled copies 16 MiB `:14` (a 512x512 cube chain is 16 KiB over the 8 MiB default `:10-13`); compiler scratch 8 MiB `:23` (S16 and UA1 of the GLES3 tree need it; S16 linked at 166,292 KB of RAM, 63.44% of the window `:20-22`); `CONFIG_FPU_SHARING` `:28`, `MAX_XLAT_TABLES=128`, `KERNEL_VM_SIZE=0x20000000` `:32-33`; `sram0` is 256 MiB from 0x200000 (`rpi_4b.overlay:13-15`; the overlay comment `:3-9`: carve-out, scratch, dEQP code with 1.1 MB of test data and a 64 MiB C heap, `deqp/prj.conf:19`) |
| glmark2 (`driver/samples/zephyr/glmark2`) | `boards/rpi_4b.conf`, `boards/rpi_4b.overlay` | carve-out 96 MiB `:12` (refract peaks near 56 MB, terrain needs over 30.5 MB `:6-11`); tiled copies 24 MiB `:21`; `CONFIG_YAKOGL_V3D_TFU=y` `:31` (terrain's and refract's `glGenerateMipmap` is one TFU job per mip chain); `MAX_XLAT_TABLES=192` `:38`; `sram0` 288 MiB from 0x200000, to 0x12200000 (`rpi_4b.overlay:16`); image 12,325,040 B, 194,192 KiB RAM at the 48 MiB carve-out, the data files 9,420,183 B (`driver/docs/glmark2.md:244,254`) |
| gl_test (`driver/samples/zephyr/gl_test`) | `boards/rpi_4b.conf`, `.overlay` | carve-out 48 MiB `:7`; 96 MiB window (`.overlay:14`); the Pi 4 `main_rpi4.c` |

Both images also take the board layer: `CONFIG_YAKOGL_BOARD=y` in glmark2's
`prj.conf:6` (window, screen, record, watchdog, end line;
`driver/zephyr/lib/board/Kconfig:74-80`) with `CONFIG_YAKOGL_BOARD_LIFETIME_S=900`
and `_HOLD_S=20` (`prj.conf:48-49`). The dEQP image has its own board file,
`driver/samples/zephyr/deqp/src/v3d_board.c`: mailbox, HDMI console, the EGL
default display with the V3D clock switch (`:10-14`), the result record
(`:15-27`), the watchdog (`:1169`) and a bounded lifetime: the PM watchdog is
fed for `CONFIG_V3D_DEQP_LIFETIME_S` (default 240) and the record is printed
`V3D_DEQP_PHOTO_S` (30) s before it (`deqp/Kconfig:175-190`). The CMake
branch is `deqp/CMakeLists.txt:95-131` (sources `v3d_board.c`, `rpi_mbox.c`,
`fbcon.c`, `rpi_wdog.c`; `--wrap=eglGetDisplay` and the info-log taps
`:200-209`); `CMakeLists.txt:38` skips the common sample glue for V3D and Mali.

### 1.3 What the Pi 4 measured (the bar)

- dEQP-GLES2: 29 rows, **17,126 pass, 0 fail, 34 NotSupported, 3 quality / 2
  compatibility warnings, 0 resets**, at `46f93767`, 2026-10-24
  (`tools/cts/RESULTS.md:34`). The 34 NotSupported are the same on three
  boards and accepted (`RESULTS.md:36`).
- glmark2-es2: **195, 33 scenes, 0 failed, 0 GL errors**, main `951fd612`,
  2026-11-21; gl_test 13 of 13 (`RESULTS.md:85`). Per-scene ms, terrain 161.8,
  shadow 7.3, refract 17.3, sum 347.5 ms (`RESULTS.md:226-261`).
- Stage 5's bar for the Pi 5 is the roadmap's: "the Pi 4's counts, or the
  refusals named" (`pre-study-rpi5.md:619`); P12 is the gate rows
  (`:618`), P11 the display (`:617`). Not a performance bar.

## 2. What the Pi 5 has today

| Piece | State | Citation |
|---|---|---|
| Zephyr option | `CONFIG_YAKOGL_GPU_V3D71` exists, selects the PM watchdog and the mailbox; the TFU "is not driven yet (refused by name)"; binning is polled | `zephyr/Kconfig:98-114`; `zephyr/CMakeLists.txt:98-111`; `src/osal/zephyr_v3d71.c:38-52` |
| OSAL | VA space 512 MiB, carve-out is a `__noinit` static of `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES`; init refuses unless the PM watchdog is armed; `osal_zephyr_v3d_default_platform()` answers the clock switch itself (`rpi5_v3d.c`), so a caller can pass `EGL_DEFAULT_DISPLAY` | `src/osal/zephyr_v3d71.c:97-110,192-196`; `driver/zephyr/lib/board/rpi5/rpi5_v3d.c:3-12` |
| Cargo feature, `yakogl_backend_v3d71`, FFI | **not in the tree yet** (`gpu-v3d71` absent; `backend71` ready, `fill71` in review). `ffi71.rs` and the pure modules exist, none in `src/v3d/mod.rs` | `docs/notes/v3d71-gl-table.md:30-36,85-120` |
| Pure modules | `admit71`, `attr71`, `clearplan71`, `draw71`, `format71`, `frame71`, `ident71`, `limits71`, `prim71`, `record71`, `rt71`, `sampler71`, `texfill71`, `texstate71`, `texture71`, `tfuplan71`, `uniform71`, `zskeep71` | `ls src/v3d/*71*.rs` |
| C | `v3d_job71.{c,h}` (29 of Mesa's `gl_*.clif` jobs equal, 35 named differences), `v3d_tex71.*`, `v3d_tfu71.*`, `v3d_frame71_tri.c` | `v3d71-gl-table.md:181-191`; `driver/src/v3d/*71*` |
| Compiler | 7.1 online back end: es100 126 of 126 compile; `qpu_corpus_71` 873 of 1090 (the 217 refusals are desktop-GLSL sources or named); points and lines variants; `link71` (GL link entry) is in review | `docs/notes/v3d71-backend.md:931-970,801-824` |
| Uniform stream | `uniform71.rs` fills `CONSTANT`, `UNIFORM`, the four viewport words and **names every other contents a refusal** (no texture words yet: `texuni71`) | `src/v3d/uniform71.rs:6-22` |
| Board facts | triangle PASS (Mesa's shaders and ours), TFU mip-chain job PASS with `IOC` although `HUB_IDENT1` bit 17 reads 0 | `tools/cts/RESULTS.md:710-731`; `v3d71-gl-table.md:229-237` |
| Console and screen | console is `uart10` at 115200 (`pre-study-rpi5.md:77`); `CONFIG_YAKOGL_BOARD_RPI5` gives a 32 bpp mailbox framebuffer, untried on the board, with no GPU platform, record screen or watchdog | `driver/zephyr/lib/board/Kconfig:35-49`; `rpi5/rpi5_screen.h:10-11`; `rpi5-display.md:46-134` (the firmware's plane is RGB565; own 32 bpp plane is P11 stage 2, `rpi5_scanout32`) |
| Probes' memory | every `rpi_5` image is 32 MiB of `sram0` from 0x200000 (low, below 4 GiB, U10) | `tools/probes/rpi_5/rpi5_triangle/boards/rpi_5.overlay:14-17` |
| Stage and tools | no `pi5` in `board-stage.sh` (`usage` `:175`), no `rpi_5` in `deps.toml`'s stage-1 board case (`:331-336`), `deps-env.sh` lists four boards (`:21`), `check.sh` has no `rpi_5` row (`rpi4 gl_test image` is `:4632`) | read |

## 3. The images to build

### 3.1 Which images

1. **gl_test** (`GT`) for `rpi_5` — the P9 item `gltest71`, not P13's; it is
   the first thing the `common` stage runs and the 7.1 table's first board
   proof. P13 only needs it in `common-tree.txt`'s row (the row's
   `gl_test` line builds `driver/samples/zephyr/gl_test` with the board's
   files, `common-tree.txt:9-11`).
2. **glmark2** (`GM`) for `rpi_5`: `boards/rpi_5.conf` and `.overlay` in
   `driver/samples/zephyr/glmark2/boards/`.
3. **dEQP-GLES2**: `boards/rpi_5.conf` and `.overlay` in
   `driver/samples/zephyr/deqp/boards/`, one `tools/cts/pi5-full-tree.txt`
   (the Pi 4's 29 images, then re-cut where section 6 says), and
   `texture-2048.conf` as it is for T5.
4. A stage-1 tree for `rpi_5` (VK-GL-CTS at the pinned commit, about
   1.9 GB, built in place: `tools/cts/deps.toml:321-352`): the
   `cts-build-zephyr.sh` takes its compile flags from the deqp sample
   configured for the board (`tools/cts/cts-build-zephyr.sh:80-104`), so the
   sample must **configure** for `rpi_5` first (item D1 below); the A76's
   `-mcpu` then comes by itself.

### 3.2 What differs from the Pi 4 files (and why a new file, not an edit)

- **No `v3d_board.c` reuse as is.** The Pi 4 board file allocates the
  mailbox framebuffer and, **if that fails, returns before installing the
  record tap, the "v3d_deqp: build ..." line and the watchdog line**
  (`v3d_board.c:1152-1157`: `no screen: the UART still has every line`). On
  the Pi 5 the firmware framebuffer is untried (`rpi5_screen.h:10-11`) and
  its plane is RGB565 (`rpi5-display.md`, section 1), while `fbcon` is 32 bpp
  only (`rpi5-display.md`, section 1). So a Pi 5 deqp image would print no
  `v3d_deqp: build ... DONE` line and `board.sh pi5` would end at its run
  bound with `verdict=no_end_line`. The Pi 5 needs a board file of its own
  that installs the tap and the record over the UART without a screen (the
  record already goes to the UART: `v3d_board.c:20-27`, `:352`, `:421`),
  arms the BCM2712 watchdog (`YAKOGL_RPI5_WDOG`, selected by
  `YAKOGL_GPU_V3D71`, `zephyr/Kconfig:101`, hold seconds
  `driver/zephyr/lib/board/Kconfig:17-21`) with the lifetime
  `V3D_DEQP_LIFETIME_S`, and passes `EGL_DEFAULT_DISPLAY` straight through
  (the OSAL's default platform, `rpi5_v3d.c:8-12,31-44`; the wrap in
  `deqp/CMakeLists.txt:200-209` is then needed only for the info-log taps).
  New files: `driver/samples/zephyr/deqp/src/v3d71_board.c`, and a
  `CONFIG_YAKOGL_GPU_V3D71` branch in `deqp/CMakeLists.txt:95` so the Pi 4
  branch `:95-131` stays byte-identical.
- **Console**: UART only. The framebuffer console, the on-screen record and
  `fbcon` return with P11; until then the record is read from the UART by
  `board.sh pi5` (its console bridge `BOARD_UART`, never a path in a file).
- **glmark2**: `CONFIG_YAKOGL_BOARD` depends on `SOC_BCM2711` and three
  others, not `SOC_BCM2712` (`driver/zephyr/lib/board/Kconfig:76-77`), and
  `glmark2/prj.conf:6` sets it for every board. The Pi 5 needs either
  `SOC_BCM2712` added to that `depends on` (a shared file, small) or an
  override of the symbol in `boards/rpi_5.conf` plus a Pi 5 board layer
  (window, record, end line) over `CONFIG_YAKOGL_BOARD_RPI5`. Until the
  display (P11 stage 2) exists, glmark2 runs `--off-screen` (the FBO-sized
  window, `driver/docs/glmark2.md:150-162`): its per-scene numbers and
  `glmark2-picture:` readback values come from the UART and need no panel.
  That is a different measurement from on-screen (`glmark2.md:158-160`) and
  the RESULTS row must say so.
- **Zephyr board name** `rpi_5` in every list of boards: the stage-1 case
  `tools/cts/deps.toml:331-336`, the usage text `tools/cts/deps-env.sh:21`,
  `deqp-tree-images.sh:64-67` (`rpi_5) NAME=pi5-deqp-images ...
  BUILD_DEF=V3D_BUILD`, with `-DYAKOGL_WERROR=ON` as the T6 row has,
  `:66`) and `common-images.sh:86-100` (`pi5) ZBOARD=rpi_5 ARM=()`,
  `MARKER='board: PM watchdog resets'`, the string `board.sh pi5` already
  uses, `board.sh:332`). The wrapper `tools/cts/pi5-deqp-images.sh` is one
  line like `pi4-deqp-images.sh:11`.

### 3.3 Memory and binary size on the Pi 5

Starting values, taken from the Pi 4 files and **not measured** on the Pi 5
(they are the first thing the first build and board session settle):

| Image | `sram0` | carve-out / tiled copies / scratch | Why |
|---|---|---|---|
| gl_test | 96 MiB from 0x200000 | 48 / 8 (default) / 2 MiB | `gl_test/boards/rpi_4b.conf:7`, `.overlay:5,14` |
| dEQP | 256 MiB (to 0x10200000) | 48 / 16 / 8 MiB; T5: 64 / 24 | `deqp/boards/rpi_4b.conf:9,14,23`; `texture-2048.conf:16-17`; `board.sh:331` already allows exactly this window |
| glmark2 | 288 MiB (to 0x12200000) | 96 / 24 MiB, TFU on | `glmark2/boards/rpi_4b.conf:12,21,31`; **`board.sh:331`'s `LOAD_END` is 0x10200000, 32 MiB short**: a `pi5` glmark2 run needs the window raised (to 0x12200000, or a per-image rule) in `tools/board.sh`, the file the plan's `texverdict71` and `stage71` items also edit |

Limits that bound these on the Pi 5:

- The GPU's address space is 512 MiB (`VA_SPACE 0x20000000`,
  `src/osal/zephyr_v3d71.c:105`), carve-out and its page table inside it
  (`:109-110`): glmark2's 96 MiB fits, with the same slack as on the Pi 4.
- `sram0` is a three-cell `reg <0x0 0x200000 SIZE>`
  (`rpi5_triangle/boards/rpi_5.overlay:14-17`); every image stays low, below
  4 GiB, because the V3D's page-table entries and the mailbox buffer
  address must fit (U10, `pre-study-rpi5.md:582`; the load address and EL
  are D1's answer: 0x200000, EL2, `RESULTS.md:724`). Pi 5 boards come with
  2 to 16 GB; the firmware's own reserved memory on the Pi 5 is **not read in
  this tree**: the first big image must be checked against the DTB the D1
  probe prints (`rpi5_dtb`) before the 288 MiB is trusted. Open.
- `CONFIG_MAX_XLAT_TABLES` and `CONFIG_KERNEL_VM_SIZE` are per image
  (`deqp/boards/rpi_4b.conf:32-33`, `glmark2/boards/rpi_4b.conf:38-39`;
  the probes use 16, `rpi5_triangle/prj.conf:19`); the non-cacheable alias of
  the carve-out costs a table per 2 MiB (`glmark2/boards/rpi_4b.conf:36-37`).
  Take the Pi 4's.
- `CONFIG_NUM_IRQS` is 280 by default and too small for the V3D's IRQs 281
  and 282 (`pre-study-rpi5.md:441-446`); the OSAL polls the bin
  (`zephyr_v3d71.c:40-44`), so no image needs it until the interrupt step.
- The boot partition has to hold the image (the Pi 4's glmark2 is 12.3 MB,
  `driver/docs/glmark2.md:254`); the Pi 5's free space is unknown. The
  stage should refuse an image over a limit as the DK stage does
  (`board-stage.sh:328-345`) once the limit is known.
- Time: the Pi 4's image lifetime is 240 s, the return to Linux took 213 to
  227 s on the Pi 5 probes (`RESULTS.md:724-731`), against bounds of 420 s
  each (`board.sh:238,357`): 29 images are about 3.7 hours of board time
  plus the T5 image; glmark2 runs 33 scenes of 10 s (`glmark2.md:42`) in its
  900 s lifetime (`glmark2/prj.conf:48`).

## 4. dEQP-GLES2 on the 7.1 table: what each image needs

Sources of the "has" column: the P9/P10 table of `v3d71-gl-table.md`, the
merged pure modules (section 2), and the compiler counts of
`v3d71-backend.md:931-970`. **Open** means no module, test or reference
exists yet. The refusals the shared code produces are the Pi 4's (the same
`state.rs`/`backend.rs` decision: `v3d71-gl-table.md:149-151,161-163`); the
Pi 4's list of what 4.2 refuses and which dEQP rows it costs is
`docs/notes/v3d-refusals.md` (GLES3 rows, but the strings are shared).

| Image(s) (cases) | GL features | After P9/P10 the 7.1 table has | Lacks (open) |
|---|---|---|---|
| SMALL 227 (`info`, `prerequisite`, `color_clear`, `implementation_limits`, `read_pixels`, `depth_range`, `dither`, `default_vertex_attrib`, `shader_api`, `buffer`) | `glClear`, `glReadPixels`, limits answers, default attributes, shader API, buffer objects | clear plan (`clearplan71`), readback (the 4.2 path serves 7.1 unchanged, `readback71` finding), limits (`rt71`, `limits71`), buffers; no default-attribute BO (`attr71.rs:139-142`: `default_vertex_attrib` is a generic-attribute answer, not a BO) | `implementation_limits` expects the **7.1 limits** (8 targets, 7680 px) through the shared limit answers, which the table must report; the ES 2.0 caps stay |
| FF 5 (`flush_finish`) | `glFlush`/`glFinish` | synchronous submit (a second submit finishes the first, `zephyr_v3d71.c:45-46`) | none expected |
| T1 181, T2 248, T3 304, T4 67, T5 7, TF 17, RT 200 (`texture.*`, `texture_functions`, `random.texture`) | 2D and cube textures, formats L8/A8/LA/RGB/RGBA 4444/5551/565, wrap, NPOT, filtering, mip chains (`glGenerateMipmap`), 1024 and 2048 sizes, `texture2D`/`textureCube`/`texture2DLod`/projective | `texture71` compose, `texstate71`, `sampler71`, `format71` (per-GL-format answers), `tfuplan71` (mip chain through the TFU with `IOC`, board-proved by `rpi5_tfu`), `texfill71` (C structs). Seamless cube filtering is predicted, not proved (U8, `v3d71-gl-table.md:82`) | **P10 wiring** (`mip71`); the **texture uniform words** (`texuni71`, running); **vertex texture fetch** (`texture.vertex`, T1; glmark2 terrain too); **OSAL TFU** (`osaltfu71`, in review: `Kconfig:114` still says refused); **compiled shaders with a texture** (`shaders71tex`, running); **cube non-seamless** requests are refused by name, which dEQP's `texture.filtering.cube.*` may hit; board proof of the sampled texture (`boardtex71`) |
| F1 1923 (`fragment_ops`: blend, depth, stencil, scissor, `depth_stencil`, `stencil` ops, `blend` equations and functions, `interaction`) | blending in all equations, depth and stencil tests and ops, scissor, write masks | `draw71` (cfg bits, blend, write masks, stencil, scissor, viewport) checked against Mesa's 7.1 clif for the six gl_test shapes only; `zskeep71` (Z/S kept between jobs) | Mesa references for **blend, depth/stencil ops** (`refs71b`, ready) and the compose check (`compose71d`); the shared refusal that fails case 59 on the Pi 4 (`v3d71-gl-table.md:161`) applies unchanged. The 4.2 table has a separate row about "draw reads stencil an earlier job wrote" that the 7.1 builder no longer causes (G1, `:192-216`) |
| FB1 390, FB2 319 (`fbo.api`, `fbo.completeness`, `fbo.render`) | FBO completeness by format, renderbuffers, render to texture, depth/stencil attachments, RGB565/RGB5_A1/RGBA4 targets | `rt71` (render-target config per GL format, 8 targets, 7680 px), `frame71`, `admit71` | formats with fewer than 8 alpha bits are refused by the shared `state.rs:969` path on the Pi 4 (`v3d71-gl-table.md:163`); a 7.1 answer is **open** and is the biggest unknown of the two images; a sampled render target needs the P10 wiring and the TFU |
| H 654 (`rasterization`, `clipping`) | points (size, sprite), lines (width, strips/loops), triangle clipping, viewport | `prim71` (draw packet values), compiler Points and Lines variants (`v3d71-backend.md:801-824`), viewport and clip window (`compose2` check) | Mesa **point/line** references (`refs71b`, `compose71d`); line and point rasterization rules at 7.1 are not read in this tree: **open** |
| VA 571, DR 101 (`vertex_arrays`, `draw`) | indexed and array draws, element types, strides, offsets, `drawRangeElements` | `prim71` (indexed/array, index type, instancing), `attr71` (16 arrays, stride, no default-attribute BO) | indexed and instanced references (`refs71b`); the six gl_test cases use `glDrawArrays` only |
| AL 57 (`attribute_location`), U1 1267 (`uniform_api`), LA 19 (`light_amount`) | `glBindAttribLocation`, 16 attribute slots, uniform queries and sets, many lights | `attr71`, compiler | the general uniform-stream fill beyond the six programs: `uniform71` names every other contents a refusal (`uniform71.rs:6-22`). Arrays, matrices, samplers and textures-size words come with `texuni71` |
| D 292, E 352, S1 2949, S2 1910, S3 2171, S4 2012, S5f 98, S5v 100 (`shaders.*`: operators, matrices, loops, functions, random, indexing, conversions, linkage) | the shader compiler at 7.1: control flow, loops, derivatives, built-ins, uniform arrays | the 7.1 compiler compiles all of `tools/shaders/es100` and 873 of the 1090 corpus programs; 19 + 212 programs with a branch (`v3d71-backend.md:931-970`) | `link71` (the GL link entry) must merge first. The compiler's refusals are named (`Alloc(DoesNotFit)` closed; the list is `tools/shaders/known-uncompilable-v3d71.txt`). **Our code is 1.5 to 1.8 times Mesa's (43,151 vs 28,118 fs instructions)**, so a program that fits at 4.2 may exceed the 7.1 limits: expected as a named refusal |
| NS 642 (`negative_api`, `state_query`) | error codes and `glGet*` | shared GL core | none for the table; the Pi 5 values answers (limits, renderer string) must be the 7.1 ones |
| MISC 70 (`depth_stencil_clear`, `polygon_offset`, `multisample`, `lifetime`) | Z/S clear, polygon offset, multisample (reports no MSAA on the V3D), context lifetime | `clearplan71`, `zskeep71`; offset is a `draw71` field | Z/S clear through `Clear Render Targets` and `ZS Clear Values` (checked in `compose`), polygon-offset reference (**open**) |
| XT 12 (`clip_control`, `debug_marker`, `multisampled_render_to_texture`) | extensions the table reports or refuses | shared | the `multisample*` answer must stay the refusal of the 4.2 table |

**Expected refusals.** (a) The same 34 NotSupported as on every board
(`RESULTS.md:36`), unchanged; (b) the two stencil refusals of the Pi 4
until the shared decision is lifted (cases 59 and 67 of gl_test,
`v3d71-gl-table.md:161-163`); (c) `V3D 7.1: ...` refusals of the table
(`attr71.rs:144-159`: attribute type, more than 16 arrays, stride above 32
bits, a divisor above 65,535, more than 15 reads of an attribute; more in
`admit71.rs`); (d) a revision that is not 7.1.10 (`ident71.rs`); (e) for
shaders, the compiler's named refusals. A case that fails with none of these
is a new cause and is one issue each; the bar is that every non-pass has one
of these names.

**The order dEQP needs the 7.1 table to grow**, cheapest first: SMALL, FF,
MISC (clear, readback, limits) → AL, U1, S* (the compiler through `link71`
and a general uniform fill) → F1 (`refs71b` → `compose71d` → blend/depth/
stencil in the table) → VA, DR, H (`prim71`, points/lines) → FB1, FB2, then
T1-T5 (the P10 wiring with the TFU and texture uniform words).

## 5. glmark2 on the 7.1 table: what each scene needs

The scenes are `driver/docs/glmark2.md:42-60` (33 entries of ten seconds).
The expected figures come from nothing on the Pi 5; the Pi 4 column is
`RESULTS.md:226-261`.

| Scene (entries) | Features | Needs of the 7.1 table (after P9/P10) | Lacks |
|---|---|---|---|
| build (2): client arrays and VBOs | attribute arrays, VBO, indexed draw, depth test | `attr71`, `prim71`, `draw71`, `zskeep71` | indexed-draw reference (`refs71b`); the general uniform fill (matrices) |
| texture (3): nearest, linear, mipmap | 2D texture, filters, `glGenerateMipmap` of an uploaded texture | `texture71`/`sampler71` and the TFU chain | `mip71`, `texuni71`, `osaltfu71` |
| shading (4), bump (3), pulsar (1), jellyfish (1), ideas (1) | per-pixel lighting, normal/height maps, blending, many draws | compiler (matrices, `normalize`, loops of the `shading` scenes), blend, textures | `link71`; blend references; textures |
| effect2d (2), desktop (2: blur, shadow) | full-window fragment convolution; render-to-texture composition passes | FBO colour render target sampled as a texture | the sampled render target path (P10) |
| buffer (3): map, subdata, interleave | `GL_OES_mapbuffer` (two of three) | shared; buffers as in SMALL | none expected |
| terrain (1) | **vertex texture fetch**, a multi-pass render-to-texture chain, NPOT mip chains by TFU | `sampler71`, `texture71`, TFU | vertex-stage texture words and shader (`texuni71`, `shaders71tex`), `osaltfu71`; 24 MiB of tiled copies, carve-out over 30.5 MB (`glmark2/boards/rpi_4b.conf:6-11`) |
| shadow (1), refract (1) | **`GL_OES_depth_texture`**, a depth-only framebuffer, a 1600x1200 RGBA8 target with its mip chain every frame | D24S8 sampled as a depth texture (`format71`'s depth rows: not yet a compared board shape), `Clear Render Targets`, `zskeep71` | shadow's depth-texture answer (on the 4.2 table a compared depth texture is an R32F TMU type, the pin of commit `8b3ee326`; the 7.1 answer is **open**); refract's chain needs the TFU in the render image (`glmark2/boards/rpi_4b.conf:13-31`) |
| conditionals (3), function (2), loop (3) | branches, calls and loops in the fragment shader | the compiler's branched programs (19 + 212 compile) | `link71`; our instruction counts exceed Mesa's by 1.5x (a larger loop may refuse by name) |

Other things glmark2 needs that are not scenes:

- The EGL attribute lists (`config`) are the shared GL core's, not the
  table's; `check.sh` pins them for the Pi 4 (`tools/check.sh:2123`). Nothing
  Pi 5-specific is expected.
- The per-scene counters, `glmark2-counters:` and the frame cost line, read
  V3D exports gated on `CONFIG_YAKOGL_GPU_V3D` (`docs/notes/glmark2-scene-counters.md:27-29,137`):
  on the Pi 5 they must either read the 7.1 table's counters or print `absent`
  (the OSAL already says its packet-skip counters are off,
  `zephyr_v3d71.c:50-52`). Decision: print `absent`; the 4.2 export set is
  not copied.
- `tools/cts/common-tally.py` and `board.sh`'s record words (`GLMARK2` kind
  18, `driver/docs/glmark2.md:289`) are board-specific; the UART lines
  carry the numbers on the Pi 5 until a record screen exists.

## 6. The runner and the stage

`tools/cts/board-stage.sh pi5 common|deqp` ("stage71" in the plan: a
separate item, because it edits `board-stage.sh`, `deps.toml`,
`deps-env.sh`):

- `layout=rpi_5 key=RPI5` and the lease variables `RESOURCE_RPI5_SSH`,
  `RESOURCE_RPI5_CONSOLE_*` next to `:182`/`:285-296`; no `BOARD_HDMI_TIMINGS`
  (`board.sh:245`: pi5 ignores it); `clear_stale_once` is skipped like the Pi
  4's, tryboot (`:301`).
- `common` builds GM and GT with `cts-common.sh pi5` (`cts-common.sh:47`
  lists the three boards; add `pi5`, and `common-images.sh:86-100`); run
  through `board.sh pi5 tree` unchanged. `common-tally.py` needs the `pi5`
  name (`cts-common.sh:53`).
- `deqp` builds `pi5-full-tree.txt` through `pi5-deqp-images.sh` and runs
  `board.sh pi5 tree` (the Pi 4's two lines, `board-stage.sh:403-405`).
  `needed_deps` (`:138-150`) gets `rpi_5` and the stage-1 case in
  `deps.toml:331-336`. The `pi5-full-tree.txt` starts as a copy of the Pi 4's
  29 blocks; where a block mixes cases whose feature arrives at different
  times (for example FB1 and T1), re-cut it so a missing feature does not hide
  the other cases of the image (a hung or refused-everything image costs a
  full 240 s lifetime).
- The board session is `target:rpi5` for ssh and tryboot, and
  `target:rpi5-console` only to observe or recover (the plan's rule); the run
  is from one origin/main commit, and the PM watchdog is the way back (a
  `pi5` image that cannot arm it refuses its GPU init,
  `zephyr_v3d71.c:192-196`). Addresses and UART paths come from the
  environment, never a file.
- `tools/check.sh` rows (P12, `gate71plan`): `zephyr rpi5 deqp image,
  -Werror` and `rpi5 glmark2 image, -Werror` next to the `rpi4 gl_test`
  row (`check.sh:4632`), with the stage-1 build as a `gap` until
  `DEQP_WORK` for `rpi_5` exists on the runner (`CLAUDE.md`: a gap is not a
  pass). The deqp sample's own `-Werror` (`YAKOGL_WERROR`) must hold for the
  7.1 branch of `CMakeLists.txt`.

## 7. Ordered item list

Size as in `pre-study-rpi5.md:601`: S a day or less once its inputs exist, M
a few days, L a week or more. "Dep" names plan items (merged ones in
brackets). None of this is started by this note.

| # | Item | Size | Depends on | Files (new unless said) | Done when |
|---|---|---|---|---|---|
| 1 | **D0 `deqp-rpi5-sample`**: the deqp sample configures and builds for `rpi_5` (no board run): `boards/rpi_5.conf` and `.overlay`, `src/v3d71_board.c` (UART record, watchdog, no screen, section 3.2), the `CONFIG_YAKOGL_GPU_V3D71` branch of `deqp/CMakeLists.txt:95`; `-DYAKOGL_WERROR=ON`, the image prints `v3d_deqp: build` and the end lines | M | `backend71`, `gltest71` (for the cargo feature and its Zephyr link) | deqp sample only; `CMakeLists.txt` edited at the V3D branch (Pi 4 branch unchanged) | `west build -b rpi_5 ... deqp -DYAKOGL_WERROR=ON` links; `strings` has the two end formats |
| 2 | **`stage71`**: `board-stage.sh pi5 common`, then `deqp` (items 3-4 plug in) | S | `gltest71` | `tools/cts/board-stage.sh`, `cts-common.sh`, `common-images.sh` (`pi5`), `deps-env.sh`, `deps.toml` | `board-stage.sh pi5 common` builds GT; `--dry-run` runs; selftests row in the same commit |
| 3 | **Stage 1 for `rpi_5`**: the stage-1 case in `deps.toml`, one build of VK-GL-CTS for the A76 (about 1.9 GB, a farm build, not a board) | S (a long build) | 1, 2 | `tools/cts/deps.toml`, `deps-env.sh` | `DEQP_WORK` marker present; `check-deqp-images.py` accepts the config |
| 4 | **`pi5-full-tree.txt` and `pi5-deqp-images.sh`**; the `deqp-tree-images.sh` board arm `rpi_5` | S | 1, 3 | `tools/cts/pi5-full-tree.txt`, `pi5-deqp-images.sh`, `deqp-tree-images.sh:64-67` | `--dry-run` lists 29 images; the table check (`cts-selftest`) covers every mustpass name once |
| 5 | **glmark2 image for `rpi_5`**: the board files, the `YAKOGL_BOARD` question (section 3.2), `--off-screen`, the counters line `absent` | M | 2, `gltest71`, `mip71` (TFU), P10 | `glmark2/boards/rpi_5.{conf,overlay}`, a Pi 5 branch of `main.cpp`/`platform.c` or the board layer; `Kconfig` `depends on` if chosen | the image builds `-Werror`; the host harness prints 33 scenes |
| 6 | **`board.sh` pi5 windows and end rule for GM/deqp**: raise `LOAD_END` for the 288 MiB image, a `deqp`-record verdict check; selftest | S | 5 (and `texverdict71`, same file) | `tools/board.sh` | `board.sh --dry-run pi5 tree` for a GM row |
| 7 | **7.1 table: P9 complete** (`backend71`, `cases71`, `gltest71`, `board71`): the six gl_test cases pass, 59 and 67 named | M (running) | plan items | n/a | `RESULTS.md` Pi 5 row |
| 8 | **P10 complete** (`mip71`, `texuni71`, `shaders71tex`, `osaltfu71`, `board71-tex`): textures, TFU mip chains, the first cube-map case | M (running) | 7 | n/a | `boardtex71` verdict |
| 9 | **7.1 draw state for fragment ops**: `refs71b` then `compose71d`, then the table takes blend, depth/stencil ops, indexed/instanced, points/lines; a gl_test case per group | M | 7, `refs71b`, `compose71d` | `src/v3d/*71*.rs`, tests | host compare equal; new cases pass |
| 10 | **Run `SMALL`, `FF`, `MISC` on the board** (the first three images) and record the refusals by name | S (board) | 1-4, 7 | `records/` (ignored), RESULTS row | 3 records with `verdict=done`, 0 resets |
| 11 | **Run the shader images** (D, E, S1-S5, AL, LA, TF, RT, U1) | M (board, 11 images) | 8 for TF/RT, `link71` | RESULTS row | each row a record; non-passes carry a name or an issue |
| 12 | **Run `F1`, `VA`, `DR`, `H`, `FB1`, `FB2`, `NS`, `XT`, then `T1`-`T5`** | M (board, 11 images) | 9, 8 | same | the Pi 4's counts, or each non-pass named |
| 13 | **Run GM and GT through `board-stage.sh pi5 common`** (off-screen first) | S (board) | 5, 6, 8 | RESULTS row | 33 scenes, failed 0 or named, GL errors named |
| 14 | **The whole tree from one origin/main commit** and the Pi 5 rows of `RESULTS.md`, STATUS.md (Japanese Pi 5 line), `ROADMAP.md` stage 5 (the stage 5 bar), the comment for #7 | M | 10-13, P11 (for on-screen glmark2 only), P12 | docs only | stage 5 recorded |

Sequence: 1 and 2 may start as soon as the cargo feature is in the tree
(`backend71`); 3 is a farm build and can run beside them; 5 and 6 follow 2;
7 to 9 are the other plan items; the board runs 10 to 13 come last, in that
order, each from one commit.

## 8. Open questions

- **Firmware memory on the Pi 5**: is a 256 MiB (or 288 MiB) window from
  0x200000 free of firmware use on every board size? Not read in this tree
  (section 3.3). First check: the `rpi5_dtb` output of D1 and one boot of the
  gl_test image with the largest overlay.
- **Pi 5 limits that change a Pi 4 figure**: `implementation_limits`,
  `GL_MAX_DRAW_BUFFERS` (8 on 7.1 vs 4) and `state_query` values; whether the
  table reports ES 2.0 limits or the 7.1 hardware's, so `dEQP-GLES2`
  expectations stay the Pi 4's (the table must keep `YAKOGL_REPORT_ES2=y`,
  `deqp/boards/rpi_4b.conf:4-7`).
- **Seamless cube (U8)**: dEQP's `texture.filtering.cube.*` and
  `texture.wrap.*` cases exercise edges; if the 7.1 TMU is seamless only, the
  non-seamless requests are named refusals (`v3d71-gl-table.md:82`).
- **Compile size**: ours is 1.5 to 1.8 times Mesa's instruction count
  (`v3d71-backend.md:960,972`); an `S`-image program may exceed a limit at 7.1
  that it did not at 4.2. It is expected as a refusal by name; the shortening
  passes of the 7.1 scheduler (`hoist`, `ldvary_back`, reorder) are the
  remedy and belong to P8, not P13.
- **Display** (P11): glmark2 off-screen needs no panel; the on-screen
  numbers, pictures by eye and the record screen wait for
  `rpi5_scanout32` and the board layer (`rpi5-display.md`, sections 2-4).
