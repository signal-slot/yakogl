# YakoGL on the Raspberry Pi 4 (V3D 4.2): backend plan

Status: started 2026-09-23. Board: **Raspberry Pi 4 Model B, 8 GB** (bootloader 2226a853,
2025/12/08), HDMI on the lab capture card, microSD through the microSD OTA bridge
(`signal-slot/microsd-usb-adaptor`). Nothing has run on the GPU yet. The board does not
boot through the bridge today (`SD TIMEOUT` on the first CMD18 of `start4.elf`; the Pi
clocks the card at ~40 MHz while the CSD says 25 MHz); that is being fixed in the adaptor
repository. Everything below comes from sources, not from silicon, and says so where a
value still has to be read on the board.

References (read on the lab host):

- Mesa 26.2.1, `src/broadcom/` and `src/gallium/drivers/v3d/` (MIT: may be transcribed
  with attribution). Tree: `~/pvr-lab/mesa-build/mesa-26.2.1`.
- Linux v6.17 (e5f0a69), sparse checkout of `drivers/gpu/drm/v3d/`,
  `drivers/pmdomain/bcm/`, `drivers/firmware/raspberrypi.c`, `drivers/clk/bcm/`,
  `drivers/mailbox/bcm2835-mailbox.c`, the bcm2711 dts (GPL: facts and file:line only,
  never transcribed). Citations below marked `linux:` are to that tree.
- Zephyr: `boards/raspberrypi/rpi_4b` exists in v4.0 and in the 4.4 workspace.
  From Zephyr af17c0c8 (the revision `west.yml` pins, used for the Pi 4 images
  since 2026-10-01) the board enables Zephyr's own mailbox (`&mbox0`) and firmware
  driver (`/firmware`, rpi_fw). Its ISR drains every reply from MAIL0 while our
  `rpi_mbox.c` polls MAIL0, so every call of ours timed out (-116): no
  framebuffer and no V3D clock raise. **fps recorded on af17c0c8 before the
  commit that disables both nodes (`common/rpi_4b_sram.dtsi`) were at the
  firmware's default V3D clock and headless**; pass/fail results were not
  affected (the full tree at 29460f96 equals sweep 30 case by case). Measured
  2026-10-03 with three tryboots (records in `records/pi4/mbox-29460f96/`): the
  2026-09-29 image (Zephyr v4.4.1) answers on today's board, 29460f96 built on
  v4.4.1 answers, and 29460f96 on af17c0c8 answers once both nodes are
  disabled. The lasting form is `rpi_mbox.c` on Zephyr's rpi_fw API
  (tools/cts/RESULTS.md, Work list).

## Why the Pi 4 and not the Pi 5

V3D 4.2 has been in Mesa since 2019; V3D 7.1 (Pi 5) is newer and its QPU drops the
accumulators (four register-file read ports instead), so the ISA work is different, not
a superset. The Pi 4 also has no SMS block in front of the GPU (7.1 does:
linux:`v3d_drv.c:266-294`). Starting from 4.2 gives the most reference code per question.

## What is reused

- The GL core, EGL, the state model and gl_test: through `struct yakogl_backend` as for
  the GC7000.
- The OSAL shape: submit takes the job, the fence comes back.
- The shader ABI (`ShcShader` reflection). The binary changes, and one GL program becomes
  **three** QPU programs (see below).

## What is new, and how V3D differs from the two GPUs we have

- **No firmware on the GPU** (like the GC7000): the host writes control lists (CL) into
  memory and starts them by writing registers. There is no ring and no kernel CCB.
- **A tiler** (like the PowerVR): each frame is a binning control list (BCL, thread CT0)
  and a render control list (RCL, thread CT1), with a tile-allocation buffer and a tile
  state array (TSDA). The binner can run out of memory and asks for more by interrupt.
- **Three shaders per program**: a coordinate shader (CS, binning), a vertex shader (VS,
  rendering) and a fragment shader (FS), all referenced from one 36-byte
  `GL_SHADER_STATE_RECORD`. Uniforms are a per-draw u32 stream the host writes.
