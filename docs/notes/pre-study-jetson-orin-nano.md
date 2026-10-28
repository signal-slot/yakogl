<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pre-study: NVIDIA Jetson Orin Nano (Tegra234, GA10B)

Date 2026-10-28, main `965ac247d`. Host research only: no file under `src/`
or `tools/` changed, no board run, no board yet.

**Verdict: no open route is shown today, and the port is not scheduled.**
Mesa's NVK and NAK know the GA10B chip by name, but nothing read says NVK
runs on a Tegra Ampere GPU, and Mesa itself marks Tegra "not currently
supported". The kernel half (does `nouveau` bind to Tegra234 at all, and does
it need GSP firmware there) is **not answered by this note**: see "How this
note was made". Section 7 names the exact files that answer it. Until they
are read, the port stays unscheduled; section 8 is the list to use if they
show a route.

## How this note was made

The worker that wrote this had no network access: the Linux kernel, NVIDIA's
Jetson documentation and the Zephyr upstream could not be fetched. What is
cited as read was read on disk:

- **Mesa 26.3.0-devel** (`VERSION` of the tree at
  `$CONNECTED_DEPS_DIR/mesa`; a development snapshot, not a release). Paths
  below are relative to that tree. The tree holds `src/nouveau/` (NAK, NVK,
  the winsys, the drm-shim) and `src/gallium/drivers/nouveau/`.
- **Zephyr** `VERSION` 4.4.99 (`$ZEPHYR_WS/zephyr`), searched for Tegra
  support.
- This repository at `965ac247d`.

Everything marked **UNREAD** below is a fact about the Linux kernel or about
NVIDIA's documents that was not read in this pass. It is a question, not a
finding. No such item is used to support the verdict.

## 1. The hardware

| Fact | Status | Source |
|---|---|---|
| The GPU is GA10B, chipset id `0x17b`, shader model 8.7 (SM87) | read | `src/nouveau/winsys/nouveau_device.c:65-66` (`chipset == 0x17b` returns 87) |
| GA10B has 8 shared-memory carve-outs, the A100 set (0 to 164 KB) | read | `nouveau_device.c:238-246` ("Same for GA10B") |
| Tegra GPUs are reported by the kernel as platform `SOC`, with no VRAM | read | `nouveau_device.c:434-437`, `src/nouveau/vulkan/nvk_physical_device.c:1681-1686` |
| SoC name Tegra234, CPU cluster, memory size, GPU unit counts, clocks | UNREAD | Jetson Orin Nano data sheet and module design guide (NVIDIA Jetson download centre) |
| Display path (the Tegra display engine, HDMI or DisplayPort out through the carrier board) | UNREAD | Jetson Linux Developer Guide, display section; Linux `drivers/gpu/drm/tegra/` |
| Boot chain before our image (the BootROM, the MB1/MB2 stages, UEFI, then a kernel or a bare image) | UNREAD | Jetson Linux Developer Guide, "Boot flow" |
| Which of the above is public documentation | UNREAD | the same documents; the Tegra234 TRM is, as far as this note knows, not public (UNREAD, to be confirmed) |

The one hardware fact Mesa gives about the memory: on Tegra the GPU memory is
system memory only (`nvk_physical_device.c:1681-1686` asserts
`vram_size_B == 0` and one heap), and maps are coherent only if requested
(`src/nouveau/vulkan/nvkmd/nouveau/nvkmd_nouveau_mem.c:152`, `:201`;
`docs/drivers/nvk.rst`, `NVK_DEBUG=coherent` "only applies to Tegra devices").

## 2. The open-source stack today

**Mesa.**

- NVK (Vulkan) supports "Kepler ... up to and including Ada, as well as
  consumer Blackwell" and is conformant "for all officially supported GPUs"
  (`docs/drivers/nvk.rst`, "Hardware support"). It needs "at least a Linux 6.6
  kernel" (same file) and the VM_BIND uAPI: the device open fails without
  it (`src/nouveau/vulkan/nvkmd/nouveau/nvkmd_nouveau_pdev.c:75-77`).
