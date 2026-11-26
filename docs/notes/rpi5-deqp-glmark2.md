<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Raspberry Pi 5: what P13 (dEQP-GLES2 and glmark2) needs

Status: plan, every reference reread on the tip `ab919063` (2026-11-24).
Nothing here was run on a board, and no code is changed by this note. It is item P13 of
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
calls (header usage `:9-17`, `usage()` `:181`). The Pi 4 is `pi4)
layout=rpi_4b key=RPI4` (`:188`): the layout names the host dependencies
(`needed_deps`, `:144-155`: the west workspace and SDK for every image, the
glmark2 source `GLMARK2_SRC` for `common` `:147`, the board's stage-1 tree
`DEQP_WORK` for `deqp` `:152`). The board comes from `BOARD_HOST`/`RESOURCE_RPI4_SSH`
and `BOARD_UART`/`RESOURCE_RPI4_CONSOLE_*` (`:298-307`; the Pi 4 also needs
`BOARD_HDMI_TIMINGS`, `:310-311`, and boots by tryboot, so it has no stale
one-shot images to clear, `:313`).

| Set | What the stage does | Where |
|---|---|---|
| `common` | `cts-common.sh pi4 build` (two images, labels GM glmark2 and GT gl_test, `tools/cts/common-tree.txt:14-17`) then `run` | `board-stage.sh:375-376`; `tools/cts/cts-common.sh:56-79`; images by `tools/cts/common-images.sh` (board table `:86-100`: `pi4) ZBOARD=rpi_4b`, no watchdog arm, marker `board: PM watchdog resets`; one `west build` per row `:212`) |
| `deqp` | `pi4-deqp-images.sh tools/cts/pi4-full-tree.txt build/deqp/pi4-<sha8>` builds 29 images, then `board.sh pi4 tree` runs them | `board-stage.sh:414-417`; `tools/cts/pi4-deqp-images.sh:11` (one line: `deqp-tree-images.sh rpi_4b`); the builder `tools/cts/deqp-tree-images.sh:157-159` (cmake + ninja of `driver/samples/zephyr/deqp` with `-DDEQP_SRC`/`-DDEQP_BUILD` from the stage 1 and one `-DCONFIG_DEQP_CASE=<filter>` per image; its board arms `:78-81` are `rpi_4b`, `nanopc_t6`, `verdin_imx95`) |

The table `tools/cts/pi4-full-tree.txt` has 29 `# image <LABEL>:` blocks
(`:15-71`), each case of the 17,165 mustpass names of
`tools/cts/gles2-mustpass.txt` in exactly one block (header `:7-9`). The one
image that needs more memory carries the marker `(with texture-2048.conf)`
(`:61`), which `deqp-tree-images.sh:100,117` turns into
`-DEXTRA_CONF_FILE=texture-2048.conf` (`driver/samples/zephyr/deqp/texture-2048.conf:30-31`:
72 MiB carve-out, 24 MiB of tiled copies). After each build the resolved
`.config` is checked to still be a conformance image
(`deqp-tree-images.sh:164`, `tools/cts/check-deqp-images.py`).

The runner is `tools/board.sh <board> tree <table> <outdir>`
(`TREE_IMAGE_DIR=<dir>` makes `<dir>/<label>/zephyr/zephyr.bin` the image of
a row, `tools/board.sh:182`; `tree_rows` `:2823`; `cmd_tree` `:2935`,
dispatched at `:3220`). `board.sh pi5` already exists for the probes (its
arm `:379-419`): tryboot, the BCM2712 PM watchdog as the way back to Linux
(marker `:391`), a 256 MiB load window (`YAKO_ADDR=0x200000;
LOAD_END=0x10200000`, `:390`; the comment above it, `:386-389`, still says
no `rpi_5` overlay exists, while gl_test's 96 MiB `rpi_5.overlay` is now in
the tree), a run bound of 420 s and a return bound of 420 s
(`RUN_BOUND_DEFAULT=420; RETURN_BOUND_DEFAULT=420`, `:419`), and an
end-line regex that **already contains the dEQP end lines**
`deqp: summary: ` and `v3d_deqp: build <sha> (DONE|INCOMPLETE|STOPPED)` and
gl_test's `v3d_gl_test RECORD end` (`:405`). So a Pi 5 `tree` run needs no new end
rule; it needs images that print those lines on the UART (section 3.2).

### 1.2 The images and their memory

| Image | Kconfig/board files on the Pi 4 | Figures |
|---|---|---|
| dEQP (`driver/samples/zephyr/deqp`) | `boards/rpi_4b.conf`, `boards/rpi_4b.overlay` | `CONFIG_YAKOGL_GPU_V3D=y` `:8`; carve-out 48 MiB `:9`; tiled copies 16 MiB `:14` (a 512x512 cube chain is 16 KiB over the 8 MiB default `:10-13`); compiler scratch 8 MiB `:23` (S16 and UA1 of the GLES3 tree need it, `:20-22`); `CONFIG_FPU_SHARING` `:28`, `MAX_XLAT_TABLES=128`, `KERNEL_VM_SIZE=0x20000000` `:32-33`; `sram0` is 256 MiB from 0x200000 (`rpi_4b.overlay:14`; the overlay comment `:5-7`: carve-out, scratch, dEQP code with 1.1 MB of test data and a 64 MiB C heap, `deqp/prj.conf:19`) |
| glmark2 (`driver/samples/zephyr/glmark2`) | `boards/rpi_4b.conf`, `boards/rpi_4b.overlay` | carve-out 96 MiB `:12` (refract's and terrain's needs, `:6-11`); tiled copies 24 MiB `:21`; `CONFIG_YAKOGL_V3D_TFU=y` `:31` (terrain's and refract's `glGenerateMipmap` is one TFU job per mip chain, `:24-29`); `MAX_XLAT_TABLES=192`, `KERNEL_VM_SIZE=0x20000000` `:38-39`; `sram0` 288 MiB from 0x200000, to 0x12200000 (`rpi_4b.overlay:16`, comment `:5-9`); 194,192 KiB RAM at the 48 MiB carve-out, the data files 9,420,183 B (`driver/docs/glmark2.md:425,411`) |
| gl_test (`driver/samples/zephyr/gl_test`) | `boards/rpi_4b.conf`, `.overlay` | carve-out 48 MiB `:7`; 96 MiB window (`.overlay:14`); the Pi 4 `main_rpi4.c` |