- **Blending is fixed-function** (`BLEND_CFG`), so no blend shader variants, as on the
  GC7000.

## Addresses and sequences (BCM2711)

The SoC maps bus 0x7exxxxxx to ARM 0xfexxxxxx (linux:`bcm2711.dtsi:41`), and DMA masters
see ARM physical 0 at bus 0xc0000000 (`:45`).

| Block | ARM physical | Source |
|---|---|---|
| V3D hub, core0 | 0xfec00000, 0xfec04000 (0x4000 each) | linux:`bcm2711.dtsi:620-629` |
| V3D interrupt | GIC SPI 74 (INTID 106), one line for hub + core | same |
| PM (power, reset) | 0xfe100000 | linux:`bcm2711.dtsi:109-116` |
| V3D ASB bridge | 0xfec11000 (`rpivid_asb`) | same, and linux:`bcm2835-power.c:161-165` |
| Mailbox 0 | 0xfe00b880, property channel 8 | linux:`bcm283x.dtsi:100`, `raspberrypi.c:23` |

**Power-on (M1)**, linux:`bcm2835-power.c:286-340, 383-386`. The GRAFX power island is
left to the firmware on the 2711 (`:209-210, 233-234`); the driver only does:

1. V3D clock on, ~1 us, clock off (32 clocks for the reset to propagate).
2. `PM_GRAFX (0x10c) |= V3DRSTN (bit 6)`; every PM and ASB write carries the password
   0x5a000000.
3. Clock on.
4. ASB master (`0x0c`), then slave (`0x08`): clear `REQ_STOP` (bit 0), wait for `ACK`
   (bit 1) to read 0. The bridge ID at `0x20` reads "brdg" (0x62726467).

The clock is the **firmware** clock 5 (linux:`bcm2711-rpi.dtsi:96`), set with the property
tag `SET_CLOCK_RATE` 0x00038002 (`SET_CLOCK_STATE` 0x00038001). The PM node also names a
CPRMAN V3D clock: which of the two the step-1 toggle really needs is a board question.
The RPi firmware power domain 10 (`SET_DOMAIN_STATE`) is **not** what the 2711 V3D node
uses.

**Identity**: hub `IDENT1` (0x0c) TVER[3:0] = 4, REV[7:4] = 2, NCORES = 1; core `IDENT0`
(0x00) VER[31:24]. `MMU_DEBUG_INFO` (hub 0x1238) gives PA/VA width. The raw values are
read on the board.

**MMU (M2)**, linux:`v3d_mmu.c`: one level, 4 KiB pages, 32-bit PTE = PFN[23:0] | VALID
(28) | WRITEABLE (29) (BIGPAGE 30, SUPERPAGE 31). A 4 GiB VA space is a 4 MiB table, but
we only need the carve-out's range. Program `MMU_PT_PA_BASE` (0x1204) = PA >> 12,
`MMU_CTL` (0x1200) = enable + abort/interrupt on invalid PTE, write violation and cap
exceeded, `MMU_ILLEGAL_ADDR` (0x1230) = scratch page >> 12 | bit 31, `MMUC_CONTROL`
(0x1000) = enable, then flush: `MMUC_CONTROL` FLUSH, wait FLUSHING; `MMU_CTL`
TLB_CLEAR, wait TLB_CLEARING. VA 0 is never mapped.

**Submit (M3)**, linux:`v3d_sched.c:225-330`, offsets from core0:

- Bin: `PTB_BPOS` (0x30c) = 0; invalidate caches; `CT0QMA`/`CT0QMS` (0x170/0x174) =
  tile alloc address/size; `CT0QTS` (0x15c) = TSDA | bit 1; `CT0QBA` (0x160) = BCL start;
  `CT0QEA` (0x168) = BCL end **starts it**.
- Render, after bin done: invalidate caches; `CT1QBA`/`CT1QEA` (0x164/0x16c).
- Cache invalidate: `L2TCACTL` (0x30) = L2TFLS; `SLCACTL` (0x24) = 0x0f0f0f0f.
  `L2TFLSTA`/`L2TFLEND` (0x34/0x38) = 0/~0 once at init. A host read-back after render
  needs the clean: TMUWCF, then L2TFLS with FLM = 2, each polled.