- Tegra is carved out of conformance: `nvk_is_conformant` returns false for
  anything that is not a discrete card, with the comment "Tegra is not
  currently supported" (`nvk_physical_device.c:88-91`). The enumeration of
  such a GPU is gated by `NVK_I_WANT_A_BROKEN_VULKAN_DRIVER` for "GPUs for
  which hardware support is poorly tested or completely broken"
  (`docs/drivers/nvk.rst`); that the flag covers Tegra specifically is not
  stated, and what the gate does on GA10B was not tested.
- Tegra code exists in NVK but is thin and old-chip: the SoC memory heap
  (`nvk_physical_device.c:1681`), coherent-map handling
  (`nvkmd_nouveau_mem.c:152`), ASTC only for `NV_DEVICE_TYPE_SOC`
  (`nvk_physical_device.c:105-109`), and a Tegra GOB layout in the image
  library that is selected "for now always desktop, but will be different for
  Tegra" (`src/nouveau/nil/modifiers.rs:112`).
- The Gallium `nvc0` driver's Tegra block names only
  `0x0ea` (TK1, GK20A), `0x12b` (TX1, GM20B), `0x13b` (TX2, GP10B) as the
  Tegra sector layout, and the default branch says "Xavier's GPU and
  everything else" is not
  (`src/gallium/drivers/nouveau/nouveau_screen.c:377-387`). Mesa's CI for
  `nouveau` on Tegra lists a Jetson TK1 and a `tegra210-p3450` (Jetson Nano)
  only (`src/gallium/drivers/nouveau/ci/gitlab-ci-inc.yml:47-81`; `grep -i
  tegra` in the nouveau CI directory finds those two).
- The old GL driver is out for this generation: "Starting with Mesa 25.1, all
  Turing ... and later GPUs will get NVK+Zink as their OpenGL implementation
  ... The Nouveau GL driver is no longer supported on these cards"
  (`docs/drivers/nvk.rst`). So for GA10B the only candidate is NVK, with Zink
  for GL; ES 2.0 completeness on it is therefore Zink's, and was not measured.
  How far `nvc0` codegen goes past Volta was not read.
- The kernel is asked, not assumed: Mesa reads chipset, platform and the 3D,
  compute and copy classes from the kernel
  (`nouveau_device.c:362-447`, `:575-579`). Mesa does not know the GA10B 3D
  class. Whether it is `AMPERE_A` (`0xC697`) or `AMPERE_B` (`0xC797`)
  (`src/nouveau/headers/nvidia/classes/clc797.h:32`) is therefore
  **UNREAD**, and matters for section 3.

**Linux.** UNREAD. The questions and the files that answer them:

- Does the kernel `nouveau` driver have a Tegra234 / GA10B platform-device
  match? Read `drivers/gpu/drm/nouveau/nvkm/engine/device/tegra.c` (the match
  table and the chipset it hands to the common code) and
  `drivers/gpu/drm/nouveau/nvkm/engine/device/base.c` (the GA10B chipset
  case, if any).
- Does Ampere need GSP firmware in nouveau, and is there a GA10B path without
  it? Read `drivers/gpu/drm/nouveau/nvkm/subdev/gsp/` and the GA10x case in
  `base.c`. Mesa has no knowledge of this.
- Display: `drivers/gpu/drm/tegra/` is the display driver for the older
  Tegra display; whether Tegra234's display engine is in tree, and whether it
  needs firmware, is UNREAD.
- Firmware: where Ampere GSP images come from (`linux-firmware`) and whether
  they are redistributable is UNREAD. The Jetson user space uses NVIDIA's
  own `nvgpu`, whose source is in NVIDIA's Jetson kernel tree (UNREAD).

**ISA and command-stream knowledge we could rely on.** All of it is in Mesa and
is MIT-licensed (read, not copied):

- NAK, the Rust shader compiler: `src/nouveau/compiler/nak/`. The encoder for
  Volta and later is `sm70_encode.rs` (4846 lines), the Ampere/Ada latency
  tables are `sm80_instr_latencies.rs` ("for Ampere and Ada only"), the
  scoreboard choice is `sm70.rs` (`sm >= 80` selects `SM80Latency`). The C API
  entry points are in `api.rs`. Whether NAK has any special case for SM87 was
  not found by grep (only `>= 80` checks), so SM87 is presumed to take the SM80
  path; this is an inference.
- Shader program header (SPH v4): `compiler/nak/sph.rs` and
  `headers/nvidia/classes/clc797sph.h`.
