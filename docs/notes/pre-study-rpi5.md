<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: Raspberry Pi 5 (BCM2712, VideoCore VII, V3D 7.1)

Date 2026-10-28, main `90b53ca2`. Host reading only: no board, no file under
`src/`, `driver/` or `tools/` changed, nothing built or run. The Pi 5 has not
arrived.

## How to read this note

Every claim says where it was read. The paths are:

- `MESA/` is Mesa 26.2.1, the tree the project vendors unmodified as
  `third_party/mesa-26.2.1` (only the parts the gate needs) and the full tree
  this study read (`src/broadcom/`, `src/gallium/drivers/v3d/`,
  `include/drm-uapi/v3d_drm.h`). MIT: facts and short quotes are fine, the code
  is read and not copied into our files except where `docs/online-shaderc/`
  already transcribes it with attribution.
- `ZEPHYR/` is the Zephyr workspace this repository builds with (`VERSION`
  4.4.99): `soc/brcm/bcm2712/`, `dts/arm64/broadcom/bcm2712.dtsi`,
  `boards/raspberrypi/rpi_5/`.
- `repo:` is a file of this repository.
- `LINUX/` is Linux **v6.18** (`https://raw.githubusercontent.com/torvalds/linux/v6.18/...`):
  `drivers/gpu/drm/v3d/{v3d_regs.h,v3d_drv.c,v3d_mmu.c,v3d_irq.c,v3d_sched.c,v3d_gem.c}`
  and `arch/arm64/boot/dts/broadcom/bcm2712.dtsi`.
- `RPI/` is the Raspberry Pi fork's branch `rpi-6.12.y`
  (`https://raw.githubusercontent.com/raspberrypi/linux/rpi-6.12.y/...`):
  `arch/arm64/boot/dts/broadcom/{bcm2712.dtsi,bcm2712-ds.dtsi,bcm2712-rpi.dtsi,bcm2712-rpi-5-b.dts}`,
  `arch/arm/boot/dts/overlays/vc4-kms-v3d-pi5-overlay.dts` and
  `drivers/gpu/drm/vc4/{vc4_drv.c,vc4_hvs.c,vc4_hdmi.c}`.
- `DOCS/` is Raspberry Pi's documentation repository, branch `master`
  (`https://raw.githubusercontent.com/raspberrypi/documentation/master/documentation/asciidoc/computers/...`):
  `config_txt/boot.adoc`, `config_txt/conditional.adoc`,
  `config_txt/memory.adoc`, `raspberry-pi/boot-eeprom.adoc`, and the
  `raspberrypi/rpi-eeprom` `README.md`; and the page
  `https://www.raspberrypi.com/documentation/computers/raspberry-pi.html`
  (sections "EEPROM boot flow", "Fail-safe OS updates (tryboot)", the
  `BOOT_ORDER` table), read in its characters 100,000 to 200,000 only.
- GPL files (Linux, the fork): facts and file:line only, nothing copied. The
  local Linux checkout in the dependency cache holds only `panthor`, so the
  files above were read over the network. **Line numbers**: for the device
  trees, `v3d_irq.c` and the dtsi/dts files they are the reader's counts from
  the SPDX line (may be off by one); for `v3d_sched.c` and `v3d_gem.c` the
  reading tool returned no line numbers, so those are cited **by function
  name** and by statement order inside it, which is stable and checkable.
- **Not read.** The firmware's own code (closed), the rest of the
  raspberrypi.com hardware page (characters 0 to 100,000 gave only headings
  and 200,000 on were not read; its "Raspberry Pi bootloader configuration"
  body, `PARTITION_WALK` and the `TRYBOOT` property table, is in those
  parts), the second part of `vc4_hdmi.c` (the
  reading covered its first 100,000 characters, which did not reach the
  BCM2712 PHY initialisation), and the Linux `Documentation/arch/arm64/booting.rst`
  (not read). Where a section needs one it says so and
  section 7 names what would answer it. The earlier Pi 4 note's SMS citation
  (`repo:docs/notes/pi4-v3d-bringup.md`, v6.17 `v3d_drv.c:266-294`) is
  confirmed at v6.18 as `v3d_drv.c:287-315` (section 3.4).
- A task statement named `v3d_packet_v71.xml`. At Mesa 26.2.1 that file does
  not exist: one file, `MESA/src/broadcom/cle/v3d_packet.xml` (`<vcxml
  gen="4.2" min_ver="42" max_ver="71">`, line 1), carries both generations as
  `min_ver` / `max_ver` attributes, and `MESA/src/broadcom/cle/meson.build:7-8`
  generates `v3d_packet_v42_pack.h` and `v3d_packet_v71_pack.h` from it.

## 1. The hardware