- Interrupts: core `INT_STS` (0x50), clear by writing it to `INT_CLR` (0x58). FRDONE bit
  0, FLDONE bit 1, OUTOMEM bit 2 (edge; FLDONE waits until OOM is served: give the binner
  `PTB_BPOA`/`BPOS` (0x308/0x30c)). Hub: MMU faults. With one IRQ line, both are checked.
- Hang: the Linux timeout is 500 ms with progress judged by `CTnCA`/`CTnRA` moving; the
  reset is the full PM/ASB power cycle above, then MMU and interrupts again.

## Control lists (M4, M5)

From Mesa `v3dx_draw.c`, `v3dx_rcl.c`, `cle/v3d_packet.xml` (packets with `max_ver=42`
variants must use those):

- BCL: `TILE_BINNING_MODE_CFG` (v42), `FLUSH_VCD_CACHE`, `START_TILE_BINNING`, state
  (`CLIP_WINDOW`, `CFG_BITS`, `CLIPPER_XY_SCALING` in 1/256 px on 4.2, Z scale/offset,
  `VIEWPORT_OFFSET`, blend), `VCM_CACHE_SIZE`, `GL_SHADER_STATE` (record 32-byte aligned),
  `VERTEX_ARRAY_PRIMS`, `FLUSH`.
- RCL: `TILE_RENDERING_MODE_CFG_COMMON`, `_CLEAR_COLORS_PART1`, `_COLOR`, `_ZS_CLEAR_VALUES`
  (last), `TILE_LIST_INITIAL_BLOCK_SIZE` (must match the BCL), per layer
  `MULTICORE_RENDERING_TILE_LIST_SET_BASE` + `SUPERTILE_CFG`, the GFXH-1742 dummy tile
  (twice), a generic tile list (implicit coordinates, loads, `BRANCH_TO_IMPLICIT_TILE_LIST`,
  `STORE_TILE_BUFFER_GENERAL`, `END_OF_TILE_MARKER`), `SUPERTILE_COORDINATES` per
  supertile, `END_OF_RENDERING`.
- A clear with no draw is a TLB clear only (`v3d_tlb_clear`). On 4.2 a D24S8 buffer cannot
  clear depth without stencil through the TLB (GFXH-1461).
- Sizes: tile alloc = align(tiles * 128, 4 KiB) + 8 KiB + ..., TSDA = tiles * 256
  (`common/v3d_util.c:272-313`).
- Render to a **raster** RGBA8888 buffer first: no tiling code is needed until textures
  (textures must be LT or UIF).

The first byte-exact reference is Mesa itself on the same Pi under Linux:
`V3D_DEBUG=clif` dumps a real job's BCL, RCL and indirect buffer. That is the M4/M5
equivalent of the DDK capture on the AM62P.

## Shader compiler

**The product path is our own compiler only** (decided by the user, 2026-09-24): GLSL ES
1.00 through the shared front end (`src/shaderc/online/`: pp, parser, sema, lower to the
IR) into a V3D 4.2 back end in `src/shaderc/online/v3d/`, running in the image. There is
no offline table for this GPU, and working pixels are not the bar: the compiler must be
correct by construction and checked instruction by instruction.

`tools/v3d/v3d_shc.c` (Mesa's v3d compiler, built inside the Mesa 26.2.1 tree) is an
**instrument, not a product path**: the reference our output is compared with on the same
source, as the reference compilers' words are for the other two GPUs.
- 2026-09-24: 1896 of 2699 corpus program pairs compile through it (`tools/v3d/glsl2v3d.py`;
  every failure is glslang: 547 `gl_FragData` in the shared translator, 256 qt3d particle
  VS needing extensions). Its front half follows Mesa's GL path (glspirv.c,
  gl_nir_linker.c, st_glsl_to_nir.cpp, cited in the file).
- Point and line variants: `--prim`. Sampler indices are the SPIR-V binding.