- Class headers (methods, texture header TIC, constant buffer, QMD): the
  files `src/nouveau/headers/nvidia/classes/cl*.h`, generated from NVIDIA's
  open-gpu-doc by `src/nouveau/headers/update-from-open-gpu-doc.py`. The
  Ampere set is `clc697.h`/`clc797.h` (3D), `clc6c0.h`/`clc7c0.h` (compute),
  `clc6c0qmd.h`/`clc7c0qmd.h`, `clc797tex.h`, `clc86f.h` and neighbours
  (channel / GPFIFO), `clc7b5.h` (copy).
- Push-buffer encoding: `headers/nv_push.h`, and the disassembler
  `nv_push_dump.c` that prints a captured push buffer.

## 3. What yakogl already has that applies

No existing back end extends to this GPU. A new back end is needed, in two
parts, as for Mali (`src/mali`, `src/shaderc/online/mali`).

| Existing | What it drives | Why it does not extend (read from Mesa) |
|---|---|---|
| `src/v3d`, `shaderc/online/v3d` | Broadcom V3D, QPU ISA, control lists | different company, ISA and job format |
| `src/pvr`, `shaderc/online/powervr` | PowerVR Rogue, USC ISA, firmware kick | a scalar USC ISA and a firmware-mediated submit |
| `src/vivante`, `shaderc/online/vivante` | Vivante GC, command stream | different ISA and command format |
| `src/mali`, `shaderc/online/mali` | Mali Valhall, CSF command stream, panthor job structs | closest in shape (a queue of packed structures), but the structures differ in every field |

Differences that decide the size, from Mesa:

- **Submission.** NVIDIA GPUs execute a *push buffer*: 32-bit method headers
  followed by data, pulled from a GPFIFO ring by a channel
  (`headers/nv_push.h`; channel class `clc86f.h` and neighbours). Methods
  address a "subchannel" bound to a 3D, compute or copy class
  (`clc797.h`, `clc7c0.h`, `clc7b5.h`). Mali has command-stream instructions
  and `src/mali/cs.rs`; PowerVR and V3D have register-block control lists.
  The channel, the semaphores and the doorbell are new code. How a channel is
  created on GA10B *without* the kernel's `nvkm` (the work NVK leaves to the
  kernel through VM_BIND and EXEC ioctls, `nvkmd_nouveau_pdev.c:75-77`) is
  **UNREAD**; it may need firmware-assisted setup that only the RM or `nvgpu`
  does.