| Item | What | Source |
|---|---|---|
| SoC | Broadcom BCM2712, 4x Cortex-A76 at 2.4 GHz, 512 KB L2 per core, 2 MB shared L3 | `ZEPHYR/boards/raspberrypi/rpi_5/doc/index.rst`, "Hardware" |
| GPU | VideoCore VII, "OpenGL ES 3.1, Vulkan 1.2" (the board page's words) | same |
| GPU core | V3D **7.1**. Mesa identifies it as `devinfo->ver = major*10 + minor` from `CORE0_IDENT0[31:24]` and `CORE0_IDENT1[3:0]`, and takes `rev` from `HUB_IDENT3[15:8]`, `compat_rev` from `[23:16]` | `MESA/src/broadcom/common/v3d_device_info.c` (the `ver`, `rev` and `compat_rev` lines) |
| GPU revisions | Several 7.1 revisions exist and differ in the ABI: 7.1.2 has no R/B swap bit in the texture state, 7.1.5 adds it; 7.1.6 and later do not subtract 1 from the CSD batch count; 7.1.7 (and 7.1.6 with `compat_rev` >= 4) have the `max0` unpack; **7.1.10 is "2712D0"** and adds the draw-index shader record | `v3d_packet.xml:1686-1692` and `:1313`; `MESA/src/gallium/drivers/v3d/v3dx_draw.c:1519-1521`; `MESA/src/broadcom/common/v3d_device_info.h:94-112` |
| Which revision a given Pi 5 carries | **Unknown** (question U2) | |
| Memory | LPDDR4X, 1 to 16 GB | `ZEPHYR/.../rpi_5/doc/index.rst` |
| Display | Dual 4Kp60 HDMI. The HDMI path is on the **SoC**, not RP1. A frame goes: planes in memory, the **HVS** (hardware video scaler, `brcm,bcm2712-hvs`, registers at 0x10_7c580000, size 0x1a000, a display list in its own memory, three per-channel end-of-frame interrupts through `disp_intr` 2, 9, 16, `iommus = <&iommu4>`) composes them per channel into a FIFO; the FIFO feeds a **pixel valve** (`pixelvalve0` at 0x7c410000, SPI 101; `pixelvalve1` at 0x7c411000, SPI 110); the pixel valve feeds `hdmi0`/`hdmi1` (`brcm,bcm2712-hdmi0/1`, nine register regions each: hdmi, dvp, phy, rm, packet, metadata, csc, cec, hd; hot-plug and CEC interrupts on `aon_intr`; a reset on the `dvp` block). All of it sits under a display node `vc4: gpu` (`brcm,bcm2712-vc6`) that has no registers of its own. In the Linux driver the output-to-FIFO map is: output 0 to FIFO 0, output 2 to FIFO 2, outputs 1, 3, 4 to FIFO 1 (`vc4_hvs_get_fifo_from_output`). The Pi 5 board file gives the HDMI blocks the firmware clocks 13 (`hdmi`) and 14 (`bvb`), the `dvp` audio clock and the 27 MHz crystal, the HVS the firmware clocks 4 (`core`) and 16 (`disp`) | `RPI/bcm2712.dtsi` (`pixelvalve0` 292, `pixelvalve1` 298, `hdmi0` 370, `hdmi1` 399, `vc4: gpu` 446, `hvs` 612; the first reading said 609); `RPI/bcm2712-rpi-5-b.dts` (the `&hdmi0`, `&hdmi1`, `&hvs` overrides at about 275-290); `RPI/drivers/gpu/drm/vc4/vc4_hvs.c` (`vc4_hvs_get_fifo_from_output`, `vc4_hvs_bind`, `vc4_hvs_dt_match`) |
| Does the firmware leave a scanout running | **Partly answered.** The Linux driver treats the firmware as the owner of a first display: `__vc4_hvs_alloc` starts its display-list allocator after `HVS_BOOTLOADER_DLIST_END`, "which preserves the bootloader's setup"; `vc4_drm_bind` calls `aperture_remove_all_conflicting_devices` (a framebuffer set up before the driver) and, when the firmware's own display driver is not the one in use (`firmware_kms()` false, checking the nodes `raspberrypi,rpi-firmware-kms` and `...-kms-2711`), sends `RPI_FIRMWARE_NOTIFY_DISPLAY_DONE` to stop it; `vc4_hdmi_bind` enables the HDMI clocks when `VC4_HD_VID_CTL_ENABLE` is already set. Together: the firmware **can** leave HVS, pixel valve and HDMI running with a framebuffer, and Linux has code to adopt or stop it. Whether a Zephyr image started with `kernel=` sees one, and at what address, is not in these files (U3). The HDMI PHY initialisation for BCM2712 was not reached by the reading | `RPI/drivers/gpu/drm/vc4/vc4_hvs.c` (`__vc4_hvs_alloc`), `vc4_drv.c` (`vc4_drm_bind`, `firmware_kms`), `vc4_hdmi.c` (`vc4_hdmi_bind`) |
| Display clocks and the firmware | The vc4 driver asks the firmware for the maximum core clock (`rpi_firmware_clk_get_max_rate`, `RPI_FIRMWARE_CORE_CLK_ID`) to decide 4096x2160 and HDMI 2.0, and defers probing without the firmware; the pixel and `bvb` clocks are plain `clk_set_rate` / `clk_set_min_rate` calls (minimum 120 MHz or about 101% of the TMDS rate for `hdmi`; 75, 150 or 300 MHz for `bvb`) | `RPI/vc4_hvs.c` (`vc4_hvs_bind`), `vc4_hdmi.c` (the clock requests) |
| I/O chip | RP1 sits behind a PCIe link. Zephyr's device tree puts the RP1 GPIO block under the node `pcie2` and names `pcie1` as the board's `zephyr,pcie-controller` | `ZEPHYR/dts/arm64/broadcom/bcm2712.dtsi` (`pcie1`, `pcie2`, `rp1`), `rpi_5_bcm2712.dts` (`chosen`). Whether Zephyr's `pcie1`/`pcie2` names match Linux's is not checked |
| Console | `uart10`, a PL011 (`arm,pl011`), 115200 baud in the board file; the 3-pin debug connector between the HDMI ports takes the Raspberry Pi Debug Probe | `rpi_5_bcm2712.dts`, `bcm2712.dtsi`, `.../doc/index.rst` "Serial Communication" |
| Interrupt controller | GICv2 (`arm,gic-v2`), 280 interrupts, architected timer at 54 MHz | `bcm2712.dtsi` (`gic`), `ZEPHYR/soc/brcm/bcm2712/Kconfig.defconfig` |
| Boot chain, before our image | On BCM2711 and BCM2712 the **boot ROM** runs first. It looks for `recovery.bin` in the root of the boot partition and, if valid, runs it instead of the flash contents; otherwise it runs the bootloader in the board's **SPI flash EEPROM** (Pi 4 and later; other models load `bootcode.bin` from the boot partition). The Pi 5's EEPROM images are the `firmware-2712` set (Pi 5, Pi 500, CM5), updated with `rpi-eeprom-update`, and Pi 5 alone has an **A/B** update of two EEPROM partitions. The EEPROM bootloader then loads the VideoCore firmware and the files named by `config.txt` from the boot partition. The EEPROM boot-flow section says of the Pi 5: "Instead of loading `start.elf`, the firmware loads the Linux kernel", so a Pi 5 card needs no `start4.elf`/`fixup4.dat`. `BOOT_ORDER` default `0xf41` is SD first, then USB mass storage, repeating. What the EEPROM stage hands over (registers, exception level) is not stated; the board page names only `config.txt`, `zephyr.bin` and the DTB | `DOCS/raspberry-pi/boot-eeprom.adoc` ("Raspberry Pi 4 and Pi 5 boot", `recovery.bin`, A/B); raspberrypi.com hardware page, "EEPROM boot flow" and the `BOOT_ORDER` table; `raspberrypi/rpi-eeprom` `README.md` (`firmware-2712`) |
| Boot chain, our image | The firmware reads `config.txt` from the first FAT partition of the SD card (MBR, FAT32 per the board page) and starts `kernel=`. Defaults by model: `kernel8.img` on Pi 4; **`kernel_2712.img` on Pi 5, 500, 500+ and CM5, and if absent the firmware loads `kernel8.img`** (the default Pi 5 kernel has a 16K page size). `arm_64bit` is ignored on the Pi 5 (64-bit only). `os_prefix` puts the kernel, DTB, `cmdline.txt` and overlays under a prefix. `enable_uart=1` creates the serial console; `uart_2ndstage=1` turns on UART logging in the firmware. `armstub` is the Arm stub, chosen by firmware per model, and "sets up low-level hardware, such as the interrupt controller, before the kernel runs". The Zephyr board page builds `zephyr.bin` this way with three files on the card: `config.txt` (`kernel=zephyr.bin`, `enable_uart=1`, `uart_2ndstage=1`), `zephyr.bin` and the firmware's `bcm2712-rpi-5-b.dtb` | `DOCS/config_txt/boot.adoc` (`kernel`, `arm_64bit`, `os_prefix`, `enable_uart`, `uart_2ndstage`, `armstub`); `ZEPHYR/.../rpi_5/doc/index.rst`, "Blinky" and "Serial Communication" |
| The arm64 image the firmware accepts | Zephyr's arm64 image starts with the Linux arm64 Image header: a branch to `__start`, `text_offset` 0 ("Image load offset from start of RAM"), the effective image size (`_flash_used`), flags with the page size, and the magic `ARM\x64`. Zephyr links the image at `sram0`, 0x200000. `kernel_address` is not in the config documentation read. So: the Pi 5 Zephyr route works with the Linux image header and a link address of 0x200000, and no `kernel_address` key is in the board page | `ZEPHYR/arch/arm64/core/header.S` (the `_image_header` section); `ZEPHYR/dts/arm64/broadcom/bcm2712.dtsi` (`sram0: memory@200000`, `reg = <0x0 0x200000 0x80000>`); `DOCS/config_txt/boot.adoc` (no `kernel_address`) |

**Public documentation.** Found: the product brief and the hardware page the
Zephyr board page links (`https://datasheets.raspberrypi.com/rpi5/raspberry-pi-5-product-brief.pdf`,
`https://www.raspberrypi.com/documentation/computers/raspberry-pi.html`). The
hardware page was read in part: its characters 100,000 to 200,000, which hold
the "EEPROM boot flow", "Fail-safe OS updates (tryboot)" and `BOOT_ORDER`
sections cited in this note. The product brief was not opened. The boot,
`config.txt` and EEPROM pages are in the documentation repository and were
read (above). Mesa's packet XML and Linux's `v3d` driver are the register
and command-stream documentation for the GPU. For the BCM2712 itself, whether
Broadcom or Raspberry Pi publish a register manual is **unknown** (U1): the
Pi 4 work used none (`repo:docs/notes/pi4-v3d-bringup.md` cites only Linux and
Mesa).

## 2. The open-source stack today

- **Mesa**: the same gallium driver as the Pi 4, `MESA/src/gallium/drivers/v3d/`,
  built twice from the same sources, once per generation: `v3d_versions =
  ['42', '71']` with `-DV3D_VERSION=<ver>` (`meson.build:40-51`). The files that
  differ by generation are the `v3dx_*.c` ones (`v3dx_job.c`, `v3dx_draw.c`,
  `v3dx_emit.c`, `v3dx_rcl.c`, `v3dx_state.c`, `v3dx_tfu.c`); the `#if
  V3D_VERSION == 42` / `>= 71` blocks are the whole difference at that level.
  The compiler is shared: `MESA/src/broadcom/compiler/` switches on
  `devinfo->ver >= 71` / `has_accumulators` (about 120 lines in
  `compiler/*.c` and `qpu/qpu_instr.c`, `qpu_pack.c`, by a grep count).
- **Vulkan** (`v3dv`) shares `src/broadcom/`; not our concern.
- **How complete for ES 2.0 work.** The screen caps for 7.1 are the same as for
  4.2 plus a few: `depth_clip_disable` and `clip_halfz` only on >= 71
  (`v3d_screen.c:371-372`), `lower_fsat` only on < 71 (`:631`), subgroup caps
  on >= 71 (`:397-402`). The driver exposes compute when the kernel has the CSD
  (`caps->compute = screen->has_csd`, `:294`) and reports `glsl_feature_level`
  330 (`:307`). Mesa's own CI on 7.1 is not something this study read, so
  "complete for ES 2.0" rests on: the driver is one source for both
  generations and ES 2.0 is a subset of what it implements. A measured pass
  rate on a Pi 5 is **unknown** (U9).
- **Kernel**: Linux `drivers/gpu/drm/v3d/`. What its job ABI looks like to a
  client is readable in `MESA/include/drm-uapi/v3d_drm.h`: the same ioctls for
  both generations (`SUBMIT_CL`, `SUBMIT_TFU`, `SUBMIT_CSD`, perfmon), the only
  7.1 additions being `struct drm_v3d_submit_tfu` gaining `v71 { ioc; pad; }`
  (`v3d_drm.h:344-346`). We do not use the kernel ABI (no OS GPU driver), so
  what we need from Linux is its **hardware** sequence: register map, MMU, SMS,
  power, clock, job start and interrupts (section 3.4).
- **Firmware for the GPU**: the kernel driver loads **no** firmware for the
  GPU: the probe, job-start and reset code read in section 3.4 shows no
  firmware request (H1 grepped `drivers/gpu/drm/v3d/*.c` of v6.18 for
  `request_firmware`: no match), and the host writes control lists and starts them by registers, as on the
  Pi 4 (`repo:docs/notes/pi4-v3d-bringup.md`, "What is new"). The only
  firmware the GPU depends on is the VideoCore firmware's **clock and power**
  service: the fork's device tree gives the V3D `clocks = <&firmware_clocks 5>`
  and a power domain and a reset on the `pm` watchdog node (section 3.4), the
  same clock id 5 the Pi 4 used.
- **ISA and command-stream knowledge we would rely on**, all in Mesa 26.2.1:
  `src/broadcom/qpu/` (`qpu_instr.[ch]`, `qpu_pack.c`, `qpu_disasm.c`: the
  packer, unpacker and disassembler for both generations),
  `src/broadcom/compiler/` (NIR to VIR to QPU), `src/broadcom/cle/v3d_packet.xml`
  and its generator, `src/broadcom/clif/` and `cle/v3d_decoder.c` (a decoder of
  a recorded job), `src/broadcom/simulator/` (`v3dx_simulator.c`, the
  vendor simulator wrapper; not buildable without the vendor library).

## 3. What yakogl already has, and what 7.1 changes

### 3.1 The verdict

The **driver side, the shader back end's ISA layer and the register file of the
compiler are generation-specific; the GL core, the OSAL shape, the job
structure, the texture and tiling code and the shader front end carry over.**
A new directory is not needed for the GL side: `src/v3d/` (the backend
tables, 6,399 lines in `backend.rs`) is written against the control-list model,
which 7.1 keeps (binning list, render list, tile list, tile allocation, TSDA,
shader state record). Three places hold 4.2 as a constant and need a second
value:

1. `driver/src/v3d/` (5,000 lines of C): the packet encoders are generated from
   the 4.2 half of the XML (`driver/src/v3d/generated/v3d_packet_v42_pack.h`,
   `v3d_cl_packets.h` names `V3D42_*`), the registers are 4.2's
   (`driver/src/v3d/v3d_regs.h`), the power-on is the BCM2711's
   (`v3d_hw.c`, PM and ASB blocks).
2. `src/shaderc/online/v3d/` (20,588 lines): the ISA module is 4.2. The QPU
   FFI hands Mesa a device of `ver = 42` (`repo:src/shaderc/online/v3d/qpu_mesa.c`,
   `yakogl_v3d42_devinfo`), and `docs/online-shaderc/v3d-backend.md` records
   the 4.2 choices (accumulators, SFU through magic addresses).
3. `tools/v3d/`: `v3d_shc.c` builds a device of `ver = 42`
   (`tools/v3d/v3d_shc.c:677-680`); `cl_golden.c`, `frame_check.c`,
   `hw_mock_test.c` check the 4.2 packets and registers.

Support in the vendored tools that makes this cheaper than it sounds:
`tools/v3d/mesa_ref.sh` already generates `v3d_packet_v{21,42,71}_pack.h`
and builds Mesa's decoder, which loads the XML for a given version; and
Mesa's drm-shim already has a 7.1 GPU: `MESA/src/broadcom/drm-shim/v3d_noop.c`
lists `"71"` first and `"42"` second and picks by `V3D_GPU_ID`, default `71`
(`:43-70`, `:213`).

### 3.2 Packets (read from `v3d_packet.xml`)

Where a packet exists in both generations, the 7.1 one is the `min_ver="71"`
entry. Differences that change bytes we write today:

| Packet or struct | 4.2 | 7.1 | Lines |
|---|---|---|---|
| Clear (code 25) | `Clear Tile Buffers` (flags for Z/stencil and all RTs) | `Clear Render Targets`, no fields; Mesa emits it in both the clear-only tile list and the generic one | `:480,485`; `v3dx_rcl.c:331-340`, `:647-656` |
| Blend Cfg (84) | RT mask 4 bits | RT mask 8 bits (eight render targets) | `:777-795` |
| Depth Bounds Test Limits (93) | absent | new, two floats | `:826` |
| Cfg Bits (96) | has `Early Z enable`, `Early Z updates enable`, 2-bit `Line Rasterization` | no early-z bits; adds `Z Clipping mode` (22, 2 bits, enum `Z Clip Mode`), `Z Clamp Mode` (10), `Depth Bounds Test Enable` (5); 1-bit `Line Rasterization` | `:831-870`, enum `:322` |
| Clipper XY Scaling (110) | half-size in 1/256 px | in 1/64 px | `:919-927` |
| Clipper Z no guardband (112) | absent | new | `:934` |
| Tile Binning Mode Cfg (120) | width, height, 4x flag, double-buffer, max BPP, number of RTs | width, height, `Log2 Tile Width/Height` (8 to 64 px); the MSAA, BPP and RT-count fields are gone from this packet | `:943-994` |
| Tile Rendering Mode Cfg (121) | Common, Color, ZS Clear Values, Clear Colors Part1/2/3 (sub-ids 0 to 5) | Common (adds log2 tile size and `Depth-buffer disable`, drops max BPP), ZS Clear Values, and **Render Target Part1/2/3** per RT (sub-ids 1 to 4): internal type with clamp (5 bits, enum `Render Target Type Clamp`), BPP, stride (7 bits), base address (11 bits, units of 512 bits), then 40 + 56 bits of clear colour | `:996-1157` |
| GL Shader State Record | 36 bytes; has `Address of default attribute values`, the two `separate input and output VPM blocks` flags | 32 bytes; those three fields are gone; adds `Never defer FEP depth writes` (bit 20); the uniforms address of each stage moves 4 bytes down | `:1197-1254` and `:1256-1311` |
| GL Shader State Record Draw Index | absent | for 7.1.10 only (`v3d_device_has_draw_index`): extra `cs_basevertex`, `cs_drawindex`, `vs_basevertex`, `vs_drawindex` bits | `:1313-1342`; `v3d_device_info.h:94-97` |
| TMU Config Parameter 2 | LOD query bit | adds `DIM query` (25) and `Write conversion` (26) | `:1560-1590` |
| Texture Shader State | `AHDR`, `sRGB`, `Reverse Standard Border Color`, "Array Stride" 26 bits at 32 | `Transfer func` (3 bits: none, sRGB, PQ, HLG, PQ BT.1886, HLG BT.1886), `Reverse`, `R/B swap` (7.1.5 and later), `Array Stride` 24 bits at 33, chroma pointers `Cb`/`Cr` and offsets | `:1592-1707` (the 7.1.2 / 7.1.5 incompatibility is the comment at `:1686-1690`) |
| Texture data formats | no BSTC | adds `bstc8_srgb`, `bstc10*`, `rgba10x6*`, `rgb10_a2_{hlg,pq}_bt1886*` (values 46 to 63) | `:271-284` |
| Render target maxima | 4 targets, 4096 px | 8 targets, 7680 px | `MESA/src/broadcom/common/v3d_limits.h:44,63` |
| Clipper granularity, CLE read-ahead | 256.0, 256 bytes | 64.0, 1024 bytes | `v3d_device_info.c` (the `case 42` / `case 71`) |

TFU (the texture formatter our texture and mipmap paths use): the
register layout moves. 4.2 keeps format and mip count in `ICFG` and `IOA`
(`V3D33_TFU_ICFG_*`, `V3D33_TFU_IOA_*`); 7.1 puts the output format, stride,
`NUMMM` and `DIMTW` in a **new register `IOC`**, moves `ICFG` fields (`OTYPE`
shift 16, `IFORMAT` shift 23), and can write raster output, which 4.2 cannot
(`MESA/src/broadcom/common/v3d_tfu.h`; `v3dx_tfu.c:58-60`, `:165-202`). Our
`driver/src/v3d/v3d_regs.h` has `ICFG`, `IOA`, `IOS` but no `IOC`.

CSD (compute): 7.1 has no `PROPAGATE_NANS` bit in `cfg[5]`, and from 7.1.6 the
batch count is not minus one (`v3dx_draw.c:1519-1535`). We use no compute for
ES 2.0, so this only matters if ES 3.1 compute is ever taken.

### 3.3 The shader ISA (read from `src/broadcom/qpu/` and `compiler/`)

Mesa's own comment says it: "for v3d71 there were a lot of changes"
(`qpu_pack.c`, above `v3d71_add_ops`). The compiler and QPU packer are
therefore two ISAs behind one `struct v3d_qpu_instr`:

- **No accumulators.** `devinfo->has_accumulators = ver < 71`
  (`v3d_device_info.c`). Magic write addresses 0 to 4 (`r0` to `r4`) are
  "Reserved on V3D 7.x", address 5 is `r5` on 4.x and `QUAD` on 7.x, 55 is `REP`
  on 7.x (`qpu_instr.h:94-100,137`). The signal map has a separate 7.1 table
  whose last column is **`RF0`**: `ldunif`, `ldtmu` and similar write the
  implicit destination `rf0` instead of `r5`/`r4` (`qpu_pack.c:164-` against
  `v3d42_sig_map`). The register allocator is split on `has_accumulators`
  (`vir_register_allocate.c`, about 20 sites), and the scheduler has a 7.1
  branch at about 25 sites (`qpu_schedule.c`).
- **SFU is no longer a write to a magic address.** On 4.x `recip`, `rsqrt`,
  `exp`, `log`, `sin`, `rsqrt2` are magic write addresses 19 to 24; they are
  "Reserved on V3D 7.x" (`qpu_instr.h:115-120`) and appear as ALU add ops
  `V3D_QPU_A_RECIP`, `_RSQRT`, `_EXP`, `_LOG`, `_SIN`, `_RSQRT2` in the 7.1
  table (`qpu_pack.c`, `v3d71_add_ops`). Our back end's SFU lowering
  (`docs/online-shaderc/v3d-backend.md`, "SFU precision") is 4.2's write-then-
  read-`r4` pattern and must be redone.
- **Operand encoding.** The 4.2 opcode tables are keyed on `mux.a`/`mux.b`
  (`v3d42_add_ops`, `qpu_pack.c:457`); the 7.1 tables on `raddr` masks with
  four read addresses (`raddr_a` to `raddr_d`) and a small-immediate bit per
  operand, `small_imm_a` to `small_imm_d` ("since V3D 7.x",
  `qpu_instr.h:56-59`, `:438-455`). This is the repository note's "four
  register-file read ports instead" (`repo:docs/notes/pi4-v3d-bringup.md`,
  "Why the Pi 4 and not the Pi 5").
