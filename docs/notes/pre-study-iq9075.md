<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: Qualcomm Dragonwing IQ-9075 EVK (QCS9075, Adreno 663)

Written 2026-10-28, before the board (EVK-IQ9075M-STARTER-1.2) arrives. It
answers the eight questions of the pre-study request. It is a design note:
code and later notes will cite it, so each claim says where it was read.

## How to read this note (what was and was not read)

Every claim carries one of three marks.

- **read**: read in this session, with the file and line. Sources:
  - `mesa:` Mesa 26.2.1, the same release as `third_party/mesa-26.2.1/UPSTREAM`
    (commit `da14d65e4499`). Used as the full tree
    (`src/freedreno/`, `src/gallium/drivers/freedreno/`, `docs/`), because
    `third_party/mesa-26.2.1` holds only the Broadcom, Imagination, Panfrost
    and shared directories. Mesa's freedreno files carry `SPDX-License-
    Identifier: MIT`: they may be read and transcribed with attribution, as
    for the other back ends.
  - `zephyr:` the Zephyr tree the gate builds with (`ZEPHYR_WS`, `VERSION`
    4.4.99).
  - `repo:` this repository at main `965ac247`.
- **not read**: the source exists but could not be read in this session. The
  session had no network access and no copy of the Linux `drivers/gpu/drm/msm/`
  tree (the only Linux tree found, `linux-v6.18-src` in the dependency cache, holds
  only `drivers/gpu/drm/panthor`, `drivers/iommu` and some headers). Qualcomm's documentation and the kernel's `msm` device table
  therefore were **not read**. Section 7 lists each as an open question with the
  file that answers it. Nothing in this note about the `msm` driver, the
  firmware file names, the boot chain, the display controller or the SoC's
  memory map is a finding; where such a thing is mentioned, it is named as
  unknown.
- **inferred**: derived from read facts; the inference is stated.

Legal frame, as for the Mali port (`driver/docs/nanopc-t6-mali.md`, top):
Mesa is MIT, so its register XML and compiler may be read and transcribed with
attribution. The Linux `msm` driver is expected to be GPL (**not read**: its
SPDX lines must be checked before any file is used), and the DRM uapi header
`include/drm-uapi/msm_drm.h` is MIT (Mesa's copy, `mesa:include/drm-uapi/
msm_drm.h:1-20`). Until the licence is read, treat `msm` as "facts and
file:line only". Nothing from Qualcomm's non-redistributable SDK or from the
vendor's firmware images goes into the tree; the firmware files are fetched on
the board's side and turned into a header at build time that is never
committed, as for `pvr_fw_blob.h` and `mali_csffw.bin`.

## 1. The hardware

| Item | Value | Mark and source |
| --- | --- | --- |
| Board | Dragonwing IQ-9075 EVK, EVK-IQ9075M-STARTER-1.2 | the request |
| SoC | QCS9075 | the request |
| GPU | Adreno 663 | the request (Qualcomm's own device description); Mesa's table has the entry, below |
| GPU architecture | a6xx family. Mesa's table gives the 663 the a6xx base plus the "gen4" property set, `CHIP.A6XX` | read, `mesa:src/freedreno/common/freedreno_devices.py:130-154` (a6xx_base), `:226-266` (a6xx_gen4), `:652-684` (the entry) |
| Chip id | `0x06060300` as the kernel reports it (major 6, minor 6, revision 3, patch 0). Mesa's table row is `0xffff06060300` named `FD663`, a row that ignores the fuse bits 47:32 | read, `freedreno_devices.py:654`; the matching rule `mesa:src/freedreno/common/freedreno_dev_info.c:24-65`; the gpu-id is made from the chip id as `major*100 + minor*10 + revision` = 663, `freedreno_dev_info.h:506-515` |
| Same row | the entry `GPUId(chip_id=0x6060201, name="FD644")`, commented "Called A662 in kgsl", shares the block with the 663 | read, `freedreno_devices.py:653` |
| Display path (how a frame reaches the panel or HDMI) | unknown | not read; Section 7, Q4 |
| Boot chain | unknown | not read; Section 7, Q7 and Q8 |
| CPU cores | unknown | not read; Section 7, Q9 |
| Public documentation | see Section 7, Q10 | not read |

What the 663 row says, which the other GPUs of the block share
(`freedreno_devices.py:652-684`): 3 CCUs, tile alignment 96 x 16, tile
maximum 1024 x 1024, 32 visibility-stream pipes, 32 KiB compute shared memory,
wave granularity 2, `fibers_per_sp = 128 * 4 * 16`, and no
`highest_bank_bit` (the 660 row, `:618-650`, sets 16 and `fibers_per_sp =
128 * 2 * 16`; the kernel reports the bank bit and the UBWC swizzle through
`MSM_PARAM_HIGHEST_BANK_BIT` and `MSM_PARAM_UBWC_SWIZZLE`,
`mesa:include/drm-uapi/msm_drm.h:74-95`). The GMEM size is not in Mesa's table:
Mesa reads it from the kernel (`MSM_PARAM_GMEM_SIZE`,
`msm_drm.h:75`). Its value for the 663 is unknown (Q3). The a660 is the
nearest GPU Mesa's drm-shim and CI know: 1.5 MiB of GMEM in the shim
(`mesa:src/freedreno/drm-shim/freedreno_noop.c:291-295`).

One 663-specific fact, read from Mesa's Vulkan driver for the vendor kernel
(`tu_knl_kgsl.cc:1941-1946`): "kgsl unfortunately hardcodes some settings for
certain GPUs", for gpu_id 663 and 680 the macrotile mode is the 8-channel one,
and for the 663 alone `bank_swizzle_levels = 0x4` ("level2_swizzling_dis =
1"). UBWC does not matter to a first, linear-target port (Section 3), but a
display scan-out of a UBWC buffer would (Q6).

## 2. The open-source stack today

**Userspace (read).** Mesa 26.2.1 has both drivers for a6xx, with one shader
compiler, `ir3`, in `src/freedreno/ir3/`:

- `freedreno` (Gallium): "Freedreno GLES and GL driver for Adreno 2xx-6xx
  GPUs. It implements up to OpenGL ES 3.2 and desktop OpenGL 4.5"
  (`mesa:docs/drivers/freedreno.rst:4-5`). The a6xx code is
  `src/gallium/drivers/freedreno/a6xx/` (18 `fd6_*.cc` files, 12,751 lines
  by `wc -l fd6_*.cc`).
- `turnip` (Vulkan 1.3, `freedreno.rst:14`), `src/freedreno/vulkan/`. Not
  our concern.
- How complete for ES 2.0: Mesa's CI runs dEQP-GLES2, GLES3 and GLES31 on an
  **a660** (the same sub-generation: `deqp-freedreno-a660.toml` lists the
  three suites, the renderer check is `FD660`, the version check
  `GL ES 3.2.*git`, `mesa:src/freedreno/ci/deqp-freedreno-a660.toml:1-31`).
  The expected-failure list for it has 152 lines, one of which is a GLES2 case
  (`freedreno-a660-fails.txt`; `grep -c dEQP-GLES2` gives 1), plus 62 flakes
  and 11 skips. So ES 2.0 on this sub-generation is as complete as Mesa's own
  ES 3.2 driver. Mesa's CI has no a663 job (inferred from the file names in
  `src/freedreno/ci/`: a200, a306, a420, a530, a618, a660, a750 only). That
  Mesa passes on an a660 is evidence for the a663 only because the table puts
  them in one block; it is not a measurement.
- Open question on the Gallium side: the Gallium driver is written to run
  next to a kernel that does the memory management, the GPU page tables, the
  fences and the scheduling (Section 3, 4). Everything we would do bare-metal
  that the kernel does today has no Mesa source (Q5).

**ISA and command stream knowledge we would rely on (read).**

- The register definitions are XML, `mesa:src/freedreno/registers/adreno/`:
  `a6xx.xml` (5141 lines, 1770 `<reg` entries), `a6xx_enums.xml`,
  `a6xx_descriptors.xml` (the sampler and texture descriptor domains, 173
  lines), `adreno_pm4.xml` (2447 lines, the command packets), `a6xx_gmu.xml`
  (171 registers of the GMU), `adreno_common.xml`. `gen_header.py` and
  `rules-fd.xsd` in the same directory are the generator and the schema. Same
  idea as Mesa's `genxml` for the Mali (`src/mali/genxml/`, produced by
  `tools/mali/gen_pack_rs.py`): a Python generator over the XML into Rust
  packers, compared byte for byte with a C reference by the gate.
- The ISA is described twice: as XML (`mesa:src/freedreno/isa/ir3*.xml`:
  `ir3.xml` 91 lines, categories 0 to 7 in `ir3-cat0.xml` ... `ir3-cat7.xml`,
  `ir3-common.xml`) with an encoder and a disassembler generated from it
  (`isa/encode.c`, `isa/ir3-disasm.c`), and as the compiler's IR
  (`ir3/ir3.h`, 3420 lines). "Encoding of an ir3 instruction. All
  instructions are 64b" (`isa/ir3.xml:52`). An assembler (`ir3/ir3_parser.y`,
  `ir3_lexer.l`, `ir3_assembler.c`) and a test that round-trips
  (`ir3/tests/disasm.c`) exist. `IR3_SHADER_DEBUG=disasm` dumps NIR and the
  disassembly; `asmroundtrip` "Disassemble, reassemble and compare every
  shader" (`ir3/ir3_compiler.c:26,41`).
- The command stream is PM4 type-7 packets. The ones a first draw needs:
  `CP_LOAD_STATE6` 0x36 (`adreno_pm4.xml:557`, domain `:801`),
  `CP_DRAW_INDX_OFFSET` 0x38 (`:420`, domain `:916`), `CP_SET_DRAW_STATE`
  0x43 (`:419`, domain `:1093`), `CP_EVENT_WRITE` 0x46 (`:313`, domain
  `:1655`), `CP_SET_MARKER` 0x65 (`:483`), `CP_START_BIN` 0x50 (`:630`),
  `CP_SET_BIN_DATA5` 0x2f (`:300`), `CP_BLIT` 0x2c (`:521`),
  `CP_WAIT_FOR_IDLE` 0x26 (`:281`), `CP_REG_TO_MEM` 0x3e (`:302`).
  Most commands write registers (`docs/drivers/freedreno.rst:181-`).
- Shader upload: the driver writes the stage's base address and instruction
  size into registers (`SP_VS_BASE` 0xa81c, `SP_VS_INSTR_SIZE` 0xa824,
  `SP_PS_BASE` 0xa983, `SP_PS_INSTR_SIZE` 0xab05: `a6xx.xml:3813, 3819, 3983,
  4311`), and on a6xx preloads the instruction cache with a `CP_LOAD_STATE6`
  of `ST6_SHADER`, `SS6_INDIRECT`, the shader's BO, sized to
  `min(instrlen, instr_cache_size)` (`mesa:src/gallium/drivers/freedreno/
  a6xx/fd6_program.cc:278-288`). `instr_cache_size` is 127 in the gen4
  properties (`freedreno_devices.py:228-229`).
- Rendering: "Adreno is a mostly tile-mode renderer, but with the option to
  bypass tiling ("gmem") and render directly to system memory ("sysmem")"
  (`freedreno.rst:24-26`). The sysmem path is
  `fd6_emit_sysmem_prep` (`fd6_gmem.cc:2071`) and `render_sysmem`
  (`freedreno_gmem.c:644`); the tiled path is `emit_binning_pass`
  (`fd6_gmem.cc:1007`), `fd6_emit_tile_init` (`:1205`), `..._tile_prep`
  (`:1338`), `..._tile_renderprep` (`:1779`), `..._tile_gmem2mem` (`:1956`).
  A first triangle can use sysmem only (Section 6).
- Textures and samplers are "bindless" descriptors on a6xx; the old bindful
  registers remain (`freedreno.rst:225-250`).
- The shader's threads: the compiler knows full and half registers
  (`IR3_REG_HALF`, `ir3.h`), a scalar ALU with shared registers on gen3 and
  gen4 (`has_scalar_alu`, `freedreno_devices.py:249`), a preamble
  (`has_early_preamble` is commented out for the gen4 because of an `rcp`
  quirk, `:253-255`), the (ss), (sy), (jp), (ul) flags (`isa/ir3.xml:73-76`)
  and a 64 or 128-thread wave (`IR3_DBG_THREAD64`, `ir3_compiler.c:42`;
  `supports_double_threadsize = True`, `freedreno_devices.py:139`).

**The kernel driver and the firmware (not read).**

- The userspace-kernel boundary is read: the DRM uapi, `MSM_PARAM_*`
  (`msm_drm.h:74-128`), and the list of ioctls Mesa's shim implements
  (`freedreno_noop.c:166-181`: `GET_PARAM`, `SET_PARAM`, `GEM_NEW`, `GEM_INFO`,
  `GEM_CPU_PREP/FINI`, `GEM_SUBMIT`, `WAIT_FENCE`, `GEM_MADVISE`,
  `SUBMITQUEUE_NEW/CLOSE/QUERY`, `VM_BIND`, `PERFCNTR_CONFIG`). The shim
  reports DRM version 1.9.0 (`freedreno_noop.c:391-393`).
- That the a6xx family on the msm driver loads a **GMU** and an **SQE**
  firmware is read from Mesa, which names them: `a6xx_gmu.xml` exists, the
  documentation calls the SQE the "a6xx+ replacement for PFP/ME. This is the
  microcontroller that runs the [command stream]" (`freedreno.rst:55`) and
  names its file as `/lib/firmware/qcom/aXXX_sqe.fw` (`freedreno.rst:349-353`,
  the example is `a650_sqe.fw`). The GMU's host interface appears as the
  registers `GMU_CM3_BOOT_CONFIG` (0x1f801), `GMU_CM3_FW_INIT_RESULT`
  (0x1f81c), `GMU_CM3_CFG` (0x1f82d), `GMU_RPMH_CTRL` (0x1f8e8),
  `GMU_HFI_CTRL_STATUS` (0x1f980) (`a6xx_gmu.xml:65-152`), and the HFI
  message queue's header and message ids are in `mesa:src/freedreno/decode/
  crashdec-hfi.c:11-45` (queue header of 12 words, message header: id bits
  7:0, size 15:8, type 19:16, sequence 31:20). That is a record of an
  interface, not a driver.
- Whether the 663 needs a third firmware (a zap shader for the secure
  world), which of the file names it uses, where they come from
  (`linux-firmware` or the vendor image), and how the kernel's device table
  describes the 663 are **not read** (Q1, Q2).

## 3. What yakogl already has that applies

**Result: a new back end, `gpu-adreno`, with a new shader ISA back end. Nothing
of `src/v3d`, `src/pvr`, `src/vivante` or `src/mali` extends to this GPU below
the `backend.h` line.** What does carry over is the part above it: the GL core
(`src/gl`), the GLSL ES front end and its IR (`src/shaderc/online/`: lexer,
parser, sema, lower, fold, eval, the shared `select.rs`/`codegen.rs`/`backend.rs`
interface), the OSAL interface (`include/gles_osal.h`), `backend.h`'s table
(`src/backend.h:1-12` describes the boundary: the core never sees a register),
and the test tools (`gl_test`, dEQP, glmark2 on Zephyr).

Differences, by level, read from Mesa and from the repository:

| Level | Adreno 663 (Mesa) | What our back ends have | Consequence |
| --- | --- | --- | --- |
| Registers | one flat register space defined by XML; a6xx adds a "pipelined register" model: the CP writes registers, with state groups selected by `CP_SET_DRAW_STATE` (`adreno_pm4.xml:1093`; `freedreno.rst:168-198`) | PVR: control-stream words (`csbgen`); V3D: control-list records; Vivante: command words with a register address in the header; Mali: genxml descriptors plus a CS | A new packer generated from `a6xx.xml`; same method as `tools/mali/gen_pack_rs.py` |
| Packets | PM4 type 7 (`CP_*`, 0x00-0x7f, `adreno_pm4.xml:257-641`), submitted as an indirect buffer to a ring the SQE reads | Vivante: its own FE command stream; V3D: binner and render control lists; PVR: firmware-queued kicks; Mali: command stream and doorbell | The frame builder is new; the structure is nearest Mali (a microcontroller reads a command buffer) |
| Execution model | tile-based with an optional direct (sysmem) mode, a visibility stream from a binning pass, GMEM and the CCU caches (`freedreno.rst:24-26,130`; `fd6_gmem.cc:1007`) | V3D and Mali: tilers with their own memory model; PVR: tile accelerator; Vivante: immediate mode with no tiles | The first triangle in sysmem needs no binning pass; ES 2.0 completeness probably needs both (Q11) |
| Shader ISA | 64-bit instructions in eight categories; full and half (16-bit) registers; a vector constant file; (rpt) repeat; (ss)/(sy) synchronization flags; a scalar ALU and shared registers on gen3 and gen4 (`ir3.xml:52`; `ir3-cat0.xml` .. `ir3-cat7.xml`; `ir3.h`) | V3D QPU (64-bit, `src/shaderc/online/v3d/isa.rs`), PowerVR USC/pco (`powervr/isa.rs`), Vivante (`vivante/isa.rs`), Mali Valhall (64-bit with a flow field, `mali/`) | A new ISA table, encoder, decoder, register allocator and scheduler (nearest in size to `mali/`) |
| Vertex fetch | fixed function decodes attributes into registers before the shader (`VFD_*` registers), the compiler learns the input count (`vs_max_inputs_count = 32` for a6xx, 16 for the low variants, `freedreno_devices.py:146,174`) | PVR and V3D: fetch in hardware too; Vivante: attribute units | Same split as V3D; no vertex-fetch code in the shader |
| Varyings | `SP_VS_OUTPUT_*` / `VPC_*` registers (`a6xx.xml:3707,3168`); the linker (`ir3_shader.h`) sets the layout | similar to Vivante and V3D | New tables |
| Textures | 64-byte descriptors in memory (a6xx_descriptors.xml) fetched by cat5 instructions with a bindless or bindful index | V3D: TMU configuration; PVR: image/sampler words; Mali: texture/sampler descriptors | New |
| Memory management | kernel creates GPU address space and BOs (`GEM_NEW`, `VM_BIND`, `MSM_PARAM_VA_START`: `msm_drm.h:87`); in a Zephyr image, we do it ourselves (SMMU with the SQE-switched page tables is **not read**, Q5) | PVR (MMU via firmware), V3D (MMU), Vivante (MMU), Mali (AS0..7) | A new MMU layer or the Linux uapi, see Section 4 |
| Firmware | SQE and GMU, probably a zap shader (**not read**) | PVR firmware, Mali CSF firmware | See Q1, Q2 |

Which existing back end is nearest, by structure and by method: the **Mali**.
The Mali port runs on the board's Linux through the panthor uapi first
(`driver/docs/nanopc-t6-mali.md`; `CHANGELOG.md` "Known limits": "The `gpu-mali`
back end runs only on the NanoPC-T6's own Linux, through the panthor kernel
driver, not on Zephyr yet (#1281)"), and builds its byte reference from
Mesa's own driver on a drm-shim. Adreno has the same two properties the
Mali has: a firmware microcontroller between the host and the shader cores,
and Mesa's userspace already running on a shim (`freedreno_noop.c`, with the
README: "implements the minimum of msm in order to make shader-db work. The
submit ioctl is stubbed out to not execute anything").

**`src/shaderc/online` question.** The shared front end and IR are reused. The
Mali back end shows the size of a back end that sits on them: `mali/` is
18.5k lines without tests. The Adreno back end would be in the same range,
because ir3 is a register-machine ISA of similar richness (Section 8).

## 4. The board on Zephyr

**Zephyr does not have this SoC, nor any Qualcomm Arm SoC (read).** Evidence,
all in the `ZEPHYR_WS` tree (`VERSION` 4.4.99):

- `soc/` has no `qcom` directory (the vendors listed run from `adi` to
  `xlnx`; `ls soc` shows none for Qualcomm). `dts/arm64/` has `allwinner
  armv8-a.dtsi armv8-r.dtsi armv9-a.dtsi broadcom fvp intel nxp qemu renesas
  rockchip ti xilinx`, and `dts/arm/` has no Qualcomm directory either.
- `boards/qcom/` holds one board, `qcc744m_evk`, whose SoC in
  `boards/qcom/qcc744m_evk/board.yml` is `bl618m05q2i`, a Bouffalo Lab part
  (`soc/bflb/` exists); it is a Wi-Fi/Bluetooth RISC-V module, not an
  application processor. `dts/bindings/vendor-prefixes.txt:587` has the `qcom`
  prefix for it.
- A search of `boards`, `soc` and `dts` for `adreno`, `sa8775`, `qcs9075`,
  `lemans`, `snapdragon`, `qcs6490` and `kryo` finds nothing.

So the SoC, the board and every device node would be written by us. The repo
already does this for the AM62P (`driver/soc/ti`, `driver/boards/toradex/
verdin_am62p`), the i.MX8MP (`driver/boards/toradex/verdin_imx8mp`) and the
NanoPC-T6 (`driver/boards/friendlyarm/nanopc_t6`); Zephyr's generic Armv8-A and
Armv9-A bases (`dts/arm64/armv8-a.dtsi`, `armv9-a.dtsi`) exist. Whether
Zephyr can run on this SoC's cores and its interrupt controller (a GICv3 is
likely but **not read**), the timer, the UART (what console), the memory map,
the clocks and power domains for the GPU (on Qualcomm SoCs these sit behind a
resource manager called RPMh; Mesa's GMU registers include `GMU_RPMH_CTRL`,
`a6xx_gmu.xml:120`, but who votes for the GPU's rails and clocks when no
Linux runs is **not read**, Q5) is all unknown. The same goes for the
display's memory-map and interrupts (Q4).

**Boot route for sign-off.** The rule for the other boards is a boot from the
board's own bootloader (U-Boot or the vendor firmware loading our image), no
host (`docs/standalone-boot.md`; `docs/notes/am62p-direct-boot.md`); the Mali
development route is kexec out of Armbian (`nanopc-t6-mali.md`: "Z1 PASS ...
`t6_hello`, kexec'd out of Armbian"), development only. For this board:

- **Development route (decided now):** boot the board's Linux, run yakogl
  userspace on it through a development OSAL over the `msm` DRM uapi (the way
  `tools/mali/panthor/` does for panthor). This does not need the GMU, SQE or
  zap questions answered: the kernel owns them. It gives the first measurement
  without any of Q1 to Q8, and it needs only Q6 (display) or none (a
  headless readback).
- **Sign-off route (not decidable yet):** whether the board's firmware chain
  can load a bare-metal arm64 image, and whether the GPU can be brought up
  without the kernel (the secure-world and RPMh questions above), are unknown
  (Q7, Q8). If the GPU cannot be started from non-secure code without
  the vendor's Linux, the sign-off image cannot be a Zephyr image with its own GPU
  driver, and the item list has to stop at the development route plus a
  decision from the maintainers. This is the largest risk of the port.

## 5. What can be done before the board arrives

Each item has its check. The checks reuse the Mali instruments
(`tools/mali/`; the table at the top of `tools/mali/README-pan_shc.md`).

1. **A host reference build of freedreno on a drm-shim** (`tools/adreno/
   drm_shim_ref.sh`, like `tools/mali/drm_shim_ref.sh`: `meson setup
   -Dgallium-drivers=freedreno -Dvulkan-drivers= -Dtools=drm-shim
   -Dplatforms= -Dgles2=enabled -Dopengl=false`, then install). The shim
   is `src/freedreno/drm-shim/freedreno_noop.c`; it is selected by
   `MESA_LOADER_DRIVER_OVERRIDE=msm` and `LD_PRELOAD=libfreedreno_noop_
   drm_shim.so` (`drm-shim/README.md`). **The shim has no a663**: its table
   (`freedreno_noop.c:214-326`) lists 630 (default), 618, 660, 702, 730,
   740, 750, 810, 830 and older chips, and `FD_CHIP_ID` must equal an entry's
   `chip_id` (`:345-363`). The build therefore carries one patch, a table
   entry `{ .gpu_id = 663, .chip_id = CHIPID(6, 6, 3, 0), .gmem_size = <Q3> }`
   (a patch file with its own licence, like `tools/mali/mesa-glibc-once_
   flag.patch`). `fd_dev_info_raw` then finds the 663 row by the wildcard
   rule (`freedreno_dev_info.c:49-62`). Until the GMEM size is known, use
   the 660's 1.5 MiB (`freedreno_noop.c:293`) and say so in the record: the
   GMEM size changes the tile layout of the command stream but not the
   shader bytes. **Check:** the build prints a renderer string `FD663`
   (the name of the row, `freedreno_devices.py:654`), and
   `IR3_SHADER_DEBUG=disasm` on a small program prints its disassembly.
2. **Byte references from the shim.** The shim runs the driver's compiler
   and its state emission with no GPU. The userspace writes the "rd" dump
   of each submit itself: `msm_dump_submit` in `mesa:src/freedreno/drm/msm/
   msm_priv.h:119-165` (`RD_CHIP_ID`, `RD_GPUADDR`, `RD_BUFFER_CONTENTS`,
   `RD_CMDSTREAM_ADDR`), enabled by `FD_RD_DUMP=enable` (`freedreno_rd_
   output.c:24,41`), and it is called before the submit ioctl
   (`msm_ringbuffer.c:356`, `msm_ringbuffer_sp.c:136`), so it should work
   against the noop shim; **to be verified** by running it (the pandecode
   counterpart on the Mali port is described in `tools/mali/README-pan_shc.
   md`). The decoder is `src/freedreno/decode/` (`cffdump.c`, `cffdec.c`).
   **Check:** a one-triangle ES 2.0 program gives an `.rd` file whose
   decode shows the `CP_LOAD_STATE6` shader upload, the draw and the state
   groups; its shader bytes are the golden file for the ISA tests.
3. **The ISA table and codec from the XML** (`src/shaderc/online/adreno/
   isa/`, generated from `mesa:src/freedreno/isa/ir3*.xml` by a Python tool,
   like `tools/mali/gen_isa_rs.py` and `tools/mali/isa_ref.py`). **Check:**
   decode the shader of item 2 with our decoder and compare with
   `ir3-disasm` (the program `ir3_disasm` is built by `ir3/meson.build:154-163`),
   then encode and compare the bytes (the Mali gate pins a test count in
   `tools/mali/gate.env` for the same purpose).
4. **The packers from `a6xx.xml` and `adreno_pm4.xml`** (`src/adreno/
   genxml/`, generated by a tool like `tools/mali/gen_pack_rs.py`), compared
   with a C reference built from Mesa's own generated headers (`gen_header.py`
   output), the way `tools/mali/pack_ref.c` is. **Check:** a gate row that
   regenerates them and compares bytes (the existing `mali_generated_
   regenerate` in `tools/check.sh:1691-1694` is the pattern).
5. **An `ir3_shc` tool**, the counterpart of `pan_shc`: Mesa's compiler
   called on the program, printing the facts the driver needs besides the
   bytes (register counts, `constlen`, `instrlen`, the varying and
   attribute layouts, the thread size). Built inside the Mesa tree, never
   shipped (as `pan_shc.c`, `tools/v3d/v3d_shc.c`). The fields of
   `struct ir3_info` (`ir3.h:36-80`: `size`, `sizedwords`, `max_reg`,
   `max_half_reg`, `constlen`, `max_waves`, `double_threadsize`) are the
   list. **Check:** its bytes equal the shim driver's for the corpus the
   Mali port uses (`tools/mali/corpus-verdicts.txt`), with the gaps named
   as `pan_shc` names its gaps.
6. **A host-harness row in `tools/check.sh`:** a new `adreno` scope next to
   `mali` (`tools/check.sh:159-166`, the `scope mali local` block at `:1685`),
   running the `--lib adreno::` tests of `host-test,gpu-adreno,shaderc-online`
   with a pinned test count (`tools/adreno/gate.env`), the two regenerations
   and an `aarch64` check build. The PR that adds the scope also adds the
   `tools/check.sh` path mapping so a change in `tools/adreno` reaches only
   its row.
7. **The Zephyr SoC and board definition, built, not run:** `driver/soc/
   qcom/`, `driver/boards/qualcomm/iq9075_evk/` (names: our choice), with
   the device tree from the memory map and interrupt numbers that Q4, Q5 and
   Q9 find, and a `hello` image that only builds. The gate has no board row
   for it until a board runs one. **Check:** the Zephyr build of the image
   (the `tools/check.sh` image rows are the pattern). This item can start only
   after the Linux device tree and the Qualcomm documentation are read (they
   give the addresses); it is not a code task before that.
8. **Read the five unread sources of Section 7 and update this note**, as a
   follow-up change. This is the cheapest item and unblocks 7.

Not possible before the board: anything that runs the GPU; a statement of
the GMEM size, the firmware names and the display path that is not the
kernel's.

## 6. What needs the board

**First three things on day one**, in order. None of them is Zephyr.

1. Boot the board's own Linux and read the facts that are unknown here:
   `/proc/iomem` and `/proc/interrupts` for the GPU and display blocks, the
   running `msm` driver's boot messages (`dmesg`: the chip id, the firmware
   it loaded, the GMEM size) and `/lib/firmware`. This answers Q1 to Q4, Q6
   and Q9 and fills the `GPU, SoC, board` table the Mali note has
   (`nanopc-t6-mali.md`, "The GPU, the SoC, the board"). Record the output
   in the commit message and the issue, not in a file of the tree
   (`docs/README.md`, "Where a new finding goes"); no addresses or credentials in
   any file.
2. Run Mesa's own freedreno on the board with `FD_RD_DUMP=enable` for one
   ES 2.0 triangle, as `tools/mali/mesa-board-ref.sh` does with pandecode;
   fetch the dump. This is the descriptor reference the frame builder is
   pinned to, and the check on the host shim of Section 5 (the 663
   shader bytes from the board against the shim's).
3. Run our `gl_test` on the board's Linux through the `msm` uapi OSAL, headless,
   with a readback (no display).

**The measurements that say the port is alive**, same ladder as the other
boards:

- the *first clear*: our frame builder's clear-only frame, read back, every
  pixel equal to the clear colour (and the fence of the submit signalled);
- the *first triangle*: one triangle in a small render target, read back, with the
  covered pixels the shader's colour, and the uncovered ones the clear colour
  (the Mali record for the same step: "a red triangle over the left half of
  a 64x64 RGBA8 FBO, 2048 red + 2048 blue, 0 misplaced",
  `nanopc-t6-mali.md`);
- the *first `gl_test` case*: the sweep tool `panthor-gltest --sweep`
  (`tools/mali/panthor/`) would need an Adreno counterpart; a case passes when its checks pass; the board sweep
  ends with the PASS / REFUSED / FAIL counts as in `tools/cts/STATUS.md`.

## 7. Risks and unknowns

Each is a question and the source that answers it. All are unknown now; none
is guessed.

| Q | Question | Source that answers it |
| --- | --- | --- |
| Q1 | Which firmware files does the 663 load (GMU, SQE, and a zap-shader image if any), under which names, and are they in `linux-firmware` or only in the vendor image? Are they redistributable? | the `msm` driver's device table (Linux `drivers/gpu/drm/msm/adreno/a6xx_catalog.c` at v6.18 or later), the `WHENCE` file of `linux-firmware`, the board's `/lib/firmware` |
| Q2 | Does the upstream `msm` driver support the QCS9075 / this SoC's GPU, from which Linux version, with which device tree node (compatible strings, register ranges, interrupts, clocks, power domains, the GMU node, the IOMMU)? | Linux `arch/arm64/boot/dts/qcom/` (the SoC's `.dtsi` and the EVK `.dts`), `Documentation/devicetree/bindings/display/msm/gpu.yaml` and `gmu.yaml` |
| Q3 | What are the 663's GMEM size, UBWC settings and speed-bin / fuse handling? Mesa's table does not say (it has no `highest_bank_bit` for the row, `freedreno_devices.py:652-684`), and kgsl hard-codes a 663 exception (`tu_knl_kgsl.cc:1941-1946`) | the same device table; `MSM_PARAM_GMEM_SIZE` on the board |
| Q4 | What is the display path: which display controller and which output (DisplayPort, HDMI through a bridge, or MIPI DSI), whose firmware initialises it, can it scan out a plain linear buffer, what are its register blocks? | the EVK's hardware user guide and schematic (Qualcomm); the Linux display driver for this SoC and the EVK device tree |
| Q5 | Can the GPU be brought up from non-secure bare-metal code: who powers its rails and clocks (resource manager, GMU), who starts the GMU, is the zap-shader step needed to leave secure mode, and how are the GPU's own address-space tables set (the GPU's SMMU, and whether the SQE switches them)? | the `msm` driver's `a6xx_gmu.c`, `a6xx_gpu.c`, `adreno_gpu.c` and the SoC's IOMMU driver (GPL: facts only); Qualcomm's documentation; Mesa has the register names only (`a6xx_gmu.xml`) |
| Q6 | Does the display accept the buffer layout we render (linear, and with the 663's UBWC quirk if compressed)? | Q4's sources; `tu_knl_kgsl.cc:1941-1946` for the quirk |
| Q7 | What does the board's boot chain look like: the firmware stages before ours, what loads a kernel (UEFI or a Qualcomm loader), whether it can load a bare-metal arm64 image, whether U-Boot can be put in the chain, and what is locked (secure boot, fuses) | Qualcomm's public EVK documentation (hardware user guide, "Qualcomm Linux" boot documentation); U-Boot's Qualcomm support (`board/qualcomm/`) at a named release |
| Q8 | Is the board's sign-off route U-Boot or the vendor firmware loading our image, and does the answer depend on Q5? If the GPU cannot be started without the vendor's Linux, what does "done" mean for this port? | Q5 and Q7; a decision of the maintainers |
| Q9 | Which CPU cores does the SoC have and does Zephyr run on them? Which interrupt controller, timer, UART and memory map? | Qualcomm's documentation; the SoC `.dtsi`; Zephyr's `dts/arm64/armv8-a.dtsi` and `armv9-a.dtsi` as the base |
| Q10 | Which documents are public? Qualcomm's public documentation may cover the EVK (hardware, boot, ports) and not the GPU registers. The register knowledge is then Mesa's XML only. Is it enough? Mesa's XML covers a6xx; whether it is correct for a663 where a663 differs from a660 is shown only by running (Section 6) | Qualcomm's documentation portal (terms of use); Mesa |
| Q11 | Does ES 2.0 completeness need the binning pass (GMEM rendering), or does sysmem rendering pass dEQP-GLES2? Mesa has both (`freedreno_gmem.c:644`, `:721-723` choose between them) and a driver option; the sysmem path may be slower and may need the CCU cache settings | Mesa `freedreno_gmem.c:701-725`; the board |
| Q12 | The GPU's coherence, cache and write-combining rules for memory shared with the CPU in a bare-metal image | Linux `msm_gem.c` and the device tree; the board |
| Q13 | Is the `FD_RD_DUMP` path really independent of the kernel when run against the shim? The code reading says yes (`msm_priv.h:119-165`); not run | Section 5, item 2 |
| Q14 | Is the issue list read? CONTRIBUTING.md and CLAUDE.md say the open work is in the GitHub issues. This session could not read them (no network). The issue for this port should be filed or found, and this note linked | `gh issue list` on signal-slot/yakogl |

Nothing here needs a board address, a credential or an IP; those stay in the
environment (`CLAUDE.md`).

## 8. Proposed item list

Sizes are lines of Rust without tests, from `wc -l` of this repository at main
`965ac247` (back end `src/<gpu>/*.rs`, shader compiler `src/shaderc/online/<gpu>/*.rs`),
rounded; tests, `genxml/` and `isa/` directories are not counted, so the real
sizes are larger by the same factor for each. They are a guide, not a
measurement of the Adreno work.

| Port | GPU back end | Shader back end | State (`tools/cts/STATUS.md`, `CHANGELOG.md`) |
| --- | --- | --- | --- |
| V3D (Pi 4) | 9.2k | 20.6k (with a QPU simulator, 2.6k) | ES 2.0 done, 0 failures |
| PowerVR (AM62P) | 14.5k | 24.6k | ES 2.0 done |
| Vivante (i.MX8MP) | 9.0k | 9.9k | ES 2.0 done |
| Mali (RK3588, Linux only) | 7.9k | 18.5k | 102 of 256 `gl_test` cases pass, 0 FAIL |

Estimate for Adreno (inferred): the GPU back end between the Mali's and the
PowerVR's, 8k to 14k, and the shader back end between the Vivante's and the
PowerVR's, 12k to 25k. Reason: the ISA is richer than Vivante's (eight
categories, half registers, synchronization flags, a scalar ALU) and the GL
front end is shared; the register allocator and scheduler are the large part.
The PowerVR port also carried a firmware bring-up (`driver/src/pvr_fw_boot.c`)
that the Linux development route avoids. The bare-metal part has no
estimate, because Q5 and Q7 are unknown; it could be as small as the PowerVR's
`driver/` or impossible.

**Items, in order.** Each item is closed by a result and a comment on its
GitHub issue; an item the board is needed for says so.

| # | Item | Needs | Check at the end |
| --- | --- | --- | --- |
| A0 | Read the unread sources (Q1 to Q10, Q14), file the issue, update this note | network, Linux tree | the note has no "not read" left for Sections 1 and 7 |
| A1 | Host reference build with the 663 shim entry (`tools/adreno/drm_shim_ref.sh`) and a one-triangle program | A0 optional | renderer `FD663`; disassembly printed; `.rd` file decoded |
| A2 | `a6xx.xml` and `adreno_pm4.xml` packers, generated, with C reference and gate row | A1 | the gate row, byte-equal |
| A3 | ir3 ISA table, encoder, decoder from `isa/*.xml` | A1 | decode and encode round trip against `ir3_disasm` on the A1 shader |
| A4 | `ir3_shc` and the corpus verdicts | A1, A3 | its bytes equal the shim driver's on the corpus |
| A5 | OSAL over the `msm` uapi (development), the frame builder in sysmem mode, host tests against `.rd` dumps | A2 | host test of the clear and triangle frames equal to the dump |
| A6 | Board day one: the three things of Section 6, first clear, first triangle (shader from the shim table, not from our compiler) | the board | readback equals the clear colour, then the triangle |
| A7 | Shader back end: the triangle shapes (selection, register allocation, scheduling, encoding), then the ES 2.0 corpus | A3, A4 | bytes equal to the shim's on the triangle pair, then `gl_test` |
| A8 | Frame builder: binning pass and GMEM (if Q11 requires), textures, blending, depth and stencil, MSAA only if the other ports have it | A5, A6 | `gl_test` sweep counts; dEQP-GLES2 on the board |
| A9 | Display scan-out (Q4, Q6) | Q4 read, the board | a frame on the panel or the monitor, read by the camera tool |
| A10 | Decide the sign-off route (Q7, Q8); if it is bare-metal, the Zephyr SoC and board definition, GMU / firmware / MMU bring-up, direct boot | Q5, Q7, the board | the image boots from the board's bootloader, no host |

Reading A0 first is deliberate: three of the unknowns (Q5, Q7, Q8) decide
whether A10 exists at all, and a port that stops at A9 (Linux-hosted, like the
Mali's present state) is a smaller, still useful result.