- **Shader ISA.** NAK's SM70-family encoding is a 128-bit instruction with
  explicit scoreboard and delay fields (`sm70_encode.rs`,
  `sm80_instr_latencies.rs`: "Coupled instructions ... need delays but not
  scoreboards. Decoupled ... need scoreboards but not delays"). Valhall has
  its own encoding and dependency rules (`src/shaderc/online/mali/isa`); the register
  allocator, scheduling and legalisation cannot be shared with
  `shaderc/online/mali`.
- **Per-shader header and program state.** Every Ampere shader needs an SPH
  (`sph.rs`: v4, 32 words) beside the code. Valhall has a resource table and
  renderer-state descriptors instead.
- **Textures and samplers.** A texture is a TIC entry (`clc797tex.h`) in a
  table; a sampler is a TSC entry, both referenced by an index in a bound
  table, which is unlike Mali's descriptor-in-memory.
- **Tiling.** The memory is block-linear GOBs, desktop layout on Ampere; the
  Tegra GOB layout is for the old Tegra chips only
  (`nouveau_screen.c:377-387`, `nil/modifiers.rs`). Which layout the GA10B
  display engine scans out is UNREAD.
- **What carries over.** The front end of `src/shaderc/online/` (lexer, parser,
  sema, IR, fold, the ES 1.00 rules) is back-end independent; the Mali port
  added a back end beside it. The tests in `tests/` and the `gl_test` cases
  carry over unchanged. The GL layer (`src/gl`) is reached through the
  `YakoglBackend` table (CLAUDE.md, "Two green pull requests"): a new back
  end adds a table and must be added to every literal of that struct.

## 4. The board on Zephyr

- **Zephyr has no Tegra or Jetson support.** In the tree at `ZEPHYR_WS`
  (`VERSION` 4.4.99): `soc/` has no NVIDIA directory (the vendors listed
  there include `nxp`, `ti`, `rockchip`, `broadcom` ...), `boards/` has no
  NVIDIA vendor directory, `dts/arm64/` has none, and `dts/bindings/` holds no
  NVIDIA file. The only NVIDIA hit is the word in
  `dts/bindings/vendor-prefixes.txt:517`. Searching `boards soc dts drivers`
  for `tegra`, `jetson`, `nvidia` and `orin` as words found only unrelated
  qemu `or1k` files besides that.
- So the SoC, the board and every driver below would be new in our tree: an
  arm64 SoC definition and device tree (`dts/arm64/nvidia/` does not exist),
  the GIC and generic timer (Zephyr has them for arm64, the Tegra234 interrupt
  map is UNREAD), the UART used for the console, the memory map, and the
  clocks and resets of the GPU and of the display engine. On Tegra, the clocks
  and power are not registers an OS writes directly: the Tegra boot and
  power firmware (BPMP) is asked through a mailbox (UNREAD, Jetson Linux
  Developer Guide, "BPMP"). If so, a Zephyr BPMP client is the first driver of
  the port. This is an inference from memory of the Tegra design, unverified.
- **Boot route for sign-off.** UNREAD: whether the Orin Nano loads an arm64
  image from UEFI (`BOOTAA64.EFI`) or from U-Boot, and what the MB1/MB2 stages
  require of an image. The repository already has a precedent for each
  alternative (`docs/notes/am62p-direct-boot.md`: power-on into Zephyr through
  a second-stage loader; `docs/notes/imx8mp-netboot.md`: U-Boot netboot). The
  Linux-kernel jump, `kexec`-style, would be development only, as the task
  says.

## 5. What can be done before the board arrives

Each item names its check.

1. **A reference of Mesa's compiler bytes on the host.** Mesa ships a
   drm-shim for nouveau, `src/nouveau/drm-shim/nouveau_noop.c`, whose README
   says it implements "the minimum of nouveau in order to make shader-db
   work" and takes the chip from `NOUVEAU_CHIPSET`. The SM is derived from the
   chip id by `sm_for_chipset`, so `NOUVEAU_CHIPSET=17b` yields SM87
   (`nouveau_device.c:54-110`). Two limits, both read: the shim reports PCIe,
   not SoC (`nouveau_noop.c:285`), which does not change shader bytes; and it
   targets the old Gallium driver, so that NVK works on it is **UNREAD**
   (VM_BIND and EXEC ioctls). The Mali method's closer twin is a small C
   program in a Mesa tree that calls NAK's C entry points (as
   `tools/v3d/v3d_shc.c` and `tools/mali/pan_shc.c` do for their back ends,
   `tools/mali/README-pan_shc.md`). Check: the program prints SM87 code and
   the SPH words for a corpus of ES 1.00 programs; a host row in
   `tools/check.sh` compares the bytes to the vendored reference, the way the
   Mali rows do.
2. **The ISA tables, extracted** into a generated Rust file from NAK's
   encoder, as `tools/mali/gen_isa_rs.py` does. Check: the gate's
   generated-file row compares the bytes.
3. **A host harness for the push-buffer encoder.** Encode a clear and a
   triangle into a push buffer, then decode it with the logic of
   `headers/nv_push_dump.c`. Check: the dump matches the intended method
   stream.
4. **The Zephyr SoC and board definition, built but not run**, once the
   interrupt, memory and UART facts of section 4 are read. Check: `west
   build` of a blinky-class image in the gate's Zephyr rows (a gap row until
   `ZEPHYR_WS` has it, never silenced).
5. **Reading the UNREAD items of sections 1, 2 and 4.** This is the real
   first task: it needs network access and the Jetson documents. Check: each
   item in section 7 gets an answer with a file path and version.

## 6. What needs the board

The following cannot be planned until section 7 questions 1 to 3 are answered;
assuming a route exists:

1. **Day one, in order.** (a) A UART banner from a Zephyr image that the boot
   chain loads, with the register dump of the GPU's identification register
   (chipset `0x17b`, revision). (b) Reading the GPU's power and clock state
   before touching it, and the BPMP replies if used. (c) Creating a channel
   and submitting an empty push buffer with a semaphore release, polled.
