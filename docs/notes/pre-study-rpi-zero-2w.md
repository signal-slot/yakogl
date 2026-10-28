<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: Raspberry Pi Zero 2 W (BCM2710A1 / RP3A0, VideoCore IV)

Status 2026-10-28, written before the board arrives. Nothing here has run on
silicon. This note says what an ES 2.0 port to the Zero 2 W costs, what it
brings, what can be done on the host first, and what is still unknown.

## How to read the citations

- `mesa:<path>` is a file of Mesa 26.2.1 (the tree the Mali and V3D reference
  builds use, `tools/mali/drm_shim_ref.sh`, `tools/v3d/README.md`), read on the
  host. Line numbers are exact. Upstream code is read, not copied.
  `mesa:src/gallium/drivers/vc4/kernel/` is Mesa's userspace mirror of the
  Linux `vc4` kernel validator (its `README` says so): the kernel's view of the
  packets, but a mirror, not the kernel tree.
- `linux:<path>` is the Linux tag v6.18 on `raw.githubusercontent.com/torvalds/linux`.
  These files were read through a fetch tool that returns a summary of the
  page, not the raw bytes. A line number written `~N` is the summary's
  estimate and must be confirmed against the tag; a quoted line (a register
  offset, a `compatible`, a macro) is the tool's quotation of the file. The
  device-tree facts of `bcm2835-common.dtsi`, `bcm283x.dtsi` and `bcm2837.dtsi`
  were read this way too.
- `zephyr:<path>` is the Zephyr workspace the Pi 4 images build with
  (`VERSION` 4.4.99), read on the host.
- `repo:<path>` is this repository at the commit this note was written on.
- `bcm-guide` is *VideoCore IV 3D Architecture Reference Guide*,
  VideoCoreIV-AG100-R, 16 September 2013, "Initial release"
  (`https://docs.broadcom.com/doc/12358545`; the cover and revision page were
  read). Only its outline was read through the fetch tool, so it is cited by
  section name (3 QPU instruction set, 4 TMU, 7 VPM and VCD, 9 control lists,
  10 V3D registers, appendix A errata, appendix B base addresses), never for a
  value. Mesa's driver page calls this link "a specification for the 21553";
  the document at the link is the VideoCore IV guide.
- `rpidoc:` is `raspberrypi.com/documentation/computers/config_txt.html` and
  `.../raspberry-pi.html`, read through the same tool; only the first 100000
  characters of each were returned, so `kernel_address`, `kernel_old`,
  `gpu_mem` and the Zero 2 W specification table were not seen.
- **Not read**: the body of the Architecture Guide, the BCM2835 ARM Peripherals
  datasheet, the rest of the two Raspberry Pi pages, and the closed firmware.
  Each fact that would come from them is a named question in section 7. The
  Pi 4 boot documentation `repo:docs/standalone-boot.md` cites the `config.txt`
  keys and is used for the keys it covers.

## 1. The hardware

