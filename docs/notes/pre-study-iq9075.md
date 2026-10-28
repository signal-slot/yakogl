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
  - `repo:` this repository at main `48f0de66`.
  - `linux:` Linux, read in a shallow, sparse clone of the tag `v6.18` (the
    paths `drivers/gpu/drm/msm`, `drivers/iommu/arm/arm-smmu`,
    `arch/arm64/boot/dts/qcom`, `Documentation/devicetree/bindings/display/msm`)
    and of the tag `v7.0`. A path with no tag is `v7.0`; `linux@6.18:` marks
    a `v6.18` line. `v7.0` is the first tag with the GPU node (Section 2).
  - `lfw:` `linux-firmware`, main at `d0039fd797d7`, the `qcom/` directory and
    `WHENCE`, in a shallow sparse clone.
  - `qdoc:` Qualcomm's public pages on `dragonwingdocs.qualcomm.com` and
    `docs.qualcomm.com`, fetched on 2026-10-28 (the URL is given at the claim).
    The fetch tool returns a summary of a page, so a `qdoc:` claim is the
    summary's statement, and a number not in a quotation was not checked
    against the page's text.
  - `lore:` the public mailing-list copies of the Adreno 663 device-tree series
    (`patchew.org`, `lkml.iu.edu`).
  - `tfa:` the TF-A documentation page for this board.
- **not read**: the source exists but was not read. Five are left after this
  pass: the text of `LICENSE.qcom` (Q1); the `lemans-el2.dtso` overlay (its
  name only); the hypervisor's stage-2 policy (Q5, and in Section 4); the
  DPU formats table `dpu_formats.c` and the plane code (Q4, Q6); and
  Qualcomm's documents that need a login or an agreement (Section 7, Q10).
- **not public: <name>**: a document that Qualcomm lists and that is behind a
  login or an agreement. The page that lists them does not say which are
  (`qdoc:` 80-70029-261 "References"), and this pass did not open them, so
  Q10 names them as "listed, not opened" and no claim rests on them.
- **inferred**: derived from read facts; the inference is stated.

