<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: NVIDIA Jetson Orin Nano (Tegra234, GA10B)

Date 2026-10-28, main `788d5ae7`. Host research only: no file under `src/`
or `tools/` changed, no board run, no board yet.

**Verdict: no open route exists today, and the port is not scheduled.**
Mesa's NVK and NAK know the GA10B chip by name, but Mesa marks Tegra "not
currently supported", and the Linux `nouveau` driver has no GA10B at all: its
chipset table has no `0x17b`, its Tegra platform match lists only
`nvidia,gk20a`, `nvidia,gm20b` and `nvidia,gp10b`, and the in-tree Tegra234
device tree has no GPU node and no display node (section 2). What would have
to appear upstream first is in section 7, question 3. Section 8 is the list to
use if that happens.

## How this note was made

Four kinds of source:

- **Mesa 26.2.1** (the release tree, `VERSION` 26.2.1, in this project's
  dependency cache `mesa-src/mesa-26.2.1`). Read on disk; line numbers below
  are exact. Paths are relative to that tree.
- **Linux 7.3.0 (`Makefile`: `VERSION = 7`, `PATCHLEVEL = 3`, `SUBLEVEL = 0`,
  no extra version), commit `65ba8f34ec886ec72b2c07ddc8cf3d815c32869d`**,
  tip of `master` on 2026-10-28. A shallow clone (`git clone --depth 1`) was
  searched with `grep` and the matching lines read. Line numbers below are
  from that clone. Paths are relative to its root.
- **NVIDIA's Jetson Linux Developer Guide, release 36.4.3**
  (`https://docs.nvidia.com/jetson/archives/r36.4.3/DeveloperGuide/`), read
  through a page fetcher that answers a question about a page, not word for
  word. Where a quotation is given it is the fetcher's. Where the page did
  not say a thing, the note says "not said".
