<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: Raspberry Pi Zero 2 W (BCM2710A1 / RP3A0, VideoCore IV)

Status 2026-10-28, written before the board arrives. Nothing here has run on
silicon. This note says what an ES 2.0 port to the Zero 2 W costs, what it
brings, what can be done on the host first, and what is still unknown.

## How to read the citations

- `mesa:<path>` is a file of Mesa 26.2.1 (MIT), read in the tree the Mali and
  V3D reference builds use (`tools/mali/drm_shim_ref.sh` and
  `tools/v3d/README.md` name the tarball; `mesa-26.2.1.tar.xz`). Upstream code
  is read, not copied. `mesa:src/gallium/drivers/vc4/kernel/` is Mesa's
  userspace mirror of the Linux `vc4` kernel validator (its README says so),
  so it shows the kernel's view of the packets, but it is a mirror, not the
  kernel tree.
- `zephyr:<path>` is the Zephyr workspace the Pi 4 images build with
  (`VERSION` 4.4.99).
- `repo:<path>` is this repository at the commit this note was written on.
- **Not read in this session.** The session had no network. The following were
  not read, so nothing below is claimed from them: the Linux kernel tree
  (`drivers/gpu/drm/vc4/`, `arch/arm/boot/dts/broadcom/bcm283x*.dtsi`,
  `drivers/pmdomain/bcm/`, `drivers/firmware/raspberrypi.c`), the Broadcom
  *VideoCore IV 3D Architecture Reference Guide*, the Raspberry Pi firmware
  documentation, and Zephyr's documentation site. Where a fact would come from
  one of them it is written as an **unknown** with the file to read (section 7),
  or marked "(unread)" if it is general knowledge that a reader must confirm
  first. The first job after this note merges is to read those sources and
  turn each unread item into a cited one (item 0 of section 8).
- **Read late, partly.** One page fetch of Linux v6.17
  `arch/arm/boot/dts/broadcom/bcm283x.dtsi` (through a summarising fetch tool,
  so treat it as a pointer, not a transcript) gave: node `mailbox@7e00b880`,
  `compatible = "brcm,bcm2835-mbox"`, `reg = <0x7e00b880 0x40>`; node
  `hvs@7e400000`, `compatible = "brcm,bcm2835-hvs"`, `reg = <0x7e400000
  0x6000>`. The same fetch did not show `v3d`, `pixelvalve`, `hdmi` or
  `firmware` nodes (they may be in `bcm2835-common.dtsi` or the board files;
  not checked), and `bcm2837.dtsi` returned HTTP 429. Both files stay unread
  for Q-items in section 7.

## 1. The hardware

| Part | What | Source |
|---|---|---|
| SoC | RP3A0 is the BCM2710A1 die with the RAM stacked in the same package; it is the Pi 3's BCM2837 family (4 Cortex-A53, AArch64 capable) | task statement; core count and the A53 (unread, to confirm in the Linux dtsi) |
| GPU | VideoCore IV, driven by Mesa `vc4` | `mesa:docs/drivers/vc4.rst:4-6`, "notably used in the Raspberry Pi 0 through Raspberry Pi 3" |
| GPU identity | Mesa's drm-shim reports V3D `IDENT0 = 0x2000000`, `IDENT1 = 1`, "a VC4 2.1 device"; the real values are read from `V3D_IDENT0..2` by the kernel and handed to Mesa | `mesa:src/broadcom/drm-shim/vc4_noop.c:85-86`, `mesa:src/broadcom/drm-shim/README.md` (vc4_noop section), `mesa:include/drm-uapi/vc4_drm.h:325-327` |
| Memory | 512 MB shared with the GPU, no GPU-side MMU | RAM size: task statement. No MMU: Mesa's mirrored kernel validator writes `bo->paddr + offset` into the shader record for the three shader addresses (`mesa:.../vc4/kernel/vc4_validate.c:819`), and writes the tile state and tile alloc addresses of the binner config as plain physical addresses (`vc4_validate.c:406-415`), so the GPU takes physical (bus) addresses. Contrast V3D 4.2, which has a one-level MMU we program (`repo:docs/notes/pi4-v3d-bringup.md`, "MMU (M2)"). |
| Display | not read. The Linux driver puts the display in the same `vc4` DRM driver (task statement); the Pi 4 path we already use is the firmware's property mailbox framebuffer (`repo:docs/standalone-boot.md`, "Raspberry Pi 4"; `zephyr:boards/raspberrypi/rpi_4b/rpi_4b.dts`, node `firmware/framebuffer`). Whether the Zero 2 W firmware offers the same tags on its HDMI mini port is expected, not confirmed (question Q8). | |
| Boot chain | not read for this board. The Pi 4 chain is SPI EEPROM, then `start4.elf` from the FAT partition, then `kernel=` (`repo:docs/standalone-boot.md:303-340`). The Pi 3 family has no EEPROM; its ROM loads `bootcode.bin` and then `start.elf` from the card (unread). | |