- **New operations** present in the 7.1 tables and the compiler (checked at
  `nir_to_vir.c:1399-1444,1844-1910,4095-4176`): half-float vector ops
  `VFADD`, `VFSUB`, `VFMUL`, `VFMIN`, `VFMAX`, `VFNEG`, `VFABS` and
  `VFPACK` forms, `FTOUNORM16`, `FTOSNORM16`, `FUNPACK{U,S}NORM{LO,HI}`,
  `VFTOUNORM8`, `VFTOUNORM10{LO,HI}`, `VFTOSNORM8`, `V8DOT`, and the subgroup
  group `BALLOT`, `BCASTF`, `SHUFFLE`, `ROTQ`, `ROT`, `ALLEQ`, `ALLFEQ`. For
  ES 2.0 none is required; the `nir_op_fneg`, `fadd` and so on take the 16-bit
  path only for `bit_size == 16`.
- **Thread-switch, TMU and VPM** behave as on 4.2 in the parts the Pi 4 back
  end uses (the 7.1 scheduler adds rules for `rf0` and TMU delay,
  `qpu_schedule.c:2018-2111,2694-2716`); whether any of them changes what our
  compiler already emits is exactly what the host comparison of section 5
  answers.

### 3.4 Registers

Read from `LINUX/drivers/gpu/drm/v3d/` v6.18 (`v3d_regs.h`, `v3d_drv.c`,
`v3d_mmu.c`, `v3d_irq.c`, `v3d_sched.c`, `v3d_gem.c`):