- **Zephyr** `VERSION` 4.4.99 (the project's workspace), searched for Tegra
  support in the earlier pass of this note; the review of that pass accepted
  it. Not re-run here (the workspace is outside this session's reach).

**Not read, and why:** the Orin Nano data sheet and the Tegra234 technical
reference manual (not fetched; the TRM is not known to be public, see
section 1), the nouveau mailing list archive (`lore.kernel.org` answered
HTTP 403 to the fetcher) and the Mesa issue tracker (`gitlab.freedesktop.org`
answered with an Anubis access-denied page). In their place, two web searches
on 2026-10-28 for a GA10B or Tegra234 patch in nouveau and for a GA10B issue
in NVK found none (section 7, question 3); a search engine is weaker evidence
than the list itself. Items that need what was not read are marked
**UNREAD**: an UNREAD item is a question, not a finding, and no verdict
depends on one.

## 1. The hardware

| Fact | Status | Source |
|---|---|---|
| The GPU is GA10B, chipset id `0x17b`, shader model 8.7 (SM87) | read | Mesa `src/nouveau/winsys/nouveau_device.c:65-66` (`sm_for_chipset`: `chipset == 0x17b` returns 87; `>= 0x172` returns 86) |
| GA10B has the A100 set of shared-memory carve-outs | read | `nouveau_device.c` ("Same for GA10B" comment) |
| The kernel reports Tegra GPUs as platform `SOC`; Mesa then has no VRAM | read | `nouveau_device.c:393-394`, `:589-590`; `src/nouveau/vulkan/nvk_physical_device.c:1637-1638` ("On Tegra, we only have sysmem") |
| The Orin Nano Developer Kit is Tegra234 | read | Linux `arch/arm64/boot/dts/nvidia/tegra234-p3768-0000+p3767-0005.dts:7-8`: `compatible = "nvidia,p3768-0000+p3767-0005", "nvidia,p3767-0005", "nvidia,tegra234"`, `model = "NVIDIA Jetson Orin Nano Developer Kit"`; it includes `tegra234-p3768-0000+p3767.dtsi` (carrier p3768, module p3767). The `-0000` variant is the "Orin NX Engineering Reference Developer Kit" (`...p3767-0000.dts:8`) |
| Interrupt controller | read | `tegra234.dtsi:4081`: `gic: interrupt-controller@f400000`, `arm,gic-v3`, GICD at `0x0f400000` size `0x10000`, GICR at `0x0f440000` size `0x200000` |
| Console | read | `tegra234-p3768-0000+p3767.dtsi:11-17`: `serial0 = &tcu` and `stdout-path = "serial0:115200n8"`; `serial1 = &uarta`, `serial2 = &uarte`. `tcu` (`tegra234.dtsi:5813`, `nvidia,tegra234-tcu`) is not a UART block: it uses HSP shared mailboxes (`hsp_top0`, `hsp_aon`) to the always-on processor. The real UARTs are `uarta: serial@3100000` (`:2819`) and `uarte: serial@3140000` (`:2830`), both `nvidia,tegra234-uart`, and `uarti: serial@31d0000` (`:2936`, `arm,sbsa-uart`). Which of them is on the carrier's debug header: UNREAD |
| Memory controller | read | `tegra234.dtsi:2756` `mc: memory-controller@2c00000` (`nvidia,tegra234-mc`), `:2805` `emc: external-memory-controller@2c60000` (`nvidia,tegra234-emc`). RAM size and base: UNREAD (the boot firmware, MB1, sets up SDRAM and the carveouts: Developer Guide, "Jetson Orin Series Boot Flow") |
| GPU clocks, reset, power domain | read | `include/dt-bindings/clock/tegra234-clock.h`: `TEGRA234_CLK_GPC0CLK` 41 (`:100`), `GPU_PWR` 42 (`:102`), `GPC1CLK` 236 (`:449`), `GPUSYS` 304 (`:560`). `include/dt-bindings/reset/tegra234-reset.h:31`: `TEGRA234_RESET_GPU` 19. `include/dt-bindings/power/tegra234-powergate.h:36`: `TEGRA234_POWER_DOMAIN_GPU` 35 |
| GPU device-tree node | read: **absent** | `grep` of `tegra234.dtsi` for `gpu@`, `ga10b` finds nothing; the only GPU names are the thermal zone `TEGRA234_BPMP_THERMAL_ZONE_GPU` (`:5842`). Linux has the clock, reset and power-domain IDs of the GPU but no node that uses them |
| GPU register base | partly | Not in the mainline tree. NVIDIA's `nvgpu` logs name the device `17000000.ga10b` (NVIDIA forum thread "Gpu report: nvgpu: 17000000.ga10b nvgpu_gr_intr_handle_sm_exception:390", `https://forums.developer.nvidia.com/t/gpu-report-nvgpu-17000000-ga10b-nvgpu-gr-intr-handle-sm-exception-390-then-reboot/275418`), which suggests a base of `0x17000000`; that is a hint from a log line, not a document. Size and interrupts: UNREAD (NVIDIA's Jetson kernel tree or the TRM) |
| Display path | partly read | The display engine is "nvdisplay", outside host1x: the comment in Linux `drivers/gpu/drm/tegra/drm.c` says so, and its host1x client table (`:1399-1400`) lists only `nvidia,tegra234-vic` and `nvidia,tegra234-nvdec` for Tegra234. `tegra_dc_of_match` in `dc.c` stops at `nvidia,tegra194-dc` (no `tegra234` anywhere in `dc.c`). `tegra234.dtsi` has no display node. Clocks exist: `TEGRA234_CLK_DISP` 85, `NVDISPLAY_P0` 86, `NVDISPLAY_P1` 87 (`tegra234-clock.h`), reset `TEGRA234_RESET_NVDISPLAY` 16 (`tegra234-reset.h:28`), power domain `TEGRA234_POWER_DOMAIN_DISP` 3. Developer Guide, "Display Configuration and Bring-up", Orin page: the SOM provides DP or HDMI ("The Jetson Orin SOM provides DP or HDMI functionality"), chosen by a "Display Control Block" (DCB) blob in the kernel display dtsi plus the carrier's pinmux; UEFI enables the display at boot and hands it to the kernel ("Seamless display is supported from bootloader to kernel handover"); the NVIDIA kernel display driver normally controls it. Which output the Orin Nano Developer Kit uses: UNREAD (the page names the AGX Orin DCB only) |
| Boot chain before our image | read | Developer Guide, "Jetson Orin Series Boot Flow" (`AR/BootArchitecture/JetsonOrinSeriesBootFlow.html`): BootROM (starts "as BPMP leaves the reset state", loads BR-BCT, PSCBL1, MB1, MB1-BCT); PSCROM on the platform security controller (holds the NVIDIA and OEM keys); MB1 on the BPMP (pinmux, GPIO, security, SDRAM init, loads CCPLEX firmware, makes memory carveouts, loads MB2); MB2 Applet on the BPMP R5; MB2 on the CCPLEX (handles flashing); UEFI, which "replaces CBoot ... as the CPUBL for Jetson Linux devices". The page does not say what hands the kernel over, nor how a non-Linux image is loaded |
| Which of this is public documentation | read, partly | Public: the Jetson Linux Developer Guide, the Linux device tree and binding headers above, and the BPMP message ABI (`include/soc/tegra/bpmp-abi.h`, with `MRQ_PING` 0, `MRQ_RESET` 20, `MRQ_CLK` 22, `MRQ_PG` 66). The Tegra234 TRM and the GA10B registers: not found public in this pass, UNREAD, not assumed public |

## 2. The open-source stack today

**Mesa 26.2.1.**

- NVK (Vulkan) supports "Kepler ... up to and including Ada, as well as
  consumer Blackwell" and is conformant "for all officially supported GPUs"
  (`docs/drivers/nvk.rst`, "Hardware support", line 16). It needs a kernel
  with the VM_BIND uAPI: the device open fails without it
  (`src/nouveau/vulkan/nvkmd/nouveau/nvkmd_nouveau_pdev.c:75-78`, comment
  "NVK Requires a Linux kernel version 6.6 or later").
- Tegra is carved out of conformance: `nvk_is_conformant` returns false for
  anything that is not a discrete card, comment "Tegra is not currently
  supported" (`nvk_physical_device.c:86-91`). `NVK_I_WANT_A_BROKEN_VULKAN_DRIVER`
  enables "GPUs for which hardware support is poorly tested or completely
  broken" (`docs/drivers/nvk.rst:92-96`); the doc does not say it covers
  Tegra, and what it does on GA10B was not run.
- Tegra code in NVK is thin and for old chips: the sysmem-only heap
  (`nvk_physical_device.c:1637`), ASTC for SoC parts from Kepler on (`:107-108`),
  coherent-map handling (`nvkmd_nouveau_mem.c`), and a note in `nil/modifiers.rs`
  that the Tegra block layout is not yet separate.
- The Gallium `nvc0` driver sets `tegra_sector_layout` only for the older
  Tegra chips; the default branch comment is "Xavier's GPU and everything
  else" with it off (`src/gallium/drivers/nouveau/nouveau_screen.c:381-385`).
- The old GL driver is out for this generation: "Starting with Mesa 25.1, all
  Turing ... and later GPUs will get NVK+Zink as their OpenGL implementation"
  (`docs/drivers/nvk.rst`, line 24 region). For GA10B the only Mesa candidate
  is NVK, with Zink for GL. How complete ES 2.0 is through NVK+Zink on Tegra
  was not measured: unknown.
- Mesa asks the kernel for the chipset, platform and classes; it does not
  hard-code the GA10B 3D class. Whether it is `AMPERE_A` (`0xC697`) or
  `AMPERE_B` (`0xC797`): UNREAD (`src/nouveau/headers/nvidia/classes/clc797.h`
  has the latter's methods).

**Linux 7.3.0 at `65ba8f34`** (the clone; line numbers are from it).

- `drivers/gpu/drm/nouveau/nouveau_platform.c:80,84,88`: the only
  `.compatible` strings are `nvidia,gk20a`, `nvidia,gm20b`, `nvidia,gp10b`.
- `drivers/gpu/drm/nouveau/nvkm/engine/device/tegra.c`: `grep -i` for
  gk20a, gm20b, gp10b, tegra234, ga10b and gsp finds nothing: it is not a
  chipset table.
- `drivers/gpu/drm/nouveau/nvkm/engine/device/base.c`: the chipset is
  `(boot0 & 0x1ff00000) >> 20` (`:3214`). The Ampere cases are `0x170`
  (`:3243` card type GA100, `:3345` `nv170_chipset`), `0x172` to `0x174`,
  `0x176` and `0x177` (`:3346-3350`). `grep -i` for `0x17b`, `GA10B` and
  `tegra` in the file finds nothing: there is no case for chipset `0x17b`.
- `nvkm/subdev/gsp/ga102.c`: the GSP table has two entries, firmware version
  `570.144` and `535.113.01` (`:180-181`); booter images are declared for
  ga102, ga103, ga104, ga106 and ga107 only (`:193-203`); the signature is
  read from the ELF section `.fwsignature_ga10x` (`:157`). `grep -ri` for
  `ga10b`, `tegra234` over all of `drivers/gpu/drm/nouveau` finds only that
  section name. So the Ampere path in `nouveau` is the GSP path of the PCIe
  parts, with NVIDIA firmware loaded by file name, and no GA10B image is
  named. Where the images are distributed (the `linux-firmware` tree) and
  their licence: UNREAD (the raw `WHENCE` fetch answered 404).
- `arch/arm64/boot/dts/nvidia/tegra234.dtsi`: no GPU node, no display node.
  It has `host1x@13e00000` (`:4381`, `nvidia,tegra234-host1x`), the BPMP node
  `bpmp: bpmp` (`:5321`, `nvidia,tegra234-bpmp`, doorbell through `hsp_top0`
  `:3378`, shared memory `cpu_bpmp_tx` `sram@70000` `:5308` and `cpu_bpmp_rx`
  `sram@71000` `:5314` inside `sram@40000000` `:5298`), the AON HSP
  `hsp_aon: hsp@c150000` (`:3902`).
- `drivers/gpu/drm/tegra/drm.c:1255-1257`: "on Tegra234 ... the display
  hardware is no longer part of the host1x complex, so this driver will not
  expose any modesetting features"; `:1399-1400` list `nvidia,tegra234-vic`
  and `nvidia,tegra234-nvdec` only. `dc.c:3167` is the newest display
  controller compatible, `nvidia,tegra194-dc`.
- `drivers/firmware/tegra/` has `bpmp.c`, `bpmp-tegra186.c` (the
  HSP-doorbell transport) and `ivc.c`; `include/soc/tegra/bpmp-abi.h` is the
  message ABI (section 4).
- U-Boot (`arch/arm/mach-tegra/Kconfig`, `master`, 2026-10-28, page fetcher,
  top of the file only): the SoC choice lists Tegra20 to Tegra186 families;
  no Tegra234. Re-check with a clone before relying on it.
- NVIDIA's own open kernel modules README (`NVIDIA/open-gpu-kernel-modules`,
  `main`, page fetcher) does not mention Tegra, Jetson, Orin or GA10B; it
  says the modules "must be used with GSP firmware and user-space NVIDIA GPU
  driver components" from a matching release, for "any Turing or later GPU"
  in its PCI ID table.
- Patches: two web searches on 2026-10-28 (a nouveau GA10B/Tegra234 patch
  series; an NVK GA10B issue) found none. They did find that the Orin GPU
  runs the out-of-tree NVIDIA `nvgpu` driver (a Talos Linux packaging pull
  request, `https://github.com/siderolabs/pkgs/pull/1518`, "nvidia-tegra-nvgpu
  package for Jetson Orin (GA10B)"). A search is weaker than the list archive
  (HTTP 403 to the fetcher); treat "none found" as "none found by search".

**Reading of the evidence.** `nouveau` cannot bind to GA10B in this tree:
no chipset case, no Tegra234 compatible, no GPU node to bind. Its Ampere
path needs the GSP, and no GA10B GSP image is named anywhere read. Whether
NVIDIA ships one is UNREAD. The Jetson stack runs `nvgpu` and NVIDIA's
proprietary user space; their sources were not read.

**ISA and command-stream knowledge we could rely on.** All in Mesa 26.2.1
(MIT-licensed, read, not copied):

- NAK, the Rust shader compiler: `src/nouveau/compiler/nak/`. The encoder for
  Volta and later is `sm70_encode.rs`; the Ampere/Ada latency tables are
  `sm80_instr_latencies.rs`; `sm70.rs:31` and `:172` branch on `self.sm < 80`
  and `self.sm >= 80`. A search of `nak/lib.rs` and `sm70.rs` finds no `87`
  case, so SM87 takes the `sm >= 80` path. That is an inference from a
  search, not a test.
- Shader program header: `compiler/nak/sph.rs`,
  `headers/nvidia/classes/clc797sph.h`.
- Class headers (methods, texture header, constant buffer, QMD): the files
  `src/nouveau/headers/nvidia/classes/cl*.h`, generated from NVIDIA's
  open-gpu-doc by `src/nouveau/headers/update-from-open-gpu-doc.py`.
- Push-buffer encoding and dump: `headers/nv_push.h`, `nv_push_dump.c`.

## 3. What yakogl already has that applies

No existing back end extends to this GPU. A new back end is needed, in two
parts (a frame side and a compiler side), as for Mali (`src/mali`,
`src/shaderc/online/mali`).

| Existing | What it drives | Why it does not extend |
|---|---|---|
| `src/v3d`, `shaderc/online/v3d` | Broadcom V3D, QPU ISA, control lists | other vendor, other ISA and job format |
| `src/pvr`, `shaderc/online/powervr` | PowerVR Rogue, USC ISA, firmware kick | other ISA; firmware-mediated submit |
| `src/vivante`, `shaderc/online/vivante` | Vivante GC, command stream | other ISA and command format |
| `src/mali`, `shaderc/online/mali` | Mali Valhall, CSF stream | closest in shape (queues of packed structures), but every field differs |

Differences that decide the size, read from Mesa 26.2.1:

- **Submission.** NVIDIA GPUs run a *push buffer*: 32-bit method headers and
  data, pulled from a GPFIFO ring by a channel (`headers/nv_push.h`; channel
  class `clc86f.h`). Methods address a subchannel bound to a 3D, compute or
  copy class (`clc797.h`, `clc7c0.h`, `clc7b5.h`). NVK leaves channel and
  address-space setup to the kernel (`DRM_NOUVEAU_VM_INIT`, `VM_BIND`, `EXEC`;
  `nouveau_device.c:469-471`). How a channel is made on GA10B without the
  kernel's `nvkm` is UNREAD; it may need firmware that only NVIDIA's stack
  loads.
- **Shader ISA.** NAK's SM70-family encoding is a 128-bit instruction with
  scoreboard and delay fields (`sm70_encode.rs`, `sm80_instr_latencies.rs`).
  Valhall has its own encoding and dependency rules; allocator, scheduling
  and legalisation cannot be shared with `shaderc/online/mali`.
- **Per-shader header.** Each shader needs an SPH (`sph.rs`) beside the code;
  Valhall uses a resource table and renderer-state descriptors.
- **Textures and samplers.** TIC and TSC entries in bound tables
  (`clc797tex.h`), unlike Mali's descriptors in memory.
- **Tiling.** Block-linear GOBs. Which layout the GA10B display engine
  scans out is UNREAD.
- **What carries over.** The front end of `src/shaderc/online/` and the
  tests and `gl_test` cases. The GL layer (`src/gl`) reaches a back end
  through the `YakoglBackend` table: a new back end adds a table, and every
  struct literal of it in `tests/` and `src/` must get the new fields (see
  `CLAUDE.md`, "Two green pull requests").

## 4. The board on Zephyr

- **Zephyr has no Tegra or Jetson support** (Zephyr 4.4.99, earlier pass):
  `soc/` has no NVIDIA directory, `boards/` has none, `dts/arm64/` has none,
  `dts/bindings/` holds no NVIDIA file. The only NVIDIA hit is the word in
  `dts/bindings/vendor-prefixes.txt`. Searching `boards soc dts drivers` for
  `tegra`, `jetson`, `nvidia` and `orin` as words found only unrelated files.
- So the SoC, the board and every driver would be new in our tree: an arm64
  SoC definition (`dts/arm64/nvidia/` does not exist), the GIC-v3 (Zephyr has
  a GICv3 driver; the Tegra234 GIC addresses are in section 1) and the
  generic timer, a UART driver (the Linux console is the `tcu`, which goes
  through the HSP mailbox, so for a first banner a real UART, `uarta` or
  `uarte`, `nvidia,tegra20-uart` compatible, or the SBSA `uarti`, is the
  simpler choice; which pins reach a header: UNREAD), and the memory map.
- **Clocks, resets and power are not registers the OS writes.** In
  `tegra234.dtsi` the host1x, VIC, NVDEC, UARTs and the rest get clocks,
  resets and power domains from the node `bpmp` (`:5321`,
  `nvidia,tegra234-bpmp`), the provider of `#clock-cells`, `#reset-cells`
  and `#power-domain-cells`. The transport is read in
  `drivers/firmware/tegra/bpmp-tegra186.c`: IVC channels (`ivc.c`, a ring
  protocol) in the two shared-memory pools of the `shmem` property
  (`cpu_bpmp_tx` at `sram@70000`, `cpu_bpmp_rx` at `sram@71000`, inside
  `sram@40000000`), and `tegra186_bpmp_ring_doorbell` (`:82`) rings an HSP
  doorbell in `hsp_top0` (`hsp@3c00000`, `tegra234.dtsi:3378`). The
  messages are MRQs in `include/soc/tegra/bpmp-abi.h`: `MRQ_PING` 0,
  `MRQ_RESET` 20 (`CMD_RESET_ASSERT` 1, `CMD_RESET_DEASSERT` 2),
  `MRQ_CLK` 22 (`CMD_CLK_GET_RATE` 1, `CMD_CLK_SET_RATE` 2,
  `CMD_CLK_IS_ENABLED` 6, `CMD_CLK_ENABLE` 7, `CMD_CLK_DISABLE` 8) and
  `MRQ_PG` 66 (`CMD_PG_SET_STATE` 1, `PG_STATE_OFF` 0, `PG_STATE_ON` 1). So a
  Zephyr BPMP client (IVC over shared memory plus an HSP doorbell) is the first
  driver of the port. The IDs the GPU needs are in section 1: clocks 41, 42,
  236, 304; reset 19; power domain 35. Whether the doorbell and the shared
  memory are still usable from a bare image (what MB1 and UEFI leave
  configured, and the SMMU/stream-ID state): UNREAD.
- **GPU register block and interrupts:** UNREAD. The mainline device tree has
  no GPU node (the log-line hint of section 1 is not a source for a design).
  The data would come from NVIDIA's Jetson kernel tree (a vendor tree,
  licence UNREAD) or the TRM.
- **Boot route for sign-off.** What the Developer Guide says (release 36.4.3):
  - UEFI is the CPU bootloader. Its default OS loader is `L4tLauncher`, which
    takes a kernel and device tree from the file system, from partitions or
    through GRUB, selected by the `L4TDefaultBootMode` variable (page
    `SD/Bootloader/UEFI.html`). Boot devices are eMMC, SD, UFS, NVMe and USB
    (USB on T234 only); SD and USB come before the fixed devices.
  - The page shows replacing the default `BOOTAA64.efi` with GRUB, with a
    backup of `L4tLauncher`'s file and `efibootmgr` to go back. It does not
    say that an arbitrary EFI application is supported. UEFI has a built-in
    shell.
  - Secure boot (`SD/Security/SecureBoot.html`): the fuse-based chain of trust
    "ends at the Bootloader"; after that UEFI Secure Boot checks payloads
    against its key database. "If the -u command option is omitted, all
    images flashed to the Jetson device are not signed." Whether a dev kit
    ships fused: not said. A custom EFI binary under enabled UEFI Secure Boot
    would need a signature in `db`; the page does not address it.
  - Mainline U-Boot has no Tegra234 SoC choice (section 2), so U-Boot would be
    a payload chained from UEFI, not a replacement for it: that is an
    inference from the two facts, not a documented route.
  So the candidate routes are: (a) our image as an EFI application started by
  UEFI (needs a PE/COFF stub that Zephyr's arm64 build does not make today:
  not checked), (b) a small EFI or extlinux-booted loader that carries the
  Zephyr image, as the second-stage loader of `docs/notes/am62p-direct-boot.md`
  does for the AM62P (cited by name here, not re-read in this pass), (c) U-Boot
  as an EFI payload. A kernel-based jump is development only. Which route works
  without fusing or signing: a day-one question for the board.

## 5. What can be done before the board arrives

Each item names its check.

1. **Reference bytes from Mesa's compiler on the host.** Mesa ships a
   drm-shim for nouveau, `src/nouveau/drm-shim/nouveau_noop.c`; it takes the
   chip from `NOUVEAU_CHIPSET` (`:542`), so `NOUVEAU_CHIPSET=17b` gives SM87
   through `sm_for_chipset`. Limits, read: it answers `BUS_TYPE` with PCIe
   (`:211-213`) and `NVIF` info platform `PCIE` (`:285`), not SoC, which does
   not change shader bytes. **NVK and the shim:** the shim implements
   `DRM_NOUVEAU_VM_INIT` as a no-op that succeeds (`:533`), so
   `has_vm_bind` becomes true (`nouveau_device.c:469-471`) and the VM_BIND
   gate of `nvkmd_nouveau_pdev.c:75` passes; `VM_BIND` and `EXEC` are no-ops
   too (`:534-535`). By reading, NVK can enumerate on the shim and compile a
   pipeline. That is a reading, not a run: the first check of this item is to
   build NVK against the shim and compile one SPIR-V module. The closer twin
   of the Mali method is a small C program in a Mesa tree that calls NAK's C
   entry points (as `tools/mali/pan_shc.c`, `tools/mali/README-pan_shc.md`).
   Check: the tool prints SM87 code and the SPH words for a corpus of ES 1.00
   programs; a host row in `tools/check.sh` compares them to the vendored
   reference, like the Mali rows.
2. **ISA tables, extracted** into a generated Rust file from NAK's encoder,
   as `tools/mali/gen_isa_rs.py` does. Check: the gate's generated-file row
   compares the bytes.
3. **A host harness for the push-buffer encoder.** Encode a clear and a
   triangle, decode with the logic of `nv_push_dump.c`. Check: the dump
   matches the intended methods.
4. **A BPMP client on the host.** Write the message framing against the ABI
   header (`MRQ_CLK`, `MRQ_RESET`, `MRQ_PG`; IVC framing from `ivc.c`) and
   test it with a mock shared-memory pair. Check: a host test row that
   encodes `CMD_CLK_ENABLE` for clock 41 and `CMD_PG_SET_STATE` for domain 35
   and compares the bytes to a table written from the header.
5. **The Zephyr SoC and board definition, built but not run**, from the
   addresses in section 1 and the console choice. Check: `west build` of a
   blinky-class image in the gate's Zephyr rows (a `gap` row where
   `ZEPHYR_WS` lacks it, never silenced).
