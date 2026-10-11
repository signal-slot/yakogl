<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL on the Raspberry Pi 4 (V3D 4.2)

The design facts of the V3D back end: the hardware addresses and sequences,
the control lists, and what the shader compiler must do for this GPU. The
bring-up log (M1 to M6, the first runs, the QEMU smoke test) is
`tools/cts/log/board-sessions/pi4-v3d-bringup.md`. How an image boots on the
board is `docs/standalone-boot.md`, "Raspberry Pi 4".

The code: the driver side is `driver/src/v3d/` (`v3d_hw.c` power, identity,
MMU and submit; `v3d_cl.h` and `v3d_frame.c` the control lists; `v3d_job.c`
the job builder), the YakoGL back end is `src/v3d/`, the board glue is
`driver/zephyr/lib/board/rpi4/`, and the shader back end is
`src/shaderc/online/v3d/` (`docs/online-shaderc/v3d-backend.md`).

References: Mesa 26.2.1 `src/broadcom/` and `src/gallium/drivers/v3d/` (MIT:
may be transcribed with attribution). Linux v6.17 `drivers/gpu/drm/v3d/`,
`drivers/pmdomain/bcm/`, `drivers/firmware/raspberrypi.c` and the bcm2711
device tree (GPL: facts and file:line only, never transcribed). Citations
marked `linux:` are to that tree.

## Why the Pi 4 and not the Pi 5

V3D 4.2 has been in Mesa since 2019; V3D 7.1 (Pi 5) is newer and its QPU drops the
accumulators (four register-file read ports instead), so the ISA work is different, not
a superset. The Pi 4 also has no SMS block in front of the GPU (7.1 does:
linux:`v3d_drv.c:266-294`). Starting from 4.2 gives the most reference code per question.

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
CPRMAN V3D clock: which of the two the step-1 toggle really needs is still open
("Measured on the board").
The RPi firmware power domain 10 (`SET_DOMAIN_STATE`) is **not** what the 2711 V3D node
uses.

**Identity**: hub `IDENT1` (0x0c) TVER[3:0] = 4, REV[7:4] = 2, NCORES = 1; core `IDENT0`
(0x00) VER[31:24]. `MMU_DEBUG_INFO` (hub 0x1238) gives PA/VA width. The values read on
the board are under "Measured on the board".

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
- The colour target is a **raster** RGBA8888 buffer (the scan-out). Textures must be LT
  or UIF (`driver/src/v3d/v3d_tex.c`, `v3d_tiling_mesa.c`).

The byte-exact reference is Mesa itself on the same Pi under Linux: `V3D_DEBUG=clif`
dumps a real job's BCL, RCL and indirect buffer.

## Shader compiler

**The product path is our own compiler only** (decided by the user, 2026-09-24): GLSL ES
1.00 through the shared front end (`src/shaderc/online/`: pp, parser, sema, lower to the
IR) into a V3D 4.2 back end in `src/shaderc/online/v3d/`, running in the image. There is
no offline table for this GPU, and working pixels are not the bar: the compiler must be
correct by construction and checked instruction by instruction.

`tools/v3d/v3d_shc.c` (Mesa's v3d compiler, built inside the Mesa 26.2.1 tree) is an
**instrument, not a product path**: the reference our output is compared with on the same
source, as the reference compilers' words are for the other two GPUs. Its front half
follows Mesa's GL path (glspirv.c, gl_nir_linker.c, st_glsl_to_nir.cpp, cited in the
file). Point and line variants: `--prim`. Sampler indices are the SPIR-V binding.

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

The gates: a byte-exact decode/encode round trip over every QPU word v3d_shc produces for
the corpus, plus Mesa's `qpu/tests/qpu_disasm.c` vectors, with every builder default
asserted against the corpus; and on the board, gl_test cases that exercise each shape the
compiler emits, with pixel read-back on every side of each condition.

## Measured on the board

Read under Linux on 2026-09-24 (debugfs `v3d_ident` / `v3d_regs`,
`vcgencmd`): V3D 4.2.14.0, 1 core, 2 slices, 8 QPUs, MMU; HUB_IDENT0..3
0x42554856 0x000e1124 0x00000100 0x00000e00; CTL_IDENT0..2 0x04443356
0x81001422 0x40078121; MMU_DEBUG_INFO 0x550 (PA and VA 35 bits); **VPM 65536
bytes** (IDENT1.VPM_SIZE 8); V3D clock 500 MHz. The back end refuses to start
on a VPM of another size (`V3D_VPM_BYTES` in `src/shaderc/online/codegen.rs`).

Mesa's byte reference, taken on the same board under `V3D_DEBUG=clif`: the
shader state record and the attribute records equal ours field for field
apart from addresses. The control lists differ only where the cause is
known: the initial tile block size (64 bytes in Mesa 25.0.7, 128 in 26.2.1),
the Y flip, winding bit and store format (Mesa renders to a bottom-up UIF
FBO, we to the top-down raster scan-out), and NUMBER_OF_LAYERS against our
BLEND_ENABLES with mask 0.

Open:

- Whether the power-on toggle needs the firmware clock 5 or the CPRMAN V3D
  clock. Both requests are accepted and power-on succeeds either way.
- The V3D's view of ARM memory above 1 GiB on the 8 GB board (the PTE PFN is
  24 bits: a 36-bit PA per linux:`v3d_mmu.c:101`, but the bus limit has to be
  read, not assumed).
- Zephyr af17c0c8 (the revision `west.yml` pins) enables its own mailbox
  (`&mbox0`) and firmware driver (rpi_fw). Its ISR takes every reply from
  MAIL0 while our `rpi_mbox.c` polls MAIL0, so both nodes are disabled in
  `driver/samples/zephyr/common/rpi_4b_sram.dtsi`. The lasting form is
  `rpi_mbox.c` on Zephyr's rpi_fw API (`tools/cts/RESULTS.md`, Work list).