Public documentation, as far as we know it (unread, each to be confirmed):
Broadcom released the VideoCore IV 3D Architecture Reference Guide (2013)
and, per Mesa's own driver documentation, a specification PDF for the 21553
"closely related to the VC4 GPU present in the Raspberry Pi" and a snapshot of
the matching Android driver (`mesa:docs/drivers/vc4.rst:284-290`, with the two
`docs.broadcom.com/doc/12358545` and `12358546` links). The BCM2835 ARM
Peripherals datasheet covers the peripherals; the BCM2837 differs (unread).
The firmware's behaviour, the clocks and the power domains are partly
documented by the firmware wiki and by the Linux driver sources only.

## 2. The open-source stack today

- **Mesa** `src/gallium/drivers/vc4/`: "a nearly conformant GLES2 driver, and
  the hardware has achieved GLES2 conformance with other driver stacks"
  (`mesa:docs/drivers/vc4.rst:21-22`). It also exposes OpenGL 2.1 with caveats.
  Known holes it lists: 4-byte index buffers, no occlusion queries, no
  primitive-mode reduction (`vc4.rst:30-60`). Screen limits in
  `mesa:.../vc4/vc4_screen.c`: 2048 max 2D texture (`:195`), no 3D textures
  (`:197`), 8 varyings (`:199`), 8 inputs and 8 vertex outputs, one fragment
  output (`:150-152`), 16 samplers (`vc4_screen.h:58`), 12 mip levels
  (`vc4_screen.h:57`), constant buffer 0 of 16 K floats (`:155`), no clip planes (`:212`), no
  texrect (`:210`).
- **Kernel** `vc4` DRM (unread). Its submit ABI is visible in
  `mesa:include/drm-uapi/vc4_drm.h:91-` (`struct drm_vc4_submit_cl`: the binner
  control list, the shader records, the uniforms, the BO handles, the tile
  rectangle, the read/write surfaces for the render list, the clear values and
  the `FIXED_RCL_ORDER` flag). For us this ABI matters only as the shape of one
  frame: we have no kernel, we write the registers (as on the Pi 4).
- **Firmware needed by the GPU**: the Mesa driver "makes no use of the closed
  source VPU firmware on the VideoCore IV block, instead talking directly to the
  GPU block from Linux" (`mesa:docs/drivers/vc4.rst:14-16`). So there is no GPU
  firmware to load or to keep out of the tree, as on the Pi 4 V3D and the
  GC7000, unlike the PowerVR. The VPU firmware (`start.elf`) still runs first
  and still owns clocks, power and the framebuffer tags; it is the boot loader's
  part, not ours.
- **ISA and command-stream knowledge** we would rely on, all in Mesa:
  - QPU encoding: `vc4_qpu_defines.h` (add and mul opcodes, `raddr` and
    `waddr` magic addresses, the signal bits, conditions, pack and unpack
    modes), `vc4_qpu.h`, `vc4_qpu.c` (the encoder), `vc4_qpu_disasm.c`
    (the decoder), `vc4_qpu_validate.c`, `vc4_qpu_schedule.c`,
    `vc4_qpu_emit.c`, `vc4_register_allocate.c`.
  - Compiler: NIR to QIR (`vc4_program.c`, `vc4_qir.c`, `vc4_opt_*.c`,
    `vc4_nir_lower_*.c`), then QIR to QPU. The entry for a standalone host
    tool is `vc4_shader_ntq` (`vc4_program.c:2180`), the way
    `tools/v3d/v3d_shc.c` calls Mesa's v3d compiler.
  - Control-list packets: `kernel/vc4_packet.h` is a C header with every packet
    id and size (`:28-81`, sizes `:85-120`), the bit fields of binner and render
    config (`:265-290`) and the load/store flags. There is **no XML** for vc4,
    unlike `src/broadcom/cle/v3d_packet.xml` for V3D and `genxml` for Mali.
    The user-space emitters are in `vc4_emit.c`, `vc4_draw.c`, `vc4_job.c`;
    the render list generator is `kernel/vc4_render_cl.c`.
  - Hardware rules the kernel enforces, readable as the list of "what the
    hardware does not do safely": `kernel/vc4_validate.c` and
    `kernel/vc4_validate_shaders.c` (TMU address clamping, uniform stream reset
    rules, branch and delay-slot rules, threaded-shader constraints).
  - Debug and capture: `VC4_DEBUG` flags `cl`, `qpu`, `qir`, `nir`, `shaderdb`,
    `dump`, `norast` (`vc4_screen.c:48-71`) and `vc4_cl_dump.c`.
  - A no-GPU harness: `mesa:src/broadcom/drm-shim/vc4_noop.c` (the submit ioctl
    is stubbed, enough "to make shader-db work", `README.md`).