2. **Alive means**, in order: the semaphore value appears (the host reaches the
   GPU); the first clear of a render target is read back correct from memory;
   the first triangle (one NAK-compiled vertex and fragment pair, with
   the SPH) reads back with the expected texels; then the first `gl_test`
   case (as for the other ports, `tools/cts/RESULTS.md` gets the row).
3. Display is a separate measurement after rendering works, because the
   scan-out path is UNREAD.

## 7. Risks and unknowns

Each is a question and the source that would answer it.

1. Does `nouveau` bind to Tegra234 at all? `nvkm/engine/device/tegra.c`
   match table, `base.c` chipset case for `0x17b`, Linux at the latest
   stable tag.
2. Does Ampere in `nouveau` require GSP firmware, and is there a GA10B image?
   `nvkm/subdev/gsp/`, `linux-firmware` `nvidia/` tree, its licence file.
3. If 1 and 2 are "no route", what would have to appear upstream first?
   Read the nouveau mailing list (`nouveau@lists.freedesktop.org`) and
   Mesa's issues for "GA10B"; this note found none on disk.
4. Can a channel be created and run without the kernel's `nvkm`? Only
   `nvgpu` (NVIDIA's Jetson kernel tree) and NVIDIA's open-gpu-doc would say.
5. What is the GA10B 3D class, `0xC697` or `0xC797`? The class list a Jetson
   kernel returns; Mesa reads it at run time (`nouveau_device.c:575-579`).
6. Does NAK need an SM87 case? Compile a corpus with `NOUVEAU_CHIPSET=17b` and
   look for asserts or unexpected latency tables (section 5, item 1).
7. Does NVK enumerate a Tegra GPU (the `NVK_I_WANT_A_BROKEN_VULKAN_DRIVER`
   gate) and run a trivial compute shader on a Jetson with Linux? Needs a
   board, with Linux, as a control; this also answers 1 and 2 in practice.
8. Boot chain: which stages are signed, and may a non-NVIDIA image be loaded
   from UEFI or U-Boot? Jetson Linux Developer Guide, "Secure boot" and
   "Boot flow".
9. Display engine: is scanout from a plain framebuffer possible without the
   display firmware? Jetson Linux Developer Guide; `drivers/gpu/drm/tegra/`.
10. Licence of everything we read or generate from: NAK and the headers are MIT
    in Mesa; anything taken from NVIDIA's Jetson SDK (BSP, kernel
    modules, firmware) is non-redistributable unless its licence says
    otherwise, and does not go into the tree. The licence of the Jetson
    boot firmware is UNREAD.

## 8. A proposed item list

Only after question 1 and 2 give a route; sizes are in terms of the done
ports (`tools/cts/STATUS.md`, `CHANGELOG.md`).

| # | Item | Size, against the done ports |
|---|---|---|
| 0 | Answer section 7 questions 1 to 4 (a note, no code) | small; it decides whether the rest exists |
| 1 | Zephyr SoC, board, console, memory map, built not run | smaller than the AM62P / i.MX8MP definitions only if a Tegra clock and power client is not needed; otherwise larger |
| 2 | Boot route and a loadable image (UEFI or U-Boot) | as the netboot / direct-boot notes; the unknown is the vendor chain |
| 3 | `src/nvidia` channel, memory, semaphore, one clear | the "first light" of Mali / PowerVR; GPFIFO setup is the new risk |
| 4 | NAK-based reference tool and host row (section 5, item 1) | as `pan_shc` |
| 5 | Online back end `src/shaderc/online/nvidia`: ISA tables, register allocation, scoreboards, SPH | the largest item; the Mali back end's per-feature notes (`docs/notes/mali-*.md`, over 30 files) show the kind of long tail to expect |
| 6 | Frame: 3D class state, vertex fetch, textures (TIC/TSC), blend, depth | comparable to `src/mali/frame*.rs` |
| 7 | Display scan-out | UNREAD; may be as large as the Vivante or PowerVR display bring-up |
| 8 | `gl_test` sweep, glmark2, dEQP-GLES2 on the board | the dEQP stage reached 0 failures on the three ports by the same route |

No item above is scheduled. Reopen this note when the Linux files of section
7, questions 1 and 2, have been read; if they show no route, record what
would have to appear upstream and close it.
