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
- `linux:<path>` is the Linux tag v6.18 (tag object `f7b88edb52c8`, commit
  `7d0a66e4bb90`), a shallow `git clone --branch v6.18
  https://github.com/torvalds/linux`, read on the host. Line numbers are exact
  and checked against that tree. This replaces the first pass, which read these
  files through a page summariser and wrote `~N`.
- `zephyr:<path>` is the Zephyr workspace the Pi 4 images build with
  (`VERSION` 4.4.99), read on the host.
- `repo:<path>` is this repository at the commit this note was written on.
- `bcm-guide` is *VideoCore IV 3D Architecture Reference Guide*,
  VideoCoreIV-AG100-R, 16 September 2013, "Initial release"
  (`https://docs.broadcom.com/doc/12358545`, 958776 bytes, 111 pages). Page
  numbers are the printed numbers, which equal the PDF page numbers (the
  contents list is page 3 to 6). The PDF was read as page images. **Pages read**:
  3-6 (contents), 13-15 (architecture overview), 17-19 (QPU pipeline), 35-37
  (add and mul ops, instruction restrictions), 53-57 (VPM and VCD), 82-86
  (register map, thread control), 93-96 (cache, interrupt and identity
  registers), 108-110 (errata, base addresses). **Not read**: the rest of
  section 3 (pages 20-34, encodings), sections 4-6, 7 pages 58-61, 8, 9 (control
  lists, 65-81), 10 pages 87-92 and 97-104, 11. Mesa's driver page calls this
  link "a specification for the 21553"; the document is the VideoCore IV guide.
- `bcm2835-periph` is *BCM2835 ARM Peripherals* (`https://pip.raspberrypi.com/documents/RP-008249-DS`,
  dated 06 February 2012), pages 4-7 read as images (address map, bus aliases).
  `bcm2836-local` is *Quad-A7 control* Rev 3.4, 18-08-2014
  (`https://pip.raspberrypi.com/documents/RP-008250-DS`, the ARM-local
  peripherals that `rp3a0.adoc:11-14` names for the BCM2837 and RP3A0), pages
  1-6 read as images (contents, address map, interrupt routing, mailboxes); its
  register pages 7-19 were not read.
- `rpidoc:<path>` is the AsciiDoc source of the Raspberry Pi documentation,
  `https://github.com/raspberrypi/documentation`, branch master at commit
  `4a6cdf2f2840`, path `documentation/asciidoc/computers/<path>`, shallow
  clone read on the host. The published pages on raspberrypi.com answered 403
  to `curl` (with and without a browser User-Agent), so the pages themselves
  were not fetched raw; they are generated from these files, but the published
  text was not compared with them. `config_txt/` is the `config.txt` page;
  `kernel_address`, `kernel_old` and `gpu_mem` are in `legacy_config_txt/`, the
  page the `config.txt` page links as "legacy"; the Zero 2 W specification is
  `zero/zero-models.adoc` and `processors/rp3a0.adoc`.
- `rpifw:` is the file listing of the `boot/` directory of
  `https://github.com/raspberrypi/firmware` from the GitHub contents API on
  2026-10-28 (names only, no file read).
- **Not read**: the closed firmware, the BCM2835 ARM Peripherals pages beyond
  4-7 and the BCM2837-specific differences from it (if any), and the pages of
  `bcm-guide` listed above. Each fact that would come from them is a named
  question in section 7. The Pi 4 boot documentation
  `repo:docs/standalone-boot.md` cites the `config.txt` keys and is used for the
  keys it covers.

## 1. The hardware