## 3. What yakogl already has that applies

**Verdict: a new back end is needed.** No existing back end extends. What
carries over is the layer above the back ends and the method.

Carries over unchanged:

- The GL core, EGL, the state model and gl_test, through the `YakoglBackend`
  table (`repo:src/backend.rs:1341`).
- The shader front end: preprocessor, parser, sema and IR
  (`repo:src/shaderc/online/`), the `ShcShader` reflection, and the
  "a program, not a stage" rule: Mesa's vc4 also keys the vertex shader by
  the fragment shader's inputs (the `vattrs_live` and `vattr_offsets` of the
  compiled VS/CS are per program, `mesa:.../vc4/vc4_draw.c:152-208`), and
  there are three shaders per draw (FS, VS, CS), as on V3D (`repo:docs/online-shaderc/v3d-backend.md`).
- The shape of the V3D port: host-side register sequences, a binner list
  and a render list per frame, no firmware, an OSAL for memory and fences
  (`repo:docs/notes/pi4-v3d-bringup.md`, "What is reused"), the Zephyr image
  and the Pi firmware boot route (`repo:docs/standalone-boot.md`).

Why `src/v3d` does not extend (differences read from the Mesa sources):

| Area | VC4 (`mesa:.../vc4/`) | V3D 4.2 (`repo` notes, Mesa `src/broadcom`) |
|---|---|---|
| Packet language | `vc4_packet.h`: 8-bit opcode ids, e.g. `GL_SHADER_STATE` = 64, `TILE_BINNING_MODE_CONFIG` = 112, `TILE_RENDERING_MODE_CONFIG` = 113, `CLEAR_COLORS` = 114, `STORE_TILE_BUFFER_GENERAL` = 28 (`vc4_packet.h:28-74`) | `v3d_packet.xml` with different ids and fields (`TILE_BINNING_MODE_CFG`, `TILE_RENDERING_MODE_CFG_*`, `STORE_TILE_BUFFER_GENERAL` with a layer model) |
| Frame structure | one binning list; the render list is built by the driver and the hardware **runs a tile list per tile**, with 32x32 pixel tiles when 4x multisample else 64x64 (`vc4_job.c:241-245`, `kernel/vc4_gem.c:146-150`); tile state array 48 bytes per tile, 4 KiB aligned, tile alloc starts at 32 bytes per tile plus 1 MiB of spare (`kernel/vc4_validate.c:336-415`) | tile alloc 128 bytes per tile and a 256 byte TSDA entry per tile (`repo:docs/notes/pi4-v3d-bringup.md` "Control lists") |
| Shader record | `GL_SHADER_STATE_RECORD` of `36 + 8 * nattr` bytes (`100 + 4 * nattr` in the extended form), three code pointers at offsets 4, 16, 28, per attribute a 8-byte attribute record (`kernel/vc4_validate.c:135-147`, `:738-742`); the record's low 3 bits of the address carry the attribute count (`:748`) | a 36-byte record with 3 shader pointers and uniform-stream addresses, different layout |
| Vertex handling | vertex data is fetched by the **VCD** into the VPM, per-attribute offsets for the CS and the VS (`vc4_draw.c:201-208`); the VS and CS read inputs through `VPM` reads and write outputs back with VPM writes | V3D 4.2 uses a VPM too, but with a different setup; shaders loaded by `ldvpm`/vertex-cache packets (not re-read here) |
| GPU addresses | physical, no MMU (above) | MMU, 4 KiB pages, page table we own |
| Uniforms | the driver builds a **uniform stream per shader** consumed in order by `QPU_R_UNIF`, with the stream address reset by writes to `QPU_W_UNIFORMS_ADDRESS` and the `texture config` words implicit (`vc4_qpu_defines.h`, `QPU_R_UNIF` and `QPU_W_UNIFORMS_ADDRESS`; `vc4_qir.h:217-277`) | the same idea; different `ldunif` rules |
| Blend | no fixed-function blend unit in the packet list (there is no blend packet in `vc4_packet.h:28-74`); blending is **in the fragment shader** (`vc4_nir_lower_blend.c`, using `QPU_SIG_COLOR_LOAD` to read the tile buffer, `vc4_qpu_defines.h` signals) | fixed function `BLEND_CFG` |
| Shader ISA | 64-bit words, **two register files A and B of 32 registers** read through `raddr_a`/`raddr_b`, accumulators `r0..r3` writable and `r4`, `r5` special, 16-way SIMD over 4 physical lanes, add and mul pipes with `pack` and `unpack` modes and per-lane flags `ZS/ZC/NS/NC/CS/CC` (`vc4_qpu_defines.h:30-65` ops, `:157-` conditions). Branches are 4 delay-slot relative branches (`vc4_validate_shaders.c:652`) | register file plus accumulators r0-r5, signals `ldunif`, `ldvary`, `ldtmu`, `thrsw` and different magic addresses (`repo:driver/docs/rpi4-v3d.md` "Shader compiler") |
| Add pipe ops | 24 ops incl. `FADD FSUB FMIN FMAX FMINABS FMAXABS FTOI ITOF ADD SUB SHR ASR ROR SHL MIN MAX AND OR XOR NOT CLZ V8ADDS V8SUBS`; the mul pipe has `FMUL MUL24 V8MULD V8MIN V8MAX V8ADDS V8SUBS` (`vc4_qpu_defines.h:30-65`). **No 32-bit integer multiply** (only `MUL24`), no divide, no hardware `sin`/`cos`/`pow`: the SFU only has recip, rsqrt, exp, log (`QPU_W_SFU_RECIP` to `QPU_W_SFU_LOG`, `:106-109`) | different set |
| Texture unit | two TMUs, the sample is requested by writing `S`, `T`, `R`, `B` to `QPU_W_TMU0_*`/`TMU1_*`, with two uniform words of config per sample (`vc4_qir.h` `QUNIFORM_TEXTURE_CONFIG_P0/P1`) | TMU with a different config, TFU for copies |
| Thread model | `THRSW`, at most two fragment-shader threads, constrained by the register allocator (`vc4_register_allocate.c:338-340`, `reg_class_*[fs_threaded]`); VS and CS are single-threaded (`kernel/vc4_validate.c:805-809`) | up to 4 threads |