6. **Read the UNREAD items** of sections 1, 2, 4 and 7 that need only
   documents: the Orin Nano carrier design (which display output, which
   debug UART), the `linux-firmware` tree and its licence, the nouveau list
   archive. Check: each item in section 7 gets an answer with a path and a
   version.

## 6. What needs the board

Assumes section 7, question 3 has landed; not scheduled.

1. **Day one, in order.** (a) A UART banner from a Zephyr image that the
   boot chain loads. (b) Through the BPMP: read the GPU clock and power
   state, then power the GPU on; a BPMP reply is the first sign of life.
   (c) Read the GPU's identification register (chipset `0x17b`, revision).
   Before (a): decide the boot route (section 4); that is the first thing to
   try, because a signed chain may refuse a bare image.
2. **Alive means**, in order: the identification register reads `0x17b`; an
   empty push buffer with a semaphore release, polled, shows the value; the
   first clear of a render target reads back right from memory; the first
   triangle (one NAK-compiled vertex and fragment pair with the SPH) reads
   back with the expected texels; the first `gl_test` case (its row goes into
   `tools/cts/RESULTS.md`).
3. Display is a separate measurement after rendering works; its path is
   UNREAD.

## 7. Risks and unknowns

Each is a question and the source that would answer it.

1. Does `nouveau` bind to Tegra234 in a newer tree? Answered "no" at Linux
   7.3.0, `65ba8f34` (clone). Re-check against a newer commit before relying
   on it.