- **The part that does not move.** Hub `IDENT0..3` (0x08 to 0x14, the field
  layout is not versioned), `MMU_DEBUG_INFO` (0x1238: `PA_WIDTH` 11:8,
  `VA_WIDTH` 7:4), `CLE_CT0QBA` (0x160), `L2TCACTL` (0x30) and `SLCACTL` (0x24)
  have no version conditionals in `v3d_regs.h`. The Pi 4 cache-flush and bin
  start registers of `v3d_regs.h` (ours) therefore need no change by this
  evidence.
- **What moves at 7.1** (`v3d_regs.h` writes the macro as `(ver)`): the **TFU
  block** from 0x400 to 0x700 (`CS` 0x700, `SU` 0x704, `ICFG` 0x708, `IIA`
  0x70c, `ICA` 0x710, `IIS` 0x714, `IUA` 0x718, **`IOC` 0x71c, new**, `IOA`
  0x720, `IOS` 0x724, `COEF0..3` 0x728 to 0x734; `COEF0_USECOEF` and `CRC` are
  4.x only); the **GMP** block (`STATUS` 0x600, `CFG` 0x604, `VIO_ADDR` 0x608
  against 0x800 to 0x808); the **CSD** configuration registers (0x930 to 0x948,
  current 0x958 to 0x97c, plus `CFG7` at 0x94c / 0x974, against 0x904 to 0x91c /
  0x920 to 0x940); the **interrupt bits** (`INT_CSDDONE` bit 6 against 7,
  `INT_PCTR` bit 5 against 6, a new hub bit `GMPV` 6); `MMUC_CONTROL_CLEAR` bit
  11 against 3; the performance-counter field widths. The **GCA** block exists
  only below 4.1 and is not on 7.1.
- **SMS** (7.1 only): `REE_CS` at 0x00000 and `TEE_CS` at 0x00400 of a separate
  `sms` register region; bits `POWER_OFF` 30, `CLEAR_POWER_OFF` 29, `LOCK` 28,
  `SVP_MODE_*`, a 4-bit `STATE` field 3:0 (the values are named in
  `v3d_drv.h:287-291`: IDLE 0x0, ISOLATING_FOR_RESET 0xa, RESETTING 0xb,
  ISOLATING_FOR_POWER_OFF 0xc, POWER_OFF_STATE 0xd; **corrected by H1**, the
  summary said there were none).
  The driver maps `sms` only for `ver >= V3D_GEN_71` (`v3d_drv.c:356-360`) and,
  before reading any identity, writes `CLEAR_POWER_OFF` to `TEE_CS` and polls
  `STATE` for `V3D_SMS_IDLE` with a limit of 100 **ms** (`wait_for(COND, MS)`,
  `v3d_drv.h:529`; the summary said 100 iterations), logging "Failed to
  power up SMS" but not failing (`v3d_idle_sms`, `v3d_drv.c:289-303`); power-off
  writes `POWER_OFF` and polls for the off state (`:305-317`). Line numbers
  in this section are 2 lower than the real v6.18 ones; the exact ones are in
  `driver/docs/rpi5-v3d71-facts.md`.
- **Probe order** (`v3d_drv.c`): compatible `brcm,2712-v3d` selects `V3D_GEN_71`
  (`:278-284`); register regions `hub`, `core0` and, for 7.1, `sms`
  (`:346-358`); an **optional** clock, enabled (`:360-368`); `v3d_idle_sms`
  (`:370`); then the DMA mask is `30 + PA_WIDTH` bits and the VA width
  `30 + VA_WIDTH` read from `MMU_DEBUG_INFO` (`:372-378`); `HUB_IDENT1` version
  is `TVER*10 + REV`, checked against the device tree (`:380-385`); the reset
  is optional, the 4.2 "bridge" region is the fallback (`:395-409`); a 4 KiB
  zeroed write-combined MMU scratch page (`:417-423`).
- **MMU** (`v3d_mmu.c`): one level; 4 KiB, 64 KiB (bigpage) and 1 MiB
  (superpage) entries; `VALID` bit 28, `WRITEABLE` 29, `BIGPAGE` 30,
  `SUPERPAGE` 31; the PFN is limited to 24 bits (`BUG_ON(... >= BIT(24))`,
  `:101`), which with 4 KiB pages reads as 36 physical bits (the reader's
  inference, `PA_WIDTH` is the register to confirm). The file mentions no 7.1
  difference. Our Pi 4 table format therefore carries over on this evidence.
- **Job start** (`v3d_sched.c` at v6.18, cited by function name, not by line:
  a second fetch tool gave approximate line numbers that could not be
  verified against a checkout, so no line number is claimed; H1 adds them;
  the same order as the Pi 4):
  `v3d_bin_job_run` writes `V3D_PTB_BPOS` 0, calls `v3d_invalidate_caches`,
  writes `CLE_CT0QMA`/`QMS` when the job has a tile-state area, `CT0QTS` with
  `ENABLE | qts` when it has a tile state size, then `CLE_CT0QBA` (start) and
  last `CLE_CT0QEA` (end): "writing the end register is what starts the job".
  `v3d_render_job_run` calls `v3d_invalidate_caches` and writes `CLE_CT1QBA`
  then `CLE_CT1QEA`; a `/* XXX: Set the QCFG */` comment marks the queue
  configuration as not written. Neither has a `V3D_GEN_71` branch. The TFU job
  (`v3d_tfu_job_run`) writes `IIA`, `IIS`, `ICA`, `IUA`, `IOA`, then on 7.1
  `V3D_V7_TFU_IOC` from `args.v71.ioc`, then `IOS`, `COEF0`, `COEF1..3` (on
  7.1 always, on 4.x only if `COEF0_USECOEF`), and kicks with `ICFG |
  V3D_TFU_ICFG_IOC`; it does not invalidate the caches. The compute job
  (`v3d_csd_job_run`) writes `cfg[1..6]`, on 7.1 `V3D_V7_CSD_QUEUED_CFG7` 0,
  then `cfg[0]` last. One 7.1 revision rule: the indirect work-group count
  rewrite uses `num_batches - 1` when `ver < 71` or `ver == 71 && rev < 6`.