The two compilers are therefore independent below the IR. The V3D back end's
`prepare/select/sched/alloc` (`repo:src/shaderc/online/v3d/`, about 25 000 lines
with `chain.rs`) cannot be reused as code. What is reusable is the structure:
an ISA model with an encoder, a decoder and a round-trip test against the
reference compiler's words; a selector from the shared IR; a scheduler and an
allocator; a simulator for the unit tests (`src/shaderc/online/v3d/sim.rs`).
The reference tool pattern is `tools/v3d/v3d_shc.c`; for VC4 it would call
`vc4_shader_ntq` on SPIR-V-made NIR as `v3d_shc` does.

`src/shaderc/online/mali`, `vivante`, `powervr` are further from this GPU than
V3D is (different execution model: Mali Valhall and the GC7000 are
scheduled differently; PowerVR needs a firmware).

## 4. The board on Zephyr

Read in `zephyr:` (4.4.99):

- **There is no BCM2710 or BCM2837 SoC and no Zero 2 W or Pi 3 board.**
  `zephyr:boards/raspberrypi/` holds `rpi_4b`, `rpi_5`, `rpi_pico`,
  `rpi_pico2`, `rpi_debug_probe`; `zephyr:soc/brcm/` holds `bcm2711`,
  `bcm2712` and `bcmvk`; `zephyr:dts/arm64/broadcom/` holds `bcm2711.dtsi`,
  `bcm2712.dtsi` and `viper-a72.dtsi`. A recursive search of `dts`, `drivers`,
  `soc` and `boards` for `bcm2835|bcm2836|bcm2837|bcm2710` found only one
  text mention in `boards/qemu/cortex_a72/doc/index.rst`.