| Part | What | Source |
|---|---|---|
| Board | Zero 2 W: the Linux device tree has `compatible = "raspberrypi,model-zero-2-w", "brcm,bcm2837"`, includes `bcm2837.dtsi`, and `memory@0` with `reg = <0 0x20000000>` (512 MB) | `linux:arch/arm/boot/dts/broadcom/bcm2837-rpi-zero-2-w.dts` (the arm64 path is a one-line include of it) |
| CPU | four Cortex-A53, started by `spin-table` with release addresses `0xd8`, `0xe0`, `0xe8`, `0xf0` | `linux:arch/arm/boot/dts/broadcom/bcm2837.dtsi` (`cpus`) |
| Bus map | peripheral bus `0x7e000000` is ARM physical `0x3f000000` (16 MB), the ARM-local block `0x40000000` (4 KB); DMA masters see ARM physical 0 at bus `0xc0000000` (`dma-ranges = <0xc0000000 0x00000000 0x3f000000>`) | `bcm2837.dtsi`, `soc` |
| GPU | V3D at bus `0x7ec00000`, size `0x1000`, `compatible = "brcm,bcm2835-v3d"`, `interrupts = <1 10>` (bank 1, bit 10: see Q4 for the number) | `linux:.../bcm2835-common.dtsi`, node `v3d` |
| GPU identity | the kernel checks `V3D_IDENT0` against `V3D_EXPECTED_IDENT0` = `(2 << 24) \| 'V' \| '3' << 8 \| 'D' << 16` (a "V3D" tag with technology version 2 in the top byte); Mesa's drm-shim answers `IDENT0 = 0x2000000`, `IDENT1 = 1`, "a VC4 2.1 device" | `linux:drivers/gpu/drm/vc4/vc4_regs.h:36-41`, `vc4_v3d.c` `vc4_v3d_bind` (~420), `mesa:src/broadcom/drm-shim/vc4_noop.c:85-86`, `mesa:src/broadcom/drm-shim/README.md` ("This will be a VC4 2.1 device") |
| GPU memory | physical addresses only, no GPU MMU. Mesa's validator mirror writes `bo->paddr + offset` for the three shader code addresses and writes the tile state and tile alloc addresses as plain physical addresses; the kernel driver's bin memory is a 16 MB CMA buffer that must not cross a 256 MB boundary (the address must fit the hardware's top bits) | `mesa:.../vc4/kernel/vc4_validate.c:399-410` (tile BO), `:326-330`; `linux:.../vc4_v3d.c` `bin_bo_alloc` (~226). Contrast V3D 4.2, which has a one-level MMU we program (`repo:docs/notes/pi4-v3d-bringup.md`, "MMU (M2)") |
| Display | HDMI only on this board (the dts enables `hdmi` with `power-domains = <&power RPI_POWER_DOMAIN_HDMI>`). In the Linux driver a frame reaches the screen as follows: planes write an HVS (hardware video scaler, bus `0x7e400000`, `0x6000` bytes, `interrupts = <2 1>`) display list, the HVS feeds a FIFO to a pixelvalve (`0x7e206000`, `0x7e207000`, `0x7e807000`) and then the HDMI encoder (`0x7e902000`) | `linux:.../vc4_hvs.c` (`vc4_hvs_atomic_flush` ~893 writes planes then `SCALER_CTL0_END`; `vc4_hvs_install_dlist` ~802 writes `SCALER_DISPLISTX`; `vc4_hvs_get_fifo_from_output` ~507), `bcm283x.dtsi` (`hvs`), `bcm2835-common.dtsi` (pixelvalve, `hdmi`) |
| Display, the way we would use | not the HVS: the firmware owns the display at boot, the kernel driver tells it to stop with `RPI_FIRMWARE_NOTIFY_DISPLAY_DONE` before it takes over, and it keeps the firmware's boot display list (the dlist allocator starts at `HVS_BOOTLOADER_DLIST_END`). We keep the firmware's scan-out and ask for a framebuffer with the property tags `ALLOCATE` `0x00040001`, `SET_PHYSICAL_WIDTH_HEIGHT` `0x00048003`, `SET_DEPTH` `0x00048005`, `GET_PITCH` `0x00040008`, as the Pi 4 image does | `linux:.../vc4_drv.c` `vc4_drm_bind` (~350-375), `vc4_hvs.c` `__vc4_hvs_alloc` (~1135), `linux:include/soc/bcm2835/raspberrypi-firmware.h`; `repo:docs/standalone-boot.md` "Raspberry Pi 4". That this firmware offers the tags on this board is expected, not read (Q8) |
| Power and clocks | the firmware mailbox (`mailbox@7e00b880`, `brcm,bcm2835-mbox`, `reg <0x7e00b880 0x40>`, `interrupts <0 1>`); the `raspberrypi,bcm2835-firmware` node (`mboxes = <&mailbox>`), a `raspberrypi,bcm2835-power` node (`firmware = <&firmware>`) giving firmware power domains, and `RPI_POWER_DOMAIN_V3D` = 10; clock id `V3D` after `EMMC=1` (the Pi 4 note gives it as 5); `cprman@7e101000` | `linux:bcm283x.dtsi` (`mailbox`, `clocks`), `linux:bcm2835-rpi.dtsi`, `linux:include/dt-bindings/power/raspberrypi-power.h`, `linux:raspberrypi-firmware.h`; `repo:docs/notes/pi4-v3d-bringup.md:89` |
| Boot chain | before our image: ROM, then (Pi 3 family) the firmware files from the card. The Pi 4's chain (SPI EEPROM, `start4.elf`, `kernel=`) is documented in `repo:docs/standalone-boot.md:303-340`; that the Pi 3 family ROM loads `bootcode.bin` from the boot partition is read (`rpidoc`, boot sequence); that `start.elf` follows is **not read** in that text (Q7) | `rpidoc` raspberry-pi page |

Public documentation: Broadcom's VideoCore IV 3D Architecture Reference Guide
(`bcm-guide`, 2013), linked from Mesa's driver page together with a snapshot
of the matching Android driver (`mesa:docs/drivers/vc4.rst:285-293`, links
`docs.broadcom.com/doc/12358545` and `/docs/12358546`). Its outline covers the
QPU instruction encoding (ALU, branch, semaphore, load-immediate, condition
codes, pack/unpack, small immediate, signalling bits, instruction
restrictions), the TMU, the VPM and VCD, the control record ids, shader state
and primitive list formats, and the V3D register map (executor, identity,
interrupt, cache, scheduler registers), plus errata and base addresses. The
BCM2835 ARM Peripherals datasheet was not fetched; BCM2837 differences from it
are unread. The kernel driver and Mesa remain what we read in detail.
Boot, from `rpidoc`: the boot ROM loads `bootcode.bin` from the boot partition
on models without an EEPROM (the Pi 3 family; Pi 4 has the EEPROM). The Zero 2 W
is in the list of models that accept `arm_64bit`, which defaults to 0 there
(1 only on the Pi 4, 400, CM4, CM4S); the default `kernel` there is
`kernel7.img`.

## 2. The open-source stack today

- **Mesa** `src/gallium/drivers/vc4/`: "a nearly conformant GLES2 driver, and
  the hardware has achieved GLES2 conformance with other driver stacks"
  (`mesa:docs/drivers/vc4.rst:18-22`). It also exposes OpenGL 2.1 with caveats:
  4-byte index buffers are truncated to 2 bytes, no occlusion queries, no
  primitive-mode reduction (`vc4.rst:26-60`). Screen limits in
  `mesa:.../vc4/vc4_screen.c`: 2D texture 2048 (`:195`), no 3D textures
  (`:197`), 8 varyings (`:199`), shader inputs 8 (`:152`), outputs 8 for the
  vertex stage and 1 for the fragment stage (`:153`), constant buffer 0 of 16 K
  floats (`:155`), 16 samplers (`vc4_screen.h:58`), 12 mip levels
  (`vc4_screen.h:57`), no clip planes (`:212`), no texrect (`:210`).
- **Kernel** `linux:drivers/gpu/drm/vc4/`: one driver for the 3D block and the
  display (`vc4_drv.c`: `brcm,bcm2835-vc4` is generation 4, `bcm2711-vc5` is 5;
  it binds `vc4_hvs`, `vc4_hdmi`, `vc4_vec`, `vc4_dpi`, `vc4_dsi`, `vc4_txp`,
  `vc4_crtc`, `vc4_v3d` in that order). The submit ABI is
  `mesa:include/drm-uapi/vc4_drm.h:91` (`struct drm_vc4_submit_cl`: binner
  control list, shader records, uniforms, BO handles, the tile rectangle, the
  render surfaces, clear values) and the parameters `V3D_IDENT0..2`, branches,
  ETC1, threaded FS, fixed RCL order (`:325-333`). The kernel's job sequence is
  small (read from `linux:.../vc4_gem.c`, `vc4_irq.c`, `vc4_v3d.c`):
  - Before a bin job: `vc4_flush_caches` writes `V3D_L2CACTL` with `L2CCLR`
    and `V3D_SLCACTL` with the four clear fields (T1CC, T0CC, UCC, ICC)
    (~412). Before a render job only the texture caches (T1CC, T0CC) (~432).
  - `submit_cl(n, ca, ea)` writes `V3D_CTnCA` (start address) and then
    `V3D_CTnEA` (end address); the write of the end address starts the thread.
    Thread 0 is the binner list, thread 1 the render list
    (`vc4_submit_next_bin_job` ~450, `vc4_submit_next_render_job` ~493).
  - Registers (offsets as written in the file): `V3D_IDENT0` `0x000`,
    `L2CACTL` `0x020`, `SLCACTL` `0x024`, `INTCTL` `0x030`, `INTENA` `0x034`,
    `INTDIS` `0x038`, `CT0CS` `0x100`, `CT1CS` `0x104`, `CT0EA` `0x108`,
    `CT1EA` `0x10c`, `CT0CA` `0x110`, `CT1CA` `0x114`, `PCS` `0x130`, `BFC`
    `0x134`, `RFC` `0x138`, `BPCA` `0x300`, `BPCS` `0x304`, `BPOA` `0x308`,
    `BPOS` `0x30c`, `SRQPC` `0x430` (`linux:.../vc4_regs.h:36-130`).
  - Interrupts: `V3D_INT_FRDONE` bit 0, `FLDONE` bit 1, `OUTOMEM` bit 2,
    `SPILLUSE` bit 3 (`vc4_regs.h:78-81`). `FLDONE` (binner done) hands the job
    to the render thread, `FRDONE` completes the frame. On `OUTOMEM` the handler
    masks it, and a work item writes a new overflow slot into `V3D_BPOA`
    (bus address) and `V3D_BPOS` (size), clears the bit and unmasks it
    (`vc4_irq.c`: `vc4_irq` ~196, `vc4_overflow_mem_work` ~56-108). The
    overflow memory is a 16 MB buffer in 512 KB slices (`vc4_v3d.c`
    `bin_bo_alloc` ~226).
  - Hang: a 100 ms timer reads `V3D_CTnCA`; if neither thread moved, the
    driver power-cycles the V3D and re-enables the interrupts
    (`vc4_gem.c` `vc4_hangcheck_elapsed` ~313, `vc4_reset` ~277).
  - Init: the only register `vc4_v3d_init_hw` writes is `V3D_VPMBASE = 0`
    ("take all the memory that would have been reserved for user QPU
    programs") (`vc4_v3d.c` ~168).
- **Firmware needed by the GPU**: none. Mesa says it "makes no use of the
  closed source VPU firmware on the VideoCore IV block, instead talking
  directly to the GPU block from Linux" (`mesa:docs/drivers/vc4.rst:14-16`). The
  VPU firmware (`start.elf`) still runs first and owns clocks, power and the
  framebuffer tags; it is the boot loader's part, not ours.
- **ISA and command-stream knowledge** we would rely on, all in Mesa:
  - QPU encoding: `vc4_qpu_defines.h` (add ops `:30-55`, mul ops `:57-66`,
    read and write addresses from `:70`, signals from `:121`),
    `vc4_qpu.h`/`vc4_qpu.c` (encoder), `vc4_qpu_disasm.c` (decoder),
    `vc4_qpu_validate.c`, `vc4_qpu_schedule.c`, `vc4_qpu_emit.c`
    (`vc4_generate_code` `:592`), `vc4_register_allocate.c`. Mesa also keeps a
    newer `mesa:src/broadcom/qpu/` (`qpu_pack.c`, `qpu_disasm.c`), which is
    the V3D 3.x/4.x encoding, not VC4's.
  - Compiler: NIR to QIR (`vc4_program.c`, `vc4_qir.c`, `vc4_opt_*.c`,
    `vc4_nir_lower_*.c`), then QIR to QPU. The entry for a standalone host
    tool is `vc4_shader_ntq` (`vc4_program.c:2180`, the `fs_threaded`
    argument), the way `tools/v3d/v3d_shc.c` calls Mesa's v3d compiler.
  - Control-list packets: `mesa:.../vc4/kernel/vc4_packet.h` is a C header
    (no XML, unlike `src/broadcom/cle/v3d_packet.xml` for V3D): the opcode
    enum `:31-81` (for example `GL_SHADER_STATE` 64, `TILE_BINNING_MODE_CONFIG`
    112, `TILE_RENDERING_MODE_CONFIG` 113, `CLEAR_COLORS` 114,
    `STORE_TILE_BUFFER_GENERAL` 28), the sizes from `:84`
    (`TILE_BINNING_MODE_CONFIG_SIZE` 16, `GL_SHADER_STATE_SIZE` 5), the
    load/store and render formats at `:194-196` and `:303-305`. User-space
    emitters: `vc4_emit.c`, `vc4_draw.c`, `vc4_job.c`; the render list the
    kernel builds: `kernel/vc4_render_cl.c`.
  - Hardware rules the kernel enforces, readable as the list of what the
    hardware does not do safely: `kernel/vc4_validate.c`,
    `kernel/vc4_validate_shaders.c` (TMU address clamping, uniform stream,
    branch and delay-slot rules, threaded-shader rules).
  - Debug and capture: `VC4_DEBUG` flags and `vc4_cl_dump.c`
    (`mesa:.../vc4/vc4_screen.c`, debug table), and the no-GPU shim
    `mesa:src/broadcom/drm-shim/vc4_noop.c`: "the minimum of vc4 in order to
    make shader-db work. The submit ioctl is stubbed out"
    (`drm-shim/README.md`).

## 3. What yakogl already has that applies

**Verdict: a new back end is needed.** No existing back end extends. What
carries over is the layer above the back ends and the method.

Carries over unchanged:

- The GL core, EGL, the state model and gl_test, through the `YakoglBackend`
  table (`repo:src/backend.rs:1341`).
- The shader front end: preprocessor, parser, sema and IR
  (`repo:src/shaderc/online/`) and the `ShcShader` reflection. A program is
  three shaders as on V3D (`repo:docs/online-shaderc/v3d-backend.md`): Mesa's
  vc4 keys the coordinate and vertex shader by the fragment shader's inputs.
- The shape of the V3D port: host-side register sequences, a binner list and
  a render list per frame, no firmware on the GPU, an OSAL for memory and
  fences (`repo:docs/notes/pi4-v3d-bringup.md`, "What is reused"), the Zephyr
  image and the Pi firmware boot route (`repo:docs/standalone-boot.md`).
- **The power-on code of the Pi 4**: the GRAFX/V3D reset and ASB sequence is
  the same driver (`linux:drivers/pmdomain/bcm/bcm2835-power.c`:
  `PM_GRAFX` `0x10c`, `PM_V3DRSTN` bit 6, `ASB_V3D_S_CTRL` `0x08`,
  `ASB_V3D_M_CTRL` `0x0c`, password `0x5a000000`, steps clock pulse, reset
  release, clock on, clear `REQ_STOP` master then slave, `bcm2835_asb_power_on`
  ~300). For the Pi 4 we did this on a bridge at a different base and let the
  firmware own the GRAFX island (`repo:docs/notes/pi4-v3d-bringup.md:79-93`);
  on this SoC the same driver also powers the GRAFX parent itself when the
  firmware is not asked. Which way the Zero 2 W firmware leaves the island is
  Q3.

Why `src/v3d` does not extend. The VC4 column is read from Mesa; the V3D
column is what the repository's own V3D notes say.

| Area | VC4 (`mesa:src/gallium/drivers/vc4/`) | V3D 4.2 (`repo` notes) |
|---|---|---|
| Packet language | `vc4_packet.h`: 8-bit opcodes, `GL_SHADER_STATE` 64, `TILE_BINNING_MODE_CONFIG` 112, `TILE_RENDERING_MODE_CONFIG` 113, `CLEAR_COLORS` 114 (`:31-81`) | `v3d_packet.xml`, different ids and fields |
| Frame | one binner list, the render list built by the driver; tile 32x32 with 4x multisample else 64x64 (`vc4_job.c:241-245`); tile state 48 bytes per tile, tile alloc 32 bytes per tile rounded to 256 plus 1 MiB, tile alloc offset 4 KiB aligned (`kernel/vc4_validate.c:370-410`) | tile alloc 128 bytes per tile and a 256-byte TSDA entry per tile (`pi4-v3d-bringup.md:139`) |
| Shader record | `GL_SHADER_STATE_RECORD`, size from the attribute count (`kernel/vc4_validate.c:135`, `gl_shader_rec_size`), three code pointers; the `GL_SHADER_STATE` packet (5 bytes, `vc4_packet.h:104`) carries the attribute count in the low 3 bits of its address, 0 meaning 8 (`kernel/vc4_validate.c:748-750`) | 36-byte record, three shaders (`pi4-v3d-bringup.md:60-62`) |
| Memory | physical, no MMU | MMU, 4 KiB pages, page table we own |
| Blend | no blend packet in `vc4_packet.h:31-81`; blending is lowered into the fragment shader (`vc4_nir_lower_blend.c`, invoked at `vc4_program.c:2217`), reading the tile buffer with the colour-load signal (`vc4_qpu_defines.h:129`) | fixed function `BLEND_CFG` |
| Shader ISA | 64-bit words; add pipe 24 ops (`FADD FSUB FMIN FMAX FMINABS FMAXABS FTOI ITOF ADD SUB SHR ASR ROR SHL MIN MAX AND OR XOR NOT CLZ V8ADDS V8SUBS`, `vc4_qpu_defines.h:30-55`); mul pipe `FMUL MUL24 V8MULD V8MIN V8MAX V8ADDS V8SUBS` (`:57-66`). **No 32-bit integer multiply (only `MUL24`), no divide**; the SFU has recip, rsqrt, exp, log (`:106-109`) | register file plus accumulators, different magic addresses and signals |
| Branches | validated: target = `after_delay_ip + (imm >> 3)`, after three delay slots (`ip + 4`), none inside a delay slot (`kernel/vc4_validate_shaders.c:621-697`) | different |
| Uniforms | a per-shader uniform stream read through `QPU_R_UNIF` (`vc4_qpu_defines.h:71`), restartable by a write to `QPU_W_UNIFORMS_ADDRESS` (`:93`) | same idea, different rules |
| Textures | two TMUs; a sample is requested by writing coordinates to `QPU_W_TMU0_S` and following registers (`vc4_qpu_defines.h:110`), config words come from the uniform stream | TMU with another config, TFU for copies |
| Threads | fragment shaders may be threaded (`THRSW`), vertex and coordinate shaders may not (`kernel/vc4_validate.c:799-807`); the allocator has a separate register class for threaded FS (`vc4_register_allocate.c:386-402`) | up to 4 threads |

The two compilers are independent below the IR. The V3D back end
(`repo:src/shaderc/online/v3d/`: `isa`, `encode`, `decode`, `select`, `sched`,
`alloc`, `sim`, `roundtrip`) cannot be reused as code. What is reusable is
the structure: an ISA model with an encoder, a decoder and a round-trip test
against the reference compiler's words; a selector from the shared IR; a
scheduler and an allocator; a simulator for unit tests. The reference tool
pattern is `tools/v3d/v3d_shc.c`. `src/shaderc/online/mali`, `vivante`,
`powervr` are further from this GPU than V3D is.

## 4. The board on Zephyr

Read in `zephyr:` (4.4.99):

- **There is no BCM2710/BCM2837 SoC and no Zero 2 W or Pi 3 board.**
  `zephyr:boards/raspberrypi/` holds `common`, `rpi_4b`, `rpi_5`, `rpi_pico`,
  `rpi_pico2`, `rpi_debug_probe`; `zephyr:soc/brcm/` holds `bcm2711`, `bcm2712`
  and `bcmvk`. A recursive text search of `boards`, `soc`, `dts` and `drivers`
  for `bcm2835`, `bcm2836`, `bcm2837`, `bcm2710`, `rpi_3` and `rpi_zero` finds
  nothing.
- **The Pi 4 board is a template, not a base.** `bcm2711` selects a Cortex-A72,
  puts the peripherals at `0xfe000000` and a GIC-400 (`arm,gic-v2`) at
  `0xff841000` (`zephyr:dts/arm64/broadcom/bcm2711.dtsi`). The Zero 2 W has
  four Cortex-A53, peripherals at ARM `0x3f000000` and the ARM-local block at
  `0x40000000` (section 1). Its interrupt controller is **not a GIC**:
  `bcm2837.dtsi` uses `interrupt-parent = <&local_intc>` for the timer and the
  PMU, with `local_intc` the ARM-local controller (`bcm2836-l1-intc` per the
  bindings of the family), and the main controller `intc` as the VideoCore
  banks behind it.
- **Zephyr has the hook for a non-GIC arm64 SoC.**
  `CONFIG_ARM_CUSTOM_INTERRUPT_CONTROLLER` (`zephyr:arch/arm64/Kconfig:17-29`)
  maps `arch_irq_*` to the SoC layer; the vector code calls
  `z_soc_irq_get_active` and `z_soc_irq_eoi` instead of the GIC
  (`zephyr:arch/arm64/core/isr_wrapper.S:61-65,112-116`) and
  `z_arm64_interrupt_init` calls `z_soc_irq_init`
  (`zephyr:arch/arm64/core/irq_init.c:27-30`). The ARM arch timer driver
  depends on `GIC || ARM_CUSTOM_INTERRUPT_CONTROLLER`
  (`zephyr:drivers/timer/Kconfig.arm_arch:8`). A new SoC `bcm2837` can select
  the A53 and the arch timer plus this option and write a small controller
  for the two levels (ARM-local routing of timer and mailboxes; the VideoCore
  bank holding the V3D line). A53 CPU nodes already exist
  (`zephyr:dts/arm64/nxp/nxp_mimx8mp_a53.dtsi`).
- **What the GPU and display need**, from the Linux sources above and the Pi 4
  work: the mailbox (`0x7e00b880`, `0x3f00b880` in ARM space) for the V3D
  clock (set-rate tag `0x00038002`) and the framebuffer tags; the power
  domain (firmware domain 10, or the PM/ASB sequence); the V3D register
  block (`0x3f..` alias of bus `0x7ec00000`, i.e. ARM `0x3fc00000`);
  the V3D interrupt, bank 1 bit 10 of the VideoCore controller; a watchdog
  (`repo:docs/standalone-boot.md` "Raspberry Pi 4", `repo:docs/notes/pi4-v3d-bringup.md`).
  Zephyr's firmware driver (`rpi_fw`, `brcm,bcm2711-mbox`) is tied to the
  2711 mailbox; `bcm2837.dtsi` describes the 2837 one as `brcm,bcm2835-mbox`,
  `0x40` bytes, interrupt `<0 1>`. Whether the register layout matches is Q6.
- **Boot route for sign-off**: the vendor firmware loads our image from the
  card, as on the Pi 4 (`repo:docs/standalone-boot.md:303-345`): `config.txt`
  with `kernel=`, `arm_64bit=1`, `enable_uart=1`; the keys are documented at
  raspberrypi.com `config_txt`, linked in that file. U-Boot is not needed on
  that route. Which files the Pi 3 family firmware wants on the card, the
  default load address for a 64-bit `kernel=` and the memory split are Q7. A
  kernel-based jump (Linux then `kexec` into our image) is development only.
- **Memory**: `memory@0` is `0x20000000` (512 MB) in the Linux dts; the
  firmware's GPU split (`gpu_mem`) and the cache alias the V3D should be handed
  (the dts dma-ranges base is `0xc0000000`; which alias that is, and the other aliases, are
  unread) are Q5.

## 5. What can be done before the board arrives

Each item has its check. None needs the board or a vendor SDK.

1. **Reference build of Mesa vc4 on the host with the vc4 drm-shim.** The
   method of `tools/mali/drm_shim_ref.sh`: build Mesa 26.2.1 with
   `-Dgallium-drivers=vc4` and the shim (`mesa:src/broadcom/drm-shim/meson.build`),
   run a one-draw GL program with `MESA_LOADER_DRIVER_OVERRIDE=vc4` and
   `LD_PRELOAD=libvc4_noop_drm_shim.so` (`drm-shim/README.md`). The shim
   answers `GET_PARAM`, `CREATE_BO` and the other ioctls and stubs the submit
   (`vc4_noop.c`). Output wanted: QPU words (`VC4_DEBUG` qpu dumps), the
   control lists (`vc4_cl_dump.c`) and shader records, uniforms. Check: the
   build reproduces on a clean tree and a recorded program's words and lists
   are stored as test data. Open point (Q9): the shim stubs the submit, so
   the bytes come from `VC4_DEBUG=cl` or from an `LD_PRELOAD` that records
   `struct drm_vc4_submit_cl` (`vc4_drm.h:91`).
2. **`vc4_shc` tool** (like `tools/v3d/v3d_shc.c`): Mesa's vc4 compiler on our
   program (CS, VS, FS) through `vc4_shader_ntq` (`vc4_program.c:2180`),
   emitting QPU words and uniform stream descriptions. Check: runs on the
   gl_test corpus, byte-identical on a second run.
3. **ISA model, encoder, decoder** (`src/shaderc/online/vc4/`, as
   `v3d/isa.rs`, `encode.rs`, `decode.rs`). Check: a byte-exact decode/encode
   round trip over every word `vc4_shc` produces, plus `vc4_qpu_disasm.c`
   vectors (`repo:docs/online-shaderc/v3d-backend.md`). A host `cargo test` row.
4. **Packet encoders** for the binner and render lists, golden-tested against
   the recorded lists (as `tools/v3d/cl_golden.c`). Check: byte equality; a
   `tools/check.sh` row.
5. **QPU interpreter** for the compiler's unit tests (as `v3d/sim.rs`),
   validated on Mesa's words.
6. **Register-sequence mock** for the driver: the sequence of section 2
   (cache flush, `CTnCA`, `CTnEA`, `FLDONE`, `FRDONE`, `OUTOMEM` with
   `BPOA/BPOS`, hang reset) against a mock, as `tools/v3d/hw_mock_test.c`.
7. **Zephyr SoC and board definition built but not run**: `soc/brcm/bcm2837`
   (A53, custom interrupt controller shell), board `rpi_zero_2w` with a flat
   memory map from the dts and a console, building the existing samples. Check:
   `west build` produces the image in a new gate row, or a named `gap`.
8. **Host harness row in `tools/check.sh`**: a `gpu-vc4` scope like `gpu-v3d`
   (`repo:tools/check.sh:263` maps a feature to a scope; `:1402-1460` is the
   V3D block) running the lib tests, the QPU round trip and the packet
   golden test.
9. **Read what this session only skimmed** (Q1, Q5, Q7, Q8, Q10): the Broadcom
   guide body, the rest of the Raspberry Pi pages, and confirm the
   `~N` line numbers of the Linux files. Check: the list of questions below
   shrinks, each answer with its source.

## 6. What needs the board

The three things to try on day one, in order:

1. **Firmware boot and UART.** The Zephyr hello image on the card with the
   `config.txt` of the Pi 4 note. Measures: the load address works, which
   cores run, the timer frequency (`CNTFRQ_EL0`), the UART.
2. **Mailbox and V3D identity.** Ask the firmware for the V3D clock and power
   state, read `V3D_IDENT0` (expected `0x02443356`: `(2 << 24) | 'V' | '3' << 8 | 'D' << 16`
   from `vc4_regs.h:37-41`), `IDENT1`, `IDENT2`, and try the PM/ASB reset.
   Measures: the block is where the dts says; the interrupt line; the cache
   alias.
3. **First clear, then first triangle.** A binner list with one clear (a tile
   buffer clear and store to a firmware framebuffer, no shader), then one
   triangle with a fixed shader set from `vc4_shc`. Measures: the clear colour
   on the HDMI capture; the triangle's pixels against the Mesa reference frame
   on the host (the `tools/v3d/frame_check.c` pattern).

The port is alive when the first clear reads back the colour written (frame
store and display path), the first triangle reads back the expected pixels,
and the first `gl_test` case passes through the product path, then the sweep.

## 7. Risks and unknowns

An unknown stays an unknown. Each is a question with what would answer it.

- **Q1. What does the Broadcom guide say that Mesa and Linux do not?** Register
  bit fields beyond the names in `vc4_regs.h`, the QPU's float behaviour, VPM
  and DMA limits, the base addresses (appendix B) and errata (appendix A).
  Answer: `bcm-guide` sections 3, 7, 10 and the appendices (outline only
  read), and `vc4_regs.h` in full (only the offsets above were read).
- **Q2. What does the hardware do that the kernel validator guards against?**
  `kernel/vc4_validate_shaders.c` refuses direct TMU reads without a clamp,
  general VPM DMA, and branches in delay slots; which are hardware hangs and
  which are only isolation for a multi-user kernel? We are single user, but a
  rule we break by accident may hang the GPU. Answer: the validator comments,
  the commit history of that file, a hang on the board.
- **Q3. Who powers the GRAFX island on the Zero 2 W?** The dts has both a
  firmware-based power node (`bcm2835-rpi.dtsi`) and the PM block
  (`bcm283x.dtsi`, `pm`); the fetched `bcm2835-common.dtsi` `v3d` node has no
  `power-domains` or `clocks`. The Pi 4 note found the firmware leaves the
  island on (`pi4-v3d-bringup.md:79-93`). Answer: the Linux `bcm2835-power.c`
  / `bcm2835-rpi.dtsi` /`bcm2837-rpi*.dtsi` as read raw, and the V3D clock
  state at boot on the board.
- **Q4. What is the interrupt route for the V3D line and the timer?** The dts
  gives `interrupts = <1 10>` (bank, bit) on `intc`, with `intc` behind
  `local_intc`. The flat number and the ARM-local routing registers are not
  read. Answer: `linux:drivers/irqchip/irq-bcm2835.c`, `irq-bcm2836.c`. Risk:
  the first non-GIC arm64 interrupt controller in this project.
- **Q5. Where does the GPU see RAM, and what flush does the CPU need?** The
  dts says DMA masters see ARM 0 at bus `0xc0000000`; the other bus aliases
  (cached, uncached) and whether the A53 data cache must be cleaned before a
  job are unread. Answer: the Architecture Guide, Raspberry Pi firmware
  documentation, the board.
- **Q6. Is the BCM2837 mailbox the same register layout as the BCM2711's?**
  Zephyr's `brcm,bcm2711-mbox` and `rpi_fw` are tied to the 2711 (the Pi 4
  images had a conflict with Zephyr's own mailbox node, `pi4-v3d-bringup.md`
  header). Answer: `linux:drivers/mailbox/bcm2835-mailbox.c`.
- **Q7. What does the Pi 3 family firmware need on the card besides
  `bootcode.bin`, where does it load a 64-bit `kernel=`, and what is the
  default memory split?** Known: `arm_64bit=1` must be set explicitly on the
  Zero 2 W (default 0, `rpidoc`). Unread: `kernel_address`, `gpu_mem`, the
  load address. Answer: the rest of the `config_txt` page (from character
  100000), the first boot.
- **Q8. Does the Zero 2 W firmware give a framebuffer through the property
  mailbox on HDMI as the Pi 4 does, and in what format?** The tags exist in
  `raspberrypi-firmware.h`. The render list can store RGBA8888, BGR565 or
  dithered BGR565 (`vc4_packet.h:194-196,303-305`). Answer: the firmware
  documentation, the board.
- **Q9. Can the vc4 drm-shim give the bytes of the lists and records on the
  host?** Answer: build it and run item 1 of section 5.
- **Q10. Are vertex-stage texture look-ups supported?** `vc4_screen.c:159-160`
  sets one sampler count for both stages; the ES 2.0 minimum is zero. We did
  not find the decision. Answer: `ntq_emit_tex` (`vc4_program.c:400`), the
  guide, a test.
- **Q11. QPU float against dEQP.** Mesa and the hardware passed GLES2
  conformance (`vc4.rst:21-22`); the precision groups against our shaders are
  untested. Answer: the first sweep.
- **Q12. How slow is the CPU side?** Four A53 at a clock the firmware sets
  (unread). The AM62P has A53 cores too and the fixed cost of a
  `glDrawElements` there is measured (`repo:docs/notes/gl-draw-fixed-cost.md`);
  the new part is memory: textures, the binner memory (32 bytes of tile alloc
  per tile plus 1 MiB, `vc4_validate.c:385-397`, and the overflow slots) and the
  framebuffer in 512 MB.
- **Q13. Framebuffer size and refresh** of the panel on the HDMI mini port. A
  board setup question, not a design one.
- **Q14. What does ES 2.0 on VC4 cost in lost features?** From Mesa: 2048
  textures, no 3D textures, 8 varyings, no occlusion queries, 16-bit indices
  only, colour buffers RGBA8888 and BGR565 (`vc4.rst:26-60`,
  `vc4_screen.c:195-199`, `vc4_packet.h:303-305`). The dEQP-GLES2
  not-supported list of the three boards is in `repo:docs/es2-cts-failures.md`;
  the VC4 list is unknown until a sweep. ES 3.0 is out of reach as Mesa exposes
  this GPU (no 3D textures at all), so stage 4 does not apply.

## 8. A proposed item list

Sized against the three finished ports (`repo:tools/cts/STATUS.md`,
`repo:CHANGELOG.md`). The Pi 4 is the closest: its plan started 2026-09-23,
M1 to M6 (first triangle through the GL back end) passed on the board
2026-09-24 (`repo:docs/notes/pi4-v3d-bringup.md` header), and dEQP-GLES2 read
0 failures by 2026-10-15/16 (STATUS.md). S is a day or less, M several days, L a
week or more in units of that port, not calendar promises.

| # | Item | Size | Check |
|---|---|---|---|
| 0 | Read the Broadcom guide, the Raspberry Pi boot pages, the Linux files at the tag; update the unknowns | S | each Q has a cited answer |
| 1 | `tools/vc4/`: Mesa vc4 + drm-shim build script, `vc4_shc`, a list recorder | M | clean build; stored reference words and lists |
| 2 | QPU ISA model, encoder, decoder, round trip (`src/shaderc/online/vc4/`) | M | round trip over the corpus and `vc4_qpu_disasm.c` vectors |
| 3 | Packet and record encoders for binner and render lists; golden test | M | byte equality with item 1 |
| 4 | Zephyr SoC `bcm2837`, custom interrupt controller shell, board `rpi_zero_2w`, build-only | M | image builds in a gate row |
| 5 | Driver: mailbox clock and power, reset, list start, interrupts, overflow memory, hang reset, mock test | M | host mock test; day-one checks 1 and 2 |
| 6 | Board day one: UART, identity, clear, triangle with Mesa's own shaders | S (board) | section 6 |
| 7 | Compiler: select, schedule, allocate for FS/VS/CS, uniform stream, VPM I/O, SFU; no 32-bit multiply, no divide | L | corpus words vs `vc4_shc`; QPU sim; gl_test shader cases |
| 8 | Back end `src/vc4`: draw, state, clears, depth and stencil, blend in the FS, cull, point and line variants, scissor | L | gl_test sweep |
| 9 | Textures: layouts (`vc4_tiling*.c`), TMU config, mipmaps, cube, NPOT, ETC1 | M | texture host checks vs Mesa; gl_test texture cases |
| 10 | Display: firmware framebuffer, swap, vsync fence | S | panel |
| 11 | MSAA 4x (32x32 tiles), `glReadPixels`, blit paths | M | gl_test, CTS msaa rows |
| 12 | dEQP-GLES2 stage 1 on the board, the not-supported list, fixes | L | zero failures; accepted not-supported listed |
| 13 | glmark2 scenes, performance on 512 MB | M | RESULTS.md row |

Items 1 to 5 need no board and can land before it arrives; item 6 is the first
board session; the rest follow the Pi 4's order. The largest risks are Q4
(interrupts), Q5 (cache and address), and item 7 (the QPU has fewer
operations than the other GPUs, so more shader shapes need multi-instruction
sequences).