- **Caches and reset** (`v3d_gem.c`, by function): `v3d_invalidate_caches`
  runs the L3 (`v3d_flush_l3`, a GCA flush on hardware older than 4.1), L2C
  (`v3d_invalidate_l2c`, a no-op from 3.3), L2T (`v3d_flush_l2t`) and slice
  (`v3d_invalidate_slices`, `SLCACTL`) steps outermost first; `v3d_clean_caches`
  flushes the TMU write combiner and does a synchronous L2T clean with a poll
  and a timeout log. The cache functions have **no** 7.1 branch. `v3d_reset`
  disables interrupts, idles the GCA, resets the SMS (`v3d_reset_sms`, a no-op
  below 7.1, otherwise a reset request polled until the state leaves the
  isolating/resetting states) and the V3D (`v3d_reset_v3d`: the reset control
  if the node has one, else the bridge), reinstalls the page table, resets the
  interrupts. `v3d_init_hw_state` only calls `v3d_init_core`; it does not
  enable interrupts.
- **Interrupts** (`v3d_irq.c`, line numbers by the reader): two lines: hub
  (`platform_get_irq(pdev, 0)`, name `v3d_hub`) and core 0
  (`platform_get_irq_optional(pdev, 1)`, `v3d_core0`), all shared; a node with
  one interrupt gets a single line `v3d` (`:259-291`). The core handler reads
  and acknowledges `V3D_CTL_INT_STS` (`:107-110`) and handles `OUTOMEM`
  (`:112-120`, edge signalled, schedules the overflow work), `FLDONE` (bin
  done, `:122-125`), `FRDONE` (render done, `:127-130`), `CSDDONE`
  (`:132-135`). The hub handler reads and acknowledges `V3D_HUB_INT_STS`
  (`:159-162`) and handles `TFUC` (`:164-167`), the MMU faults `WRV`, `PTI`,
  `CAP` (`:169-235`, reading `MMU_VIO_ID` and `MMU_VIO_ADDR`, with a separate
  7.1 AXI-id table and a 0x7f mask against 0xff, `:202-224`) and, on 7.1 only,
  the hub `GMPV` bit (`:237-240`). `v3d_irq_enable` sets and clears the core
  and hub masks (`:303-315`). **Out of memory** (`v3d_overflow_mem_work`,
  `:39-84`): allocate a 256 KB binner overflow buffer, flush the MMU, write
  `V3D_PTB_BPOA` and `V3D_PTB_BPOS`; our Pi 4 path already does this.
- **Device tree: the GPU node.** The GPU is **not** in the fork's
  `bcm2712.dtsi`, `bcm2712-rpi.dtsi` or `bcm2712-rpi-5-b.dts` (searched for
  `v3d`, `sms` and `brcm,2712-v3d`; the only `gpu` node is the register-less
  `vc4: gpu`, `RPI/bcm2712.dtsi:446`; mainline v6.18 has none either: H1
  checked), and the first pass of this study found
  none in mainline v6.18's `bcm2712.dtsi` (mainline master was not read). The overlay
  `vc4-kms-v3d-pi5-overlay.dts:89-94` sets `target = <&v3d>; status =
  "okay"`, which names a label defined elsewhere. It is in **`RPI/bcm2712-ds.dtsi`**
  (`bcm2712-rpi-5-b.dts:11` includes it), `v3d: v3d@2000000`, lines 410-424 (the first reading said 409-421):
  `compatible = "brcm,2712-v3d"`; `reg-names = "hub", "core0", "sms"` with
  `reg = <0x10 0x02000000 0x0 0x4000>` (hub, 16 KiB),
  `<0x10 0x02008000 0x0 0x6000>` (core 0, 24 KiB),
  `<0x10 0x02030800 0x0 0x0700>` (SMS, 1792 bytes);
  `power-domains = <&pm BCM2835_POWER_DOMAIN_GRAFX_V3D>`; `resets = <&pm
  BCM2835_RESET_V3D>`; `clocks = <&firmware_clocks 5>`; `interrupts = <GIC_SPI 250
  IRQ_TYPE_LEVEL_HIGH>, <GIC_SPI 249 IRQ_TYPE_LEVEL_HIGH>`; `status =
  "disabled"` (the overlay enables it). The node has no `interrupt-names`,
  so the first interrupt is the hub's (`platform_get_irq(pdev, 0)`) and the
  second the core's: **hub SPI 250, core SPI 249**. `pm` is
  `pm: watchdog@7d200000` (`bcm2712-ds.dtsi:122`, `#power-domain-cells = <1>`).
  The property is spelt `clocks-names = "v3d"` in the source (line 420 of the
  checkout, not 418), not `clock-names`. **H1 answered the question:** it is
  harmless, because Linux asks for the clock with a `NULL` name
  (`v3d_drv.c:362`), which takes the first `clocks` entry; the Pi 4 node of the
  same tree has the same typo (`bcm2711-rpi-ds.dtsi:565`). The addresses use two address cells, as the other nodes of the SoC
  (`0x10 0x02000000` is 0x10_0200_0000). The firmware interface is the
  one of the Pi 4: `raspberrypi,bcm2835-firmware` with `mboxes = <&mailbox>`,
  a `firmware_clocks` child (`raspberrypi,firmware-clocks`, `#clock-cells = <1>`)
  and a `power: power` child (`raspberrypi,bcm2835-power`, `RPI/bcm2712-rpi.dtsi:8-35`),
  and a `firmware-reset` child.