- **The Pi 4 board is a template, not a base we can reuse.** `bcm2711` selects
  `CPU_CORTEX_A72` and the ARM architectural timer
  (`zephyr:soc/brcm/bcm2711/Kconfig`), puts the peripherals at `0xfe000000`
  and the GIC-400 (`arm,gic-v2`) at `0xff841000`
  (`zephyr:dts/arm64/broadcom/bcm2711.dtsi`, node `gic`), the mailbox at
  `0xfe00b880` with GIC SPI 33, and loads at `0x200000`
  (`zephyr:boards/raspberrypi/rpi_4b/rpi_4b.dts`, `dram0`).
  The Zero 2 W's addresses differ: the BCM2837 peripherals are expected at
  `0x3f000000` and the ARM-local block at `0x40000000` (unread: Linux
  `bcm2837.dtsi`, `bcm2836-l1-intc`). The interrupt controller is **not a GIC**
  on this SoC family (unread; question Q4).
- **Zephyr has the hook for a non-GIC arm64 SoC.**
  `CONFIG_ARM_CUSTOM_INTERRUPT_CONTROLLER` (`zephyr:arch/arm64/Kconfig:17-29`)
  maps `arch_irq_*` to the SoC layer; the vector code calls
  `z_soc_irq_get_active` and `z_soc_irq_eoi` instead of the GIC
  (`zephyr:arch/arm64/core/isr_wrapper.S:61-65,112-116`), and
  `z_arm64_interrupt_init` calls `z_soc_irq_init`
  (`zephyr:arch/arm64/core/irq_init.c:27-30`). The ARM arch timer driver
  depends on `GIC || ARM_CUSTOM_INTERRUPT_CONTROLLER`
  (`zephyr:drivers/timer/Kconfig.arm_arch:8`). Boards that already use it for
  arm64 are the Renesas RZ family and TI `am64x`, `j722s`
  (`zephyr:soc/ti/k3/am64x/Kconfig:23`). So a new SoC `bcm2837` can select
  `CPU_CORTEX_A53` + `ARM_ARCH_TIMER` + `ARM_CUSTOM_INTERRUPT_CONTROLLER` and
  write a small controller for the two levels (the ARM-local routing of the
  timer and mailboxes, and the VideoCore "armctrl" bank with the V3D line).
  Cortex-A53 CPU nodes already exist in several arm64 dtsi
  (`zephyr:dts/arm64/nxp/nxp_mimx8mp_a53.dtsi`, `qemu/qemu-virt-a53.dtsi`).
- **What the GPU and display need on Zephyr**, from the Pi 4 work we have: the
  firmware mailbox for the clock and the power state, the framebuffer tag, a
  watchdog (`repo:docs/standalone-boot.md` "Raspberry Pi 4", `rpi_mbox.c` and
  `rpi_wdog.c` named there, `repo:docs/notes/pi4-v3d-bringup.md` "Power-on").
  For the Zero 2 W the V3D register block address, its interrupt line, the
  power domain and clock id are **unread**: Linux `bcm283x.dtsi` (`v3d`),
  `drivers/pmdomain/bcm/bcm2835-power.c` and
  `drivers/firmware/raspberrypi.c` answer them (Q3, Q5). Zephyr's Pi 4
  firmware driver (`firmware/rpi_fw`, mailbox `brcm,bcm2711-mbox`) is tied to
  the bcm2711 mailbox; the BCM2837 mailbox has the same register layout
  expected from the family, unread (Q6).
- **Boot route for sign-off**: the vendor firmware loads our image from the
  card, as on the Pi 4 (`repo:docs/standalone-boot.md`, "The product form").
  A `config.txt` with `kernel=`, `arm_64bit=1`, `enable_uart=1` and
  `kernel_address` if the load address differs from `0x200000`
  (the keys are documented at raspberrypi.com `config_txt`, linked in
  `docs/standalone-boot.md:337-341`; the Pi 3 family default load address is
  unread, Q7). U-Boot is not needed and the Pi 4 route does not use it. A
  kernel-based jump (Linux then `kexec` into our image) is development only.
- **Memory map facts that Zephyr needs** (unread, Q7): where the firmware's
  GPU reserve is (`gpu_mem`), which bus alias the V3D uses to see RAM
  (cached or uncached), and the cache maintenance that results.
  The Pi 4 board reserves the first 2 MiB for the firmware and 8 MiB at the top
  for the VideoCore (`zephyr:boards/raspberrypi/rpi_4b/rpi_4b.dts`, `dram0` and
  `vc_mem`).

## 5. What can be done before the board arrives

Each item has its check. None needs the board or a vendor SDK.