Both images also take the board layer: `CONFIG_YAKOGL_BOARD=y` in glmark2's
`prj.conf:6` (window, screen, record, watchdog, end line;
its `depends on` at `driver/zephyr/lib/board/Kconfig:76-78`) with `CONFIG_YAKOGL_BOARD_LIFETIME_S=900`
and `_HOLD_S=20` (`prj.conf:48-49`). The dEQP image has its own board file,
`driver/samples/zephyr/deqp/src/v3d_board.c`: mailbox, HDMI console, the EGL
default display with the V3D clock switch (`:10-14`, the wrap `:147`), the
result record (`:15-27`), the watchdog (`rpi_wdog_announce()`, `:1169`) and
a bounded lifetime: the PM watchdog is fed for `CONFIG_V3D_DEQP_LIFETIME_S`
(default 240) and the record is printed `V3D_DEQP_PHOTO_S` (30) s before it
(`deqp/Kconfig:175-190`). The CMake branch is `deqp/CMakeLists.txt:118-156`
(sources `v3d_board.c`, `rpi_mbox.c`, `fbcon.c` `:127`, `rpi_wdog.c` `:133`;
`RPI_WDOG_HOLD_S` `:150`); `--wrap=eglGetDisplay` and the info-log taps are
`:232-240`; `CMakeLists.txt:50` skips the common sample glue for V3D and
Mali.

### 1.3 What the Pi 4 measured (the bar)

- dEQP-GLES2: 29 rows, **17,126 pass, 0 fail, 34 NotSupported, 3 quality / 2
  compatibility warnings, 0 resets**, at `46f93767`, 2026-10-24
  (`tools/cts/RESULTS.md:34`). The 34 NotSupported are the same on three
  boards and accepted (`RESULTS.md:36`).
- glmark2-es2: **195, 33 scenes, 0 failed, 0 GL errors**, main `87881c78`,
  2026-11-24 (`board-stage.sh pi4 common`); gl_test 13 of 13; terrain 34.77
  ms a frame (`RESULTS.md:85`). The per-scene table's Pi 4 column is the run
  of `c4af4917`: terrain 161.79, shadow 7.34, refract 17.27, sum 347.45 ms
  (`RESULTS.md:218-261`).
- Stage 5's bar for the Pi 5 is the roadmap's: "the Pi 4's counts, or the
  refusals named" (`pre-study-rpi5.md:619`); P12 is the gate rows
  (`:618`), P11 the display (`:617`). Not a performance bar.

## 2. What the Pi 5 has today