2. Does Ampere in `nouveau` need GSP firmware, and is there a GA10B image?
   Read: the GSP path names GA102-family boosters and `.fwsignature_ga10x`;
   no GA10B. Open: the `linux-firmware` `nvidia/` tree and its licence.
3. What would have to appear upstream first? Read from the gaps above: a
   `0x17b` case in `base.c`; a Tegra234 compatible in `nouveau_platform.c`
   with SoC clock, reset and power glue through the BPMP; a GPU node in
   `tegra234.dtsi`; a GA10B init path (GSP or not); and a Tegra234 display
   driver (`tegra_dc_of_match` stops at Tegra194). Whether any patch exists:
   two web searches found none; the list archive (`lore.kernel.org/nouveau`,
   HTTP 403) and the Mesa issue tracker (Anubis page) could not be read. Search
   them for "GA10B" and "Tegra234" by hand before reopening.
4. Can a channel be created and run without the kernel's `nvkm`? NVIDIA's
   Jetson kernel tree (`nvgpu`) and open-gpu-doc.
5. GA10B 3D class, `0xC697` or `0xC797`? The class list a Jetson kernel
   returns; Mesa reads it at run time.
6. Does NAK need an SM87 case? Compile a corpus with `NOUVEAU_CHIPSET=17b`
   (section 5, item 1) and watch for asserts.