What the back end must do that the other two did not:
- **A program, not a stage.** The VS writes its varyings in the order the FS reads them
  (Mesa keys the VS by the FS's `input_slots`), so the VS is generated at `glLinkProgram`.
- **Three programs per GL program**: FS, VS and the coordinate shader (CS) for binning.
  The CS is the VS without the varyings (clip position and scaled xy only), so one
  lowering produces both.
- **Point and line variants** of the FS (point/line coordinate as implicit varyings).
- QPU specifics (Mesa `compiler/`, `qpu/`, MIT): the add/mul dual-issue word, accumulators
  r0-r5 beside the register file, signals (ldunif, ldvary, ldtmu, thrsw, ldtlb), the magic
  write addresses (TLB, VPM, TMU), the uniform stream the host fills per draw, VPM stores
  ending with VPMWT (GFXH-1684), thread switches with their delay slots, and the thread
  count as a function of register pressure.

Order:
1. The ISA module: `isa.rs`, `encode.rs`, `decode.rs`, `roundtrip.rs` (in progress). The
   gate is a byte-exact decode/encode round trip over every QPU word v3d_shc produces for
   the corpus, plus Mesa's `qpu/tests/qpu_disasm.c` vectors, with every builder default
   asserted against the corpus.
2. Selection and assembly for the smallest programs (pass-through VS/CS, constant and
   varying FS), compared with v3d_shc on the same source. Then register allocation and
   instruction pairing, uniforms, textures (TMU), control flow.
3. On the board: gl_test cases that exercise each shape the compiler emits, with pixel
   read-back on every side of each condition. A case lands with the feature it tests.

## Board port

- Zephyr `rpi_4b`: one A72, GIC-400, mini-UART console. No mailbox driver, no display.
  Its `sram0` is 512 KiB: an image with a GPU carve-out needs a larger memory node and an
  MMU region for the V3D, PM, ASB and mailbox blocks.
- Display: the firmware framebuffer through the mailbox (`FRAMEBUFFER_ALLOCATE` 0x40001,
  physical/virtual size, depth, pitch). Not with `dtoverlay=vc4-kms-v3d`, which hands HDMI
  to the ARM side. The returned address is a bus address.
- Boot: `start4.elf`, `fixup4.dat`, the dtb, `config.txt` with `arm_64bit=1`,
  `kernel=zephyr.bin`, `enable_uart=1`. The microSD bridge replaces the kexec route:
  the image is written over USB and the Pi rebooted.
- A Linux harness (`driver/tools/v3dlab/`, like gclab) is possible on Raspberry Pi OS by
  unbinding `v3d` (`brcm,2711-v3d`); the display stays with `vc4`. A plain unbind may let
  genpd power the domain off, so the harness binds with `driver_override` and keeps it.

## Order of work

1. Get the Pi booting from the bridge (adaptor repository, in progress).
2. Boot Raspberry Pi OS once; record `/sys` for v3d, the firmware clock rate, and a
   `V3D_DEBUG=clif` dump of a clear and of one triangle (kmscube or a 30-line EGL
   program). These are the byte references for M4/M5.
3. M1 power and identity, M2 MMU, M3 an empty BCL+RCL and both done interrupts; first as
   the Linux harness (fast iteration), then the same `driver/src/v3d/` files in Zephyr.
4. M4 TLB clear to a raster buffer, M5 one triangle with `v3d_shc` shaders, both with
   pixel read-back against the clif reference.
5. M6 the YakoGL backend (`src/v3d/`) and gl_test on silicon.

## On the board, 2026-09-24: M1..M5 pass

Raspberry Pi 4 Model B 8GB (bootloader 2025/12/08), the card in the Pi's own slot (the
microSD bridge is not used). Linux (Raspberry Pi OS trixie, kernel 6.12.62+rpt-rpi-v8,
Mesa 25.0.7) stays the default boot; each Zephyr image is booted once with the
firmware's tryboot (`/boot/firmware/tryboot.txt`, `sudo reboot "0 tryboot"`) and ends
with a PM watchdog reset (`v3d_clear/src/rpi_wdog.c`, about 60 s after boot), which
brings the board back into Linux with nobody at the power switch.