| Piece | State | Citation |
|---|---|---|
| Zephyr option | `CONFIG_YAKOGL_GPU_V3D71` exists, selects the PM watchdog and the mailbox; binning is polled. Its help text still says the 7.1 TFU "is not driven yet (refused by name)" (`:121-122`), but the OSAL has its own 7.1 TFU entry `osal_v3d71_tfu` since `eeb4e485`; only the 4.2-shaped TFU entries stay refused | `zephyr/Kconfig:106-122`; `zephyr/CMakeLists.txt:98-111`; `src/osal/zephyr_v3d71.c:39-59` (`osal_v3d71_tfu_present` `:840`) |
| OSAL | VA space 512 MiB, carve-out is a `__noinit` static of `CONFIG_YAKOGL_V3D_CARVEOUT_BYTES`; init refuses unless the PM watchdog is armed; `osal_zephyr_v3d_default_platform()` answers the clock switch itself (`rpi5_v3d.c`), so a caller can pass `EGL_DEFAULT_DISPLAY` | `src/osal/zephyr_v3d71.c:106,114-120,201-205`; `driver/zephyr/lib/board/rpi5/rpi5_v3d.c:3-12,31` |
| Cargo feature, `yakogl_backend_v3d71` | **not in the tree yet**: no `gpu-v3d71` in `Cargo.toml` (`gpu-v3d` is `:101`) and no `*71` `mod` line in `src/v3d/mod.rs`. The job assembly `backend71.rs` (`f1df7fe7`, `7f791cde`), `fill71.rs` (`90776db5`) and the table glue `glue71.rs` (`699353e9`) are merged as pure modules, host-tested through `#[path]` (`tests/v3d71_backend71.rs`, `v3d71_backend71_shapes.rs`, `v3d71_glue.rs`); hooking them up is `feat71` | `docs/notes/v3d71-gl-table.md:35,86-109`; `docs/notes/v3d71-table71.md:19-22` |
| Pure modules | `admit71`, `attr71`, `backend71`, `backend71tex`, `clearplan71`, `draw71`, `ffi71`, `fill71`, `format71`, `frame71`, `glue71`, `ident71`, `limits71`, `mip71`, `prim71`, `record71`, `rt71`, `sampler71`, `texfill71`, `texglue71`, `texstate71`, `texture71`, `texuni71`, `tfuplan71`, `uniform71`, `zskeep71` | `ls src/v3d/*71*.rs` |
| C | `v3d_job71.{c,h}` (compared with Mesa's `gl_*.clif` jobs, 35 named differences), `v3d_tex71.*`, `v3d_tfu71.*`, `v3d_frame71_tri.c` | `v3d71-gl-table.md:181-191`; `driver/src/v3d/*71*` |
| Compiler | 7.1 online back end: es100 126 of 126; `qpu_corpus_71` 874 of 1090 (216 refusals, each listed); glmark2-es2's 69 linkable pairs all link; the dEQP-GLES2 `.test` library 1778 of 1778; harvested dEQP-GLES2 programs 9720 of 10599 (879 refused, 6 of them 7.1-only); points and lines variants 126 of 126; the GL link entry `link71` is merged (`966a39a5`) | `docs/notes/v3d71-backend.md:950-958,973-1014,801-824` |
| Uniform stream | `uniform71.rs` fills `CONSTANT`, `UNIFORM`, the four viewport words and **names every other contents a refusal**; the texture words are a separate merged module, `texuni71.rs` (`b066d160`), not yet wired | `src/v3d/uniform71.rs:13-22` |
| Board facts | triangle PASS (Mesa's shaders and ours), TFU mip-chain job PASS with `IOC` although `HUB_IDENT1` bit 17 reads 0 | `tools/cts/RESULTS.md:806-829` (rows `:823-825`); `v3d71-gl-table.md:227-237` |
| Console and screen | console is `uart10` at 115200 (`pre-study-rpi5.md:77`); `CONFIG_YAKOGL_BOARD_RPI5` gives a 32 bpp mailbox framebuffer, untried on the board, with no GPU platform, record screen or watchdog | `driver/zephyr/lib/board/Kconfig:39-49`; `rpi5/rpi5_screen.h:10`; `rpi5-display.md` sections 1-2 (the firmware's plane is RGB565; own 32 bpp plane is P11 stage 2, `rpi5_scanout32`) |
| Images for `rpi_5` | **gl_test** has `boards/rpi_5.conf` and `.overlay` (`1bb98759`): `GPU_V3D` and `GPU_V3D71` (`:10-11`), 48 MiB carve-out (`:14`), `uart10`, no screen; `sram0` 96 MiB from 0x200000 (`rpi_5.overlay:17-19`, below the DTB at 0x2efec600). Per that commit it compiles with `-DYAKOGL_WERROR=ON` and **fails at the link**: the sample's CMake takes the 4.2 branch, and without `gpu-v3d71` `g_backend` is the 4.2 table. **deqp and glmark2 have no `rpi_5` board files** | `driver/samples/zephyr/gl_test/boards/rpi_5.conf`, `rpi_5.overlay`; `gl_test/README.md:208` |
| Probes' memory | every probe image is 32 MiB of `sram0` from 0x200000 (low, below 4 GiB, U10) | `tools/probes/rpi_5/rpi5_triangle/boards/rpi_5.overlay:15-17` |
| Stage and tools | no `pi5` in `board-stage.sh` (`usage` `:181`), `cts-common.sh` (`:47`) or `common-images.sh` (`:86-100`); no `rpi_5` arm in `deqp-tree-images.sh` (`:78-81`), and `images-selftest.sh:403-404` uses `rpi_5` as its example of a board **without** an arm (expects rc 2), so adding the arm changes that case too; no `rpi_5` in `deps.toml`'s `DEQP_WORK` board case (`:408-415`) or `deps-env.sh`'s list (`:21`); `check.sh` has the row `zephyr rpi_5 probe images` (`:5026`) but no `rpi_5` gl_test, deqp or glmark2 row (`rpi4 gl_test image` is `:4937`) | read |

## 3. The images to build

### 3.1 Which images

1. **gl_test** (`GT`) for `rpi_5` — the P9 item `gltest71`, not P13's; its
   board files are merged, its link waits for `feat71` (section 2). It is
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
   1.9 GB, built in place: `DEQP_WORK`, `tools/cts/deps.toml:401-415`). It
   is a **separate full build**: the cached sparse checkout
   `VK-GL-CTS-gles3` (`DEQP_GLES3_TEST_DIR`, `deps.toml:255-258`:
   `data/gles3/shaders` only, no `external/fetch_sources.py`) must **never**
   be used or named as `DEQP_WORK`; that entry's own note says a partial
   tree there would break the board build. The
   `cts-build-zephyr.sh` takes its compile flags from the deqp sample
   configured for the board (`tools/cts/cts-build-zephyr.sh:80-104`), so the
   sample must **configure** for `rpi_5` first (item D1 below); the A76's
   `-mcpu` then comes by itself.

### 3.2 What differs from the Pi 4 files (and why a new file, not an edit)

- **No `v3d_board.c` reuse as is.** The Pi 4 board file allocates the
  mailbox framebuffer and, **if that fails, returns before installing the
  record tap, the "v3d_deqp: build ..." line and the watchdog line**
  (`v3d_board.c:1150-1157`: `no screen: the UART still has every line`;
  the tap is installed at `:1162`, the build line `:1165`, the watchdog
  `:1169`). On
  the Pi 5 the firmware framebuffer is untried (`rpi5_screen.h:10-11`) and
  its plane is RGB565 (`rpi5-display.md`, section 1), while `fbcon` is 32 bpp
  only (`rpi5-display.md`, section 1). So a Pi 5 deqp image would print no
  `v3d_deqp: build ... DONE` line and `board.sh pi5` would end at its run
  bound with `verdict=no_end_line`. The Pi 5 needs a board file of its own
  that installs the tap and the record over the UART without a screen (the
  record already goes to the UART: `v3d_board.c:20-27`, `:352`, `:419-423`),
  arms the BCM2712 watchdog (`YAKOGL_RPI5_WDOG`, selected by
  `YAKOGL_GPU_V3D71`, `zephyr/Kconfig:109`, hold seconds
  `driver/zephyr/lib/board/Kconfig:17-21`) with the lifetime
  `V3D_DEQP_LIFETIME_S`, and passes `EGL_DEFAULT_DISPLAY` straight through
  (the OSAL's default platform, `rpi5_v3d.c:8-12,31`; the wrap in
  `deqp/CMakeLists.txt:232-240` is then needed only for the info-log taps).
  New files: `driver/samples/zephyr/deqp/src/v3d71_board.c`, and a
  `CONFIG_YAKOGL_GPU_V3D71` branch in `deqp/CMakeLists.txt:119` so the Pi 4
  branch `:118-156` stays byte-identical. gl_test's `rpi_5` link failure
  (section 2; per `1bb98759` the 4.2 branch links `rpi_wdog.c` and
  `rpi_mbox.c` a second time beside the board layer's `rpi5_wdog` and
  `rpi5_mbox`) is the same trap for the deqp sample.
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
  window, `driver/docs/glmark2.md:324-326`): its per-scene numbers and
  `glmark2-picture:` readback values come from the UART and need no panel.
  That is a different measurement from on-screen (`glmark2.md:326`) and
  the RESULTS row must say so.
- **Zephyr board name** `rpi_5` in every list of boards: the stage-1 case
  `tools/cts/deps.toml:408-415`, the usage text `tools/cts/deps-env.sh:21`,
  `deqp-tree-images.sh:78-81` (`rpi_5) NAME=pi5-deqp-images ...
  BUILD_DEF=V3D_BUILD`, with `-DYAKOGL_WERROR=ON` as the T6 row has,
  `:80`; with it the `images-selftest.sh:403-404` case that uses `rpi_5` as
  the board without an arm) and `common-images.sh:86-100` (`pi5) ZBOARD=rpi_5 ARM=()`,
  `MARKER='board: PM watchdog resets'`, the string `board.sh pi5` already
  uses, `board.sh:391`). The wrapper `tools/cts/pi5-deqp-images.sh` is one
  line like `pi4-deqp-images.sh:11`.

### 3.3 Memory and binary size on the Pi 5

Starting values, taken from the Pi 4 files and **not measured** on the Pi 5
(they are the first thing the first build and board session settle):

| Image | `sram0` | carve-out / tiled copies / scratch | Why |
|---|---|---|---|
| gl_test | 96 MiB from 0x200000 | 48 / 8 (default) / 2 MiB | `gl_test/boards/rpi_4b.conf:7`, `.overlay:5,14` |
| dEQP | 256 MiB (to 0x10200000) | 48 / 16 / 8 MiB; T5: 72 / 24 | `deqp/boards/rpi_4b.conf:9,14,23`; `texture-2048.conf:30-31`; `board.sh:390` already allows exactly this window |
| glmark2 | 288 MiB (to 0x12200000) | 96 / 24 MiB, TFU on | `glmark2/boards/rpi_4b.conf:12,21,31`; **`board.sh:390`'s `LOAD_END` is 0x10200000, 32 MiB short**: a `pi5` glmark2 run needs the window raised (to 0x12200000, or a per-image rule) in `tools/board.sh`, the file the `texverdict71` and `stage71` items (neither on main) also edit |

Limits that bound these on the Pi 5:

- The GPU's address space is 512 MiB (`VA_SPACE 0x20000000`,
  `src/osal/zephyr_v3d71.c:114`), carve-out and its page table inside it
  (`:118-120`): glmark2's 96 MiB fits, with the same slack as on the Pi 4.