7. Does NVK enumerate a Tegra GPU behind `NVK_I_WANT_A_BROKEN_VULKAN_DRIVER`
   and run a compute shader on a Jetson with Linux? Needs the board.
8. Boot chain: the stages are read (section 1). Open: is a dev kit fused,
   may an unsigned EFI application or loader start from UEFI, and does the
   BPMP/IVC state survive into a bare image? Developer Guide "Secure boot"
   says fusing is a flash-time choice; the rest needs the board.
9. Display: UEFI hands a display to the kernel ("Seamless display"). Open: can
   a bare image drive scan-out from the framebuffer UEFI set up, or must it
   program the nvdisplay engine itself, and which output (DP or HDMI) the
   Orin Nano Developer Kit uses? Developer Guide "Display Configuration and
   Bring-up" (Orin page names the AGX DCB only); the carrier design.
10. Is the BPMP IVC channel usable by a second client while the BPMP firmware
    still talks to UEFI? `drivers/firmware/tegra/ivc.c` shows the protocol;
    the answer needs the board.
11. Licence: NAK and the headers are MIT in Mesa. Anything from NVIDIA's
    Jetson SDK (BSP, kernel modules, firmware) is non-redistributable unless
    its licence says otherwise, and does not go into the tree. The licence of
    the Jetson boot firmware: UNREAD.