1. **Reference build of Mesa vc4 on the host with the vc4 drm-shim.** The method
   of `tools/mali/drm_shim_ref.sh` (build Mesa 26.2.1 with
   `-Dgallium-drivers=vc4`, EGL, GLES2 and the drm-shim; run a one-draw GL
   program with `MESA_LOADER_DRIVER_OVERRIDE=vc4` and
   `LD_PRELOAD=libvc4_noop_drm_shim.so`, `mesa:src/broadcom/drm-shim/README.md`).
   Output: the shader binaries (`VC4_DEBUG=qpu,shaderdb,nir,qir`) and the
   control lists and records (`VC4_DEBUG=cl`, `vc4_cl_dump.c`; the submit ioctl
   still receives `bin_cl`, `shader_rec`, `uniforms` from the driver,
   `vc4_drm.h:91-`). Check: the build reproduces on a clean tree and a recorded
   program's QPU words and packets are stored as test data. **Open point:** the
   noop shim stubs the submit; whether the bytes are reachable only through
   `VC4_DEBUG=cl` or by wrapping the ioctl is to be measured (Q9).
2. **`vc4_shc` tool** (like `tools/v3d/v3d_shc.c`): Mesa's vc4 compiler run on
   our SPIR-V for a program (FS, VS, CS), emitting QPU words and uniform
   stream descriptions as JSON. Check: runs on the gl_test and femtovg-style
   corpus and gives byte-identical words on a second run.
3. **ISA model, encoder, decoder** (`src/shaderc/online/vc4/isa.rs`
   style). Check: a byte-exact decode/encode round trip over every word
   `vc4_shc` produces plus `vc4_qpu_disasm.c` vectors, as for V3D
   (`repo:docs/online-shaderc/v3d-backend.md`). A host `cargo test` row.
4. **Packet encoders** for the binning and render lists in Rust or C with
   golden tests against the Mesa-recorded lists. Check: byte equality with the
   recorded `cl` dumps; a `tools/hostharness`-style row (as the V3D
   `cl_golden.c`).
5. **QPU interpreter** for the unit tests of the compiler (as `v3d/sim.rs`),
   validated on Mesa's words.
6. **Zephyr SoC and board definition built but not run**: `soc/brcm/bcm2837`
   (A53 cores, custom interrupt controller shell), a `rpi_zero_2w` board with
   a PL011 console and a flat memory map from the Linux dts, building an
   image of the existing samples. Check: `west build` produces the image in the
   gate's Zephyr rows as a new row, or a gap named, never a pass without
   build. It is still unrun until the board arrives.
7. **Host harness row in `tools/check.sh`**: a `vc4` scope like `v3d`
   (`repo:tools/check.sh:158` maps paths to a scope; `:1419-1486` shows the
   rows for `gpu-v3d`) running the lib tests with a `gpu-vc4` feature, the
   QPU round trip and the packet golden test.
8. **Read the unread sources** (section 1) and turn the unknowns of section 7
   into cited facts. Check: this note's unknown list shrinks, each answer
   with its file and line.

## 6. What needs the board

The three things to try on day one, in order:

1. **Firmware boot and UART.** Put the Zephyr hello image on the card, with the
   `config.txt` above, and read the console. Measures: the load address
   works, the cores that run, the clock rate the firmware gave
   (`CNTFRQ_EL0`, the timer tick).