- `sram0` is a two-address-cell `reg <0x0 0x200000 SIZE>`
  (`rpi5_triangle/boards/rpi_5.overlay:15-17`); every image stays low, below
  4 GiB, because the V3D's page-table entries and the mailbox buffer
  address must fit (U10, `pre-study-rpi5.md:582`; the load address and EL
  are D1's answer: 0x200000, EL2, x0 = the DTB, `RESULTS.md:820`).
- Firmware memory. What the tree records: `rpi5_dtb/README.md:42-46` says
  the firmware "loaded the image at 0x200000 and entered it at EL2 through
  TF-A's BL31 with x0 = 0x2efec600, the DTB (80201 bytes, 322 nodes)"
  (also `docs/standalone-boot.md:491-492`), so the 256 MiB window (to
  0x10200000) and the 288 MiB one (to 0x12200000) both end below the DTB;
  gl_test's `rpi_5.overlay:9-11` relies on the same address. The device
  tree's `memory@0` is "filled by the bootloader"
  (`driver/docs/rpi5-v3d71-facts.md:256-257`). **The `rpi5_dtb` README
  records nothing about `/reserved-memory`, `/memreserve/` entries or the
  `memory` node's `reg`**: the probe prints only nodes whose compatible
  names `v3d`, `2712-pm`, `vc4`, `hvs`, `vc6` or `firmware-clocks` (`:5-8`)
  and skips any node with children (`:41`), which `/reserved-memory` is.
  So whether the firmware or its BL31 reserves memory below 0x12200000, and
  how much RAM the `memory` node reports, is **unknown** (on the Pi 4 the
  firmware's region starts at 0x3c100000, `driver/docs/glmark2.md:425`).
  Open (section 8).
- `CONFIG_MAX_XLAT_TABLES` and `CONFIG_KERNEL_VM_SIZE` are per image
  (`deqp/boards/rpi_4b.conf:32-33`, `glmark2/boards/rpi_4b.conf:38-39`;
  the probes use 16, `rpi5_triangle/prj.conf:19`; gl_test's `rpi_5.conf`
  takes the Pi 4 gl_test's 64 and 0x10000000); the non-cacheable alias of
  the carve-out costs a table per 2 MiB (`glmark2/boards/rpi_4b.conf:33-37`).
  Take the Pi 4's.
- `CONFIG_NUM_IRQS` is 280 by default and too small for the V3D's IRQs 281
  and 282 (`pre-study-rpi5.md:441-446`); the OSAL polls the bin
  (`zephyr_v3d71.c:40-44`), so no image needs it until the interrupt step.
- The boot partition has to hold the image (glmark2 links in 9,420,183
  bytes of data files alone, `driver/docs/glmark2.md:411`); the Pi 5's free
  space is unknown. The stage should refuse an image over a limit as the DK
  stage does (`dk_limits`, `board-stage.sh:335-345`) once the limit is known.
- Time: the Pi 4's image lifetime is 240 s, the return to Linux took 213 to
  227 s on the Pi 5 probes (`back_after`, `RESULTS.md:821-825`), against
  bounds of 420 s each (`board.sh:419`): 29 images are about 3.7 hours of
  board time plus the T5 image; glmark2 runs 33 scenes of 10 s
  (`glmark2.md:67`) in its 900 s lifetime (`glmark2/prj.conf:48`).

## 4. dEQP-GLES2 on the 7.1 table: what each image needs

Sources of the "has" column: the P9/P10 table of `v3d71-gl-table.md`, the
merged pure modules (section 2), and the compiler counts of
`v3d71-backend.md:950-1018`. **Open** means no module, test or reference
exists yet. The refusals the shared code produces are the Pi 4's (the same
`state.rs`/`backend.rs` decision: `v3d71-gl-table.md:149-151,160-162`); the
Pi 4's list of what 4.2 refuses and which dEQP rows it costs is
`docs/notes/v3d-refusals.md` (GLES3 rows, but the strings are shared).

| Image(s) (cases) | GL features | After P9/P10 the 7.1 table has | Lacks (open) |
|---|---|---|---|
| SMALL 227 (`info`, `prerequisite`, `color_clear`, `implementation_limits`, `read_pixels`, `depth_range`, `dither`, `default_vertex_attrib`, `shader_api`, `buffer`) | `glClear`, `glReadPixels`, limits answers, default attributes, shader API, buffer objects | clear plan (`clearplan71`), readback (the 4.2 path serves 7.1 unchanged, `readback71` finding), limits (`rt71`, `limits71`), buffers; no default-attribute BO (`attr71.rs:139-142`: `default_vertex_attrib` is a generic-attribute answer, not a BO) | `implementation_limits` expects the **7.1 limits** (8 targets, 7680 px) through the shared limit answers, which the table must report; the ES 2.0 caps stay |
| FF 5 (`flush_finish`) | `glFlush`/`glFinish` | synchronous submit (a second submit finishes the first, `zephyr_v3d71.c:45-46`) | none expected |
| T1 181, T2 248, T3 304, T4 67, T5 7, TF 17, RT 200 (`texture.*`, `texture_functions`, `random.texture`) | 2D and cube textures, formats L8/A8/LA/RGB/RGBA 4444/5551/565, wrap, NPOT, filtering, mip chains (`glGenerateMipmap`), 1024 and 2048 sizes, `texture2D`/`textureCube`/`texture2DLod`/projective | `texture71` compose, `texstate71`, `sampler71`, `format71` (per-GL-format answers), `tfuplan71` (mip chain through the TFU with `IOC`, board-proved by `rpi5_tfu`), `texfill71` (C structs). Seamless cube filtering is predicted, not proved (U8, `v3d71-gl-table.md:82`) | **P10 wiring** (`mip71` is merged as a pure module, `src/v3d/mip71.rs`, but not wired); the texture uniform words are merged as a pure module (`texuni71.rs`, `b066d160`) but not wired (no `mod` line in `src/v3d/mod.rs` for it or `mip71`, checked at the tip; only tests use them, through `#[path]`); the textured draw through `glue71`/`fill71` is merged (`57cac44f`: `glue71::ops_textured`, `fill71::fill_textured`; the `gl_tex2d` fill test of `tests/v3d71_glue_tex.rs` no longer ignored), though no 7.1 table calls it yet (`feat71`); **vertex texture fetch** (`texture.vertex`, T1; glmark2 terrain too); the OSAL's 7.1 TFU entry is merged (`osal_v3d71_tfu`, `eeb4e485`; the Kconfig help `zephyr/Kconfig:121-122` still says refused); **compiled shaders with a texture** (`shaders71tex`, not on main); **cube non-seamless** requests are refused by name, which dEQP's `texture.filtering.cube.*` may hit; board proof of the sampled texture (`boardtex71`, not on main) |
| F1 1923 (`fragment_ops`: blend, depth, stencil, scissor, `depth_stencil`, `stencil` ops, `blend` equations and functions, `interaction`) | blending in all equations, depth and stencil tests and ops, scissor, write masks | `draw71` (cfg bits, blend, write masks, stencil, scissor, viewport) checked against Mesa's 7.1 clif for the six gl_test shapes; `zskeep71` (Z/S kept between jobs); Mesa's 7.1 blend, depth and line references (`refs71b`, merged `9e8c8fc9`); the stencil-op reference (`gl_stencilops`, `4bd1bd2d`) with its `draw71` compare (`tests/v3d71_stencilops.rs`, `bac626fa`); the compose check against the three references refs71b added (`compose71d`, `tests/v3d71_compose5.rs`, `8bc24775`) | the table taking these states (waits for `feat71`), and `draw71`'s gaps that `compose71d` keeps `#[ignore]`d: the Z update under `GL_EQUAL`, and no `PacketSpec` for `LINE_WIDTH`, `POINT_SIZE` and `BLEND_CCOLOR`; case 59's refusal "the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in" is the 4.2 table's (`src/v3d/backend.rs:3823`) and the 7.1 one gives the same string (`src/v3d/zskeep71.rs:485`; `glue71.rs:89-93`; `v3d71-gl-table.md:160`). The builder's own D24S8 Z-only/stencil-only gap is fixed (G1, `v3d71-gl-table.md:191-216`) |
| FB1 390, FB2 319 (`fbo.api`, `fbo.completeness`, `fbo.render`) | FBO completeness by format, renderbuffers, render to texture, depth/stencil attachments, RGB565/RGB5_A1/RGBA4 targets | `rt71` (render-target config per GL format, 8 targets, 7680 px), `frame71`, `admit71` | formats with fewer than 8 alpha bits (RGBA4, RGB5_A1) are no longer refused: the 4.2 table draws them as their RGBA8 storage (`src/v3d/state.rs:748-757`, test `:1121-1135`) and the 7.1 glue does the same (`src/v3d/glue71.rs:97-101`); `v3d71-gl-table.md:162` still names the old refusal for case 67. Not run on the Pi 5: a board answer is **open**; a sampled render target needs the P10 wiring and the TFU |
| H 654 (`rasterization`, `clipping`) | points (size, sprite), lines (width, strips/loops), triangle clipping, viewport | `prim71` (draw packet values), compiler Points and Lines variants (`v3d71-backend.md:801-824`), viewport and clip window (`compose2` check); the Mesa **point** reference is in the tree (`gl_prims`, `4bd1bd2d`; points and wide lines in `gl_primswide`, `9e8c8fc9`), its draw packets compared through `glue71`/`fill71` (`tests/v3d71_glue_prims.rs`, `ed36498d`) and through `prim71` (`compose71d`, `tests/v3d71_compose5.rs`, `8bc24775`) | `draw71`'s own packet specs, kept `#[ignore]`d: no `PacketSpec` for `POINT_SIZE`/`LINE_WIDTH` (`compose71d`), and one shared program flags gl_PointSize on every primitive where Mesa compiles a variant without it (`gap_psiz_all_prims`, `tests/v3d71_glue_prims.rs`); line and point rasterization rules at 7.1 are not read in this tree: **open** |
| VA 571, DR 101 (`vertex_arrays`, `draw`) | indexed and array draws, element types, strides, offsets, `drawRangeElements` | `prim71` (indexed/array, index type, instancing), `attr71` (16 arrays, stride, no default-attribute BO) | the table and the GL core behind them: the indexed and instanced comparisons are on the host (`gl_indexed` through `glue71`/`fill71`, `tests/v3d71_glue_prims.rs`, `ed36498d`; `tests/v3d71_instanced.rs`, `0e13f9fc`), leaving `glue_prims`' three named gaps (`gap_index_widen`, `gap_restart_as_lists`, `gap_psiz_all_prims`); the six gl_test cases use `glDrawArrays` only |
| AL 57 (`attribute_location`), U1 1267 (`uniform_api`), LA 19 (`light_amount`) | `glBindAttribLocation`, 16 attribute slots, uniform queries and sets, many lights | `attr71`, compiler | the general uniform-stream fill beyond the six programs: `uniform71` names every other contents a refusal (`uniform71.rs:22`). The sampler and texture-size words are `texuni71.rs` (merged, not wired) |
| D 292, E 352, S1 2949, S2 1910, S3 2171, S4 2012, S5f 98, S5v 100 (`shaders.*`: operators, matrices, loops, functions, random, indexing, conversions, linkage) | the shader compiler at 7.1: control flow, loops, derivatives, built-ins, uniform arrays | the 7.1 compiler compiles all of `tools/shaders/es100`, 874 of the 1090 corpus programs, all 1778 programs of the dEQP-GLES2 `.test` library and 9720 of 10599 harvested dEQP-GLES2 programs (`v3d71-backend.md:973-1011`); the GL link entry `link71` is merged (`966a39a5`) | the GL table (`feat71`). Of the harvested set's 879 refusals, 873 are refused at 4.2 as well and 6 `bind_aliasing` programs are 7.1-only ("more than 16 vertex attributes", `v3d71-backend.md:1003-1011`, listed in `tools/shaders/known-uncompilable-v3d71-deqp-gen.txt`). Code size: 164 corpus programs are over twice Mesa's word count, no es100 one (`v3d71-backend.md:1015-1018`), so a program that fits at 4.2 may exceed a 7.1 limit: expected as a named refusal |
| NS 642 (`negative_api`, `state_query`) | error codes and `glGet*` | shared GL core | none for the table; the Pi 5 values answers (limits, renderer string) must be the 7.1 ones |
| MISC 70 (`depth_stencil_clear`, `polygon_offset`, `multisample`, `lifetime`) | Z/S clear, polygon offset, multisample (reports no MSAA on the V3D), context lifetime | `clearplan71`, `zskeep71`; offset is a `draw71` field | Z/S clear through `Clear Render Targets` and `ZS Clear Values` (checked in `compose`), polygon-offset reference (**open**) |
| XT 12 (`clip_control`, `debug_marker`, `multisampled_render_to_texture`) | extensions the table reports or refuses | shared | the `multisample*` answer must stay the refusal of the 4.2 table |

**Expected refusals.** (a) The same 34 NotSupported as on every board
(`RESULTS.md:36`), unchanged; (b) the stencil refusal of gl_test case 59
("the draw reads stencil an earlier job wrote ...", the same string from
`backend.rs:3823` and `zskeep71.rs:485`; case 67's alpha-bits refusal of
`v3d71-gl-table.md:162` is gone, see the FB row); (c) `V3D 7.1: ...` refusals of the table
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

The scenes are the table under `driver/docs/glmark2.md:67` (33 entries of
ten seconds). The expected figures come from nothing on the Pi 5; the Pi 4
column is `RESULTS.md:218-261`. All glmark2-es2 shader pairs that GL can
link (69 of 93) link at 7.1 (`docs/notes/v3d71-backend.md:987,992-998`).

| Scene (entries) | Features | Needs of the 7.1 table (after P9/P10) | Lacks |
|---|---|---|---|
| build (2): client arrays and VBOs | attribute arrays, VBO, indexed draw, depth test | `attr71`, `prim71`, `draw71`, `zskeep71` | the general uniform fill (matrices); the indexed comparison is on the host (`gl_indexed`, `tests/v3d71_glue_prims.rs`, `ed36498d`), its u8/u32-widening gap (`gap_index_widen`) named |
| texture (3): nearest, linear, mipmap | 2D texture, filters, `glGenerateMipmap` of an uploaded texture | `texture71`/`sampler71` and the TFU chain | the wiring of `mip71`, of `texuni71` and of the OSAL's 7.1 TFU entry (both merged as code) |
| shading (4), bump (3), pulsar (1), jellyfish (1), ideas (1) | per-pixel lighting, normal/height maps, blending, many draws | compiler (matrices, `normalize`, loops of the `shading` scenes), blend, textures | the GL table (`feat71`); textures (the blend compose check is on the host: `compose71d`, `tests/v3d71_compose5.rs`, `8bc24775`, its `BLEND_CCOLOR` packet-spec gap named) |
| effect2d (2), desktop (2: blur, shadow) | full-window fragment convolution; render-to-texture composition passes | FBO colour render target sampled as a texture | the sampled render target path (P10) |
| buffer (3): map, subdata, interleave | `GL_OES_mapbuffer` (two of three) | shared; buffers as in SMALL | none expected |
| terrain (1) | **vertex texture fetch**, a multi-pass render-to-texture chain, NPOT mip chains by TFU | `sampler71`, `texture71`, TFU | vertex-stage texture words wired and the textured shaders (`shaders71tex`, not on main); 24 MiB of tiled copies, carve-out over 30.5 MB (`glmark2/boards/rpi_4b.conf:6-11`) |
| shadow (1), refract (1) | **`GL_OES_depth_texture`**, a depth-only framebuffer, a 1600x1200 RGBA8 target with its mip chain every frame | D24S8 sampled as a depth texture (`format71`'s depth rows: not yet a compared board shape), `Clear Render Targets`, `zskeep71` | shadow's depth-texture answer (on the 4.2 table a compared depth texture is an R32F TMU type, the pin of commit `8b3ee326`; the 7.1 answer is **open**); refract's chain needs the TFU in the render image (`glmark2/boards/rpi_4b.conf:13-31`) |
| conditionals (3), function (2), loop (3) | branches, calls and loops in the fragment shader | the compiler's branched programs (`branches_are_structured71`, `v3d71-backend.md:324`) | the GL table (`feat71`); a larger loop may still exceed a 7.1 limit and refuse by name |

Other things glmark2 needs that are not scenes:

- The EGL attribute lists (`config`) are the shared GL core's, not the
  table's; `check.sh` pins them for the Pi 4 (`tools/check.sh:2165`). Nothing
  Pi 5-specific is expected.
- The per-scene counters, `glmark2-counters:` and the frame cost line, read
  V3D exports gated on `CONFIG_YAKOGL_GPU_V3D` (`docs/notes/glmark2-scene-counters.md:27-29,137`):
  on the Pi 5 they must either read the 7.1 table's counters or print `absent`
  (the OSAL already says its packet-skip counters are off,
  `zephyr_v3d71.c:58-60`). Decision: print `absent`; the 4.2 export set is
  not copied.
- `tools/cts/common-tally.py` and `board.sh`'s record words (`GLMARK2` kind
  18, `driver/docs/glmark2.md:456`) are board-specific; the UART lines
  carry the numbers on the Pi 5 until a record screen exists.

## 6. The runner and the stage

`tools/cts/board-stage.sh pi5 common|deqp` ("stage71" in the plan: a
separate item, because it edits `board-stage.sh`, `deps.toml`,
`deps-env.sh`):

- `layout=rpi_5 key=RPI5` and the lease variables `RESOURCE_RPI5_SSH`,
  `RESOURCE_RPI5_CONSOLE_*` next to `:188`/`:298-307`; no `BOARD_HDMI_TIMINGS`
  (`board.sh:277`: pi5 ignores it, `:413` clears it); `BOARD_BOOT` and
  `clear_stale_once` are skipped like the Pi 4's, tryboot (`:309`, `:313`;
  both test `board = pi4` today).
- `common` builds GM and GT with `cts-common.sh pi5` (`cts-common.sh:47`
  lists the three boards; add `pi5`, and `common-images.sh:86-100`); run
  through `board.sh pi5 tree` unchanged. `common-tally.py` needs the `pi5`
  name (`cts-common.sh:53`).
- `deqp` builds `pi5-full-tree.txt` through `pi5-deqp-images.sh` and runs
  `board.sh pi5 tree` (the Pi 4's lines, `board-stage.sh:414-417`).
  `needed_deps` (`:144-155`) gets `rpi_5` and the stage-1 case in
  `deps.toml:408-415`; `DEQP_WORK` for `rpi_5` is its own
  `deqp-stage1-rpi_5` tree (`deps.toml:402-403`), never the sparse
  `VK-GL-CTS-gles3` checkout (section 3.1). The `pi5-full-tree.txt` starts as a copy of the Pi 4's
  29 blocks; where a block mixes cases whose feature arrives at different
  times (for example FB1 and T1), re-cut it so a missing feature does not hide
  the other cases of the image (a hung or refused-everything image costs a
  full 240 s lifetime).
- The board session is `target:rpi5` for ssh and tryboot, and
  `target:rpi5-console` only to observe or recover (the plan's rule); the run
  is from one origin/main commit, and the PM watchdog is the way back (a
  `pi5` image that cannot arm it refuses its GPU init,
  `zephyr_v3d71.c:201-205`). Addresses and UART paths come from the
  environment, never a file.
- `tools/check.sh` rows (P12, `gate71plan`): `zephyr rpi5 deqp image,
  -Werror` and `rpi5 glmark2 image, -Werror` next to the `rpi4 gl_test`
  row (`check.sh:4937`; the `rpi_5` probe-image row is `:5026`), with the stage-1 build as a `gap` until
  `DEQP_WORK` for `rpi_5` exists on the runner (`CLAUDE.md`: a gap is not a
  pass). The deqp sample's own `-Werror` (`YAKOGL_WERROR`) must hold for the
  7.1 branch of `CMakeLists.txt`.

## 7. Ordered item list

Size as in `pre-study-rpi5.md:601`: S a day or less once its inputs exist, M
a few days, L a week or more. "Depends on" names plan items and says for
each whether it is merged at `888583fc` (checked by `git log` and the files).
None of the P13 items below is started.

At the tip, **merged**: `backend71` (`f1df7fe7`, `7f791cde`), `fill71`
(`90776db5`), `glue71` (`699353e9`), `texuni71` (`b066d160`), `link71`
(`966a39a5`), `refs71b` (`9e8c8fc9`) and its check `compose71d`
(`tests/v3d71_compose5.rs`, `8bc24775`), the OSAL's 7.1 TFU (`osaltfu71`,
`eeb4e485`), `mip71` (`e6f6e56d`, a pure module, not wired), `texglue71`
(`e7737914`) and `backend71tex` (`5bde3959`), the two texture assembly
modules, pure and not wired; the board files and CMake branch of
`gltest71a` (`1bb98759`, `dec352ac`: `boards/rpi_5.*`, the `V3D71` branch
of gl_test's `CMakeLists.txt`, `main_rpi5.c`; its link is not on main),
of `deqp-rpi5a` (`35c98f25`: `boards/rpi_5.*`, `v3d71_board.c`, the
`CONFIG_YAKOGL_GPU_V3D71` branch of `deqp/CMakeLists.txt`; link not on
main) and of glmark2 (`a6bb122f`, the board layer's BCM2712 arm
`d4f95c29`, `glmk71bl`); `tree71` (`e4001734`: `pi5-full-tree.txt`,
`pi5-deqp-images.sh`, the `rpi_5` arm of `deqp-tree-images.sh`, item 4);
`tfu140` (`e8d59f40`: gl_test case 140 is left out of 7.1 builds).
**Not on main**: `feat71` (the `gpu-v3d71` feature, no `gpu-v3d71` in
`Cargo.toml`, no `*71` `mod` line in `src/v3d/mod.rs`), the link of the
`rpi_5` gl_test, deqp and glmark2 images (they wait for `feat71`),
`stage71` (no `pi5` arm in `board-stage.sh`), `texverdict71`, `cases71`,
`board71`, `shaders71tex`, `boardtex71`, `gate71plan`.

| # | Item | Size | Depends on | Files (new unless said) | Done when |
|---|---|---|---|---|---|
| 1 | **D0 `deqp-rpi5-sample`**: the deqp sample configures and builds for `rpi_5` (no board run): `boards/rpi_5.conf` and `.overlay`, `src/v3d71_board.c` (UART record, watchdog, no screen, section 3.2), the `CONFIG_YAKOGL_GPU_V3D71` branch of `deqp/CMakeLists.txt:95`; `-DYAKOGL_WERROR=ON`, the image prints `v3d_deqp: build` and the end lines | M | `backend71` (merged); `feat71` (not on main: the cargo feature); `gltest71` (board files merged, the Zephyr link not on main) | deqp sample only; `CMakeLists.txt` edited at the V3D branch (Pi 4 branch unchanged) | `west build -b rpi_5 ... deqp -DYAKOGL_WERROR=ON` links; `strings` has the two end formats |
| 2 | **`stage71`**: `board-stage.sh pi5 common`, then `deqp` (items 3-4 plug in) | S | `gltest71` (link not on main) | `tools/cts/board-stage.sh`, `cts-common.sh`, `common-images.sh` (`pi5`), `deps-env.sh`, `deps.toml` | `board-stage.sh pi5 common` builds GT; `--dry-run` runs; selftests row in the same commit |
| 3 | **Stage 1 for `rpi_5`**: the stage-1 case in `deps.toml`, one full build of VK-GL-CTS for the A76 (about 1.9 GB, a farm build, not a board; never the sparse `VK-GL-CTS-gles3` checkout) | S (a long build) | 1, 2 | `tools/cts/deps.toml`, `deps-env.sh` | `DEQP_WORK` marker present; `check-deqp-images.py` accepts the config |
| 4 | **`pi5-full-tree.txt` and `pi5-deqp-images.sh`**; the `deqp-tree-images.sh` board arm `rpi_5` | S | 1, 3 | `tools/cts/pi5-full-tree.txt`, `pi5-deqp-images.sh`, `deqp-tree-images.sh:78-81`, `images-selftest.sh:403-404` | `--dry-run` lists the table's images; the table check (`cts-selftest`) covers every mustpass name once. Done in the commit `cts: pi5-full-tree.txt and the rpi_5 arm of deqp-tree-images.sh`: 31 images (the Pi 4's 29 with T1 split into T1 + TV and MISC into MISC + PO, sections 4 and 6), 17165 mustpass cases matched once (`check-gles3-tree.py --mustpass gles2-mustpass.txt`), `images-selftest.sh` section 7h |
| 5 | **glmark2 image for `rpi_5`**: the board files, the `YAKOGL_BOARD` question (section 3.2), `--off-screen`, the counters line `absent` | M | 2, `gltest71` (link not on main), `mip71` (merged, not wired), P10 | `glmark2/boards/rpi_5.{conf,overlay}`, a Pi 5 branch of `main.cpp`/`platform.c` or the board layer; `Kconfig` `depends on` if chosen | the image builds `-Werror`; the host harness prints 33 scenes |
| 6 | **`board.sh` pi5 windows and end rule for GM/deqp**: raise `LOAD_END` for the 288 MiB image, a `deqp`-record verdict check; selftest | S | 5 (and `texverdict71`, not on main, same file) | `tools/board.sh` | `board.sh --dry-run pi5 tree` for a GM row |
| 7 | **7.1 table: P9 complete** (`backend71`, `cases71`, `gltest71`, `board71`): the six gl_test cases pass, or a non-pass is named (59 expected) | M (running) | `backend71`, `fill71`, `glue71` merged; `feat71`, `cases71`, `board71` and the gl_test link not on main | n/a | `RESULTS.md` Pi 5 row |
| 8 | **P10 complete** (`mip71`, `texuni71`, `shaders71tex`, `osaltfu71`, `board71-tex`): textures, TFU mip chains, the first cube-map case | M (running) | 7; `texuni71` (`b066d160`), `osaltfu71` (`eeb4e485`) and `mip71` (`e6f6e56d`) merged as modules, `mip71` and `texuni71` still not wired (no `mod` line in `src/v3d/mod.rs`; only `tests/v3d71_mip.rs` and `v3d71_texuni.rs` use them, through `#[path]`); `texglue71` (`e7737914`) and `backend71tex` (`5bde3959`) merged, pure; `glue71`'s textured draw is pinned by `tests/v3d71_glue_tex.rs` (`512f3297`); `shaders71tex`, `board71-tex` not on main | n/a | `boardtex71` verdict |
| 9 | **7.1 draw state for fragment ops**: `refs71b` then `compose71d`, then the table takes blend, depth/stencil ops, indexed/instanced, points/lines; a gl_test case per group | M | 7, `refs71b` (merged, `9e8c8fc9`), `compose71d` (merged, `tests/v3d71_compose5.rs`, `8bc24775`); Mesa references pinned on the host since: stencil ops (`bac626fa`), instanced draws (`0e13f9fc`), four targets (`050001cc`), indexed draws, points and lines through `glue71` (`ed36498d`); the table taking these states waits for `feat71` | `src/v3d/*71*.rs`, tests | host compare equal; new cases pass |
| 10 | **Run `SMALL`, `FF`, `MISC` on the board** (the first three images) and record the refusals by name | S (board) | 1-4, 7 | `records/` (ignored), RESULTS row | 3 records with `verdict=done`, 0 resets |
| 11 | **Run the shader images** (D, E, S1-S5, AL, LA, TF, RT, U1) | M (board, 11 images) | 8 for TF/RT, `link71` (merged) | RESULTS row | each row a record; non-passes carry a name or an issue |
| 12 | **Run `F1`, `VA`, `DR`, `H`, `FB1`, `FB2`, `NS`, `XT`, then `T1`-`T5`** | M (board, 11 images) | 9, 8 | same | the Pi 4's counts, or each non-pass named |
| 13 | **Run GM and GT through `board-stage.sh pi5 common`** (off-screen first) | S (board) | 5, 6, 8 | RESULTS row | 33 scenes, failed 0 or named, GL errors named |
| 14 | **The whole tree from one origin/main commit** and the Pi 5 rows of `RESULTS.md`, STATUS.md (Japanese Pi 5 line), `ROADMAP.md` stage 5 (the stage 5 bar), the comment for #7 | M | 10-13, P11 (for on-screen glmark2 only), P12 | docs only | stage 5 recorded |

Sequence: 1 and 2 may start as soon as the cargo feature is in the tree
(`feat71`; `backend71` itself is merged); 3 is a farm build and can run beside them; 5 and 6 follow 2;
7 to 9 are the other plan items; the board runs 10 to 13 come last, in that
order, each from one commit.

## 8. Open questions

- **Firmware memory on the Pi 5**: is a 256 MiB (or 288 MiB) window from
  0x200000 free of firmware use on every board size? Known: the DTB is at
  0x2efec600, above both windows (`rpi5_dtb/README.md:42-46`). Unknown: the
  `/reserved-memory` children, the `/memreserve/` entries and the `memory`
  node's `reg`, which `rpi5_dtb` does not print and its README does not
  record (section 3.3). First check: a DTB dump of those nodes (a probe
  change, not P13's) or Linux's `/proc/device-tree` on the board, then one
  boot of the largest image.
- **Pi 5 limits that change a Pi 4 figure** (answered at the tip; the Pi 4
  comparison is still open): the limit queries do not read the GPU table, so
  `YAKOGL_REPORT_ES2=y` is enough and the `dEQP-GLES2` expectations stay the
  Pi 4's.
  - Path of the setting: Kconfig `YAKOGL_REPORT_ES2` (`default y`,
    `zephyr/Kconfig:712-714`) -> `zephyr/CMakeLists.txt:158-162` sets
    `YAKOGL_REPORT_ES2` ON/OFF -> the crate feature `report-es2`
    (`Cargo.toml:81`) -> `crate::REPORT_ES2` (`src/lib.rs:89`).
  - `GL_MAX_DRAW_BUFFERS`: under `REPORT_ES2` the query is `GL_INVALID_ENUM`,
    not a figure (`src/gl/state.rs:1330-1340`; ES 2.0 core has no such
    query). Only an ES 3.0 build answers `context::max_draw_buffers()`
    (`src/gl/state.rs:1341`, `src/gl/context.rs:236-238` ->
    `backend::draw_targets()`, `src/backend.rs:930`), the back end's number.
    So 8 on 7.1 against 4 does not show under ES 2.0.
  - `GL_MAX_TEXTURE_SIZE`, `GL_MAX_RENDERBUFFER_SIZE` are the constant 4096
    for every GPU (`src/gl/state.rs:1279,1281`); `GL_MAX_VERTEX_ATTRIBS` is
    16 (`src/gl/state.rs:1273`, `src/gl/context.rs:78`);
    `GL_MAX_VERTEX/FRAGMENT_UNIFORM_VECTORS` are 384 each
    (`src/gl/state.rs:1287-1288`, `src/gl/context.rs:84-90`). None depends on
    the GPU feature or on `REPORT_ES2`.
  - `GL_MAX_VARYING_VECTORS` (`src/gl/state.rs:1289`) is 8 for `gpu-v3d` under
    `report-es2` and 15 without it (`src/gl/context.rs:212-215`), so the
    figure is the Pi 4's as long as the 7.1 build also has `gpu-v3d`
    (below).
  - `rpi_5` deqp config: `CONFIG_YAKOGL_REPORT_ES2=y` as
    `deqp/boards/rpi_4b.conf` has it (its header, lines 4-7), and no limit
    override: there is none to set.
  - The 7.1 table's own limits (`MAX_RENDER_TARGETS71` 8,
    `MAX_FRAMEBUFFER_SIZE71` 7680, `src/v3d/limits71.rs:34-38`) feed its target
    guard; `src/v3d/limits71.rs:28-31` says the GL core reports 4096 and the
    guard changes no GL query value.
  - `gpu-v3d71` must imply `gpu-v3d` (`zephyr/Kconfig:113` states it): the
    varyings cfg above matches `feature = "gpu-v3d"` only. The feature is not
    in `Cargo.toml` yet (`feat71`, not on main), so the implication is a requirement on that
    item, not a fact of the tree.
  - Hardware figures rest on `third_party/mesa-26.2.1/src/broadcom/common/v3d_limits.h`
    and `limits71.rs`: Mesa's `v3d_screen.c` is not vendored.
  - Open: no case-by-case comparison with a real Pi 4 `dEQP-GLES2` run
    (`implementation_limits`, `state_query`) was made; it needs the board
    runs.
- **Seamless cube (U8)**: dEQP's `texture.filtering.cube.*` and
  `texture.wrap.*` cases exercise edges; if the 7.1 TMU is seamless only, the
  non-seamless requests are named refusals (`v3d71-gl-table.md:82`).
- **Compile size**: 164 of the 1090 corpus programs are over twice Mesa's
  word count, no es100 program is (`v3d71-backend.md:1015-1018`); an
  `S`-image program may exceed a limit at 7.1
  that it did not at 4.2. It is expected as a refusal by name; the shortening
  passes of the 7.1 scheduler (`hoist`, `ldvary_back`, reorder) are the
  remedy and belong to P8, not P13.
- **Display** (P11): glmark2 off-screen needs no panel; the on-screen
  numbers, pictures by eye and the record screen wait for
  `rpi5_scanout32` and the board layer (`rpi5-display.md`, sections 2-4).