## 8. A proposed item list

Only after section 7, question 3 has landed upstream. Sizes are against the
done ports (`tools/cts/STATUS.md`, `CHANGELOG.md`).

| # | Item | Size, against the done ports |
|---|---|---|
| 0 | Close the UNREAD items of sections 1, 2, 4, 7 (a note, no code) | small; decides whether the rest exists |
| 1 | Zephyr SoC, board, console, memory map, BPMP client, built not run | larger than the AM62P / i.MX8MP definitions: they needed no firmware client for clocks |
| 2 | Boot route and a loadable image (UEFI or U-Boot) | as the netboot / direct-boot notes; the unknown is the vendor chain |
| 3 | `src/nvidia` channel, memory, semaphore, one clear | the "first light" of Mali / PowerVR; GPFIFO setup is the new risk |
| 4 | NAK-based reference tool and host row (section 5, item 1) | as `pan_shc` |
| 5 | Online back end `src/shaderc/online/nvidia`: ISA tables, register allocation, scoreboards, SPH | the largest item; the Mali back end's per-feature notes (`docs/notes/mali-*.md`, over 30 files) show the long tail |
| 6 | Frame: 3D class state, vertex fetch, textures (TIC/TSC), blend, depth | comparable to `src/mali/frame*.rs` |
| 7 | Display scan-out | UNREAD; may be as large as the Vivante or PowerVR display bring-up |
| 8 | `gl_test` sweep, glmark2, dEQP-GLES2 on the board | the dEQP stage reached 0 failures on the three ports by the same route |

No item above is scheduled. Reopen this note when a GA10B chipset case and a
Tegra234 platform match appear in Linux `nouveau`.