Measured under Linux first (debugfs `v3d_ident` / `v3d_regs`, `vcgencmd`):
V3D 4.2.14.0, 1 core, 2 slices, 8 QPUs, MMU; HUB_IDENT0..3 0x42554856 0x000e1124
0x00000100 0x00000e00; CTL_IDENT0..2 0x04443356 0x81001422 0x40078121;
MMU_DEBUG_INFO 0x550 (PA and VA 35 bits); **VPM 65536 bytes** (IDENT1.VPM_SIZE 8; the
16384 assumed until then was wrong); V3D clock 500 MHz.

Mesa's byte reference for the M5 frame, taken on the same board (a surfaceless EGL
program drawing the same clear and triangle into a 200x100 FBO under
`V3D_DEBUG=clif`): the shader state record and both attribute records equal ours field
for field apart from addresses; the FS/VS/CS code (9/29/22 words) is byte-identical to
`v3d_shc`'s; the control lists differ only where the cause is known (Mesa 25.0.7 uses a
64-byte initial tile block where 26.2.1 uses 128; Mesa renders to a bottom-up UIF FBO,
we to the top-down raster scan-out, so the Y flip, the winding bit and the store format
differ; Mesa emits NUMBER_OF_LAYERS, we emit BLEND_ENABLES with mask 0).

The runs (each image prints its build; the tree was a87ed25c plus the uncommitted
M5 and watchdog changes this commit carries):
- `v3d_clear`: `v3d_clear: PASS (M1-M4, every pixel read back)`: identity equal to
  the Linux reading, `mmu_init: 0`, bin and render done, `0 of 460800 target pixels
  wrong, guard row 0 of 1280 changed`.
- `v3d_triangle`: `v3d_triangle: PASS (M1-M5, every pixel read back)`: 460800 pixels,
  56248 inside / 401832 outside / 2720 edge, 0 wrong in each, largest inside channel
  difference 1 (tolerance 3), all four probes diff 0, guard row unchanged;
  `M1 power_on: 0, PM_GRAFX 0x00001040`; firmware clock 5 `off: 0`, `on: 0`;
  `VPM: board 65536 bytes, compiled for 65536: same`.
- The watchdog reset returned the board to Linux (uptime 0 min) about 100 s after the
  tryboot reboot, without a power cycle.