Legal frame, as for the Mali port (`driver/docs/nanopc-t6-mali.md`, top):
Mesa is MIT, so its register XML and compiler may be read and transcribed with
attribution. The Linux `msm` driver is GPL-2.0-only (read: `linux:drivers/gpu/
drm/msm/adreno/a6xx_catalog.c:1`, `adreno_gpu.c`/`msm_drv.c:1` the same;
`a6xx_gmu.c:1` is `GPL-2.0`), and the DRM uapi header `include/drm-uapi/
msm_drm.h` is MIT (Mesa's copy, `mesa:include/drm-uapi/msm_drm.h:1-20`). Treat
`msm` as "facts and file:line only": no line of it goes into the tree. The
device tree of the board is `BSD-3-Clause` (`linux:arch/arm64/boot/dts/qcom/
lemans-evk.dts:1`; `lemans.dtsi` at `linux@6.18:` line 1 is the same), which
may be read and transcribed with its notice. The GPU firmware files are
"Redistributable" by `linux-firmware`'s own entry (`lfw:WHENCE:7172`, "See
LICENSE.qcom and NOTICE.qcom for details", SPDX `LicenseRef-LICENSE.qcom`);
the text of `LICENSE.qcom` was **not read** (it was not in the tree paths
tried), so the terms (for example whether a binary may be embedded in an image
we ship) are not known. Nothing from Qualcomm's non-redistributable SDK or from
the vendor's firmware images goes into the tree; the firmware files are
fetched on the board's side and turned into a header at build time that is
never committed, as for `pvr_fw_blob.h` and `mali_csffw.bin`.

## 1. The hardware

| Item | Value | Mark and source |
| --- | --- | --- |
| Board | Dragonwing IQ-9075 EVK, EVK-IQ9075M-STARTER-1.2 | the request |
| SoC | QCS9075 | the request |
| GPU | Adreno 663 | the request (Qualcomm's own device description); Mesa's table has the entry, below |
| GPU architecture | a6xx family. Mesa's table gives the 663 the a6xx base plus the "gen4" property set, `CHIP.A6XX` | read, `mesa:src/freedreno/common/freedreno_devices.py:130-154` (a6xx_base), `:226-266` (a6xx_gen4), `:652-684` (the entry) |
| Chip id | `0x06060300` as the kernel reports it (major 6, minor 6, revision 3, patch 0). Mesa's table row is `0xffff06060300` named `FD663`, a row that ignores the fuse bits 47:32 | read, `freedreno_devices.py:654`; the matching rule `mesa:src/freedreno/common/freedreno_dev_info.c:24-65`; the gpu-id is made from the chip id as `major*100 + minor*10 + revision` = 663, `freedreno_dev_info.h:506-515` |
| Same row | the entry `GPUId(chip_id=0x6060201, name="FD644")`, commented "Called A662 in kgsl", shares the block with the 663 | read, `freedreno_devices.py:653` |
| Kernel names | board `qcom,lemans-evk`, then `qcom,qcs9100`, then `qcom,sa8775p`; the model string is "Qualcomm Technologies, Inc. Lemans EVK". So the QCS9075 board is "lemans" in Linux, and `sa8775p` is the SoC family name in compatibles | read, `linux:arch/arm64/boot/dts/qcom/lemans-evk.dts:17-18` |
| GPU in the kernel | `qcom,adreno-663.0`, GPU at 3d00000 (a node, status "disabled" in the SoC file and "okay" in the EVK file). Present from `v7.0`; **absent in `v6.18`**, which has only the GPU's SMMU and clock controller | read, `linux:lemans.dtsi:4287-4308`, `lemans-evk.dts:496-498`; `linux@6.18:lemans.dtsi` (no `gpu@` node; `adreno_smmu` at `:4145`, `gpucc` at `:4131`). Series: `lore:` "DRM/MSM: Support for Adreno 663 GPU", v9 of 2025-12-30, all four patches applied by the arm64 maintainer for the "arm64-for-6.20" tree (`patchew.org/linux/20251230-a663-gpu-support-v9-0-186722e25387@oss.qualcomm.com`) |
| GMEM | 1.5 MiB (`SZ_1M + SZ_512K`) | read, `linux:drivers/gpu/drm/msm/adreno/a6xx_catalog.c:1048` (the `0x06060300` entry, `:1042`) |
| CPU | eight cores, `qcom,kryo`, two clusters of four (MPIDR affinity 0x0..0x300 and 0x10000..0x10300), PSCI as the enable method, GICv3 and the Arm generic timer. The core design behind "kryo" is not in the device tree. Qualcomm: "Two Quad Core Qualcomm Kryo Gen 6 CPUs, built on Arm v8.2 Cortex technology", 512 kB L2 per core, 2 MB shared L3 per cluster, and a separate real-time subsystem with four Cortex-R52 | read, `linux:lemans.dtsi:48-240` (cores), `:562-563` (PSCI), `:6711-6712` (GICv3), `:8576-8577` (timer); `qdoc:` `docs.qualcomm.com/bundle/publicresource/80-73417-1/topics/device-description.md` "Device description" |
| Display controller | `qcom,sa8775p-dpu`, a DPU catalog file named `dpu_8_4_sa8775p.h`; Qualcomm names it "Dual Adreno DPU 1199". Interfaces: INTF_0 and INTF_4 are DisplayPort, INTF_1 and INTF_2 are DSI; 3, 6, 7 and 8 are for DP MST and "marked as INTF_NONE for now". Maximum line width 5120 | read, `linux:lemans.dtsi:5012` (mdss), `:5048-5049` (dpu), `drivers/gpu/drm/msm/disp/dpu1/catalog/dpu_8_4_sa8775p.h:8-18,319-382`, `dpu_kms.c:1516`, `msm_mdss.c:568`; `qdoc:` 80-73417-1 "Device description" |
| Display outputs on the EVK | two DisplayPort controllers (`mdss0_dp0`, `mdss0_dp1`, `qcom,sa8775p-dp`) wired to two `dp-connector` nodes labelled "EDP0" and "EDP1", and two DSI controllers. **No HDMI node**: Qualcomm's board page lists "2 x mini-DP (one with MST)" and "2 x DSI (one to a DSI flex, one to expansion)". The kernel's EVK file enables the two DP (`:613-649`); the DSI path is in the SoC file only | read, `linux:lemans.dtsi:5136,5220,5333,5429`, `lemans-evk.dts:71-90,609-649`; `qdoc:` `dragonwingdocs.qualcomm.com/Ubuntu/devices/iq9075-evk/device-overview` |
| Boot chain | Qualcomm firmware (XBL) → TEE and hypervisor → UEFI (EDK II) or U-Boot → systemd-boot → kernel; Section 7, Q7 | read, `qdoc:` `dragonwingdocs.qualcomm.com/System/Boot/boot-architecture`, `.../System/Boot/apss-cold-boot-flow` |
| Console | a Qualcomm GENI debug UART, 115200 8N1, the kernel's `stdout-path` | read, `linux:lemans-evk.dts:23,38,873-875` |
| Public documentation | see Section 7, Q10 | read |

What the 663 row says, which the other GPUs of the block share
(`freedreno_devices.py:652-684`): 3 CCUs, tile alignment 96 x 16, tile
maximum 1024 x 1024, 32 visibility-stream pipes, 32 KiB compute shared memory,
wave granularity 2, `fibers_per_sp = 128 * 4 * 16`, and no
`highest_bank_bit` (the 660 row, `:618-650`, sets 16 and `fibers_per_sp =
128 * 2 * 16`; the kernel reports the bank bit and the UBWC swizzle through
`MSM_PARAM_HIGHEST_BANK_BIT` and `MSM_PARAM_UBWC_SWIZZLE`,
`mesa:include/drm-uapi/msm_drm.h:74-95`). The GMEM size is not in Mesa's table:
Mesa reads it from the kernel (`MSM_PARAM_GMEM_SIZE`,
`msm_drm.h:75`). The kernel's value for the 663 is 1.5 MiB, the same as the
660 entry (`linux:a6xx_catalog.c:1048` and the 660 entry's `.gmem` at `:1028`),
which is the figure of Mesa's drm-shim for the a660
(`mesa:src/freedreno/drm-shim/freedreno_noop.c:291-295`). The bank bit and the
swizzle are set by the kernel for the 663 alone: `highest_bank_bit = 13` and
`ubwc_swizzle = 0x4` (`linux:a6xx_gpu.c:784-787`), where the 660 gets 16
(`:774-782`, the block of a650/a660/a690/a730/a740 entries). The GPU's top clock is
800 MHz (`linux:lemans.dtsi:4345-4350`; `qdoc:` "Adreno 663 (up to 800 MHz)",
`.../Ubuntu/devices/iq9075-evk/device-overview`).

One 663-specific fact, read from Mesa's Vulkan driver for the vendor kernel
(`tu_knl_kgsl.cc:1941-1946`): "kgsl unfortunately hardcodes some settings for
certain GPUs", for gpu_id 663 and 680 the macrotile mode is the 8-channel one,
and for the 663 alone `bank_swizzle_levels = 0x4` ("level2_swizzling_dis =
1"). The upstream kernel agrees on the swizzle (`ubwc_swizzle = 0x4`, above)
and the series' cover letter gives it as a difference from the 660: "Level2
swizzling is recommended to be disabled" (`lore:` v9 cover letter). UBWC
does not matter to a first, linear-target port (Section 3), but a display
scan-out of a UBWC buffer would (Q6).

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

**The kernel boundary and the firmware, as Mesa shows them (read).**

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

**The kernel driver and the firmware (read, Linux `v7.0`).**

- **The device table row.** The 663 is the entry with chip id `0x06060300`,
  family `ADRENO_6XX_GEN4`: SQE `a660_sqe.fw` (the 660's own file), GMU
  `a663_gmu.bin`, GMEM 1.5 MiB, quirks `HAS_CACHED_COHERENT` and
  `HAS_HW_APRIV`, hardware clock gating table `a690_hwcg`, protect table
  `a660_protect`, and three speed bins (fuse value 0, 169 and 113 map to bins
  0, 0 and 1). It has **no `zapfw` field**: unlike the 660 entry
  (`.zapfw = "a660_zap.mdt"` at the 660's row), so the zap-shader file name
  must come from the device tree (below). `linux:drivers/gpu/drm/msm/adreno/
  a6xx_catalog.c:1042-1065`. The same row is in `v6.18` (`linux@6.18:` same
  file, `:1011-1032`), with the driver then ahead of the device tree.
- **Three firmware files.** (1) SQE, the command-stream microcontroller,
  loaded by the host into a GPU-visible buffer (the 4-byte header is skipped:
  `memcpy(ptr, &fw->data[4], fw->size - 4)` in `adreno_fw_create_bo`,
  `linux:adreno_gpu.c:627-639`, called for the SQE at `a6xx_gpu.c:1126`).
  (2) GMU, a microcontroller (inferred to be Arm Cortex-M3 class from the
  register names `GMU_CM3_*`), whose image the host copies into the GMU's
  instruction and data memory blocks in `a6xx_gmu_fw_load`
  (`a6xx_gmu.c:828-`), then starts with `GMU_CM3_BOOT_CONFIG`
  (`a6xx_gmu.c:935`). The host also programs the RPMh resource controller
  (`rscc`) and the power domain controller (`gmu_pdc`) registers
  (`a6xx_gmu_rpmh_init`, `a6xx_gmu.c:590-660` at `v6.18`), which the device
  tree gives as two of the GMU node's three register ranges
  (`linux:lemans.dtsi:4356-4359`). (3) The zap shader, a signed image for the
  secure world. The driver loads it through the secure monitor:
  `zap_shader_load_mdt` reads the `zap-shader` child node, its reserved
  memory and its `firmware-name`, loads the MDT with `qcom_mdt_load` and
  authenticates it with `qcom_scm_pas_auth_and_reset(pasid)`
  (`linux:adreno_gpu.c:30-160`; the `firmware-name` branch is `:62-90`, and the
  rule "for new targets, we require the firmware-name property" is `:86-98`).
  Then the CP leaves secure mode: `CP_SET_SECURE_MODE` in `a6xx_hw_init`
  (`linux:a6xx_gpu.c:1565-1568`). If the node is absent, the driver writes
  `RBBM_SECVID_TRUST_CNTL` itself and warns "Zap shader not enabled", and "if
  you mess this up you are about to crash horribly" (`:1570-1585`).
  **Consequence for a bare-metal port:** the zap shader runs in the secure
  world, loaded by the TEE's service; non-secure code does not do it (Q5).
- **The device tree's firmware name for the board.** `qcom/sa8775p/
  a663_zap.mbn`, set in the EVK file on the `zap-shader` node
  (`linux:lemans-evk.dts:500-502`); the node's reserved memory is
  `pil_gpu_mem`, 0x2000 bytes with `no-map` (`lemans.dtsi:798-801`,
  `:4310-4312`).
- **The GPU and GMU nodes** (`linux:lemans.dtsi:4287-4392`):
  - GPU: `qcom,adreno-663.0` and `qcom,adreno`; three register ranges
    (`kgsl_3d0_reg_memory` 0x40000 bytes, `cx_mem`, `cx_dbgc`); one interrupt;
    two SMMU stream ids `<&adreno_smmu 0 0xc00>` and `<&adreno_smmu 1 0xc00>`;
    OPPs 405, 530, 676, 778 and 800 MHz with RPMh corner levels; an
    interconnect path GPU to memory; the speed-bin cell `gpu_speed_bin`
    (`lemans.dtsi:1044`, a cell of the QFPROM node at `:1038`).
  - GMU: `qcom,adreno-gmu-663.0`, `reg-names` "gmu", "rscc", "gmu_pdc"
    (as the binding requires: `linux:Documentation/devicetree/bindings/display/
    msm/gmu.yaml:163-175`); interrupts "hfi" and "gmu"; seven clocks (`gmu`,
    `cxo`, `axi`, `memnoc`, `ahb`, `hub`, `smmu_vote`); two power domains `cx`
    and `gx` of the GPU clock controller; one SMMU stream; one 500 MHz OPP.
  - The GPU clock controller is `qcom,sa8775p-gpucc` (`lemans.dtsi:4394`), the
    GPU's SMMU `adreno_smmu` (`:4408`).
- **The SMMU and who owns the page tables.** The GPU's SMMU is an Arm MMU-500
  (`"qcom,sa8775p-smmu-500", "qcom,adreno-smmu", "qcom,smmu-500",
  "arm,mmu-500"`, `lemans.dtsi:4408-4411`), a separate instance from the
  application SMMU (`apps_smmu`, `:6497`) that the display and other masters
  use. The kernel's Adreno hook (`linux:drivers/iommu/arm/arm-smmu/
  arm-smmu-qcom.c`) treats stream id 0 as the GPU (`:219`) and hands the
  GPU driver `set_ttbr0_cfg` and `get_ttbr1_cfg` (`:378-379`). The GPU driver
  uses `set_ttbr0_cfg` to give the GPU per-process page tables
  (`linux:drivers/gpu/drm/msm/msm_iommu.c:582`) and the SQE switches them
  with the packet `CP_SMMU_TABLE_UPDATE` (TTBR0, ASID, context bank 0:
  `linux:a6xx_gpu.c:266-273`). Before that, the GPU driver asks the secure monitor to set the
  SMMU aperture, "always use context bank 0, so hard code this"
  (`linux:adreno_gpu.c:651-657`, `qcom_scm_set_gpu_smmu_aperture(0)`). So
  **the page-table base registers of the GPU's context bank are written by the
  non-secure kernel, but the aperture is a secure-monitor call.**
- **Bandwidth table.** The GMU is given a per-GPU bandwidth table, with a
  function of its own for the 663 (`a663_build_bw_table`,
  `linux:a6xx_hfi.c:610`, chosen at `:819-820`): another thing the host must
  send over the HFI queue before the GPU runs.

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
| Memory management | kernel creates GPU address space and BOs (`GEM_NEW`, `VM_BIND`, `MSM_PARAM_VA_START`: `msm_drm.h:87`); in a Zephyr image, we do it ourselves: an Arm MMU-500 for the GPU, per-process TTBR0 tables switched by the SQE (`CP_SMMU_TABLE_UPDATE`), one secure-monitor call for the aperture (Section 2, "The SMMU"; Q5) | PVR (MMU via firmware), V3D (MMU), Vivante (MMU), Mali (AS0..7) | A new MMU layer or the Linux uapi, see Section 4 |
| Firmware | SQE (`a660_sqe.fw`) and GMU (`a663_gmu.bin`) loaded by the host, and a signed zap shader (`qcom/sa8775p/a663_zap.mbn`) loaded through the secure monitor (Section 2) | PVR firmware, Mali CSF firmware | See Q1, Q5 |

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
Armv9-A bases (`dts/arm64/armv8-a.dtsi`, `armv9-a.dtsi`) exist.

**What the Linux device tree says about the SoC (read).** The cores are eight
`qcom,kryo` on Arm v8.2 (so the Armv8-A base fits), started by PSCI; the
interrupt controller is a GICv3 with one redistributor region for eight cores;
the timer is the Arm generic timer (`linux:lemans.dtsi:48-240,562,6711-6720,
8576`); the console is a Qualcomm GENI serial engine (`uart10`,
`lemans.dtsi:2347`, a driver Zephyr does not have). The GPU's clocks come from
`gcc` and `gpucc`, its power from the `GPU_CC_CX_GDSC` and `GPU_CC_GX_GDSC`
domains, the voltage levels from RPMh (`rpmhcc`, the `apps_rsc` block at
`lemans.dtsi:6788`; the GPU OPPs are RPMh corner levels, `:4317-4350`). Seen
from the Linux driver, a bare-metal GPU driver would have to: program the
clock controllers and the power domains; send RPMh requests for the rails;
copy and start the GMU (Section 2) with its `rscc` and `gmu_pdc` set-up; load
the SQE; build per-process page tables; and get the zap shader loaded and
authenticated by the secure monitor. Each of these except the last is an
ordinary non-secure register sequence that the kernel shows. The last is a
call into the TEE (`qcom_scm_pas_auth_and_reset`, `linux:adreno_gpu.c:149`)
and the interface of that call on a board without Qualcomm's Linux is **not
read** (Q5).

**EL, and the Qualcomm hypervisor.** The EVK's firmware runs a Qualcomm
hypervisor between the TEE and UEFI; by default "the Linux kernel runs in the
nonsecure EL1 state", and in "KVM mode" UEFI shuts the hypervisor down and
hands off in EL2 (`qdoc:` `dragonwingdocs.qualcomm.com/System/Boot/
apss-cold-boot-flow`). The kernel tree has an overlay `lemans-el2.dtso` for the
second case (name only, not read). Whether a stage-2 translation by the
hypervisor limits the GPU's or the SMMU's registers in the default mode is
**not read** (Q5).

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
- **Sign-off route (the loading half is answered, the GPU half is not):**
  - *Loading.* The chain is XBL, then the TEE and the hypervisor, then UEFI
    (EDK II) or U-Boot, then systemd-boot, then the kernel as an EFI image
    (`qdoc:` `.../System/Boot/boot-architecture`). Qualcomm documents U-Boot as
    a replacement for EDK II ("the other stages stay the same"), and
    TF-A's board page shows a second replacement: a self-built `fip.elf` in
    place of `uefi.elf` in the image, signed with a test key that "lets it run
    on devices in non-secure mode" (`tfa:` `tf-a.docs.trustedfirmware.org/en/
    latest/plat/qti/lemans_evk.html`). systemd-boot starts any EFI image on
    the boot partition; a UKI "can execute ... directly without any supporting
    .conf file" (`qdoc:` `.../System/Kernel/uefi-and-systemd-boot`). So a bare
    arm64 image can be started as an EFI application or a U-Boot payload, if
    UEFI secure boot is not enforced; with it enforced, the image has to be
    signed with the board's key or its hash enrolled
    (`developer.ridgerun.com`, third-party, not Qualcomm). Whether a dev
    kit ships with secure boot off is **not stated** by any page read (Q7).
  - *GPU.* Whether the TEE's zap-shader service works for our caller, and
    what the hypervisor allows, is **not read** (Q5). If the GPU cannot be
    started from non-secure code without the vendor's Linux, the sign-off
    image cannot be a Zephyr image with its own GPU driver, and the item list
    has to stop at the development route plus a decision from the
    maintainers. This is still the largest risk of the port, but it is now
    narrowed to one call (the zap shader) and one policy (the hypervisor).

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
   entry `{ .gpu_id = 663, .chip_id = CHIPID(6, 6, 3, 0), .gmem_size = 1536 * 1024 }`
   (a patch file with its own licence, like `tools/mali/mesa-glibc-once_
   flag.patch`). `fd_dev_info_raw` then finds the 663 row by the wildcard
   rule (`freedreno_dev_info.c:49-62`). The GMEM size is the kernel's own
   (1.5 MiB, `linux:a6xx_catalog.c:1048`), the same as the shim's 660
   entry (`freedreno_noop.c:293`); the record says it comes from the
   kernel's table, not from a measurement on the board. The GMEM size
   changes the tile layout of the command stream but not the shader bytes. **Check:** the build prints a renderer string `FD663`
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
   the device tree from the memory map and interrupt numbers of the Linux
   `lemans.dtsi` (read, Section 1 and 2; its CPUs, GICv3, timer and `uart10`
   are the first nodes), and a `hello` image that only builds. The GENI
   serial engine needs a Zephyr driver we do not have. The gate has no board
   row for it until a board runs one. **Check:** the Zephyr build of the image
   (the `tools/check.sh` image rows are the pattern). The addresses are
   known; whether the image can be started is Q7.
8. **Done in this note's second pass (item A0):** the sources of Section 7
   were read; what remains is listed there.

Not possible before the board: anything that runs the GPU; a measured GMEM
size (the kernel's table says 1.5 MiB), the firmware the board really loads,
and the secure-boot state of the board.

## 6. What needs the board

**First three things on day one**, in order. None of them is Zephyr.

1. Boot the board's own Linux and read the facts that only the board gives
   (the kernel version it runs, since the GPU node needs `v7.0`; the firmware
   it loads; the secure-boot state; the display's start-up state):
   `/proc/iomem` and `/proc/interrupts` for the GPU and display blocks, the
   running `msm` driver's boot messages (`dmesg`: the chip id, the firmware
   it loaded, the GMEM size) and `/lib/firmware`. This confirms Q1 to Q4 and
   answers Q6, and fills the `GPU, SoC, board` table the Mali note has
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

Each is a question, what the sources read in this pass say (**answered**,
**partly**, or **open**), and what still answers it. An answer that only the
board can give stays a question.

| Q | Question | Status and what was read | What still answers it |
| --- | --- | --- | --- |
| Q1 | Which firmware files does the 663 load, under which names; upstream or only in the vendor image; redistributable? | **Answered, except the licence text.** SQE `qcom/a660_sqe.fw`, GMU `qcom/a663_gmu.bin` (`linux:a6xx_catalog.c:1045-1046`), zap shader `qcom/sa8775p/a663_zap.mbn` (`linux:lemans-evk.dts:500-502`). **All three exist in `linux-firmware`**: `qcom/a660_sqe.fw` (43,292 bytes, "Version: v1.12"), `qcom/a663_gmu.bin` (55,892 bytes, "Version: v3.05.01"), `qcom/sa8775p/a663_zap.mbn` (1,054,680 bytes, "Version: v0.09") (`lfw:WHENCE:7070-7079`, `lfw:qcom/`, main at `d0039fd797d7`). The Adreno section ends "Licence: Redistributable. See LICENSE.qcom and NOTICE.qcom for details" (`lfw:WHENCE:7172`). Nothing is only in Qualcomm's images for the EVK's GPU by these sources. The zap image is signed for the SoC family, and the kernel says it may be "signed with a (potentially) device specific key" (`linux:adreno_gpu.c:86-98`), so a board may need its own file | the text of `LICENSE.qcom` (**not read**); the board's `/lib/firmware` and `dmesg` |
| Q2 | Does upstream `msm` support this GPU; from which Linux version; which device-tree node? | **Answered.** The driver table has the 663 in `v6.18` (`linux@6.18:a6xx_catalog.c:1011-1032`), the binding has `qcom,adreno-gmu-663.0` (`linux:Documentation/devicetree/bindings/display/msm/gmu.yaml:163`), but the device tree gets the GPU and GMU nodes only in **`v7.0`** (`linux:lemans.dtsi:4287-4392`, enabled for the EVK at `lemans-evk.dts:496-502`); `v6.18` has no `gpu@` node. Compatibles, registers, interrupts, clocks, power domains and the SMMU streams are in Section 2. Hardware revision in the kernel is `0x06060300`, GMEM 1.5 MiB. | none for the facts; the board's kernel version decides which one it runs |
| Q3 | GMEM size, UBWC settings, speed bins? | **Answered from the kernel's table.** GMEM 1.5 MiB (`linux:a6xx_catalog.c:1048`); `highest_bank_bit = 13`, `ubwc_swizzle = 0x4` (`linux:a6xx_gpu.c:784-787`); three speed bins from the fuse `gpu_speed_bin` (`a6xx_catalog.c:1060-1064`, `lemans.dtsi:1044`), OPPs masked by bin (`:4317-4350`). `MSM_PARAM_GMEM_SIZE` on the board would confirm. | the board (`dmesg`, `MSM_PARAM_GMEM_SIZE`) |
| Q4 | Display path: controller, output, firmware, linear scan-out, register blocks? | **Partly.** Controller: Adreno DPU, catalog `dpu_8_4_sa8775p.h`, two display controllers (Qualcomm: "Dual Adreno DPU 1199"). Outputs on the EVK: two mini-DisplayPort (one MST) and two DSI, no HDMI (Section 1). Register blocks: the `mdss0` node at `lemans.dtsi:5012` and its children (`:5048,5136,5220,5333,5429`); the DPU's own blocks are in `dpu_8_4_sa8775p.h` (sspp, lm, ctl, intf). Whose firmware initialises the display at boot, and whether it leaves a frame buffer, is not stated by any page read. Linear scan-out: the DPU catalog lists DMA and VIG pipes; the formats table was not read | the board; `dpu_formats.c` and the plane code (not read) |
| Q5 | Can the GPU be brought up from non-secure bare-metal code? Rails, clocks, GMU, zap shader, page tables? | **Partly, the unknown is narrowed.** The kernel shows the non-secure sequence: clock and power-domain set-up (`gcc`, `gpucc`, `GPU_CC_CX_GDSC`, `GPU_CC_GX_GDSC`), RPMh set-up from the host (`a6xx_gmu_rpmh_init`, `a6xx_gmu.c:590-660` at `v6.18`), host copy of the GMU image and `GMU_CM3_BOOT_CONFIG` (`a6xx_gmu.c:828,935`), SQE in a buffer (`adreno_gpu.c:627-639`), per-process tables switched by `CP_SMMU_TABLE_UPDATE` (`a6xx_gpu.c:266-273`). Two steps call the secure monitor: the **zap shader** (`qcom_scm_pas_auth_and_reset`, `adreno_gpu.c:149`) and the **SMMU aperture** (`qcom_scm_set_gpu_smmu_aperture(0)`, `adreno_gpu.c:656`). Without the zap shader the driver writes `RBBM_SECVID_TRUST_CNTL` itself and warns it may "crash horribly" (`a6xx_gpu.c:1570-1585`); whether that write is allowed on this board is not stated. The 663's GMU needs the bandwidth table over HFI (`a6xx_hfi.c:610`) | the board; Qualcomm's secure-monitor interface for these two calls (not public in the pages read) |
| Q6 | Does the display accept our buffer layout (linear; UBWC quirk if compressed)? | **Open.** Quirk is `ubwc_swizzle = 0x4` on the GPU side (Q3). The DPU side formats were not read | Q4's sources; the board |
| Q7 | Boot chain: stages before ours, what loads a kernel, bare-metal image, U-Boot, what is locked? | **Answered, except the board's lock state.** PBL → XBL (EL3, secure) → Qualcomm TEE → Qualcomm hypervisor → EDK II **or U-Boot** (nonsecure EL1) → systemd-boot → kernel (EFI stub, UKI in `EFI/Linux/`); UEFI is in the `abl_a`/`abl_b` partitions (`qdoc:` `.../System/Boot/boot-architecture`, `.../System/Boot/apss-cold-boot-flow`, `.../System/Kernel/uefi-and-systemd-boot`). U-Boot replaces EDK II with the other stages unchanged. TF-A has a `lemans_evk` platform: a self-built `fip.elf` replaces `uefi.elf`, `bl2.mbn` replaces `tz.mbn`, signed with a test key that "lets it run on devices in non-secure mode"; its port "only allows booting OP-TEE OS" and QTISECLIB support is "still work under progress" (`tfa:`). Flashing is over USB in emergency download mode, DIP switch 3 up (`qdoc:` `docs.qualcomm.com/bundle/publicresource/80-90441-253/topics/force-edl-mode.md`; the page's figure caption calls it "S2-8"). Secure boot is enabled "only after blowing a QFPROM" fuse and UEFI keys can be enrolled only once (`developer.ridgerun.com`, a third-party page). **No page read says a dev kit ships with secure boot on or off.** A bare image can be started as an EFI application or U-Boot payload if the board does not enforce secure boot | the board: whether it enforces UEFI secure boot |
| Q8 | Sign-off route: U-Boot or the vendor firmware loading our image; does it depend on Q5? | **Narrowed.** Loading is possible by the two routes in Q7; the choice between them is a decision of the maintainers. It still depends on Q5: the zap shader and the aperture call | Q5; a decision of the maintainers |
| Q9 | CPU cores; does Zephyr run on them; interrupt controller, timer, UART, memory map? | **Answered for the hardware, open for Zephyr.** Eight Kryo Gen 6 cores on Arm v8.2 (two clusters of four), PSCI, GICv3, Arm generic timer, a GENI debug UART (Section 1, 4). Zephyr's Armv8-A base fits; the SoC support and the GENI UART driver would be ours. The memory map is the `lemans.dtsi` (`reg` of each node) | Zephyr bring-up on the board |
| Q10 | Which documents are public? | **Partly.** Public and read: the device description (`qdoc:` 80-73417-1), the EVK hardware overview page (80-75289-1; it gave no table, the board's features came from the Ubuntu device overview page), the boot architecture, APSS cold boot, UEFI and systemd-boot pages, the EDL page, the flashing pages (`dragonwingdocs.qualcomm.com/Linux/devices/iq9075-evk/update-software/overview.md`). **Listed, not opened** (`qdoc:` 80-70029-261 "References"): "QCS9075M Data Sheet" 80-73418-1, "QCS9075M Technical Reference Manual" 80-73418-5, "QCS9075M Pin Assignment and GPIO Configuration Spreadsheet" 80-73418-1A, "Design Package, IQ-9075 EVK Design Files" DP25-73418-42, the "Qualcomm Linux Build Guide" 80-70029-254 and "Qualcomm Linux Yocto Guide" 80-70029-27; the references page does not say which need a login. No public GPU register document was found; the register knowledge stays Mesa's XML | the TRM and the EVK design package, if the maintainers have access; the board |
| Q11 | Does ES 2.0 completeness need the binning pass (GMEM rendering), or does sysmem rendering pass dEQP-GLES2? Mesa has both (`freedreno_gmem.c:644`, `:721-723` choose between them) and a driver option; the sysmem path may be slower and may need the CCU cache settings | open | Mesa `freedreno_gmem.c:701-725`; the board |
| Q12 | The GPU's coherence, cache and write-combining rules for memory shared with the CPU in a bare-metal image | open | Linux `msm_gem.c` and the device tree; the board |
| Q13 | Is the `FD_RD_DUMP` path really independent of the kernel when run against the shim? The code reading says yes (`msm_priv.h:119-165`); not run | open | Section 5, item 2 |
| Q14 | Is the issue list read? | **Answered.** Issue #171 is the task issue of the first pass (state closed, pull request #176, `gh issue view 171`); it holds the prompt and the eight questions this note answers, and no other open item for the port. The second pass is recorded in its pull request | none |

Nothing here needs a board address, a credential or an IP; those stay in the
environment (`CLAUDE.md`).

## 8. Proposed item list

Sizes are lines of Rust without tests, from `wc -l` of this repository at main
`48f0de66` (back end `src/<gpu>/*.rs`, shader compiler
`src/shaderc/online/<gpu>/*.rs`, not recursive; the test files named
`*tests*.rs` and the shader `tests.rs` are left out), rounded. The counts of
the first pass reproduce at this commit; `genxml/` and `isa/` directories are not counted, so the real
sizes are larger. They are a guide, not a
measurement of the Adreno work.

| Port | GPU back end | Shader back end | State (`tools/cts/STATUS.md`, `CHANGELOG.md`) |
| --- | --- | --- | --- |
| V3D (Pi 4) | 9.2k | 20.6k (with a QPU simulator, 2.6k) | ES 2.0 done, 0 failures |
| PowerVR (AM62P) | 14.5k | 24.6k | ES 2.0 done |
| Vivante (i.MX8MP) | 9.0k | 9.9k | ES 2.0 done |
| Mali (RK3588, Linux only) | 7.9k | 18.5k | 123 of 256 `gl_test` cases PASS, 3 FAIL, 36 REFUSED (two identical runs, recorded at main `788d5ae7`, `tools/cts/STATUS.md:15`) |

Estimate for Adreno (inferred): the GPU back end between the Mali's and the
PowerVR's, 8k to 14k, and the shader back end between the Vivante's and the
PowerVR's, 12k to 25k. Reason: the ISA is richer than Vivante's (eight
categories, half registers, synchronization flags, a scalar ALU) and the GL
front end is shared; the register allocator and scheduler are the large part.
The PowerVR port also carried a firmware bring-up (`driver/src/pvr_fw_boot.c`)
that the Linux development route avoids. The bare-metal part has no
estimate, because Q5 and the secure-boot half of Q7 are still open; it could be as small as the PowerVR's
`driver/` or impossible. The A0 reading removed the other unknowns: the SoC's
device tree gives the memory map and the CPU, interrupt, timer and UART
blocks (Section 4), and the GPU's start-up sequence is in the kernel (Section 2).

**Items, in order.** Each item is closed by a result and a comment on its
GitHub issue; an item the board is needed for says so.

| # | Item | Needs | Check at the end |
| --- | --- | --- | --- |
| A0 | Read the unread sources (Q1 to Q10, Q14), file the issue, update this note | network, Linux tree | **done by the second pass**: Sections 1 and 7 have no "not read" left except the five sources listed under "not read" in the read-marks section at the top |
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

A0 is read. Two unknowns (Q5 and the secure-boot half of Q7) decide
whether A10 exists at all, and only the board answers them; Q8 follows from them, and a port that stops at A9 (Linux-hosted, like the
Mali's present state) is a smaller, still useful result.