2. **Mailbox and V3D identity.** Ask the firmware for the V3D clock and power,
   read `V3D_IDENT0..2` (expected `IDENT0` to hold the "V3D" tag with
   version 2 in its top byte as Mesa's shim reports, `vc4_noop.c:85`), and
   try a reset. Measures: the register block is where the dtsi says, the
   interrupt line, whether the bus alias for cache coherence is right.
3. **First clear, then first triangle.** A binner list with one clear (a tile
   buffer clear and store to a firmware framebuffer, no shader), then one
   triangle with a fixed shader pair from `vc4_shc`. Measures: the clear colour
   on the HDMI capture card; the triangle's pixels against the Mesa reference
   frame on the host (the `frame_check.c` pattern, `repo:tools/v3d/frame_check.c`).

The port is alive when: the first clear reads back the colour written
(frame store and display path), the first triangle reads back the expected
pixels, and the first `gl_test` case passes through the product path
(gl_test case 1 and the plain triangle cases, then the sweep).

## 7. Risks and unknowns

An unknown stays an unknown. Each is a question, with what would answer it.

- **Q1. What are the V3D register offsets and the bin/render start sequence on
  VC4?** The Pi 4 note takes them from Linux `v3d_*.c`. Answer: Linux
  `drivers/gpu/drm/vc4/vc4_regs.h` and `vc4_gem.c`
  (`vc4_submit_next_bin_job`, `vc4_submit_next_render_job`), the Broadcom
  architecture guide. Not read.
- **Q2. What does the hardware do that the kernel validator guards against?**
  `kernel/vc4_validate_shaders.c` refuses direct TMU reads without a clamp,
  general VPM DMA, uniform-address changes that are not an unconditional ADD,
  branches inside delay slots; which of these are correctness hardware bugs and
  which are only isolation for a multi-user kernel? We are single user, but
  the rules we break by accident may hang the GPU. Answer: the validator
  comments, the Linux commit messages of `vc4_validate_shaders.c`, a hang on
  the board.
- **Q3. What clock and power do we ask the firmware for?** The Pi 4 path is
  property tags `SET_CLOCK_RATE`/`SET_CLOCK_STATE` and a PM/ASB bridge
  (`repo:docs/notes/pi4-v3d-bringup.md`). On BCM2837 Linux powers the V3D
  through `bcm2835-power.c` or the firmware domain. Answer: `bcm283x.dtsi`
  (`v3d`, `power`), `bcm2835-power.c`, `raspberrypi.c`.
- **Q4. What is the interrupt route for the V3D line and the timer?** Not a GIC
  as far as we know (unread). Answer: `bcm2836-l1-intc`,
  `irq-bcm2835.c`, `bcm2837.dtsi`; Zephyr side `z_soc_irq_*` (section 4).
  Risk: this is the first new interrupt controller on arm64 Zephyr in this
  project; the Zephyr tests of `ARM_CUSTOM_INTERRUPT_CONTROLLER` exist for
  other SoCs only.
- **Q5. Where does the GPU see RAM, and what cache flush does the CPU need?**
  The bus address of ARM RAM, the cache alias options, and whether the A53
  data cache must be cleaned before a job. Answer: Linux `dma-ranges` of
  `bcm283x.dtsi`, `vc4_gem.c`, the architecture guide.
- **Q6. Is the BCM2837 mailbox the same register layout as the BCM2711's?**
  Zephyr's `brcm,bcm2711-mbox` and `rpi_fw` are tied to the 2711; the Pi 4
  images had a conflict with Zephyr's own mailbox node
  (`repo:docs/notes/pi4-v3d-bringup.md`, header). Answer: Linux
  `bcm2835-mailbox.c`, `bcm283x.dtsi`.
- **Q7. What is the load address, the memory split and the DTB handling of the
  Zero 2 W firmware with `kernel=` and `arm_64bit=1`?** Answer: the
  `config.txt` documentation and the first boot.
- **Q8. Does the Zero 2 W firmware give a framebuffer through the property
  mailbox on HDMI as the Pi 4 does, and in what pixel format (the render list
  can store RGBA8888 or BGR565, `vc4_packet.h:192-196`)?** Answer: the
  firmware property interface documentation, the board.
- **Q9. Can the vc4 drm-shim give the bytes of the lists and the shader records
  on the host?** Answer: build it and run item 1 of section 5; if the noop
  shim does not expose them, wrap `DRM_IOCTL_VC4_SUBMIT_CL` with an
  `LD_PRELOAD` that records `struct drm_vc4_submit_cl` (`vc4_drm.h:91-`).
- **Q10. Are vertex-stage texture look-ups supported?** Mesa's vc4 sets one
  sampler cap for both stages (`vc4_screen.c:159-160`) but we did not find the
  decision. The ES 2.0 minimum is `GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS = 0`. The
  PowerVR and Mali notes record vertex-texture cases
  (`repo:docs/notes/es2-pvr-vertex-texture.md`,
  `repo:docs/notes/mali-texture-refusals.md`). Answer: `ntq_emit_tex` in
  `vc4_program.c:400`, the architecture guide.
- **Q11. Fixed-point precision of the QPU against dEQP.** The QPU float is
  not IEEE (no denormals, truncation in some paths: unread). Mesa and the
  hardware passed GLES2 conformance (`vc4.rst:21-22`), but the dEQP-GLES2
  precision groups against *our* shaders are untested. Answer: the first
  sweep.
- **Q12. How slow is the CPU side?** Four A53 at a clock the firmware sets (the
  Pi 4's A72 at 1.5 GHz, `rpi_4b.dts`; this board's firmware clock is unread)
  with 512 MB shared. The AM62P's CPU cores are Cortex-A53 too, and the fixed cost of a
  `glDrawElements` by step is measured there
  (`repo:docs/notes/gl-draw-fixed-cost.md`); the new part is the memory: textures, the binner memory (32 bytes of initial tile alloc per tile plus 1 MiB spare,
  `vc4_validate.c:380-401`) and the framebuffer in 512 MB with the GPU share.
- **Q13. The framebuffer size and refresh the panel will use** (the Pi 4
  setup has a 1920x720 bar panel in `config.txt`) is unknown for the Zero 2 W's
  HDMI mini port; this is a board setup question, not a design one.
- **Q14. What does ES 2.0 on VC4 cost in lost features?** From Mesa: 2048
  textures, no 3D textures, 8 varyings, no occlusion queries, 16-bit indices
  only (`vc4.rst:27-35`), colour buffers only RGBA8888 and BGR565 in the
  render config (`vc4_packet.h:192-196`, `vc4_job.c:369-371`). The dEQP-GLES2
  not-supported list for the three boards is 34 cases
  (`repo:docs/es2-cts-failures.md`, section C); the VC4 list is not known
  until a sweep. ES 3.0 is out of reach as Mesa exposes this GPU (no 3D textures at all,
  `vc4_screen.c:197`; Mesa's driver is ES 2.0 and OpenGL 2.1), so stage 4
  does not apply to it.

## 8. A proposed item list

Sized against the three finished ports (`repo:tools/cts/STATUS.md`,
`repo:CHANGELOG.md`). The Pi 4 is the closest: its plan started 2026-09-23,
M1 to M6 (first triangle through the GL back end) passed on the board
2026-09-24 (`repo:docs/notes/pi4-v3d-bringup.md` header), and dEQP-GLES2 read 0
failures by 2026-10-15/16 (STATUS.md). The first 2 days were cheap because the
compiler, the sim and the harness were built on the host first. The sizes
below are S (a day or less), M (several days), L (a week or more) in units of
that port, not calendar promises.

| # | Item | Size | Check |
|---|---|---|---|
| 0 | Read the unread sources; update this note's unknowns | S | each Q has a cited answer |
| 1 | `tools/vc4/`: Mesa vc4 + drm-shim build script, `vc4_shc`, a list recorder | M | clean build; stored reference words and lists |
| 2 | QPU ISA model, encoder, decoder, round trip (`src/shaderc/online/vc4/`) | M | round trip over the corpus and `vc4_qpu_disasm.c` vectors |
| 3 | Packet and record encoders for bin and render lists; golden test | M | byte equality with item 1 |
| 4 | Zephyr SoC `bcm2837`, custom interrupt controller shell, board `rpi_zero_2w`, build-only | M | image builds in a gate row |
| 5 | Driver: mailbox clock and power, reset, bin/render start, completion interrupt, hang reset, hardware mock test | M | host mock test; day-one board checks 1 and 2 |
| 6 | Board day one: UART, identity, clear, triangle with Mesa's own shaders | S (board) | section 6 |
| 7 | Compiler: select, schedule, allocate for FS/VS/CS, uniform stream, VPM I/O, SFU; no 32-bit multiply, no divide | L | corpus words vs `vc4_shc`; QPU sim; gl_test shader cases |
| 8 | Back end `src/vc4`: draw, state, clears, depth and stencil, blend in the FS, cull, point and line variants, scissor | L | gl_test sweep |
| 9 | Textures: T-format and linear layouts (`vc4_tiling*.c`), TMU config, mipmaps, cube, NPOT, ETC1 if the card has it | M | texture host checks vs Mesa; gl_test texture cases |
| 10 | Display: firmware framebuffer, swap, vsync fence | S | panel |
| 11 | MSAA 4x (32x32 tiles, decimate), `glReadPixels`, `glBlitFramebuffer`-like paths | M | gl_test, CTS msaa rows |
| 12 | dEQP-GLES2 stage 1 on the board, the not-supported list, fixes | L | zero failures; accepted not-supported listed |
| 13 | glmark2 33 scenes, performance on 512 MB | M | RESULTS.md row |

Items 1 to 5 need no board and can all land before it arrives; item 6 is the
first board session; the rest follow the Pi 4's order. The largest risks are
Q4 (interrupts), Q5 (cache and address), and item 7 (the QPU has fewer
operations than the other GPUs, so more shader shapes need multi-instruction
sequences).