What Mesa shows without Linux: the identity registers read
through the ioctl are the same set (`UIFCFG`, `HUB_IDENT1..3`,
`CORE0_IDENT0..2`, `v3d_drm.h:282-288`); the 7.1 values the Mesa shim reports
are `CORE0_IDENT0 = 0x07443356` (version byte 7, the "V3D" magic), `CORE0_IDENT1 =
0x81001441` (low nibble 1, four slices of four QPUs, 64 KiB of VPM by
`v3d_device_info.c`'s decode), `HUB_IDENT1 = 0x00081117`, `HUB_IDENT3 =
0x00000700` (`v3d_noop.c:44-55`). These are the shim's numbers, **not a
measurement of a Pi 5**. Our `v3d_hw.c` registers (hub `IDENT`, `INT_STS`, MMU,
`CTnQBA/QEA`, `L2TCACTL`, `SLCACTL`) are Linux's 4.x map; of those, `IDENT`, MMU,
`CTnQBA` and the two cache controls do not move at 7.1 (above); the interrupt
bits, `MMUC_CONTROL_CLEAR`, the TFU block and the new SMS block do.

### 3.5 What stays

The GL core, EGL, the state model, `gl_test`, the OSAL contract and its
V3D granule allocator (`src/osal/v3d_granules.h`), the texture layout and UIF
tiling (`driver/src/v3d/v3d_tex.c`, `v3d_tiling_*.c`: the tiling is a property
of the TMU and TLB which Mesa's `v3d_tiling.c` shares for both generations,
`MESA/src/broadcom/common/v3d_tiling.c`), the shader front end and the IR
(`src/shaderc/online/` outside `v3d/`), the job structure of `src/v3d/` (binning
list, render list, flush rules). The cube-map seamless defect of the 4.2 TMU
(`repo:docs/v3d-noseamless-cube.md`) is a 4.2 fact: whether 7.1 filters cube
edges seamlessly is **unknown** (U8); the 7.1 texture state has no seamless
field either, so the answer comes from the hardware or from Mesa's cube
handling for 7.1.

## 4. The board on Zephyr

**Zephyr has the SoC and the board**: `soc/brcm/bcm2712/` and
`boards/raspberrypi/rpi_5/` (`rpi_5/bcm2712`, `rpi_5_bcm2712_defconfig`),
contributed in 2024 (copyright lines of `Kconfig.rpi_5`), in the 4.4.99
workspace.

What it brings up and assumes, as read:

- **One core.** `bcm2712.dtsi` declares exactly one CPU node (`cpu@0`,
  `arm,cortex-a76`) and `rpi_5_bcm2712_defconfig` sets no SMP. The other three
  A76 cores are not started. The Pi 4 image runs the same way (one A72,
  `repo:docs/notes/pi4-v3d-bringup.md`, "Board port"), so this costs nothing
  we have.
- **Memory.** `zephyr,sram` is the SoC's `sram0` node, 512 KiB at 0x200000
  (`bcm2712.dtsi`), the same shape as the Pi 4's before our overlay. The Pi 4
  images resize it per sample (`repo:driver/samples/zephyr/common/rpi_4b_sram.dtsi`
  and the `boards/rpi_4b.overlay` files). The same is needed here for a GPU
  carve-out. The defconfig sets `ARM64_VA_BITS_40` and `ARM64_PA_BITS_40`. The
  image is linked at 0x200000 and carries the Linux arm64 Image header
  (section 1, "The arm64 image the firmware accepts"); what the firmware
  documentation states about the load address is nothing (U6).
- **Interrupts and clock.** GICv2 with 280 interrupts, a 54 MHz timer
  (`TIMER_READS_ITS_FREQUENCY_AT_RUNTIME=y`), `uart10` on SPI 121.
- **Console and mailbox.** `uart10` is the console and shell UART. A
  `brcm,bcm2711-mbox` node (`mbox0`) exists and is `disabled`. The Pi 4
  experience applies: Zephyr's own mailbox driver and our polling client must
  not both run (`repo:driver/samples/zephyr/common/rpi_4b_sram.dtsi`, the long
  comment).
- **What is missing for us.** The tree has **no node** for the V3D, the SMS,
  the display pipeline (HVS, pixel valves, HDMI), any clock controller other
  than one fixed UART clock, or any power domain. No driver exists for them.
  That is the same state as the Pi 4: our own `osal/zephyr_v3d.c` maps the V3D
  blocks with `device_map` (`repo:src/osal/zephyr_v3d.c:210-213`), not
  through devicetree. Addresses, clocks, power and interrupt numbers for the
  Pi 5 come from the BCM2712 device tree and Linux (U4, U5), and from the DTB
  the firmware hands the image (see item D1 in section 6, which reads them on
  the board). For the V3D they are now known from the fork's `bcm2712-ds.dtsi`
  (section 3.4): hub 0x10_0200_0000, core 0x10_0200_8000, SMS 0x10_0203_0800,
  hub SPI 250, core SPI 249, firmware clock 5, a power domain and a reset on
  `pm`.
- **PCIe.** The board enables `pcie1` (the connector) and the defconfig sets
  `CONFIG_PCIE=y`. We do not need PCIe or RP1 for the GPU. RP1 matters only
  for USB, ethernet, GPIO and some display connectors, none needed for
  sign-off.

**Boot route for sign-off.** The chain is: boot ROM, then the EEPROM
bootloader in the board's SPI flash, then the VideoCore firmware, which reads
`config.txt` and starts `kernel=` (section 1, "Boot chain"). The Zephyr board
page documents exactly that with `kernel=zephyr.bin` (`ZEPHYR/.../rpi_5/doc/index.rst`),
and it is the Pi 4 product form we already have (`repo:docs/standalone-boot.md`,
"Raspberry Pi 4", "The product form": firmware, `config.txt`, `kernel=`,
`enable_uart=1`, no U-Boot, no Linux). Nothing read mentions U-Boot for the
Pi 5, so **the vendor firmware loading our image** is the sign-off route, with
U-Boot (loaded by the firmware as `kernel=`) only if that fails. Three keys
differ from the Pi 4 form: `kernel=` must name our file (the Pi 5 default is
`kernel_2712.img`, with `kernel8.img` as the fallback), `arm_64bit` is ignored
(always 64-bit), and the Pi 4's `start4.elf` and `fixup4.dat` are not needed:
the boot-flow section says the Pi 5 firmware loads the kernel instead of
`start.elf`. The board page lists only `config.txt`, `zephyr.bin` and the DTB.

The Pi 4 test uses the firmware's `tryboot` so a failing image falls back to
Linux (`docs/standalone-boot.md`, "The test is the product's boot"). For the
Pi 5 the hardware page says "All Raspberry Pi models support `tryboot`": the
flag, set with `sudo reboot '0 tryboot'` (quoted, one argument), loads
`tryboot.txt` instead of `config.txt`, once; "a crash or reset will cause the
original `config.txt` file to be loaded on the next reboot". Under secure boot
it loads `tryboot.img` instead of `boot.img`. `conditional.adoc` has the
`[tryboot]` filter and `[pi5]` for the Pi 5 family. So **`tryboot` is
documented for the Pi 5 as for the Pi 4** (the page does not name the Pi 5;
the claim is "all models"), and the Pi 4 test form carries over; that it
works with the EEPROM a given board ships with is a day-one trial (U7). A
kernel-based jump (Linux `kexec` into our image) is a development
convenience only.

## 5. What can be done before the board arrives

Each item has its check, and each runs on the host. Items marked (R) need the
facts file of H1 first.

| # | Item | Check |
|---|---|---|
| H1 | **Turn the reading into a facts file.** The Linux reading of this note (the `v3d` driver, the `bcm2712-ds.dtsi` GPU node, the `vc4` HVS) is by summary; write it as a `driver/docs/` facts file with a `linux:` citation per register and a real line number from a sparse checkout of Linux v6.18 (`drivers/gpu/drm/v3d/`, `drivers/gpu/drm/vc4/`) and of the fork's `rpi-6.12.y` device trees, as `driver/docs/rpi4-v3d.md` has. Also read what this note could not: the BCM2712 HDMI PHY initialisation in `vc4_hdmi.c` (past the first 100,000 characters), the Raspberry Pi bootloader-configuration page, and the arm64 `booting.rst`. GPL: facts and file:line only (as for the Pi 4). | The file exists, every register and line in it matches the checkout, and U1, U3 to U7 each name their answer or stay open |
| H2 | **`v3d_shc` for 7.1.** Parametrise the device (`ver` 71, `has_accumulators` false, `vpm_size` 64 KiB, rev) in `tools/v3d/v3d_shc.c:677-680`; the vendored reference build (`tools/v3d/README.md`, "Set up once") already compiles the whole compiler. Run it over `tools/v3d/qpu_corpus.txt` and the femtovg and Qt3D programs used for the Pi 4. | Every program compiles, `v3d_qpu_disasm` of every word reads, and the 7.1 word count per program is recorded beside the 4.2 one |
| H3 | **Mesa GL driver on the drm-shim at 7.1**, the method of `tools/mali/drm_shim_ref.sh`: build `-Dgallium-drivers=v3d`, run with `V3D_GPU_ID=71` (the shim's default) and `MESA_LOADER_DRIVER_OVERRIDE`, draw with a small GL program (`tools/mali/gl_ref.py` shape), and keep the job Mesa submits via `V3D_DEBUG=clif` (the Pi 4 note used it as the byte reference, "Control lists"). The shim's `SUBMIT_CL` is a no-op (`v3d_noop.c`) but the clif dump is written by the driver before the ioctl; **to be confirmed on the first run**. Gives the 7.1 shader binaries, shader state record, texture state and tile-list bytes for the clear, the first triangle and a textured draw, to match byte for byte. | The dump decodes with `MESA/src/broadcom/clif/` for `ver` 71; the corpus of references is stored as the Pi 4's was (`tools/v3d/`) |
| H4 | **7.1 packet encoders.** Add `driver/src/v3d/generated/v3d_packet_v71_pack.h` from `mesa_ref.sh` (already produced there) and a `V3D71_*` name table in `v3d_cl_packets.h`; extend `tools/v3d/cl_golden.c` to compare ours with Mesa's packers for 7.1. | `cl_golden`: all cases equal, as the 4.2 run's 720,024 of 720,024 (`pi4-v3d-bringup.md`, "Written so far") |
| H5 | **7.1 frame builder** (`v3d_frame.c`: binning mode with log2 tile size, render-target parts, `Clear Render Targets`, 32-byte shader record, no default-attribute BO). | `tools/v3d/frame_check.c` at 7.1: Mesa's CLE decoder reads every packet, with its census, at the sizes of the 4.2 run; plus equality with H3's clear and triangle references (R for register parts) |
| H6 | **QPU FFI for 7.1** (`qpu_mesa.c` gets `yakogl_v3d71_devinfo`; the Rust mirror's `layout()` check already runs against Mesa's header). | The mirror test passes against both devices; the round trip decode/encode over every word H2 produces, byte-exact, as the 4.2 gate did |
| H7 | **ISA model and encode/decode for 7.1** (`isa.rs`, `encode.rs`, `decode.rs`): raddr operands, no accumulators, `rf0` implicit destinations, SFU as ALU ops. | The same round trip and `tests/v3d_sim_mesa.rs` with the 7.1 interpreter: Mesa's `qpu_disasm.c` vectors for 7.1 (`MESA/src/broadcom/qpu/tests/`) |
| H8 | **Host-harness row** in `tools/check.sh`: a `v3d71` scope next to `v3d` (the scope pattern is `tools/check.sh:158`). | `tools/check.sh --changed origin/main` reaches it and it is green with 0 gaps |
| H9 | **Zephyr `rpi_5` image built, not run**: `hello_world` and a no-GPU sample with an overlay that resizes `sram0`, plus a build-only gate row if the existing `rpi_4b` rows (`tools/check.sh` Zephyr image rows) are the template. Not attempted by this study. | The ELF links, `zephyr.bin` size is recorded; the board file and SoC build warning-free |
| H10 | **`hw_mock_test` for 7.1** (R): the register model of `tools/v3d/hw_mock_test.c` with the 7.x map, SMS and power sequence. | The Pi 4's 38 of 38 pattern, for 7.1 |

## 6. What needs the board

**First three things on day one** (all headless, no GPU command yet):

1. **D1. Boot and look.** A `kernel=zephyr.bin` image built as in
   `ZEPHYR/.../rpi_5/doc/index.rst` that prints on `uart10` and dumps the
   device-tree blob pointer the firmware passes and the V3D, SMS, HVS and
   clock nodes in it (the Pi 4 boots the same way; the DTB is the ground truth
   for addresses, interrupts and clocks and removes H1's dependence on a
   particular kernel tag). Proof of life: the banner, then the node list.
2. **D2. Read identity with no setup.** Read the hub and core identity
   registers and `MMU_DEBUG_INFO` from the addresses D1 found. If the read
   bus-faults, the GPU is unpowered or unclocked (U5): the sequence Linux
   follows is the firmware property mailbox clock 5 and the `pm` power domain
   and reset (section 3.4, the SMS `CLEAR_POWER_OFF` write and poll first),
   so try the firmware mailbox clock and power calls the Pi 4 uses.
   Proof of life: `CORE0_IDENT0[31:24] == 7`, `CORE0_IDENT1[3:0] == 1`, and
   `HUB_IDENT3` giving rev and compat rev (U2). Record the raw values as the
   Pi 4 note did ("Measured under Linux first").
3. **D3. Boot Linux once and record it.** Raspberry Pi OS on the same card:
   debugfs `v3d_ident` and `v3d_regs`, `vcgencmd get_config`-style clock
   readings, `V3D_DEBUG=clif` dumps of a clear and of one triangle taken on the
   board. These are the byte references that H3 can only approximate, as the
   Pi 4 work did ("The first byte-exact reference is Mesa itself on the same
   Pi under Linux").

**Measurements that say the port is alive**, in order, each with the Pi 4
analogue:

- **First clear** (M1 to M4): power-on returns, identity equal to Linux's, MMU
  initialised, an empty binning and render list both raise their done
  interrupts, and a TLB clear to a raster buffer reads back with 0 wrong
  pixels and an unchanged guard row (`v3d_clear: PASS`).
- **First triangle** (M5): a Gouraud triangle with Mesa's shaders, then ours,
  with 0 wrong inside, outside and edge and the largest channel difference
  within the tolerance of 3 (`v3d_triangle: PASS`).
- **First `gl_test` case**: the list the Pi 4 started with, cases 5, 58, 59, 66,
  67, 102 on one context (`pi4-v3d-bringup.md`, "gl_test on the V3D"), with the
  same refusals by name where a feature is not built yet.
- Display is **not** needed for any of these: the Pi 4 checks read the
  buffer back. The panel comes after (U3).

## 7. Risks and unknowns

Each is a question; none is guessed.

| # | Question | What answers it |
|---|---|---|
| U1 | Is there a public BCM2712 register manual, and which blocks does it cover? | The Raspberry Pi documentation index (`raspberrypi.com/documentation`); the documentation repository files read (boot, EEPROM, `config.txt`) are not a register manual, and the product brief was not opened |
| U2 | Which V3D 7.1 revision is on the board we will receive (7.1.2, .5, .6, .7 or .10)? It chooses the texture-state layout, the shader-record variant and the unpack ops | D2's `HUB_IDENT3`; `v3d_device_info.h:94-112` for the consequences |
| U3 | Answered in part (section 1): the path is HVS, pixel valve, HDMI, all on the SoC, and the firmware can leave a display running. Open: does a Zephyr image started with `kernel=` find a firmware framebuffer, and does the firmware's property mailbox (as on the Pi 4) still hand one out on the Pi 5? What part does RP1 play for the outputs we use (none for HDMI, by the device tree)? How is the HDMI PHY programmed for BCM2712? | Day one (D1: read `HVS` and `HD_VID_CTL` for a running scanout); `vc4_hdmi.c` beyond the first 100,000 characters; `docs/standalone-boot.md` for the Pi 4 form |
| U4 | Answered in part (section 3.4): register moves, MMU format, SMS power-up, job start, cache steps, interrupts and the out-of-memory handling. Open: the real PA width (`MMU_DEBUG_INFO.PA_WIDTH`) and whether `v3d_sched.c`/`v3d_gem.c` have line-exact citations (the reading gave function names only) | D2; a checkout of Linux v6.18 (H1) |
| U5 | The GPU node gives a firmware clock (id 5), a power domain and a reset on `pm` (section 3.4). Open: is the GPU already clocked and powered when the image starts; does the V3D clock name property (`clocks-names` as read) matter; what does `BCM2835_POWER_DOMAIN_GRAFX_V3D` do on BCM2712 and how is it requested without Linux (the property mailbox power call, as the Pi 4's)? | D2; the fork's `drivers/pmdomain/bcm/bcm2835-power.c` and `firmware_clocks` (not read) |
| U6 | At which address and in which exception level does the firmware start a `kernel=` arm64 image on BCM2712, and what does it pass in the registers (the DTB pointer)? Is any `config.txt` key needed beyond `kernel=`, `enable_uart=1`, `uart_2ndstage=1`? The card needs no `start4.elf` per the boot-flow section; the exact set is the board page's three files. | `config.txt` pages gave the file names only (no load address, no EL, no `kernel_address`); the Linux arm64 `booting.rst` (not read); Zephyr's link address 0x200000 and Image header; D1 |
| U7 | `tryboot` is documented for all models (section 4); does `sudo reboot '0 tryboot'` work on our Pi 5 with the shipped EEPROM, and does the EEPROM need updating for our files? | One trial on the board; the `TRYBOOT` property table of the hardware page (not read) |
| U8 | Does the 7.1 TMU filter cube maps seamlessly (the 4.2 defect `docs/v3d-noseamless-cube.md`)? | The first cube-map case on the board; Mesa's cube handling for 7.1 is not read |
| U9 | How much of dEQP-GLES2 passes on Mesa's 7.1 driver, and does it have known 7.1 erratum workarounds we would need (the 4.2 ones are the `GFXH-*` comments of `v3dx_*.c`)? | `grep -n 'GFXH\|HW-\|erratum' MESA/src/gallium/drivers/v3d/v3dx_*.c`, filtered by `V3D_VERSION`; not done |
| U10 | Does our one-core, 512 KiB-`sram0` Zephyr start leave the GPU's view of memory valid (cache coherence of the A76 with the V3D, DMA address mapping above 4 GiB on 8 and 16 GB boards)? The Pi 4 asked the same of its 8 GB board (`pi4-v3d-bringup.md`, "Board questions") | D2/D3 plus U4's MMU width; keep the carve-out low until measured |
| U11 | How does a new shader-record variant (7.1.10) interact with our fixed 3-programs-per-GL-program model, and does the `Never defer FEP depth writes` flag change early-z behaviour for `gl_FragDepth` shaders? | H3 references, then a `gl_test` case |
| U12 | Does the vendored `third_party/mesa-26.2.1` carry everything H2 to H6 need (the compiler is built from a Mesa tree; the vendored tree has `src/broadcom/{cle,clif,common,qpu}` only)? | `tools/v3d/mesa_ref.sh` and the "vendored reference build" row; the vendored `UPSTREAM` list |

Two things the reading **rules out**: an existing back end cannot be reused
unchanged (the packets and the ISA differ, section 3), and the Pi 4 power-on
code cannot be assumed (the BCM2711 PM and ASB blocks are the SoC's, and Linux
has an SMS block for 7.1, section 3.4). It rules in that the control-list model, the
job structure and the tiling carry over.

## 8. Proposed item list

Sized against what the three ports cost, from `tools/cts/STATUS.md`, the Pi 4
note and `ROADMAP.md`: the Pi 4 went from start (2026-09-23) to its first GL
triangle on the board the next day (M1 to M6, `pi4-v3d-bringup.md`, "On the
board, 2026-09-24"), with every register sequence and packet already read; it
reached 0 failures on dEQP-GLES2 on 2026-10-15/16 (`CHANGELOG.md`, "Results"). The
Mali port is the cautionary case: its compiler work is still being closed
case by case (`STATUS.md`, the NanoPC-T6 sweep: 102 of 256 at 2026-10-27).
Size: S = a day or less once its inputs exist, M = a few days, L = a week or
more. Order is the dependency order.

| Item | Size | Depends on | Done when |
|---|---|---|---|
| P0 The facts file (H1) from a real Linux v6.18 and `rpi-6.12.y` checkout | S | none | U1, U3 to U7 answered with line-numbered citations or left open |
| P1 Reference build at 7.1: `v3d_shc` parametrised, drm-shim run, clif references (H2, H3) | M | none | corpus compiles, references stored |
| P2 7.1 packet encoders and golden test (H4) | S | none | `cl_golden` equal |
| P3 7.1 frame builder and decoder check (H5) | M | P2 | `frame_check` clean; equals P1's clear and triangle |
| P4 `rpi_5` Zephyr image builds, overlay and sample (H9) | S | none | image links, gate row builds it |
| P5 Board M1 to M3: boot, identity, power, MMU, empty lists (D1 to D3, 7.x `v3d_hw.c`) | L | P0, P4, board | identity and both done interrupts |
| P6 Board M4 and M5: clear, triangle with Mesa's shaders (needs only P1 to P3) | M | P3, P5 | `v3d_clear` and `v3d_triangle` PASS |
| P7 QPU ISA layer for 7.1 and 7.1 FFI (H6, H7), round trip over P1's corpus | L | P1 | byte-exact round trip; `v3d_sim_mesa` at 7.1 |
| P8 Online compiler back end for 7.1: selection, scheduling, register allocation without accumulators, SFU as ALU (largest item; the 4.2 back end is 20,588 lines and is the template) | L | P7 | the pass-through, uniform, `if` and texture shapes agree with Mesa's 7.1 output on the host, then on the board |
| P9 `src/v3d/` backend table for 7.1 (record size, no default-attribute BO, the 8-target and 7680 px limits, `Clear Render Targets`, TFU `IOC`) | M | P3, P7 | `gl_test` first six cases on the board |
| P10 Textures and TFU at 7.1 (texture state with transfer function and R/B swap per revision, TMU config 2) | M | P9 | texture cases and mipmaps; cube result (U8) |
| P11 Display (U3) | M to L | P0 | a frame on HDMI; may be a separate item after the CTS |
| P12 `tools/check.sh` rows, `docs/` and `tools/cts` rows for the Pi 5 | S | P9 | gate green, 0 gaps |
| P13 dEQP-GLES2 and glmark2 on the board | L | P8 to P10 | the Pi 4's counts, or the refusals named |

The shortest path to the first triangle on the board is P0, P4, P5, P6 (about
the Pi 4's M1 to M5, with Mesa's shaders). The two items that decide the total
are P8 (the ISA and register allocator change) and P0's
successor risk, U5 and U6 (power and clock of the GPU without Linux, and the
start state of the image), which only the board answers.

## 9. Corrections after the H1 checkout

H1 re-read the Linux sources from real checkouts (Linux `v6.18`, the Raspberry
Pi fork `rpi-6.12.y`, the documentation repository; commits in
`driver/docs/rpi5-v3d71-facts.md`). Where the first reading, by page summary,
differs from the source:

| First reading | Source |
|---|---|
| SMS power-up polls with "a 100-iteration limit" (section 3.4) | The limit is 100 **ms**: `wait_for(COND, MS)` (`v3d_drv.h:529`) |
| The SMS `STATE` field has "no named values in the header" (section 3.4) | Five are named: `v3d_drv.h:287-291` |
| `v3d_drv.c` lines 278-284, 287-301, 303-315, 346-358, 360-368, 370, 372-378, 380-385, 395-409, 417-423 | All 2 lines too low; the real ones are 280-286, 289-303, 305-317, 348-360, 362-370, 372, 374-380, 382-388, 398-412, 420-426. The Pi 4 note's v6.17 `266-294` therefore reads `289-317` at v6.18 (this note had said `287-315`) |
| `clocks-names` "may be the reader's transcription" (U5) | It is exact in the source (`bcm2712-ds.dtsi:420`) and harmless (`v3d_drv.c:362` passes a `NULL` name; the Pi 4 node has the same typo) |
| The GPU node is at `bcm2712-ds.dtsi:409-421`; `hvs` at `bcm2712.dtsi:609` | 410-424; 612 |
| The reader did not grep for `request_firmware` | Done: no match in `drivers/gpu/drm/v3d/` |
| `v3d_sched.c` and `v3d_gem.c` "cited by function name" | Real lines now: `driver/docs/rpi5-v3d71-facts.md`, sections 6 and 7 |
| U5: what `BCM2835_POWER_DOMAIN_GRAFX_V3D` does on BCM2712 | The fork's `bcm2835-power.c:380-383` sets the V3D reset bit (bit 6) in `pm` register 0x304 (`PM_GRAFX_2712`); there is no ASB bridge and no domain clock on this node. Mainline v6.18 has no BCM2712 support in that driver |
| U7: "the `TRYBOOT` property table of the hardware page" | The documentation repository at the commit used has no `TRYBOOT` property in `raspberry-pi/eeprom-bootloader.adoc`; `tryboot` is described in `bootflow-eeprom.adoc:80-98` |
| U6: "config.txt pages gave... no load address, no `kernel_address`" | `config_txt/boot.adoc` names neither, but `legacy_config_txt/boot.adoc:38-40` documents `kernel_address`: 32-bit images load at `0x8000`, 64-bit at `0x200000`, `kernel_old` at `0x0`. The exception level and the register state stay open |

Confirmed unchanged: the register map of section 3.4 (offsets, bits and
version conditions, `v3d_regs.h`), the MMU format and the 24-bit PFN limit
(`v3d_mmu.c:101`), the interrupt handling and its line numbers in
`v3d_irq.c` (they were exact), the hub SPI 250 and core SPI 249 order, the
HVS/pixel valve/HDMI addresses and interrupts, and the output-to-FIFO map.
New in H1: the BCM2712 HDMI PHY sequence, the generic arm64 boot protocol, and
one point to check on the Zephyr side (the GIC interrupt IDs 281 and 282 against
the 280 interrupts the Zephyr device tree declares), all in the facts file.

## 10. H9: the Zephyr `rpi_5` image, built and not run

Built 2026-10-30 in the west workspace `west.yml` pins (Zephyr commit
`af17c0c8aa2a`, `VERSION` 4.4.99), board `rpi_5`, SoC `bcm2712` (board target
`rpi_5/bcm2712`). The gate row "zephyr  rpi5 images (H9)" (`tools/check.sh`,
`zephyr_rpi5_images`) builds Zephyr's own `hello_world` sample and this
repository's `tools/probes/rpi_5/rpi5_hello` (no GPU code), both with
`-DYAKOGL_WERROR=ON` and both with
`tools/probes/rpi_5/rpi5_hello/boards/rpi_5.overlay`, which re-sizes `sram0` to
32 MiB from 0x200000: U10 says keep the carve-out low until measured on a
board, and 32 MiB is the size the Pi 4's first clear and triangle ran with
(`driver/samples/zephyr/gl_test/boards/rpi_4b.overlay`). The Pi 5 is not in the
lab; nothing here ran on a board.

| image | `zephyr.bin` | ELF text / data / bss |
|---|---|---|
| `hello_world` | 45,212 bytes | 42,039 / 3,172 / 147,451 |
| `rpi5_hello` | 45,212 bytes | 42,176 / 3,036 / 147,451 |

Both link at the start of `sram0`: the first LOAD segment is
`0x0000000000200000` (the linker's RAM region is the overlay's 32 MiB, 192 KiB
used), and `zephyr.bin` carries the arm64 Image magic `ARM\x64` at byte 56 —
the header Zephyr links in `arch/arm64/core/header.S` — so the file is what a
`kernel=` line in `config.txt` takes. The entry symbol `__start` is at 0x201084,
not at the segment start, so the row checks the LOAD address, not the symbol.
The board file and the SoC compiled warning-free under `-Werror`: zero compiler
warnings in either build log. The `rpi5_hello` configure printed one CMake
"manually-specified variables were not used" note for `YAKOGL_DIR` — a path the
row passes for the driver module that a hello app never opens, not a board or
SoC warning.

Against U5 and U6, what the Zephyr board file and SoC assume (Zephyr 4.4.99;
lines read from the files):

- **Memory.** The SoC `dts/arm64/broadcom/bcm2712.dtsi` has one memory node:
  `sram0` at `memory@200000`, an `mmio-sram` with `reg = <0x0 0x200000
  0x80000>` — 512 KiB — under two address cells (`:40-44`, `:37-38`); the board
  dts gives the image its code and RAM there (`rpi_5_bcm2712.dts:23`,
  `zephyr,sram = &sram0`), and the defconfig sets 40-bit VA and PA
  (`rpi_5_bcm2712_defconfig:3-4`). So Zephyr assumes the image runs from
  0x200000 — the firmware's documented `kernel_address` for 64-bit kernels
  (section 9) — and says nothing about the RAM above 512 KiB: whether the
  overlay's 32 MiB is free is U10's question, and the overlay is our claim,
  not the board file's.
- **Console.** The board dts makes `uart10` both console and shell
  (`rpi_5_bcm2712.dts:24-25`) `okay` at `current-speed = <115200>`
  (`:44-46`). `uart10` is a PL011 at 0x107d001000, GIC SPI 121
  (`bcm2712.dtsi:88-96`), on a fixed 44,236,800 Hz clock (`:100-104`); the
  system timer is the armv8 arch timer at 54 MHz
  (`soc/brcm/bcm2712/Kconfig.defconfig:9-10`, frequency read at runtime,
  `rpi_5_bcm2712_defconfig:5`). The board's page asks for `enable_uart=1` and
  `uart_2ndstage=1` in `config.txt` (`doc/index.rst:99-101`): which physical
  connector carries `uart10`, and in what state the firmware leaves its clock
  and routing, the files do not say (U6).
- **Boot.** The board page: a Micro SD card with MBR and FAT32
  (`doc/index.rst:45`) carrying three files — `config.txt`, `zephyr.bin` and
  the firmware `bcm2712-rpi-5-b.dtb` from the Raspberry Pi firmware tree
  (`:47-49`, `:136-137`) — `config.txt` reading `kernel=zephyr.bin` (`:59`):
  the `kernel=` route of section 1. No load address, exception level or entry
  register state is stated — in particular whether `x0` carries the DTB
  pointer — so U6 stays open. The image links at 0x200000, which agrees with
  the documented `kernel_address` for 64-bit kernels; that is how far the
  files reach.
- **GPU (U5).** Neither board file nor SoC dtsi has a V3D, SMS, display or
  power-domain node, and `mbox0` (the `brcm,bcm2711-mbox`,
  `bcm2712.dtsi:61-69`) is `status = "disabled"`. What the firmware hands a
  `kernel=` image over in GPU power and clock stays a board question.
- **Interrupts.** The SoC declares `NUM_IRQS` 280
  (`soc/brcm/bcm2712/Kconfig.defconfig:6-7`) for the GICv2 at 0x107fff9000
  (`bcm2712.dtsi:46-53`). The V3D's hub and core interrupts are GIC IDs 282
  and 281 (Linux's SPI 250 and 249, section 9, plus the SPI base of 32):
  above 280. That is section 9's Zephyr-side point, open until a GPU node is
  added and confirmed on the board.