- `v3d_triangle` built from **our own compiler's** shaders (`src/shaderc/online/v3d/`,
  `-DTRI_JSON=.../ours_passthrough_interp_color.json`, FS/VS/CS 15/33/24 words against
  Mesa's 9/29/22): `v3d_triangle: PASS (M1-M5, every pixel read back) [shaders:
  yakogl-online]`, the same numbers as Mesa's shaders (0 wrong inside, outside and
  edge; largest channel difference 1; all four probes diff 0). The first program of our
  compiler to run on the V3D. On the host the same programs agree with the IR
  evaluator (6 of 6) and with Mesa's programs on the QPU interpreter (5 of 5,
  `tests/v3d_sim_mesa.rs`); the compiler compiles 9 of 250 corpus pairs and refuses the
  rest by name (168 NotYet, most often Tex and LoadUniform).
  Result row, from the pushed commit: `75de220a` (an ancestor of origin/main, checked
  03:43:50), image sha256 `ea80ead1...`, tryboot at 03:42:05: the same PASS line and the
  same numbers; the watchdog returned the board to Linux at 03:43:48. The build line
  had scrolled off the 1x console by the time of the capture; the run is identified
  by the tryboot target and the Linux boot in between.

- Uniforms and `if`, our compiler next to Mesa's (2026-09-24, 05:07-05:17). Six
  `v3d_triangle` images, all built from `2e759bf7` (an ancestor of origin/main,
  checked 08:42:56). The verdict now carries the build in the same photo:
  `v3d_triangle: build 2e759bf7, json <sha>`. Each Mesa image ran before its twin
  built with our compiler:

  | variant | shaders | verdict | numbers |
  |---|---|---|---|
  | plain | Mesa | PASS | 56248 inside / 401832 outside / 2720 edge, 0 wrong, largest diff 1, probes diff 0 |
  | plain | yakogl-online | PASS | the same |
  | uniforms (`-DTRI_UNIFORMS=ON`) | Mesa | PASS | 56564 / 402156 / 2080, 0 wrong, largest diff 1, probes diff 0 |
  | uniforms | yakogl-online | PASS | the same; storage vs 4 words, fs 8 words |
  | `if` (`-DTRI_IF=ON`) | Mesa | PASS | 13340 then side, 41768 else side, 1140 in the band, 0 wrong |
  | `if` | yakogl-online | PASS | the same |

  Image sha256 (first 16 hex digits), Mesa / ours: plain `f28783d46334edd6` /
  `d2d52594b9790792`, uniforms `e421dd0096974b40` / `1d2480b9e71b0bb1`, `if`
  `2b6998489a501e07` / `84c3d06eb2d9925b`. The uniform run reads each
  `QUNIFORM_UNIFORM` word from the storage the sample fills by name. Every other word is
  a NaN, so a stray read would show in the picture. The `if` picture holds both arms,
  and the check needs inside pixels on both sides. Each run returned to Linux through
  the watchdog. The sample inputs (sample, `driver/src/v3d`, the header generator) are
  unchanged from `2e759bf7` to `657eb8cb`, the pushed merge that carries them.
  What the board did not see: the SFU ops, ffma, the dot products, ffract, floor and
  ceil. They compile and agree with the IR and with Mesa on the host only, and their
  precision on silicon is unmeasured (`docs/online-shaderc/v3d-backend.md`, "SFU
  precision").

- **M6: OpenGL ES 2.0 through YakoGL on the V3D** (`v3d_gl_triangle`, the `gpu-v3d`
  build: the GL core, our online compiler compiling and linking in the image, the job
  builder, the Zephyr OSAL). Verdict `v3d_gl_triangle: PASS (OpenGL ES 2.0 through
  YakoGL, every pixel read back)`: window 460800 = 145288 inside + 311160 outside +
  4352 edge, 0 wrong in each (largest inside channel difference 1, tolerance 3),
  guard row unchanged, all four `glReadPixels` probes equal to memory; compile + link
  1 ms, frame 1 ms. Three runs, the same numbers each time:

  | image | build line | sha256 (first 16) | time |
  |---|---|---|---|
  | the uncommitted tree on `10c11d0a` (pre-commit) | scrolled off | `991293bc4d1c3b23` | 08:23 |
  | `cde8e306` (the M6 commit merged with origin) | `build cde8e306` | `564a042a9a92b097` | 09:08 |
  | `858cee53` (pushed; ancestor of origin/main, checked 09:22:14) | `build 858cee53` | `8dd5d55ffda8cbff` | 09:15 |

  `858cee53` exists because the merge onto origin/main was textually clean but did not
  build for `gpu-v3d`. Upstream's `masked_clear_applied()` sent every build that is not
  the GC7000 to `crate::pvr`. The V3D arm answers `false`: the V3D clears every channel.
  The images report `GL_VERSION OpenGL ES 3.0 YakoGL` (built without
  `CONFIG_YAKOGL_REPORT_ES2`; a dEQP-GLES2 image needs it, and an ES2 image builds).
  What this run does not cover: textures, points, scissored or masked clears
  (`clear_quad`), RGB565 and FBO targets, and depth or stencil that survive a job
  split. Each is refused by name. The GL-core tests on the `gpu-v3d` build: on the host,
  that build's gate row skips `gl::` (it has no offline table), so on this build the GL
  core is compiled but not run by any host test. The first gl_test run on the Pi closes
  that.

- **gl_test on the V3D, and the clear quad** (`v3d_gl_test`, the list 5, 58, 59, 66, 67,
  102 on one context; scissored, stencil-masked and colour-masked clears drawn as a quad
  with `COLOR_WRITE_MASKS`): **4 of 6**, PASS 5, 58, 66, 102 (maskclear: 0 of 4096 wrong
  in each of its three parts), 59 refused by name ("the draw reads stencil an earlier job
  wrote, and stencil is not kept between jobs yet"), 67 refused by name ("a colour format
  with fewer than 8 alpha bits", its GL_RGB frame). Two runs with the same verdicts:
  build `8ac1c8d1` (157 ms) and build `03566423` (150 ms; after it, no
  `GL error 0x500 in glGetIntegerv` before any case: gl_test's `reset_state` had asked
  for pname 16 because a header shadowed `GL_MAX_VERTEX_ATTRIBS`, on every board). Pushed
  as `3c94ad9d` and `468fbcfc`. The `v3d_gl_triangle` regression from the same commits:
  PASS, the same numbers, plus `clear program built: 472 bytes of code` at init.

- **Int and bool ops with our compiler** (`v3d_triangle -DTRI_INT=ON`, `es100/tri_int`:
  `int(4r)` bands (F2i), `-i*3+7` (Ineg, Imul, Iadd, I2f), `i == 1` and `i != 2` (Ieq, Ine,
  B2f), `sign()`, and a ternary read by the next add, the shape the scheduler fix of
  `d66ae677` protects): build `c553a814` (pushed as `4e94e5d4`, ancestor of origin/main,
  checked 12:19:08). int-mesa and int-ours PASS with identical counts and largest channel
  difference 0: inside i=0..3 22592 / 15202 / 9700 / 3080, g<0.5 37777 / g>0.5 12797,
  b>=0.5 12871 / b<0.5 37703, band 5674; plain-mesa and plain-ours PASS unchanged. The
  if and utri pairs passed from `2b800d49`, whose sample inputs are identical to
  `c553a814`'s (0 lines of diff over the files the sample compiles, includes or reads).
  Host: 47 of 251 corpus pairs compile; three-way 0 disagree in both builds; no emitted
  program touches the TMU (pinned, with es100/matdyn as the out-of-range test).

What the Mesa-shader runs prove is the driver side of our code (power, identity, MMU,
control lists, shader state record, submit and wait) with Mesa-compiled shaders. The
runs with our compiler's programs prove those programs on the same path: plain, with
uniforms and with an `if`. Still open: whether the power-on toggle needs the
firmware clock 5 or the CPRMAN V3D clock (both requests are accepted and power-on
succeeds either way).

## Written so far (2026-09-24, host-checked, not run on the board)

- `driver/src/v3d/v3d_cl.h`: the control-list encoder. `tools/v3d/cl_golden.c`:
  340,011 cases equal to Mesa's generated packers, 0 failed (a copy with one
  field moved one bit fails 19,999 of 20,000 cases).
- `driver/src/v3d/v3d_frame.c`: the clear frame (BCL, RCL, per-tile list) as Mesa
  builds it. `tools/v3d/frame_check.c`: Mesa's CLE decoder reads every packet, with
  Mesa's census, at 1x1, 64x64, 200x100, 65x3000, 1920x1080 and 4096x4096.
- `driver/src/v3d/v3d_hw.c`, `v3d.h`, `v3d_regs.h`: M1 power-on and identity,
  M2 page table and MMU, M3 bin/render submit and a polled wait with four distinct
  outcomes. `tools/v3d/hw_mock_test.c`: 38 of 38 checks against a register model.
- `driver/samples/zephyr/v3d_clear/`: M1..M4 in one Zephyr image for `rpi_4b`
  (Zephyr 4.4.1 workspace, SDK 1.0.1). It asks the firmware for a 1280x720
  framebuffer through the property mailbox (`src/rpi_mbox.c`), prints to the top
  half (`src/fbcon.c`, the panel console's font), powers the V3D, maps its buffers
  and the framebuffer, has the V3D clear the bottom half to green, and reads every
  pixel of it back plus a guard row above it. Builds with 0 warnings, 49 KiB
  `zephyr.bin`; `boards/rpi_4b.overlay` gives it 32 MiB of RAM. Not run yet.
- M5, one Gouraud triangle (host-checked, not run). `v3d_cl.h` gained the draw
  packets Mesa emits for the first draw of a job (v3dx_emit.c `v3dX(emit_state)`
  with every dirty bit set, v3d_job.c:436) and the two records;
  `v3d_frame_build_triangle()` writes the clear, that state, the uniform streams
  (v3d_uniforms.c rules for CONSTANT and the four viewport contents; any other
  contents is refused), the shader state record, the attribute records and
  `VERTEX_ARRAY_PRIMS`, in Mesa's order. The clear frame is byte-identical to
  before. `cl_golden`: 720,024 of 720,024 cases equal Mesa's packers.
  `frame_check --triangle`: clif decodes the whole frame at six sizes, with the
  census, every record field against v3d_shc's JSON, the QPU words and the
  uniforms. `driver/samples/zephyr/v3d_triangle/` draws
  `passthrough.vert` + `interp_color.frag` (code from v3d_shc, which is the
  reference instrument here, not a product table) and reads every pixel back;
  its checker has a host test (`host/tri_check_test.c`, 8 cases, 6 wrong
  pictures all caught). Builds for `rpi_4b` with 0 warnings, 61,528-byte
  `zephyr.bin`. Every prog_data field the builder reads comes from the JSON;
  an absent one stops the header generator (`tools/v3d/v3d_prog_header.py`).
- Not written yet: the Linux harness.

QEMU smoke test (2026-09-24, QEMU 10.2.3 `-M raspi4b -display none -kernel
zephyr.elf`, screen taken with the monitor's `screendump`): the image boots, the
property mailbox hands out the 1280x720 framebuffer (pitch 5120, phys 0x3c100000),
the console draws, the firmware reports the V3D clock at 700 MHz, and the run stops
where it must on a machine with no V3D: the bridge ID reads 0, `M1 power_on: -19`,
`v3d_clear: FAIL (M1 power-on)`. QEMU does not model the V3D or its bridge, so
nothing past that is exercised. Its ELF boot path also starts all four cores at
EL3, where the firmware's armstub parks cores 1-3 at EL2: a secondary core then
faults (PC 0x100), and Zephyr's own hello_world misbehaves the same way on this
machine (exceptions on CPU 1), so that is the emulation, and the board has the
final word. The way round it, measured the same day: build a QEMU-only copy with
`-DCONFIG_AARCH64_IMAGE_HEADER=y` (a separate build directory; the sample's own config
is unchanged) and boot its `zephyr.bin` as a raw Image, which QEMU treats like Linux and
parks cores 1-3. Then v3d_clear and v3d_triangle both boot, draw the console and stop
at M1 as they must, with one exception each: the first-use FPU trap of
`CONFIG_FPU_SHARING` (on by default for this SoC), which Zephyr handles and resumes. The
ELF route also left v3d_triangle with a black 640x480 screen (a data abort on the GIC CPU
interface while a secondary core ran the early init); with the cores parked it does not. `prj.conf` raises `CONFIG_MAX_XLAT_TABLES` to 32 and the kernel VM
space to 128 MiB: the defaults (8 tables, 8 MiB) were sized for the 512 KiB board
image, and running out of tables is a runtime assert before the console exists.

Two things only the board can settle, and the sample prints the evidence for both:
the power-on toggle in Linux uses the CPRMAN V3D clock the PM node names, while the
sample switches the firmware clock 5 (its answer is printed, not fatal); and the
framebuffer's byte order on screen (the check compares the bytes the GPU wrote, so
it does not depend on it).

To run it: a FAT boot partition with the firmware (`start4.elf`, `fixup4.dat`,
`bcm2711-rpi-4-b.dtb`), `zephyr.bin` and a `config.txt` of `kernel=zephyr.bin`,
`arm_64bit=1`, `enable_uart=1`. Through the microSD bridge this is an image for the
inactive slot (`sdusb write-slot inactive`, then `switch --rollback`), which leaves
the Raspberry Pi OS slot untouched.

## Board questions

- Raw `HUB_IDENT0..3`, `CTL_IDENT0..2`, `MMU_DEBUG_INFO` values.
- Whether the firmware leaves the GRAFX island powered with no v3d driver bound, and which
  clock the power-on toggle needs (firmware clock 5 or the CPRMAN one).
- The V3D's view of ARM memory above 1 GiB on the 8 GB board (PTE PFN is 24 bits: 36-bit
  PA per linux:`v3d_mmu.c:101`, but the bus limit is read, not assumed).