| Part | What | Source |
|---|---|---|
| Board | Zero 2 W: the Linux device tree has `compatible = "raspberrypi,model-zero-2-w", "brcm,bcm2837"`, includes `bcm2837.dtsi`, `bcm2836-rpi.dtsi` (which pulls in `bcm2835-rpi.dtsi` and `bcm2835-rpi-common.dtsi`) and the LED, USB OTG and Wi-Fi/Bluetooth files, and has `memory@0` with `reg = <0 0x20000000>` (512 MB) and `stdout-path = "serial1:115200n8"` (the 8250 auxiliary UART, not the pl011) | `linux:arch/arm/boot/dts/broadcom/bcm2837-rpi-zero-2-w.dts:7-11,14,17-19,24`, `bcm2836-rpi.dtsi:2-3` (the arm64 path is a one-line include of it: `linux:arch/arm64/boot/dts/broadcom/bcm2837-rpi-zero-2-w.dts:2`) |
| SoC, as the vendor documents it | the Zero 2 W carries the RP3A0 system-in-package: a BCM2710A1 SoC with four Cortex-A53 at 1 GHz, a VideoCore IV GPU at 400 MHz and 512 MB LPDDR2 at 450 MHz; BCM2710A1 is the die inside the BCM2837 | `rpidoc:zero/zero-models.adoc:15-18`, `rpidoc:processors/rp3a0.adoc:3,9` |
| CPU | four Cortex-A53, started by `spin-table` with release addresses `0xd8`, `0xe0`, `0xe8`, `0xf0`; the `cpus` node also names `brcm,bcm2836-smp` for 32-bit Arm | `linux:arch/arm/boot/dts/broadcom/bcm2837.dtsi:40` (`cpus`), `:53-54`, `:68-69`, `:83-84`, `:98-99` |
| Bus map | peripheral bus `0x7e000000` is ARM physical `0x3f000000` (16 MB), the ARM-local block `0x40000000` (4 KB in the dts, `0x100` for the controller); DMA masters see ARM physical 0 at bus `0xc0000000` (`dma-ranges = <0xc0000000 0x00000000 0x3f000000>`). The ARM-local document gives `0x4000_0000 .. 0x4001_FFFF` to ARM timer, IRQs and mailboxes | `linux:.../bcm2837.dtsi:8-10` (`soc`), `:12-18` (`local_intc`), `bcm2836-local` p.3 (system overview table) |
| GPU | V3D at bus `0x7ec00000`, size `0x1000`, `compatible = "brcm,bcm2835-v3d"`, `interrupts = <1 10>` (bank 1, bit 10: Q4); the Raspberry Pi include adds `clocks = <&firmware_clocks 5>` and `power-domains = <&power RPI_POWER_DOMAIN_V3D>` (Q3). The Broadcom guide gives the same base for the BCM2835 | `linux:arch/arm/boot/dts/broadcom/bcm2835-common.dtsi:138-142` (node `v3d`), `bcm2835-rpi-common.dtsi:24-27`, `bcm-guide` p.110 (Table 95: BCM2835 `0x7ec00000`; the table has no BCM2837 row) |
| GPU identity | the kernel checks `V3D_IDENT0` against `V3D_EXPECTED_IDENT0` = `(2 << 24) \| 'V' \| '3' << 8 \| 'D' << 16` (a "V3D" tag with technology version 2 in the top byte); the guide's `V3D_IDENT0` is TVER[31:24] = 2 and IDSTR[23:0] = "V3D"; Mesa's drm-shim answers `IDENT0 = 0x2000000`, `IDENT1 = 1`, "a VC4 2.1 device" | `linux:drivers/gpu/drm/vc4/vc4_regs.h:36-41`, `vc4_v3d.c:461-466` (the check in `vc4_v3d_bind`, `:418`), `bcm-guide` p.96 (Table 79), `mesa:src/broadcom/drm-shim/vc4_noop.c:85-86`, `mesa:src/broadcom/drm-shim/README.md` ("This will be a VC4 2.1 device") |
| GPU memory | physical addresses only, no GPU MMU. Mesa's validator mirror writes `bo->paddr + offset` for the three shader code addresses and writes the tile state and tile alloc addresses as plain physical addresses; the kernel driver's bin memory is a 16 MB CMA buffer that must not cross a 256 MB boundary (the binner keeps only 28 bits of the tile-alloc and overflow addresses, the top 4 coming from the tile state buffer's address) | `mesa:.../vc4/kernel/vc4_validate.c:399-410` (tile BO), `:326-330`; `linux:.../vc4_v3d.c:215-236` (the comment), `:237-268` (`bin_bo_alloc`, size at `:240`, retry at `:247-250`). Contrast V3D 4.2, which has a one-level MMU we program (`repo:docs/notes/pi4-v3d-bringup.md`, "MMU (M2)") |
| Display | HDMI only on this board (the dts enables `hdmi` with `power-domains = <&power RPI_POWER_DOMAIN_HDMI>`). In the Linux driver a frame reaches the screen as follows: planes write an HVS (hardware video scaler, bus `0x7e400000`, `0x6000` bytes, `interrupts = <2 1>`) display list, the HVS feeds a FIFO to a pixelvalve (`0x7e206000`, `0x7e207000`, `0x7e807000`) and then the HDMI encoder (`0x7e902000`) | `linux:.../vc4_hvs.c` (`vc4_hvs_atomic_flush` `:925` writes planes then `SCALER_CTL0_END` at `:991`; `vc4_hvs_install_dlist` `:834` writes `SCALER_DISPLISTX` at `:850`; `vc4_hvs_get_fifo_from_output` `:532`), `bcm2837-rpi-zero-2-w.dts:102-106`, `bcm283x.dtsi:431-435` (`hvs`), `bcm2835-common.dtsi:80-90,118-122,124-136` (pixelvalves, `hdmi`) |
| Display, the way we would use | not the HVS: the firmware owns the display at boot, the kernel driver tells it to stop with `RPI_FIRMWARE_NOTIFY_DISPLAY_DONE` (`0x00030066`) before it takes over, and it keeps the firmware's boot display list (the dlist allocator starts at `HVS_BOOTLOADER_DLIST_END`). We keep the firmware's scan-out and ask for a framebuffer with the property tags `ALLOCATE` `0x00040001`, `SET_PHYSICAL_WIDTH_HEIGHT` `0x00048003`, `SET_DEPTH` `0x00048005`, `GET_PITCH` `0x00040008`, as the Pi 4 image does | `linux:.../vc4_drv.c:352-375` (firmware lookup and `NOTIFY_DISPLAY_DONE`, in `vc4_drm_bind` `:287`), `vc4_hvs.c:1188` (`__vc4_hvs_alloc`), `:1216-1219` (dlist start), `linux:include/soc/bcm2835/raspberrypi-firmware.h:94,97,104,120,122`; `repo:docs/standalone-boot.md` "Raspberry Pi 4". That this firmware offers the tags on this board is expected, not read (Q8) |
| Power and clocks | the firmware mailbox (`mailbox@7e00b880`, `brcm,bcm2835-mbox`, `reg <0x7e00b880 0x40>`, `interrupts <0 1>`); the `raspberrypi,bcm2835-firmware` node (`mboxes = <&mailbox>`) and a `raspberrypi,bcm2835-power` node (`firmware = <&firmware>`) giving firmware power domains; the binding index `RPI_POWER_DOMAIN_V3D` = 10 (the firmware's own domain number is that plus one, 11); firmware clock id `V3D` = 5 (`EMMC` = 1, then `UART`, `ARM`, `CORE`, `V3D`); `cprman@7e101000` | `linux:arch/arm/boot/dts/broadcom/bcm283x.dtsi:100-105` (`mailbox`), `:86-98` (`clocks`), `bcm2835-rpi.dtsi:5-19`, `linux:include/dt-bindings/power/raspberrypi-power.h:22`, `linux:drivers/pmdomain/bcm/raspberrypi-power.c:119-120`, `linux:include/soc/bcm2835/raspberrypi-firmware.h:140-144`; `repo:docs/notes/pi4-v3d-bringup.md:89` |
| Boot chain | before our image: ROM, then (Pi 3 family) the firmware files from the card. The Pi 4's chain (SPI EEPROM, `start4.elf`, `kernel=`) is documented in `repo:docs/standalone-boot.md:303-340`. The ROM looks for `bootcode.bin` on the card; `bootcode.bin` loads one of the `start*.elf` files, which take over the boot, with a matching `fixup*.dat`; the loader also picks and patches the device tree and then launches the kernel with a pointer to it (Q7) | `rpidoc:raspberry-pi/bootflow-legacy.adoc:3,9`, `rpidoc:configuration/reference.adoc:12-14,18-20,33-35,114-116` |

Public documentation: Broadcom's VideoCore IV 3D Architecture Reference Guide
(`bcm-guide`, 2013), linked from Mesa's driver page together with a snapshot
of the matching Android driver (`mesa:docs/drivers/vc4.rst:285-293`, links
`docs.broadcom.com/doc/12358545` and `/docs/12358546`). Its contents (p. 3-6)
cover the QPU instruction encoding (ALU, branch, semaphore, load-immediate,
condition codes, pack/unpack, small immediate, signalling bits, instruction
restrictions), the TMU, the VPM and VCD, the control record ids, shader state
and primitive list formats, and the V3D register map, plus errata (p. 108-109)
and base addresses (p. 110). The register map (p. 82-84) lists the same
registers and offsets as `vc4_regs.h` and more: the return-address and
list-counter registers of the two control-list threads (`0x118-0x12c`), the
binner debug register `V3D_BXCF` `0x310`, the QPU scheduler `V3D_SQRSV0/1`
`0x410/0x414` and `V3D_SQCNTL` `0x418`, the user-program request registers
`V3D_SRQPC/SRQUA/SRQUL/SRQCS` `0x430-0x43c`, `V3D_VPACNTL` `0x500`, sixteen
performance counters from `0x670`, and error registers from `0xf00`. The
BCM2835 ARM Peripherals datasheet pages 4-7 give the bus aliases (Q5); the
ARM-local document pages 1-6 give the interrupt routing (Q4). BCM2837
differences from the BCM2835 datasheet are unread. The kernel driver and Mesa
remain what we read in detail.
Boot, from `rpidoc`: the boot ROM looks for `bootcode.bin` on the card on
BCM2837-based models (`rpidoc:raspberry-pi/bootflow-legacy.adoc:3,9`; the Pi 4
has the EEPROM instead, `rpidoc:configuration/reference.adoc:16`). The Zero 2 W
is in the list of models that accept `arm_64bit`
(`rpidoc:config_txt/boot.adoc:41-53`), which defaults to 0 there (1 only on the
Pi 4, 400, CM4, CM4S, `:32`); the default `kernel` there is `kernel7.img`
(`:22`); with `arm_64bit=1` the firmware picks a 64-bit kernel such as
`kernel8.img` unless an explicit `kernel` is given (`:30`).

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
    (`vc4_gem.c:436-452`, called at `:486`). Before a render job only
    `vc4_flush_texture_caches`: `L2CCLR` and T1CC, T0CC (`:454-465`, called at
    `:535`). The guide defines the same fields, one bit per slice, write-only
    (`bcm-guide` p. 93-94, Tables 71-72). These clear the GPU's own caches;
    nothing in them flushes the CPU's.
  - `submit_cl(n, ca, ea)` writes `V3D_CTnCA` (start address) and then
    `V3D_CTnEA` (end address); the write of the end address starts the thread.
    Thread 0 is the binner list, thread 1 the render list
    (`vc4_gem.c:373-382`, `vc4_submit_next_bin_job` `:473-500`,
    `vc4_submit_next_render_job` `:518-538`). The guide adds the rules behind
    that: `CTnEA` starts the thread only if it is "Stopped at end"; `CTnCA` can
    be written only while stopped, and a write sets "Stopped at end"; `CTnCS`
    has `CTRUN` bit 5, `CTSUBS` bit 4, `CTERR` bit 3 and a reset bit `CTRSTA`
    bit 15 that "stops the control thread and resets all bits in this register"
    (`bcm-guide` p. 85-86, Tables 49-51).
  - Registers (offsets as written in the file): `V3D_IDENT0` `0x000`,
    `L2CACTL` `0x020`, `SLCACTL` `0x024`, `INTCTL` `0x030`, `INTENA` `0x034`,
    `INTDIS` `0x038`, `CT0CS` `0x100`, `CT1CS` `0x104`, `CT0EA` `0x108`,
    `CT1EA` `0x10c`, `CT0CA` `0x110`, `CT1CA` `0x114`, `PCS` `0x130`, `BFC`
    `0x134`, `RFC` `0x138`, `BPCA` `0x300`, `BPCS` `0x304`, `BPOA` `0x308`,
    `BPOS` `0x30c`, `SRQPC` `0x430` (`linux:.../vc4_regs.h:36-132`; the same offsets
    are in `bcm-guide` p. 82-83, Table 48).
  - Interrupts: `V3D_INT_FRDONE` bit 0, `FLDONE` bit 1, `OUTOMEM` bit 2,
    `SPILLUSE` bit 3 (`vc4_regs.h:78-81`; `bcm-guide` p. 95, Table 75: all four
    are latched, "write 1 to clear" in `V3D_INTCTL`, and `OUTOMEM` is "essentially
    level triggered", so the cause must be removed before the bit can be
    cleared). `FLDONE` (binner done) hands the job to the render thread,
    `FRDONE` completes the frame (`vc4_irq.c:227-240`). On `OUTOMEM` the handler
    masks it (`:220-224`), and a work item writes a new overflow slot into
    `V3D_BPOA` (bus address, `:106`) and `V3D_BPOS` (size, `:107`), clears the
    bit in `V3D_INTCTL` and unmasks it in `V3D_INTENA` (`:108-109`)
    (`vc4_irq.c`: `vc4_irq` `:203`, `vc4_overflow_mem_work` `:61-111`). The
    overflow memory is a 16 MB buffer in 512 KB slices (`vc4_v3d.c:237`
    `bin_bo_alloc`, `:240` size, `:291` slice size).
  - Hang: a timer reads `V3D_CTnCA`; if neither thread moved, a work item
    resets (`vc4_gem.c` `vc4_hangcheck_elapsed` `:327-370`, `vc4_reset`
    `:290-313`). The reset is not a register write: it drops the runtime-PM
    reference and takes it again (`:301-302`), which disables the V3D clock
    (`vc4_v3d.c:372-382`) and, on the way back, enables it, writes
    `V3D_VPMBASE` and re-enables the interrupts (`:384-399`); the power domain
    goes with it through the device's `power-domains` (Q3). The guide's `CTRSTA`
    bit (`bcm-guide` p. 85) is an unused alternative for one thread.
  - Init: the only register `vc4_v3d_init_hw` writes is `V3D_VPMBASE = 0`
    ("take all the memory that would have been reserved for user QPU
    programs") (`vc4_v3d.c:162-171`, the write at `:170`). The guide explains
    why the field is small: user shaders can address only the first 64 rows of
    the VPM (errata HW-2253, `bcm-guide` p. 108).
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
  `PM_GRAFX` `0x10c` `:80`, `PM_V3DRSTN` bit 6 `:86`, `ASB_V3D_S_CTRL` `0x08`
  `:115`, `ASB_V3D_M_CTRL` `0x0c` `:116`, password `0x5a000000` `:100`;
  `bcm2835_asb_power_on` `:287-325` does clock on, about 1 us, clock off,
  reset release, clock on, then clear `REQ_STOP` on the master (`:318`) and
  the slave (`:324`); the domain switch is `:373-386`). For the Pi 4 we did
  this on a bridge at a different base and let the firmware own the GRAFX
  island (`repo:docs/notes/pi4-v3d-bringup.md:79-93`). On the Zero 2 W the
  device tree does not use that path for the V3D: its `v3d` node points at the
  firmware power domain (Q3), so this sequence is the fallback, not the
  default.

Why `src/v3d` does not extend. The VC4 column is read from Mesa; the V3D
column is what the repository's own V3D notes say.

| Area | VC4 (`mesa:src/gallium/drivers/vc4/`) | V3D 4.2 (`repo` notes) |
|---|---|---|
| Packet language | `vc4_packet.h`: 8-bit opcodes, `GL_SHADER_STATE` 64, `TILE_BINNING_MODE_CONFIG` 112, `TILE_RENDERING_MODE_CONFIG` 113, `CLEAR_COLORS` 114 (`:31-81`) | `v3d_packet.xml`, different ids and fields |
| Frame | one binner list, the render list built by the driver; tile 32x32 with 4x multisample else 64x64 (`vc4_job.c:241-245`; the guide gives a 64x64-sample tile buffer, 32x32 pixels in 4x multisample, `bcm-guide` p. 15); tile state 48 bytes per tile, tile alloc 32 bytes per tile rounded to 256 plus 1 MiB, tile alloc offset 4 KiB aligned (`kernel/vc4_validate.c:370-410`) | tile alloc 128 bytes per tile and a 256-byte TSDA entry per tile (`pi4-v3d-bringup.md:139`) |
| Shader record | `GL_SHADER_STATE_RECORD`, size from the attribute count (`kernel/vc4_validate.c:135`, `gl_shader_rec_size`), three code pointers; the `GL_SHADER_STATE` packet (5 bytes, `vc4_packet.h:104`) carries the attribute count in the low 3 bits of its address, 0 meaning 8 (`kernel/vc4_validate.c:748-750`) | 36-byte record, three shaders (`pi4-v3d-bringup.md:60-62`) |
| Memory | physical, no MMU | MMU, 4 KiB pages, page table we own |
| Blend | no blend packet in `vc4_packet.h:31-81`; blending is lowered into the fragment shader (`vc4_nir_lower_blend.c`, invoked at `vc4_program.c:2217`), reading the tile buffer with the colour-load signal (`vc4_qpu_defines.h:129`) | fixed function `BLEND_CFG` |
| Shader ISA | 64-bit words; add pipe 24 ops (`FADD FSUB FMIN FMAX FMINABS FMAXABS FTOI ITOF ADD SUB SHR ASR ROR SHL MIN MAX AND OR XOR NOT CLZ V8ADDS V8SUBS`, `vc4_qpu_defines.h:30-55`); mul pipe `FMUL MUL24 V8MULD V8MIN V8MAX V8ADDS V8SUBS` (`:57-66`). **No 32-bit integer multiply (only `MUL24`), no divide** (the guide's tables list the same ops and opcode numbers, `bcm-guide` p. 35-36, Tables 12-13); the SFU has recip, rsqrt, exp, log (`:106-109`) | register file plus accumulators, different magic addresses and signals |
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
  `bcm2837.dtsi` uses `interrupt-parent = <&local_intc>` for the timer
  (interrupts 0 to 3 of the local controller) and the PMU (9)
  (`linux:.../bcm2837.dtsi:21-33`), with `local_intc` the ARM-local controller
  (`compatible = "brcm,bcm2836-l1-intc"`, `reg = <0x40000000 0x100>`, `:12-18`),
  and the main controller `intc` (`brcm,bcm2836-armctrl-ic`, `reg = <0x7e00b200
  0x200>`) as a child of it on local interrupt 8 (`:129-134`). The two
  controllers and the route of the V3D line are Q4.
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
  clock (set-rate tag `0x00038002`, `raspberrypi-firmware.h:79`) and the
  framebuffer tags; the power domain (the firmware's domain 11, binding index
  10, `SET_DOMAIN_STATE` `0x00038030`, or the PM/ASB sequence); the V3D
  register block (`0x3f..` alias of bus `0x7ec00000`, i.e. ARM `0x3fc00000`);
  the V3D interrupt, bank 1 bit 10 of the VideoCore controller; a watchdog
  (`repo:docs/standalone-boot.md` "Raspberry Pi 4", `repo:docs/notes/pi4-v3d-bringup.md`).
  Zephyr's firmware driver (`rpi_fw`, `brcm,bcm2711-mbox`) is named for the
  2711 mailbox; Linux describes the 2837 one as `brcm,bcm2835-mbox`, `0x40`
  bytes, interrupt `<0 1>` (`bcm283x.dtsi:100-105`), and uses the same node and
  driver on the 2711 (Q6).
- **Boot route for sign-off**: the vendor firmware loads our image from the
  card, as on the Pi 4 (`repo:docs/standalone-boot.md:303-345`): `config.txt`
  with `kernel=`, `arm_64bit=1`, `enable_uart=1` (`rpidoc:config_txt/boot.adoc:20-32,96-98`;
  the Zero 2 W is in the `arm_64bit` list, `:49`). U-Boot is not needed on
  that route. The card also needs `bootcode.bin`, a matching `start*.elf` and
  `fixup*.dat` pair and `config.txt`; the default 64-bit load address is
  `0x200000` (Q7). A kernel-based jump (Linux then `kexec` into our image) is
  development only.
- **Memory**: `memory@0` is `0x20000000` (512 MB) in the Linux dts
  (`bcm2837-rpi-zero-2-w.dts:17-19`), which the firmware may patch before the
  kernel sees it (`rpidoc:configuration/reference.adoc:114`). The firmware's
  GPU split defaults to 64 MB on a board under 1 GB (`gpu_mem`,
  `rpidoc:legacy_config_txt/memory.adoc:8`), so the ARM side would see about
  448 MB (computed, not read). The alias the V3D is handed is `0xc0000000` plus
  the physical address (Q5).

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
9. **Read what the first pass only skimmed** (Q1, Q3 to Q8): done by item 0,
   with the guide pages named in the citation key and the Linux files at the
   tag. What stays: the guide pages not read (inside items 2 and 3) and what only
   the board answers (Q3 island state, Q4 routing, Q5 cache clean, Q7 entry
   state, Q8 framebuffer format). Check: the questions below carry a cited
   answer or a named board test.

## 6. What needs the board

The three things to try on day one, in order:

1. **Firmware boot and UART.** The Zephyr hello image on the card with the
   `config.txt` of the Pi 4 note. Measures: the load address works, which
   cores run, the timer frequency (`CNTFRQ_EL0`), the UART.
2. **Mailbox and V3D identity.** Read `V3D_SCRATCH` and `V3D_IDENT0` first; if the block does not answer, ask
   the firmware for the V3D clock and power domain (Q3), read `V3D_IDENT0` (expected `0x02443356`: `(2 << 24) | 'V' | '3' << 8 | 'D' << 16`
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

- **Q1. What does the Broadcom guide say that Mesa and Linux do not?**
  Answered for the pages read (list in the citation key); open for the rest.
  - *Register bit fields.* `CTnCS` (`bcm-guide` p. 85, Table 49): `CTRUN` bit 5,
    `CTSUBS` bit 4, `CTERR` bit 3, `CTMODE` bit 0 (binning thread only),
    `CTRSTA` bit 15 (write 1: stop the thread and reset the register),
    `CTSEMA` bits 14:12, `CTRTSD` bits 9:8. `CTnEA` starts a stopped thread
    only if `CTSUBS` is "Stopped at end"; `CTnCA` is writable only while
    stopped and a write sets "Stopped at end"; the return stack is one deep
    (p. 86, Tables 50-52). The interrupt bits are latched and are cleared by
    writing 1 to `V3D_INTCTL`; `OUTOMEM` is "essentially level triggered"
    (p. 95, Tables 75-77). The cache registers are write-only clear bits, one per
    slice (p. 93-94, Tables 71-72); `L2CENA` of `V3D_L2CACTL` can be read back.
    `V3D_IDENT0` reads technology version 2 and "V3D" (p. 96, Table 79).
    `V3D_SCRATCH` `0x10` is a free read/write register (p. 96, Table 78), a
    safe first write on the board.
  - *QPU float.* Both ALUs work on 32-bit float or integer data internally; the
    mul pipe can convert a float result to scaled 8-bit; `r4` is the read-only
    result port of the TMU, tile buffer and SFU; `r5` carries the varying "C"
    coefficient (p. 17-18). The pages read say nothing about rounding mode,
    denormals or NaN, so Q11 stays open. The add and mul op tables are the ones
    in `vc4_qpu_defines.h` (p. 35-36, Tables 12-13). The restriction list
    (p. 37) is the hardware's rules for instruction placement: the last three
    instructions of a program read no varying, uniform or VPM; no scoreboard wait
    in the first two instructions of a fragment shader; a value written to a
    register file is not readable by the next instruction; `r4` is not read for
    two instructions after an SFU lookup; one closely coupled peripheral access
    per instruction.
  - *VPM and DMA limits.* The VPM is 8 KB at least and up to 16 KB; the user
    window is a 2D array 16 words wide and at most 64 rows; the VPM cannot be
    reached from a fragment shader; up to two VPM write setups and two read
    setups queue, read data is ready after three instructions at the earliest, and
    extra reads return undefined data; one DMA load and one DMA store at a time
    (concurrent with each other), started by writing an address to
    `VCD_LD_ADDR` or `VCD_ST_ADDR` (p. 53-57, Tables 31-32). Pages 58-61 (the
    vertex data formats) were not read.
  - *Base address.* Appendix B (p. 110, Table 95) lists BCM2835 `0x7ec00000`,
    BCM21553 `0x08950000`, BCM21654 `0x3c00b000`; the dts has `0x7ec00000` for
    the BCM2837 family (`bcm2835-common.dtsi:138-141`). The table has no BCM2837
    row.
  - *Errata* (p. 108-109, Table 94; "affects" is BCM2835 and BCM21553, except
    where marked; the BCM2837's V3D is not named, and that it shares them is an
    assumption from the shared `brcm,bcm2835-v3d` node and `IDENT0`):
    HW-2116 PTB state counters wrap unsafely (software `flush_all_state`);
    HW-2253 user shaders reach only 64 VPM rows (no workaround); HW-2619 and
    HW-2645 16-bit TMU blend problems (BCM2835 only; preprocess the texture
    data); HW-2726 PTB drops zero-size points (discard below 0.125 or clamp in
    the coordinate shader); HW-2753 texture child images of width or height 2048
    fail (no workaround); HW-2796 no `sbwait` in the first two instructions of a
    fragment shader (add nops); HW-2806 Z write too late before a thread
    switch; HW-2885 coverage-pipe lockup (OpenVG, not ES); HW-2898 NPOT raster
    textures (pad); HW-2905 early Z on a full tile load in multisample (disable
    early Z); HW-2924 stencil config applied per batch of four quads. Whether
    Mesa's vc4 carries a workaround for each is not yet checked; that check
    belongs to items 7 to 9.
  - *Not read*: the shader encodings (p. 20-34), the TMU (p. 39-45), the control
    lists and shader state (p. 65-81), register pages 87-92 and 97-104. They are
    read against `vc4_qpu_defines.h` and `vc4_packet.h` inside items 2 and 3.
- **Q2. What does the hardware do that the kernel validator guards against?**
  `kernel/vc4_validate_shaders.c` refuses direct TMU reads without a clamp,
  general VPM DMA, and branches in delay slots; which are hardware hangs and
  which are only isolation for a multi-user kernel? We are single user, but a
  rule we break by accident may hang the GPU. The guide's instruction
  restrictions (`bcm-guide` p. 37) and errata (p. 108-109) are the hardware's own
  rules; the validator's TMU clamp and VPM DMA rules are not in the pages read.
  Answer: the validator comments, the commit history of that file, a hang on
  the board.
- **Q3. Who powers the GRAFX island on the Zero 2 W?** Answered from the
  device tree and the drivers; what the firmware leaves on at boot is a board
  question.
  - The V3D node gets `clocks = <&firmware_clocks 5>` and `power-domains =
    <&power RPI_POWER_DOMAIN_V3D>` from the Raspberry Pi include
    (`linux:arch/arm/boot/dts/broadcom/bcm2835-rpi-common.dtsi:24-27`); the first
    pass looked at `bcm2835-common.dtsi` only, where the node has neither
    (`:138-142`). `power` is `raspberrypi,bcm2835-power` with `firmware =
    <&firmware>` (`bcm2835-rpi.dtsi:15-19`), so the V3D is powered by asking the
    firmware, not by the PM and ASB registers.
  - That driver is `drivers/pmdomain/bcm/raspberrypi-power.c`. The V3D domain
    is `rpi_init_power_domain(.., RPI_POWER_DOMAIN_V3D, "V3D")` (`:213`); the
    firmware's own number is the binding index plus one, so 11 (`:119-120`); the
    request is the property tag `SET_DOMAIN_STATE` `0x00038030` with a
    `{domain, state}` pair (`:42-45,51-72`; the tag at
    `raspberrypi-firmware.h:83`). The V3D domain exists only if the firmware
    answers the new `GET_DOMAIN_STATE` probe (`:116-117,145-159,192-193`). Linux
    "treats all power domains as off at boot", although "the firmware itself may
    be keeping some domains on" (`:98-106`).
  - The PM block is also described (`bcm2835-common.dtsi:59-72`: `pm` at
    `0x7e100000`, size `0x114`, and `asb` at `0x7e00a000`, size `0x24`), with the
    V3D clock `firmware_clocks 5` (`bcm2835-rpi-common.dtsi:16-21`), and
    `bcm2835-power.c` registers GRAFX and V3D domains on it
    (`drivers/mfd/bcm2835-pm.c:25`, `bcm2835-power.c:682`). On the Zero 2 W no
    node uses them for the V3D. That driver leaves a GRAFX island alone if
    `PM_POWUP` is already set (`bcm2835-power.c:229-232`), so the Pi 4 note's
    finding (the firmware leaves the island on, `repo:docs/notes/pi4-v3d-bringup.md:79-93`)
    may or may not carry over.
  - Day-one order (section 6): read `V3D_SCRATCH` and `V3D_IDENT0` first; if the
    block does not answer, `SET_DOMAIN_STATE` for domain 11 and `SET_CLOCK_RATE`
    (`0x00038002`) for clock 5 through the property mailbox; the PM/ASB
    sequence of section 3 last. What the board answers is the state of the
    island and the V3D clock rate at the first instruction of our image.
- **Q4. What is the interrupt route for the V3D line and the timer?** Answered
  from the two irqchip drivers and the ARM-local document. It is the first
  non-GIC arm64 interrupt controller in this project, but a small one.
  - V3D `interrupts = <1 10>` is (bank, bit) on `intc`, `brcm,bcm2836-armctrl-ic`
    at bus `0x7e00b200`, ARM `0x3f00b200` (`bcm2835-common.dtsi:52-57`,
    `bcm2837.dtsi:129-134`). The driver turns it into hwirq `bank * 32 + bit` =
    42 (`linux:drivers/irqchip/irq-bcm2835.c:48,110-130`). The registers, from
    the block base: pending `0x00` (basic), `0x04` (bank 1), `0x08` (bank 2);
    enable `0x18`, `0x10`, `0x14`; disable `0x24`, `0x1c`, `0x20`
    (`irq-bcm2835.c:69-71`). So the V3D line is bit 10 of the bank 1 pending
    register (ARM `0x3f00b204`), enabled by writing `BIT(10)` to ARM
    `0x3f00b210` and disabled at `0x3f00b21c`; the enable and disable registers
    are write-one (`:92-100`).
  - Bank 1 irq 10 is also one of five "shortcut" lines: bits 10 to 14 of the
    basic pending register mirror bank 1 irqs 7, 9, 10, 18, 19, so the same
    interrupt shows as bit 12 of the basic pending register
    (`irq-bcm2835.c:55-57,74-77,225-243`). A handler checks one or both.
  - The controller's output is local interrupt 8 (`LOCAL_IRQ_GPU_FAST`) of the
    ARM-local controller `brcm,bcm2836-l1-intc` at `0x40000000`
    (`irq-bcm2836.h:59`, `bcm2837.dtsi:12-18,129-134`). Per core, the pending
    word is at `0x60 + 4 * core` (`irq-bcm2836.h:36`, `irq-bcm2836.c:144`).
    The GPU IRQ is routed by `LOCAL_GPU_ROUTING` `0x0c` (low 2 bits the core for
    IRQ, next 2 for FIQ, `irq-bcm2836.h:11-15`), which Linux never writes: its
    mask and unmask for this line do nothing (`irq-bcm2836.c:81-87`) and its init
    writes only `LOCAL_CONTROL` and `LOCAL_PRESCALER` (`:309,315`). The
    ARM-local document says the "core un-related" interrupts, the GPU IRQ among
    them, go to IRQ of core 0 after a reset and have no disable code, to be
    masked at their source (`bcm2836-local` p. 5-6, section 3.2.2). The routing
    register's own page (section 4.4, p. 11) was not read.
  - Timer: the arm-timer lines are local interrupts 0 to 3 (`bcm2837.dtsi:27-33`,
    `irq-bcm2836.h:51-54`), enabled per core in `LOCAL_TIMER_INT_CONTROL0`
    `0x40 + 4 * core`, bits 0 to 3 for IRQ (`irq-bcm2836.h:25-30`,
    `irq-bcm2836.c:25-55`); all routing bits are zero after reset, so no timer
    interrupt is delivered until set (`bcm2836-local` p. 5).
  - What a Zephyr SoC layer needs (section 4): `z_soc_irq_get_active` reads the
    core's local pending word, and for local interrupt 8 reads the basic pending
    register (and bank 1 for bit 10); enable and disable of the V3D line are
    single writes to `0x3f00b210` and `0x3f00b21c`; the timer is a write to
    `0x40000040`. Still a board question: whether the firmware's stub leaves the
    routing at its reset value (core 0 takes the V3D IRQ), and the counter
    frequency (`CNTFRQ_EL0`).
- **Q5. Where does the GPU see RAM, and what flush does the CPU need?**
  Answered for the address; the flush stays a board test.
  - The VideoCore bus has four aliases of RAM: `0x0` (L1 and L2 cached), `0x4`
    (L2 coherent, non-allocating), `0x8` (L2 cached only) and `0xC` ("direct
    uncached") (`bcm2835-periph` p. 5, the address-map figure). The text says the
    bus addresses for RAM are set up to map onto the uncached range starting at
    `0xC0000000`, and that software using DMA must use bus addresses based at
    `0xC0000000` (p. 6, section 1.2.3, and p. 7). The BCM2837 dts has the same
    window, `dma-ranges = <0xc0000000 0x00000000 0x3f000000>`
    (`linux:.../bcm2837.dtsi:10`), and Linux writes `dma_addr` values into the
    V3D registers, for example `V3D_BPOA` (`vc4_irq.c:106`). Our Pi 4 mailbox
    code adds the same `0xc0000000`
    (`repo:driver/zephyr/lib/board/rpi4/rpi_mbox.c:30-31,52`). So a V3D address
    is `0xC0000000 + physical`, in the uncached alias.
  - Source caveat: that datasheet is the BCM2835's (dated 2012); the BCM2837 is
    described by it plus the ARM-local document, and nothing read says the bus
    aliases changed.
  - The Raspberry Pi page says that on the BCM2836, BCM2837, BCM2711 and BCM2712
    the Arm CPUs have their own L2 cache, so `disable_l2cache` defaults to 1
    and "disables the CPU's access to the GPU's L2 cache"
    (`rpidoc:legacy_config_txt/memory.adoc:51-53`). The GPU's own L2 is cleared
    by `V3D_L2CACTL` (section 2).
  - The 256 MB rule of `bin_bo_alloc` applies: the 512 MB of RAM is two 256 MB
    halves in bus space, and the bin buffer must lie in one of them
    (`vc4_v3d.c:219-228`; derived, not read as a statement about this board).
  - Not answered by any source read: whether the A53 data cache must be cleaned
    before a job. Nothing read says the V3D snoops the Arm caches; the safe
    design is non-cacheable (or write-combined) mappings for every buffer the
    V3D reads or writes, as the Linux driver allocates them (`vc4_v3d.c:256`,
    `vc4_bo_create`), or a clean and invalidate around each job. The board test:
    write through a cached mapping without a clean and compare.
- **Q6. Is the BCM2837 mailbox the same register layout as the BCM2711's?**
  Yes, as far as Linux and our own Pi 4 code show.
  - Linux has one driver for both, `brcm,bcm2835-mbox`, in the shared
    `bcm283x.dtsi:100-105`; `bcm2711.dtsi:1143-1145` overrides only the
    interrupt. The driver's registers, from the block base: `MAIL0_RD` `0x00`,
    `MAIL0_POL` `0x10`, `MAIL0_STA` `0x18`, `MAIL0_CNF` `0x1c`, `MAIL1_WRT`
    `0x20`, `MAIL1_STA` `0x38`; status bit 31 is full, bit 30 empty; the VC
    answers in mailbox 0 and we write mailbox 1
    (`linux:drivers/mailbox/bcm2835-mailbox.c:32-50`). A message is the data with
    the low 4 bits replaced by the channel, and the property channel is 8
    (`linux:drivers/firmware/raspberrypi.c:20,23`).
  - Our Pi 4 code already uses exactly these offsets and works on the board
    (`repo:driver/zephyr/lib/board/rpi4/rpi_mbox.c:18-25`), so for the Zero 2 W
    only the base changes: ARM `0x3f00b880` instead of `0xfe00b880`. The name
    `brcm,bcm2711-mbox` is Zephyr's own binding name. Zephyr's `rpi_fw` was not
    re-read in this pass.
  - Interrupt: bank 0 irq 1 of `intc` (`bcm283x.dtsi:103`); the Pi 4 override is a
    GIC SPI. With polling, as `rpi_mbox.c` does, neither matters.
- **Q7. What does the Pi 3 family firmware need on the card besides
  `bootcode.bin`, where does it load a 64-bit `kernel=`, and what is the
  default memory split?** Answered as far as the documentation goes.
  - Files: `bootcode.bin` (loaded by the ROM, it loads a `start*.elf`), a
    `start*.elf` with its matching `fixup*.dat` (an unmatched pair stops the
    board), `config.txt`, the kernel image, optionally `cmdline.txt`
    (`rpidoc:configuration/reference.adoc:12-43`, `config_txt/boot.adoc:7-10`).
    The firmware also loads, patches and passes a device tree
    (`reference.adoc:114-116`). The `raspberrypi/firmware` repository's `boot/`
    directory (`rpifw:`) holds
    `bootcode.bin`, `start.elf`, `fixup.dat` and `bcm2710-rpi-zero-2-w.dtb`. Use
    the default `start.elf`: `gpu_mem=16` selects the cut-down firmware, which
    "removes support for codecs, 3D and debug logging" and limits the early
    framebuffer to 1080p at 16 bpp (`config_txt/boot.adoc:12`).
  - Load address: `kernel_address` defaults to `0x8000` for a 32-bit kernel and
    `0x200000` for a 64-bit one; `kernel_old=1` loads at `0x0`
    (`rpidoc:legacy_config_txt/boot.adoc:38-44`). `arm_64bit=1` must be set on
    the Zero 2 W (`config_txt/boot.adoc:32,49`); a `kernel=` file may be a raw
    image or a gzip archive, recognised by its signature (`:34-39`). The firmware
    fills ATAGS at `0x100` unless `disable_commandline_tags=1`
    (`legacy_config_txt/boot.adoc:24`). The Arm stub that runs before the kernel
    is built into the firmware unless `armstub` names one; it sets up the
    interrupt controller (`config_txt/boot.adoc:57-61`).
  - Memory split: `gpu_mem` defaults to 64 (MB) on a board under 1 GB, the
    minimum is 16, the recommended maximum for 512 MB is 384
    (`legacy_config_txt/memory.adoc:8,18-31,35`). On legacy kernels the GPU memory
    serves display, 3D, codec and camera, and only the Pi 4's 3D has its own MMU
    (`:14,16`). A bare-metal V3D driver of ours takes its buffers from the Arm
    side (Q5), so `gpu_mem` can stay at the default or go down (not to 16).
  - Not answered: whether a Zephyr image starts at `0x200000` as the Pi 4's does
    with `kernel=` (`repo:docs/standalone-boot.md`); which cores run at entry
    (the stub parks cores 1 to 3 at the spin-table addresses of
    `bcm2837.dtsi:54,69,84,99`, expected, not read); the memory the firmware
    reports. Day-one item 1.
- **Q8. Does the Zero 2 W firmware give a framebuffer through the property
  mailbox on HDMI as the Pi 4 does, and in what format?** The documentation read
  does not say; one indirect source points to yes.
  - The tags are in `raspberrypi-firmware.h:97-131`, and Linux calls
    `NOTIFY_DISPLAY_DONE` on every `vc4` board that has the
    `raspberrypi,bcm2835-firmware` node (`vc4_drv.c:352-375`), which the Zero 2 W
    has (`bcm2835-rpi.dtsi:5-7`). The full firmware has an "early-boot
    framebuffer" (the cut-down one limits it, `config_txt/boot.adoc:12`). The
    mailbox protocol is the one the Raspberry Pi industrial page links
    (`rpidoc:raspberry-pi/raspberry-pi-industrial.adoc:46`).
  - Format: the render list can store RGBA8888, BGR565 or dithered BGR565
    (`vc4_packet.h:194-196,303-305`); which depth the firmware grants for
    `SET_DEPTH` (16 or 32) and the pixel order are what the board answers
    (`repo:driver/zephyr/lib/board/rpi4/rpi_mbox.c:145-150` is the call to try).
    The HDMI port is a mini-HDMI "supporting a single display ... at up to 1080p
    at 60 Hz" (`rpidoc:zero/zero-models.adoc:55-56`).
- **Q9. Can the vc4 drm-shim give the bytes of the lists and records on the
  host?** Answer: build it and run item 1 of section 5.
- **Q10. Are vertex-stage texture look-ups supported?** `vc4_screen.c:159-160`
  sets one sampler count for both stages; the ES 2.0 minimum is zero. We did
  not find the decision. Answer: `ntq_emit_tex` (`vc4_program.c:400`), the
  guide, a test.
- **Q11. QPU float against dEQP.** Mesa and the hardware passed GLES2
  conformance (`vc4.rst:21-22`); the precision groups against our shaders are
  untested, and the guide pages read give no rounding or denormal rule (Q1).
  Answer: the first sweep.
- **Q12. How slow is the CPU side?** Four A53 at 1 GHz, 1.2 GHz with cooling, and a 400 MHz VideoCore IV
  (`rpidoc:zero/zero-models.adoc:15-18`, `processors/rp3a0.adoc:9`), a clock the
  firmware sets. The AM62P has A53 cores too and the fixed cost of a
  `glDrawElements` there is measured (`repo:docs/notes/gl-draw-fixed-cost.md`);
  the new part is memory: textures, the binner memory (32 bytes of tile alloc
  per tile plus 1 MiB, `vc4_validate.c:385-397`, and the overflow slots) and the
  framebuffer in 512 MB.
- **Q13. Framebuffer size and refresh** of the panel on the HDMI mini port. A
  board setup question, not a design one; the port is a mini-HDMI for one
  display up to 1080p at 60 Hz (`rpidoc:zero/zero-models.adoc:55-56`).
- **Q14. What does ES 2.0 on VC4 cost in lost features?** From Mesa: 2048
  textures (and the guide's HW-2753: a 2048-wide or -high texture child image
  fails with no workaround, `bcm-guide` p. 108), no 3D textures, 8 varyings, no occlusion queries, 16-bit indices
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
| 0 | Read the Linux files at the tag, the Broadcom guide and the Raspberry Pi sources raw; update the unknowns (done, second pass; the guide pages not read fall inside items 2 and 3) | S | each Q has a cited answer or a named board test |
| 1 | `tools/vc4/`: Mesa vc4 + drm-shim build script, `vc4_shc`, a list recorder | M | clean build; stored reference words and lists |
| 2 | QPU ISA model, encoder, decoder, round trip (`src/shaderc/online/vc4/`) | M | round trip over the corpus and `vc4_qpu_disasm.c` vectors |
| 3 | Packet and record encoders for binner and render lists; golden test | M | byte equality with item 1 |
| 4 | Zephyr SoC `bcm2837`, custom interrupt controller shell, board `rpi_zero_2w`, build-only | M | image builds in a gate row |
| 5 | Driver: mailbox clock and power (the Pi 4's `rpi_mbox.c` with base `0x3f00b880`, Q6), reset, list start, interrupts (Q4), overflow memory, hang reset, mock test | M | host mock test; day-one checks 1 and 2 |
| 6 | Board day one: UART, identity, clear, triangle with Mesa's own shaders | S (board) | section 6 |
| 7 | Compiler: select, schedule, allocate for FS/VS/CS, uniform stream, VPM I/O, SFU; no 32-bit multiply, no divide | L | corpus words vs `vc4_shc`; QPU sim; gl_test shader cases |
| 8 | Back end `src/vc4`: draw, state, clears, depth and stencil, blend in the FS, cull, point and line variants, scissor | L | gl_test sweep |
| 9 | Textures: layouts (`vc4_tiling*.c`), TMU config, mipmaps, cube, NPOT, ETC1 | M | texture host checks vs Mesa; gl_test texture cases |
| 10 | Display: firmware framebuffer, swap, vsync fence | S | panel |
| 11 | MSAA 4x (32x32 tiles), `glReadPixels`, blit paths | M | gl_test, CTS msaa rows |
| 12 | dEQP-GLES2 stage 1 on the board, the not-supported list, fixes | L | zero failures; accepted not-supported listed |
| 13 | glmark2 scenes, performance on 512 MB | M | RESULTS.md row |

Items 1 to 5 need no board and can land before it arrives; item 6 is the first
board session; the rest follow the Pi 4's order. The largest risks are Q5 (the
CPU cache clean around a job; the address is known), Q3 (whether the island is
on when our image starts), the first non-GIC interrupt controller of item 4
(Q4 is now a two-level controller with known registers), and item 7 (the QPU has fewer
operations than the other GPUs, so more shader shapes need multi-instruction
sequences).
