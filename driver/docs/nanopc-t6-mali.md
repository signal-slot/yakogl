<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL on the NanoPC-T6 (Mali-G610, RK3588): design

**Z1 PASS 2026-10-19 00:57 JST, build 576065151**: `t6_hello`, kexec'd out of
Armbian, ran on the board (console, WDT0 armed, 10 s on the system timer) and
returned through PSCI `SYSTEM_RESET` in 45 s (records:
`0dce654a8:tools/mali/board-ref/2026-10-19-z1/`). Open: the watchdog reset path has not
been exercised (no hang has been left to WDT0 yet).

Status, 2026-10-18: **designed; the first parts are built.** On main: the `gpu-mali`
skeleton and packers (#1073), the Valhall v10 ISA table, encoder and decoder (#1101),
`pan_shc` and the drm-shim byte reference (#1072), the frame builder (#1102). Open:
the development OSAL over panthor (#1068), L1 (our frame through panthor on the board)
and L3 (our Valhall back end). This page was written before the code, so it could be
reviewed against the target first. Facts carry a citation; what is not
measured yet is under "Open"; the user's decisions are under "Decisions".

References, read for this page:

- `mesa:` Mesa 26.2.1 (`src/panfrost/`, `src/gallium/drivers/panfrost/`), MIT: may be
  transcribed with attribution.
- `linux:` Linux v6.18 (7d0a66e4, 2025-11-30). `drivers/gpu/drm/panthor/` is
  **`GPL-2.0 or MIT`** in every file (`panthor_regs.h:1`, `panthor_fw.c:1`, ...), and
  `include/uapi/drm/panthor_drm.h` is MIT. The RK3588 power-domain, clock, page-table
  and RK806 drivers are GPL only (`pmdomain/rockchip/pm-domains.c`,
  `clk/rockchip/clk-rk3588.c`, `iommu/io-pgtable-arm.c`, `mfd/rk8xx-*`): facts and
  file:line only.
- `zephyr:` Zephyr main a915b2f1 (2026-10-16).
- `board:` the NanoPC-T6 under Armbian 26.8.10, Linux 6.18.55, panthor 1.5.0,
  Mesa 25.0.7, read on 2026-10-18 (read-only).
- `trace:` Mesa drawing one GLES2 triangle on the board (`PAN_MESA_DEBUG=trace,sync`):
  clear blue, red triangle over the left half of a 64x64 RGBA8 FBO; the full
  read-back gave 2048 red + 2048 blue, 0 misplaced.

## What is new, and how the Mali differs from the three GPUs we have

- **A firmware MCU, like the PowerVR.** The GPU has its own microcontroller (the "MCU") that runs
  Arm's CSF firmware (`arm/mali/arch10.8/mali_csffw.bin`). The host does not start
  jobs by register: it writes a **command stream** (CS) into a ring buffer and rings a
  doorbell. The firmware schedules it. Host and firmware talk through shared memory
  pages with a request/acknowledge toggle protocol (linux:`panthor_fw.h:403-458`).
- **The command stream is a small CPU.** 64-bit instructions (opcode in bits 63:56,
  `mesa:genxml/v10.xml:563-566`), 96 registers, 8 scoreboard slots, loads, stores,
  branches. A draw is `RUN_IDVS` after ~30 register moves that name its descriptors
  (`mesa:pan_csf.c:1433-1730`); a frame ends with `RUN_FRAGMENT` and
  `FINISH_FRAGMENT` (`pan_csf.c:1152-1213`).
- **A tiler with a growable heap**, like the V3D's binner memory but larger: chunks of
  128 KiB to 8 MiB that the host adds when the firmware reports `TILER_OOM`
  (linux:`panthor_heap.c`, `panthor_sched.c:1465-1516`).
- **The viewport transform is in the vertex shader** (`mesa:bifrost_nir.c:942`). Only
  the scissor (viewport ∩ scissor) and the depth clamp are registers.
- **IDVS**: one vertex shader is compiled into a position variant and a varying
  variant; the hardware allocates the varying buffers (`mesa:bifrost_nir.c:1299-1305`,
  `bifrost_compile.c:4367-4398`).
- **Blending is mostly fixed-function**, through a 16-byte blend descriptor read by
  the FS's `BLEND` instruction. A **blend shader** is needed for MIN/MAX,
  SRC_ALPHA_SATURATE and factor pairs that are neither equal up to inversion nor
  ONE/ZERO (`mesa:pan_blend.c:26-110`, `pan_context.c:162-230`). This is between the
  PowerVR (FS-lowered blending) and the V3D/GC7000 (fixed only).
- **No clauses in the ISA (Valhall).** Each 64-bit instruction carries a 4-bit flow
  field (waits on message slots, end); 64 registers, r48-r63 preloaded
  (`mesa:va_pack.c:68-143, 850-1087`; `compiler.h:1157-1240`).

## The GPU, the SoC, the board

| Item | Value | Source |
|---|---|---|
| GPU MMIO | 0xfb000000, 0x200000 | linux:`rk3588-base.dtsi:451`, board `/proc/iomem` |
| IRQs | SPI 92 job, 93 mmu, 94 gpu (INTID 124-126), level | linux:`rk3588-base.dtsi:459-462`, board `/proc/interrupts` |
| Coherency | none (no `dma-coherent`) | board DT; linux:`panthor_device.c:28` |
| GPU_ID | 0xa8670005: arch 10.8, rev 6, product 7 = Mali-G610 | board dmesg; linux:`panthor_regs.h:14-20` |
| Cores | shader_present 0x50005 (4 cores), 1 L2, 1 tiler; VA 48 bits, PA 40 bits; 8 AS | board dmesg; linux:`panthor_regs.h:23-33` |
| Clocks | CLK_GPU / COREGROUP / STACKS (CRU 262-264), gates CLKGATE_CON(66) bits 4/6/7; rate through SCMI clock 5 (SMC 0x82000010) | linux:`rk3588-base.dtsi:453-457, 355-366`, `clk-rk3588.c:1354-1363` |
| Power domain | PD_GPU 12, PMU 0xfd8d8000 (sequence below) | linux:`pm-domains.c:1277, 390-691` |
| Supply | vdd_gpu_s0 = RK806 DCDC1 on SPI2 (0xfeb20000, 1 MHz); boot-on, **not** always-on; 675 mV at 300-600 MHz | linux:`rk3588-nanopc-t6.dtsi:755-824`, `rk3588-opp.dtsi:118-160`; board |
| Clock now | 300 MHz from cpll (devfreq simple_ondemand) | board debugfs |
| Console | uart2, 1500000 baud | linux:`rk3588-nanopc-t6.dtsi:40-42` |

**GPU power-on (kernel side, Z1)**, from linux:`pm-domains.c` (GPL: facts only):
supply on; domain clocks on; PMU+0x14c = 0x00010000; poll PMU+0x290 bit 1; SIP SMC
0x82000003; PMU+0x10c = 0x00010000, then poll ack 0x118 bit 0 and idle 0x120 bit 0
clear; restore the QoS registers (0xfdf35000 + 0x0/0x200/0x400/0x600, offsets
0x08-0x18). Rockchip registers take a write-enable mask in bits 31:16.

**L2 and reset**, linux:`panthor_gpu.c:190-334`: soft reset is `GPU_CMD` (0x30) =
0x101 and wait for `RESET_COMPLETED` (IRQ bit 8); `GPU_COHERENCY_PROTOCOL` (0x304) =
31 (none); `L2_PWRON` (0x1a0) = 1, poll `L2_READY` (0x160). The host never powers the
shader cores: the firmware does, from `core_en_mask` (linux:`panthor_fw.c:982-983`).

**MMU (Z2)**, linux:`panthor_mmu.c`: AS n at 0x2400 + 0x40·n; AS0 is the MCU's,
user address spaces are 1-7. Page tables are the Arm LPAE stage-1 format, 4 KiB
granule, 4 levels, 48-bit VA. `TRANSCFG` = 0x420001C6 (live: AS0 and AS1) and
`MEMATTR` = 0x8F8F8F8F8C4C8F4C **live** (a read-only `/dev/mem` read of
`0xfb000000 + 0x2400 + 0x40·n + 0x8` with the GPU runtime-active; AS0, the MCU's,
and AS1, a GL client's, give the same word, 2026-10-26, record
`e3d9b7470:tools/mali/board-ref/2026-10-26-open-questions/regread.out`). Bytes 0-3, the
four attributes the page tables use (PTE attribute index 0-3), are what
`mair_to_memattr` gives for the non-coherent case (`panthor_mmu.c:785-790,
1990-2030`: 0x4C non-cacheable, 0x8F write-back with read/write allocate, 0x4C
device as non-cacheable, 0x8C write-back for `INC_OWBRWA`); bytes 4-7 read 0x8F, not
the 0x4C that a zero MAIR byte would give, and no PTE selects them. The image
writes the live word. Changes go LOCK, wait, cache flush, UNLOCK (`panthor_mmu.c:506-621`).
The whole of it, with the values the image writes: "Z2: AS0 and a cache flush"
below.

## The firmware (Z3)

The file `mali_csffw.bin` (282624 bytes; header 0.3, interface 1.5.0) is a list of
sections, each mapped at a fixed MCU address in AS0 (format
linux:`panthor_fw.c:39-140, 523-626`; parsed from the board's file):

| MCU VA | Size | Kind |
|---|---|---|
| 0x00000000, 0x00400000, 0x00410000 | 64 KiB each | read-only data |
| 0x00800000 | 128 KiB | code |
| 0x01000000, 0x02000000 | 256 KiB each | zeroed data (0x02000000 writable) |
| 0x03000000 | 256 KiB | protected mode: skipped (not supported here) |
| 0x04000000 | 64 KiB | **the shared interface**, writable, cached-coherent |

Like `pvr_fw_blob.h`, the firmware is never committed: a build step turns the
board's file into a header, and the repository holds only the generator.

Boot: load the sections, enable AS0, `MCU_CONTROL` (0x700) = 2, wait up to 1 s for
`JOB_INT` bit 31; a GLB version of 0 means the boot failed
(linux:`panthor_fw.c:944-947, 1028-1069, 1343-1403`).
**`GPU_TIMESTAMP_OFFSET` (0x88) is not needed**: panthor never writes it (every
occurrence in Linux v6.18 `drivers/gpu/drm/panthor/` is the define,
`panthor_regs.h:76`, and one read, `panthor_drv.c:776`, for the TIMESTAMP_INFO
query), and the firmware boots and runs jobs with it at its reset value: the query
and a `/dev/mem` read of `0xfb000000 + 0x88` both give 0 on the board (2026-10-26,
`panthor-query.out`, `regread.out`; `GPU_TIMESTAMP` runs at 24 MHz, the arch timer's
frequency, `panthor_drv.c:771`). The image leaves it alone.

**Host side, written 2026-10-26 (no board run yet).** What is code:

- **The file.** The board's `/lib/firmware/arm/mali/arch10.8/mali_csffw.bin`
  (Armbian's armbian-firmware), copied read-only on 2026-10-26: 282624
  bytes, sha256 `a27847ea11f8efb3136340c3ba8aab413ae25145eeb4a7f64ff5edd829a2405b`,
  header 0.3, version hash 0x01050000, build `git_sha:
  95a25d71030715381f33105394285e1dcc860a65`. It lives outside the tree
  (`MALI_CSFFW`, `tools/cts/deps.toml`). `driver/tools/mkmalifw.sh` makes
  `driver/src/fw/mali_csffw_blob.h` from it (gitignored, the form of
  `mkfwblob.sh`), after checking the CSF magic and header major 0.
- **The section list**: `driver/rust/src/gpu/mali/fw.rs` (`fw::parse`),
  panthor's checks in panthor's order plus three of ours (no overlap in AS0,
  no more data than the range, at most 16 sections). On the board's file it
  gives the table above: 7 sections to load and the protected one skipped;
  15 other entries (2 config, 8 trace buffers, 1 timeline, 4 optional
  unknown) read past. The shared section's data is 8 KiB at file offset
  0x43000; its GLB control block before boot reads version 0, features 0,
  input 0x04002000, output 0x04006000, 8 groups of stride 0xa0,
  `instr_features` 0x71; group n's control block is at 0x1000 + 0xa0·n
  (input 0x04003000 + 0x80·n, output 0x04007000 + 0x40·n, 8 streams of
  stride 0xc), its streams at + 0x40 + 0xc·m (stream m of group 0: input
  0x04003400 + 0x80·m, output 0x04007200 + 0x100·m).
- **The interface and the boot**: `driver/rust/src/gpu/mali/csf.rs`. The
  GLB/CSG/CS blocks as `repr(C)` layouts with their sizes pinned (GLB
  control 0x20, GLB input 0x88 with `idle_timer` at 0x80, CS output 0xd8,
  ...; the test `the_layouts_are_panthors` holds every field offset), the
  req/ack toggle, update and wait, the doorbells, and `csf::boot`: load
  every section where AS0 maps it (data, then zeroes, then a clean),
  `MCU_STATUS` must read disabled, clear `JOB_INT` bit 31 (refused if it
  stays set), `MCU_CONTROL` = 2, poll bit 31 for 1 s, read the GLB control
  (version 0 refused), require every group and stream block and page inside
  the shared section, panthor's counts (3-31 groups, 8-32 streams) and
  identical slots, write the global configuration, wait 100 ms for its
  toggles' acknowledge (ours; panthor does not wait there), then one `PING`
  acknowledged within 100 ms. Each stop is a numbered `CsfRefusal` with its
  value. All access is through `Mmio`; AS0 is behind the `McuMemory` trait
  (where a section is for the CPU, and a clean). The host tests run the
  sequence over a model of the MCU that boots only from loaded and cleaned
  code; the gate's firmware row runs the parser and the boot on the board's
  file (`MALI_CSFFW`), and is a gap naming the file without it.

**Predicted for the board run** (each value is what the code above writes or
expects):

| Item | Predicted |
|---|---|
| `MCU_STATUS` before the start | 0 (disabled, after Z1's soft reset) |
| `JOB_INT_RAWSTAT` bit 31 | rises within 1 s of `MCU_CONTROL` = 2; `MCU_STATUS` then 1 |
| GLB `version` | 0x01050000 = **1.5.0**, the image header's version hash; `features` 0, `instr_features` 0x71 (the board's dmesg under panthor, read 2026-10-26: "CSF FW using interface v1.5.0, Features 0x0 Instrumentation features 0x71"; the image's control block holds the same 0 and 0x71 before boot) |
| GLB input / output | 0x04002000 / 0x04006000; 8 groups, stride 0xa0 (the image's values; the firmware may rewrite them) |
| CSG 0 | input 0x04003000, output 0x04007000, 8 streams of stride 0xc; `suspend_size` and `protm_suspend_size` 24577..28672 (the 7-page buffers panthor allocates, above) |
| CS 0 of CSG 0 | input 0x04003400, output 0x04007200; `features`: work registers 96 (`cs_reg_count`, measured under panthor), scoreboards 8 |
| Written | `core_en_mask` 0x50005; `poweroff_timer` 235, `idle_timer` 19 (24 MHz ticks of 1024); `progress_timer` 0x271000; `ack_irq_mask` 0x0400050e; `req` 0x40e |
| Acknowledged | `CFG_ALLOC_EN`, `CFG_POWEROFF_TIMER`, `CFG_PROGRESS_TIMER` within 100 ms; then **`PING`** (req bit 8 toggled, ack bit 8 follows) within 100 ms |

After Z3 the kernel side must ping at least every 12 s (`PING_INTERVAL_MS`).

**Open, before the board run:**

- **Z2's map, extended to every section.** The boot needs AS0 enabled with
  every loaded section mapped at its MCU address (the 7 of the table, 896
  KiB: read-only for the data and code, writable for 0x02000000 and the
  shared section, executable only for the code; uncached on the GPU side
  except `CACHED`, as panthor maps them on a non-coherent GPU), and the
  shared section mapped uncached for the CPU, since the req/ack words take
  no cache maintenance. Z2's host side (below, "Z2: AS0 and a cache
  flush"; `gpu::mali::pgtable::Tables`, `mmu::as_enable`,
  `board::nanopc_t6_mmu::mmu_up`) maps one section, the shared one, onto
  the `t6_mmu` probe's 64 KiB backing; the other six need backing in the
  image and their own `map` calls (9 table pages, Z2's pool holds 16).
  `csf::McuMemory` is the seam: its `cpu` is the backing's CPU address for
  the section's MCU address, its `clean` the probe's
  `sys_cache_data_flush_range`. Not written here.
- The Zephyr side: a sample that embeds `mali_csffw_blob.h`, runs Z1's
  power-on, Z2's AS0 and `csf::boot`, and prints the record; and a
  `yd_t6_*` FFI entry for it. Its image row in the gate will need the
  header, and must be a gap without it, like the firmware row (`needs`).
- Whether the firmware needs a GPU cache flush after the load beyond the
  clean (panthor syncs each section for the device and flushes nothing
  else; the L2 has just been powered).

The interface (linux:`panthor_fw.h:15-269`): a **global** block (GLB) at 0x04000000
with input/output pages; 8 **command stream groups** (CSG, stride 0xa0), each with 8
**command streams** (CS, stride 0xc). **Runtime values** after the firmware boots
under panthor (2026-10-26, record `e3d9b7470:tools/mali/board-ref/2026-10-26-open-questions/`,
`panthor-query.out`, `gems.during`): `DEV_QUERY_CSIF_INFO` gives `csg_slot_count` 8,
`cs_slot_count` 8 (per group), `cs_reg_count` 96 work registers, `unpreserved_cs_reg_count`
4 (92 for a stream), `scoreboard_slot_count` 8; the same as the image's. The suspend
sizes are not in any uAPI or debugfs field; the kernel buffers panthor allocates from
them show in debugfs `gems` while a group exists: two "FW suspend buffer" objects of
28672 bytes each (7 pages), one from `suspend_size` and one from `protm_suspend_size`
(`panthor_sched.c:3506-3513`), so both fields are 24577..28672 and equal after
page rounding; a group of one queue also has a 4096-byte sync object page, a 4096-byte
ring buffer, an 8192-byte "Queue FW interface" (one 4 KiB input and one 4 KiB output
page) and a 4096-byte "Group job stats" page. The seven "FW section" objects are
64 KiB x3, 128 KiB, 256 KiB x2 and 64 KiB (the shared interface): the image's list
without the protected-mode section. To send a request the host toggles a `req` bit so it
differs from `ack`, then rings a doorbell (0x80000 + 0x10000·i; doorbell 0 is the
global one). Global init: `core_en_mask` = shader_present, the power-off, progress
and idle timers in 24 MHz arch-timer ticks, and a `PING` at least every 12 s
(linux:`panthor_fw.c:978-1009, 1313-1335`).

One group, one stream is enough for GL: our render service is one context per
module (the CLAUDE.md rule), as on the other three GPUs.

**Submit (Z4)**: a ring buffer (4-64 KiB, uncached) bound to CS 0 of CSG 0; write the
instructions, advance `insert`, ring the stream's doorbell (CSG slot + 1). Completion
is a 16-byte sync object the stream increments with `SYNC_ADD64`, and the CSG
`SYNC_UPDATE` interrupt (linux:`panthor_sched.c:1061-1079, 1316-1345, 2912-3231`).
**Hang**: the job timeout is 5 s in panthor; the reset is soft reset, L2, MMU, reload
and reboot the firmware (linux:`panthor_device.c:127-155`, `panthor_fw.c:1079-1148`).
As on every GPU here, a reset is the floor, not the fix: every board verdict names
the recovery count.

## The command stream and descriptors (L1)

The byte reference is the trace. A frame, as Mesa builds it:

1. Once per context: `HEAP_SET` (tiler heap context) and a `TILER_OOM` exception
   handler (`mesa:pan_csf.c:1903-1926, 267-388`).
2. Per draw: register moves for the shader descriptors (position, varying, fragment),
   resource tables, push uniforms, local storage, tiler context, scissor, depth
   clamp, varying size, blend array, depth/stencil descriptor, primitive and DCD
   flags, counts; then `RUN_IDVS` (`pan_csf.c:1433-1730`; the full register map is
   `v10.xml:890-928`).
3. Per frame: `FINISH_TILING`, `RUN_FRAGMENT` with the framebuffer descriptor
   (framebuffer 128 B, ZS/CRC extension 64 B, render targets 64 B each, tiler
   context 128 B), `FINISH_FRAGMENT` (returns heap chunks), `FLUSH_CACHE2`
   (`pan_csf.c:1051-1213, 557-605`).

**Mesa 25.0.7 (the board's) against 26.2.1 (the source we transcribe)**, settled
2026-10-26 by the same one-triangle program run on the board through both: 25.0.7
is the system package, 26.2.1 a build from the release tarball in
`/root/mesa-26.2.1-build` used through `LD_LIBRARY_PATH` only (`-Dgallium-drivers=panfrost
-Dplatforms= -Dglvnd=disabled -Dllvm=enabled`, native, 7 minutes; nothing installed
over the system Mesa), both with `PAN_MESA_DEBUG=trace,sync`; both read 2048 red,
2048 blue, 0 misplaced. Records `e3d9b7470:tools/mali/board-ref/2026-10-26-open-questions/
mesa-25.0.7.pandecode` and `mesa-26.2.1.pandecode`. The three open choices:

| Choice | 25.0.7 (board) | 26.2.1 (source) |
|---|---|---|
| `TSD_3` (register 30) before `RUN_IDVS` | set, `MOVE d30` to the same TLS address as `TSD_0` (trace line 58) | **not set**: no write of register 30 in the draw stream, only `TSD_0` (r24) |
| Resource table entry count (low 6 bits of the SRT pointer) | 7 = `PAN_NUM_RESOURCE_TABLES`, unaligned (`...3647`, `...36c7`) | **8**: `ALIGN_POT(PAN_NUM_RESOURCE_TABLES, MALI_RESOURCE_TABLE_SIZE_ALIGNMENT)` with the alignment 4 (`pan_cmdstream.h:262-267`, `v10_pack.h`), so 7 rounds to 8 (`...f6c8`, `...f748`) |
| OOM handler's scoreboard slot | `SET_SB_ENTRY #3` + `WAIT #8` (slot 3) | **slot 4**: `SET_SB_ENTRY #4` + `WAIT #0x10` (`PANFROST_SB_AUX` = 4, `pan_csf.h:60-66`); it first runs `SET_SB_ENTRY #0, #0` (the OTHER endpoint, `csf_oom_handler_init` in `pan_csf.c`) and keeps its counter in r31 and its register spill in two `STORE_MULTIPLE` (`d94`, `d94 + 32`) instead of one |

The frame's own scoreboard slot is 2 in both (`PANFROST_SB_RENDER`, as
`SB_RENDER` in `src/mali/frame.rs`); 26.2.1 also names the 48-bit move `MOVE48` where
25.0.7's decoder prints `MOVE`. `src/mali/frame.rs` still
follows 25.0.7 for `TSD_3` (`reg::TSD_3`, set in the draw stream) and the table count
(`RESOURCE_TABLES`): both run on the board's panthor, and moving them to 26.2.1 is a change
to make with the byte-for-byte tests (`frame_tests.rs`) against the 26.2.1 trace.

About 20 descriptor kinds cover GLES2 (`v10.xml`, 101 structs, 782 fields). We
**generate** the Rust packers from `v10.xml`, as the V3D's packets were generated
(vendored generator, committed output, a check that a regenerate equals the
committed bytes). The generator must refuse, not mask: a value that does not fit,
a `shr` with low bits set, a `log2` of a non-power of two, a `minus(1)` of 0, and
two non-zero members of one union (Mesa ORs overlapping fields together and checks
ranges only in debug builds: `gen_pack.py:83-102, 289-336`,
`util/bitpack_helpers.h:53-66`).

Choices that keep the first version small, each with its Mesa switch:

- **Linear render targets, no AFBC, no CRC** (`PAN_MESA_DEBUG=linear,nocrc` makes
  the matching reference trace). AFBC is a later performance item.
- **Clean-tile write on** for every cleared target, so a clear over empty tiles
  reaches memory (`mesa:pan_desc.c:1262-1300`).
- **No preload in the first milestone**: every frame starts from a whole colour
  clear, and its depth and stencil from a clear value (the frame's own, or the
  value a whole clear left in the plane with no frame open,
  `ds_retention::Plane`). Preload (a pre-frame shader that reads the old
  image) comes with the GL cases that need it; it samples the old image, so it
  needs the texture descriptors and the compiler's texture look-up first.
- **Tiler OOM**: start with a heap large enough for the gl_test frames and count OOM
  events; incremental rendering through the exception handler is a later item. An OOM
  is reported, never silently dropped.

## Shader compiler

The product path is our own compiler only, as on the V3D: the shared front end
(`src/shaderc/online/`) into a Valhall back end in `src/shaderc/online/mali/`, with
an encoder, a decoder and a byte-exact round trip.

- The ISA source is Kraid's `isa-v9-v14.xml` in Mesa 26.2.1
  (`src/panfrost/compiler/kraid/`), marked "Copyright Arm Ltd., MIT": 365
  instruction variants valid for v10 with exact bit fields. Mesa's older
  `valhall/ISA.xml` (262 entries) and `va_pack.c` are the second reading.
  `src/shaderc/online/mali/isa/` holds the table generated from both
  (`tools/mali/gen_isa_rs.py`), the encoder and decoder, and Mesa 26.2.1's
  disassembly; its gates are the board's binaries
  (`tools/mali/isa-ref/`) and Mesa's test vectors.
- The references are instruments, not a product path, and there are two
  (`tools/mali/README-pan_shc.md`):
  - The byte reference is Mesa 26.2.1's own GL driver (gallium panfrost on
    panthor) run on the host on Mesa's drm-shim, with no GPU
    (`tools/mali/drm_shim_ref.sh`, `gl_ref.py`): it takes the GLSL ES 1.00 text
    YakoGL receives, through the GLSL path the board's driver takes, and the
    pandecode trace holds the programs. Upstream 25.0.7 on the same shim is
    byte-equal to the board's Debian 25.0.7 on the board's two programs
    (`pan_ref_diff.py diff-ref` on the committed board traces,
    `tools/mali/testdata/board-25.0.7/`; the 25.0.7 shim is a separate build).
  - `pan_shc`, built in the Mesa tree in the pattern of `v3d_shc` (spirv_to_nir
    with the panfrost options, the driver's preprocessing and
    `bifrost_compile_shader_nir` for gpu_id 0xa8670005;
    `mesa:pan_shader.c:95-239, 500-633`), names what the trace prints as
    values: the FAU and sysval layout (which uniform or sysval each FAU word
    is), the varying layout, attribute records, preload and register counts,
    the IDVS split and `no_psiz` offsets, the early-ZS table. On the 22 regions
    of the committed case list (`tools/mali/testdata/cases.txt`) its bytes
    equal the shim driver's except for two front-end gaps of the SPIR-V path
    (mediump arithmetic, matrix-times-vector order), measured per region. Option
    B, a GLSL front end for `pan_shc` (Mesa's GLSL compiler with panfrost's
    caps), closes both gaps and is the follow-up; the same case list checks it.
  On the board the dump is `BIFROST_MESA_DEBUG=shaders` with
  `PAN_MESA_DEBUG=trace` (25.0.7 has no `PAN_MESA_DEBUG=shaders`).
- What the back end must do that the others did not: the IDVS split (position and
  varying variants from one VS); the viewport transform in the VS (scale and offset
  are push uniforms); `ATEST` then `BLEND` at the end of every FS, with the
  `BRANCHZI` blend-shader stub; blend shaders for the modes above; message slots and
  flow waits (`va_insert_flow.c:15-35`); push uniforms (FAU, 4 pages of 32 64-bit
  slots); the zero padding to 128 bytes and never an all-zero program
  (`va_pack.c:1219-1238`).

The gates are the ones the V3D has: every builder default asserted against the
corpus, decode-back equality against the reference's own bytes, and on the board
gl_test cases with pixel read-back on every side of each condition.

### The general lowering (approved 2026-10-19)

The product is Slint on femtovg. Slint adds no GLSL of its own
(`tools/shaders/femtovg/PROVENANCE.md`). On 2026-10-19 I surveyed the 42
femtovg and fvgtest programs in `tools/shaders`. Only `main.vert` and three
trivial fvgtest shaders compiled. The rest need:

- **highp (fp32) fragment arithmetic:** every femtovg fragment shader.
- **the uniform array `frag[14]`,** read at constant indices only: 40
  programs.
- **`if`** (up to 6 per shader): 31 programs.
- **one bounded loop:** 2 programs (filter_image).
- **more ops:** fmin, ffma, fdot2, flt, fmax, fsqrt, frcp, ffract,
  int compare and convert, discard, fle/feq/b2f/bcsel/fsign.
- **textures:** 24 programs (the texture slice).

The smallest real program is `fill_color.nodiscard.noscissor`, Slint's solid
fill. It is 28 IR instructions with no `if`.

The fragment stage was a plan of fixed mediump shapes (`frag.rs`: per-lane
expression trees, vectorised into fp16 pairs, a form admitted only when a
reference draws it). That could not grow into these programs, and it is
gone: the fragment stage is `nir_fs.rs` on the general lowering (S1c
below). The other three
back ends (Vivante, PowerVR, V3D) each have one generic walk of the IR, a
table-driven ALU selector and real if/loop lowering.

Mesa's own shape, which the bytes follow:
- `emit_cf_list` makes structured BIR blocks: `if` is `BRANCHZ` plus
  `JUMP`; a loop is a header block and a back-edge.
- Before that, `nir_opt_peephole_select` (limit 64) turns small `if`s into
  `bcsel`, and `nir_opt_loop_unroll` (bifrost_nir.c:268) unrolls loops.
- `bi_emit_alu` is one switch by op and bit size.
- `bi_opt_push_ubo` moves constant-offset uniform reads into FAU
  (`PAN_MAX_PUSH` 128 words; `frag[14]` is 56).
- `bi_opt_mod_prop_forward`, `bi_lower_opt_instructions` and
  `va_fuse_add_imm` run on the BIR after selection.

**The design.** One lowering for both stages, in Mesa's order:

1. The IR becomes an SSA graph: one value per lane, with its bit size;
   locals renamed per block; `if` and loops kept as structured regions
   with phis. This generalises `nir.rs`.
2. Mesa's NIR passes run on that graph, transcribed and named: the
   algebraic rules, `peephole_select`, loop unrolling, and
   `lower_alu_width`/`opt_vectorize` for fp16 pairs.
3. Selection is `bi_emit_alu` as a table: (op, bit size) to BIR (`emit.rs`,
   `ROWS`). A row is admitted only when a reference draws it.
4. Mesa's BIR passes follow (`opt.rs`).
5. BIR gets blocks with `BRANCHZ`/`JUMP`. The back half (liveness, LCRA,
   flow) works across blocks.
6. The fragment uniform block is read as Mesa reads it: FAU push, with
   LD_BUF as the fallback.

This replaced `frag.rs`'s plan and its `REFERENCED` forms (deleted), and
the shape-specific walks in `nir.rs`. Precision per value is a later shared
item: our IR has none, and Mesa takes fp16 or fp32 from GLSL precision.
Every femtovg fragment shader is highp, so the steps below do not wait for
it.

**The regression set**, green at every commit:
- every population byte-equal (VS `cmul`/`cdiv`/`cpack`; FS `sel_*`; the
  triangle samples);
- `MALI_TESTS`;
- `tools/mali/corpus-verdicts.txt`: a verdict may only move forward;
- the board sweep's PASS set may not shrink.

The old fragment path stayed live until the new one reproduced every
fragment population. The switch-over was one commit; deleting the old plan
was the next.

**Steps.** Each is one or more PRs on Mali paths only. S1 to S5 have landed
on main; S6 remains (#1384).
- **S1** the shared selector and passes, then the fragment stage on the
  graph, then the switch-over and the deletion. Landed.
  - S1a: `emit.rs` and `opt.rs`, with the vertex stage selecting through
    them; the vertex populations stay byte-equal.
  - S1b: the fragment stage on the graph (`nir_fs.rs`, selecting through
    `select.rs`), beside `frag.rs`; every fragment population byte-equal on
    it.
  - S1c: the switch-over (`da18ff68`, from `work/mali-lower-s1c` 1aef2adb8),
    then the deletion of `frag.rs`'s plan (`FsPlan`, `REFERENCED`) and the
    comparison scaffolding (`2db7bfbd`).
- **S2** highp fragment arithmetic. Landed (`302f0604`): a highp fragment
  shader compiles to fp32; `fvgtest_ftcoord` and `fvgtest_green` are
  byte-equal to Mesa. Cases 28 and 29 stay PASS. The precision rule and the
  fp16 hazard are in `docs/notes/mali-highp-fragcoord.md` (`cf0710e0`).
- **S3** the fragment uniform block. Landed (`b181ad67`): a highp fragment
  shader's block is pushed to the FAU, `fvgtest_inner` byte-equal. Case 30
  PASSes on the board; record `0dce654a8:tools/mali/board-ref/2026-10-21-s3-ubo/`
  (`c2890b3d`).
- **S4** the ALU rows femtovg needs. Landed (`6deda2ee`, with the link's
  code motion, `motion.rs`, tightened in `ed47c4c9`): each row is pinned by
  a shim reference (`tools/mali/testdata/s4_*`).
  `fill_color.nodiscard.noscissor` is byte-equal to the shim, and its
  picture matches Mesa's read-back on the board; record
  `0dce654a8:tools/mali/board-ref/2026-10-21-s4-alu/` (`c233faf1`). `gl_FragCoord`
  (the r59 preload, `U16_TO_F32`, the y transform's FAU words) followed
  (`a52638c5`), byte-equal to the shim; record
  `0dce654a8:tools/mali/board-ref/2026-10-22-s4-fragcoord/` (`64bb7fff`).
- **S5** `if`: `peephole_select` first, then blocks. Landed 2026-10-22:
  both peephole passes (the GL front end's, limit 8; bifrost's,
  limit 64, which may not speculate a uniform read) and real BIR blocks
  (`BRANCHZ`, the jump, phis, liveness, LCRA and flow across blocks).
  `fill_color.nodiscard`, `stencil.nodiscard` and `fvgtest_cover` are
  byte-equal to the shim (3 of the 35 femtovg programs with
  `fill_color.nodiscard.noscissor`); the scissor `if` keeps real blocks
  and both of its sides draw Mesa's picture on the board; record
  `0dce654a8:tools/mali/board-ref/2026-10-22-s5-if/`. The sweep's PASS set grew by
  case 31 fvgcov. What stops the next femtovg programs: `discard` (S6),
  `mix` (a fused multiply-add of a built-in, the gradient programs), the
  texture look-up.
- **S6** discard and the bounded loop. Open.

## Where the code goes

Everything original is Rust (`docs/source-form.md`, the rule); C only where Zephyr
registration forces it, as for the GC7000's kernel side.

| Layer | Path | Model |
|---|---|---|
| Cargo feature | `gpu-mali` (exactly one GPU per build; needs `shaderc-online`, `no-shader-table`) | `gpu-v3d` |
| Back end | `src/mali/` | `src/v3d/` |
| Descriptor and CS packers | generated from `v10.xml`, vendored under `third_party/mesa-26.2.1/` | the V3D packets |
| Shader back end | `src/shaderc/online/mali/` | `src/shaderc/online/v3d/` |
| Kernel side (power, MMU, firmware, CSF) | `driver/rust/src/gpu/mali/`, feature `mali`, SoC feature `rk3588` | `driver/rust/src/gpu/vivante/` |
| Zephyr registration | `driver/drivers/gpu/gpu_mali_rust.c` (thin) | `gpu_vivante_gc_rust.c` |
| Development OSAL on Linux | a std crate that implements `OsalOps` over panthor's uAPI (`include/uapi/drm/panthor_drm.h`, MIT); never in an image | `tools/mali/panthor/` (crate `yakogl-mali-panthor`); the `OsalOps` table is filled at L2, when the `gpu-mali` back end exists |
| Board glue | `driver/zephyr/lib/board/nanopc_t6/` (Zephyr side); `driver/rust/src/ip/dw_wdt.rs`, `soc/rk3588.rs`, `aarch64.rs` (feature `rk3588`) | `.../rpi4/`, `ip/imx_wdog.rs` |
| Zephyr board | `driver/boards/friendlyarm/nanopc_t6/` (target `nanopc_t6`) | `driver/boards/toradex/` |
| Gate row | `host-test,gpu-mali,shaderc-online,no-shader-table` | the V3D row |

**Gate.** Local = `tools/mali/check.sh`: the rows of the `mali` scope and
nothing else (fmt, the Mali clippy and `--lib mali::` tests, the two generated
tables, the aarch64 check, `yakogl-mali-panthor`, pan_shc when `MESA_PAN_TREE`
is set, otherwise a gap line). CI = `tools/check.sh --changed <base>`, which
selects the same scope for a diff under `src/mali`, `src/shaderc/online/mali`
and `tools/mali`. The test-count pin is `MALI_TESTS` in `tools/mali/gate.env`,
not in `tools/check.sh`: editing `tools/check.sh` selects every row, so a pin
there made each Mali pull request run the whole gate. A Mali change does not
touch `tools/check.sh`.

Zephyr main has the RK3588 SoC (GICv3, arch timer, ns16550 UARTs, a generic CRU
reset driver, SCMI over SMC with an SCMI clock driver) and the boards
`roc_rk3588_pc` and `orangepi_5_ultra`, but no NanoPC-T6 board, no Rockchip
GPIO/pinctrl/SPI driver, no RK806, no PMU power-domain driver and no GPU clocks
(zephyr a915b2f1). Development images boot from Armbian (kexec, or a file that
U-Boot loads); the product form is U-Boot booting Zephyr from the eMMC. The eMMC
loader area is never written.

## Milestones

Each one ends with something measured on the board. The L milestones run under
Armbian through panthor and are development results only; a sign-off needs the Z
milestones (Zephyr booted by U-Boot), as on every board.

- **M0, done 2026-10-18**: Armbian on the eMMC; panthor brings the GPU up; Mesa's
  triangle reads back exactly; the trace is taken.
- **L1, done 2026-10-18**: the development OSAL creates a VM, buffers, a group with
  one queue and a tiler heap through panthor, and runs the trace's frame (linear
  targets, our packers) with a full read-back equal to Mesa's. The frame itself is
  built by `src/mali/frame.rs` and, on the host, is the board's to the byte
  (`src/mali/frame_tests.rs`: 24 blocks and 51 CS words against the linear dump,
  each difference named: the OOM context and incremental-rendering FBDs are not
  built, so three blocks move; GPU-written fields; one byte range of Mesa's stack).
  On the board `tools/mali/panthor` `panthor-frame` builds it in place at its
  buffers' VAs and submits it: the whole 64x64 target reads red 2048, blue 2048,
  other 0, every pixel where the frame puts it (Mesa's program printed the same
  counts); the clear alone reads blue 4096; with the frame built and not
  submitted the target keeps its fill pattern (4096). Each twice, no kernel
  message during any run, group state 0. The shader programs are still Mesa
  25.0.7's (`tri_shaders.rs`) and no TILER_OOM handler is installed. What L1
  does not show: one frame per process, so arena reuse after a fence is not
  exercised; and the read-back is compared with the expected image per pixel
  (RGBA exact, misplaced 0), not byte-for-byte with Mesa's dumped target.
- **L2, done 2026-10-18**: a GL program draws through YakoGL end to end
  (`tools/mali/panthor` `panthor-gl`): EGL, a 64x64 RGBA8 FBO,
  glClear, tri.vert and tri.frag compiled by our Valhall back end at
  glCompileShader time and linked (`shaderc::online::mali::link::link_shaders_mali`),
  glDrawArrays, glReadPixels. The `gpu-mali` backend builds the frame in the
  OSAL's buffers at their GPU VAs and submits it through the Mali OSAL
  interface (`src/osal/mali.rs`); the panthor development OSAL
  (`tools/mali/panthor/src/osal.rs`) is that interface over panthor. On the
  board, each twice, 0 kernel messages, group state 0, no GPU work lost: red
  2048 / blue 2048 / other 0 / misplaced 0; tri.frag changed to green before
  glShaderSource: green 2048 / blue 2048 (the bytes are compiled at run time);
  the clear alone: blue 4096; tri_varying with a green colour attribute (the
  IDVS varying program at its offset 256, VARY_SPD, VARY_SIZE 12): green 2048
  / blue 2048. `frame.rs` takes what the compiler emits: promoted constants and
  the blend constants as push words (`PushWord::Constant`,
  `Sysval::BlendConstants`), the varying program, one to four float32
  attributes of up to 16. On the host, `frame_tests.rs`
  (`the_frame_with_our_programs_differs_from_l1_only_where_named`) names every
  difference from the L1 frame, and `backend_tests.rs` runs the backend and
  the GL program over a mock of the OSAL interface. What L2 does not show: one
  frame per process on the board (two slots, the fence before reuse and a
  wait that times out are exercised on the host only); a target other than
  64x64 is accepted but never measured; the varying run's colour is one
  constant, so interpolation across a triangle is not measured; no TILER_OOM
  handler.
- **L3**: our Valhall back end compiles the triangle's shaders equal to the
  reference; then gl_test and dEQP-GLES2 under Armbian, case by case.
  First slice on the host, 2026-10-18: tri, tri_varying and the triangle
  sample byte-equal to Mesa 26.2.1's shim programs (`src/shaderc/online/mali/`,
  SHADERC_NOTES.md "The Valhall back end"); not yet run on the board.
  Uniforms, 2026-10-19 (host only): float, vec2, vec3, vec4 and mat4
  uniforms in a vertex shader with `+`, `*`, dot and matrix products, and a
  mediump vec4 uniform in a fragment shader, byte-equal to the shim on 25
  programs (`tools/mali/testdata/sel_u_*`); the frame builder fills their
  push words from the GL uniform storage.
  gl_test on the board, 2026-10-19 (`panthor-gltest`, tree 7912553d9,
  record `0dce654a8:tools/mali/board-ref/2026-10-19-l3-gltest/`): of the 162 case
  numbers this build has, 1 PASS (case 85, cubetail: colour clears into
  the 1x1 faces of a cube level through an FBO, twelve read-backs exact),
  149 REFUSED, 7 SKIP (gl_test's own, by name) and 5 FAULT (SIGSEGV:
  `src/backend.h` `yakogl_heap_base()` calls the Mali table's empty
  `heap_base`, which the Rust side reads as 0; fixed by #1314, re-run
  pending). Each sweep twice with the
  same verdict lines; the 50-case regression set twice, its verdicts
  equal to the sweep's. 0 kernel messages in every run window. The board
  rebooted between the set runs (boot 5605f19e) and the sweeps (boot
  8d64eba7): another session's kexec jump at 03:26:39 (the Z1 record in
  #1315), not these runs. A third set run with the UART and a 2-minute
  dmesg tail captured (boot 9552855b throughout) gave the same 50 verdict
  lines, 0 kernel messages and 0 UART bytes. The refusals by whole string, the
  largest first: more than one varying (32), a multiply by a constant
  other than 0.0 or 1.0 (25), a texture look-up in a fragment shader (17),
  a fragment uniform other than one mediump vec4 (13), fragment arithmetic
  beyond +, * and products (11); in the frame, a depth or stencil clear or
  a masked clear (8) and a draw that writes depth (5).
  Depth, stencil and clears, 2026-10-19 (record
  `0dce654a8:tools/mali/board-ref/2026-10-19-frame-depth/`). The reference is the
  board's own Mesa 25.0.7 on `zs.c` (`tools/mali/mesa-board-ref.sh`; five
  modes, each 0 pixels off its prediction, 0 kernel messages). What its
  two-draw frame shows (`0dce654a8:tools/mali/board-ref/2026-10-19-frame-depth/mesa/
  mesa-depth.pandecode`): both draws in one RUN_FRAGMENT with one
  VERTEX_TILER_STARTED; the FBD with Z clear 1.0, S clear 90, Z Write
  Enable and the ZS/CRC extension (a linear D24S8, row stride 256, written
  back, clean tile write on), the render target at +0xC0; Force Early for a
  depth test that can fail, Weak Early for one that always passes.
  `src/mali/frame_zs_tests.rs` holds our frame to that dump byte for byte,
  with three differences named: the tiler context's place (Mesa's comes
  after the OOM context it builds and we do not), the S target base Mesa
  25.0.7 writes into an unused S part (26.2.1 merges no S part for a D24S8
  view), and the equation word of a disabled blend (Mesa leaves it at a zero
  v10.xml does not name). `panthor-gl` runs `zs.c`'s program mode for mode
  through our stack; each mode twice, every run 0 kernel messages, 0 GPU
  work lost, 0 refused, no GPU recovery in the boot's ring. By part:
  - The open frame (begin / add_draw / end_frame): `--depth` and
    `--nodepth` are two draws in one frame, read back as predicted; a
    frame slot built again after its fence is gl_test 85 in the sweep (13
    submits in one process, two slots, all 12 read-backs exact, arena
    high-water desc 288 and cs 104 bytes; `ours2/15-*.log` lines 1542-1568).
  - The ZS attachment and the depth state: `--depth` columns 0-15 red,
    16-63 green; `--nodepth` 0-47 red, 48-63 green; `--nocolour` (a
    depth-only draw with no fragment shader) 0-15 red, the rest blue.
  - Clears: `--clearhide` blue 4096 (a depth clear value hides a draw);
    `--mask` magenta 4096 (a masked clear, the quad); `--scissor-clear`
    red 1024 in the box, blue 3072 (a scissored clear, the quad).
  gl_test, two sweeps per tree, identical: the totals stay PASS 1, REFUSED
  149, SKIP 7, FAULT 5, and exactly the 13 cases of this slice changed
  verdict. 1 34 127 164 moved from "mali: a draw that writes depth is not
  built yet", and 40 from the clear line, to "mali: a vertex attribute other
  than draw-time float32 data the GL core staged is not built yet" (a
  buffer object: the Buffer descriptor needs the buffer's size, which the
  GL core's draw does not carry). The backend stages the attributes
  (`vertex_attributes`) before the frame builder checks the target's
  orientation, so whether these cases meet the window target's flip_y
  refusal next is not measured yet.
- **L3, next: the preload** (the texture slice's, as the pre-frame shader
  samples the old image). 8 gl_test cases wait for it, each with
  "mali frame: a frame without a colour clear needs the preload of the old
  image, which is not built yet": 66 102 107 162 165 167 168 202. Each
  starts a frame after a glFinish, a read-back or a framebuffer switch with
  a masked, scissored or partial clear (102: a whole clear, glFinish, a
  masked clear), and 167 and 168 also test stencil an earlier frame drew
  (the depth/stencil preload; the backend refuses that by name too:
  "mali: a frame that uses depth or stencil drawn by an earlier frame and
  not cleared since needs its preload, not built yet").
  Varyings, 2026-10-19 (`src/shaderc/online/mali/`, SHADERC_NOTES.md "The
  Valhall back end"): a vertex shader writes several varyings computed
  from attributes, laid out as Mesa's link lays them out, and a fragment
  shader's colour is mediump `+` and `*` over them, the mediump vec4
  uniform and constants, vectorised in fp16 pairs as Mesa's NIR has it;
  byte-equal to the shim on 13 vertex and 20 whole programs
  (`tools/mali/testdata/sel_v_*`). On the board (record
  `0dce654a8:tools/mali/board-ref/2026-10-19-l3-varyings/`): gl_test's own
  color_uv pair, with both varyings affine in the window position, read
  back every one of 4096 pixels equal to the per-pixel prediction written
  before the run, twice; this is the first measure of interpolation
  across a triangle. On the tree merged with #1318 the gl_test sweep
  (twice, identical) moved 63 verdicts from main's own sweep, none to
  PASS: 33 cases compile both stages now and stop in the frame (a vertex
  attribute from a buffer object 21, a frame without a colour clear 2,
  primitives 2, a colour target 2, culling, a window target and polygon
  offset 1 each) or at a texture (3); 8 vertex shaders with a computed
  varying compile and stop at a named refusal; 9 fragment refusals now
  name the product with a constant they meet; 13 are a reworded message.
  0 kernel messages in every run.
- **Z1, done 2026-10-19 (by kexec, a development result)**: a Zephyr image on
  the T6 prints on uart2 and is fed by a watchdog for a bounded time; GPU
  power-on; `GPU_ID` reads 0xa8670005; L2 ready. `t6_hello` counted 10 s and
  reset through PSCI; the watchdog proof image came back through WDT0 alone;
  `t6_gpu` powered PD_GPU, read `GPU_ID` 0xa8670005, soft reset the GPU and
  brought its L2 up (below, "Z1" and "The GPU power-on"). The sign-off form
  (U-Boot starting the image) has not run.
- **Z2**: AS0 page tables; a GPU cache flush round trip. Host side done
  2026-10-26 (design, builder, register sequence, the `t6_mmu` image and its
  gate row; below, "Z2: AS0 and a cache flush"); **the board run has not
  happened**: it waits for the owner's go-ahead and the UART.
- **Z3**: firmware loaded; the MCU boots; GLB version reads 1.5.0; a `PING` is
  acknowledged. Host side written 2026-10-26 ("The firmware (Z3)": the
  generator, the parser, the interface, the boot over a model of the MCU);
  the board run waits for Z2's board run and for its map to cover every
  section ("The firmware (Z3)", "Open, before the board run").
- **Z4**: CSG 0 / CS 0 started from our kernel side; the L1 frame renders the same
  bytes as under panthor. From here the same back end and compiler run on Zephyr,
  and gl_test and dEQP move to the product form.

### What L1 must provide

The frame builder (`src/mali/frame.rs`) builds a frame in memory it is
given. These are the facts the review of that builder found about the way
L1 and the OSAL must fit it. `panthor-frame`
(tools/mali/panthor/src/bin/panthor-frame.rs, `l1.rs`) meets each one, and
since L2 the `gpu-mali` backend does too; the citations are to
`src/mali/backend.rs` and `src/mali/frame.rs` as they stand.
Measured on the board: the HEAP_SET operand is the
`tiler_heap_ctx_gpu_va` of TILER_HEAP_CREATE (our heap's context read
0x800000015000, the trace's value), and the CSIF query gives
`cs_reg_count` 96 and `unpreserved_cs_reg_count` 4, so 92 user registers,
Mesa 25.0.7's constant.

- **Arenas are (CPU pointer, GPU VA) pairs, built in place.** The descriptors
  and the command-stream words hold absolute GPU addresses, so a frame is
  written into the memory the GPU will read, at the VA that memory has.
  `Arena::new(label, base, mem)` takes exactly that pair. The backend makes
  each slot's arenas at its OSAL buffers' VAs (`slot_arenas`, backend.rs:1093)
  and the context arena at the context buffer's (backend.rs:686).
- **The heap descriptor and the sample positions are built once per context,
  not per frame.** The GPU moves the heap descriptor's Bottom and Top while
  it works, so rebuilding it would overwrite the GPU's state.
  `frame::build_context` (frame.rs:318) builds both; the backend calls it once
  in `init` (backend.rs:690) and keeps the result.
- **A queue-init stream.** The trace starts each run with `MOVE d72` and
  `HEAP_SET d72` (the excerpt's lines 35-36 and 427-428), then `MOVE d86` /
  `MOVE32 r88` / `SET_EXCEPTION_HANDLER` for the OOM handler (lines 37-39).
  `frame::queue_init` (frame.rs:290) builds the first two (Mesa 25.0.7
  pan_csf.c:1469-1473); the OOM handler is not built. The backend runs it
  once in `init` (backend.rs:702-713). The operand of HEAP_SET,
  0x800000015000 in the trace, is not the heap descriptor's address
  (0x7fffffffb000) but the heap context TILER_HEAP_CREATE returns
  (measured, above).
- **Inputs only the OSAL has.** The tiler heap's context, first chunk and
  chunk size (`OSAL_QUERY_MALI_TILER_HEAP`, backend.rs:646); the register
  counts (`OSAL_QUERY_MALI_GPU`, backend.rs:644, turned into user registers by
  `frame::user_regs`, frame.rs:340); the 64 KiB geometry buffer, an OSAL
  buffer (backend.rs:677); and the fence of each submit (`submit`,
  backend.rs:550).
- **A slot's memory is reused only after its frame's fence.** The GPU may
  still read the last frame's descriptors, so the backend keeps two frame
  slots and waits for a slot's fence before it builds into it again
  (`wait_slot`, backend.rs:594, called from `open_frame`, backend.rs:1343).
  A wait that times out or fails keeps the slot in flight; a draw that needs
  it is refused (`SLOT_STILL_IN_FLIGHT`) until the OSAL reports the fence
  complete.
- **A frame holds every draw from its first clear to the flush.** Mesa's
  batch: the draws of one target are one RUN_FRAGMENT, so they are
  depth-tested against each other in the tile buffer, and a clear after a
  draw is a quad in the same frame. The frame builder opens, extends and
  ends a frame in three calls (`frame::begin_frame`, `add_draw`,
  `end_frame`, frame.rs:905, 929, 975), each over the same slot memory; the
  backend keeps the arenas' tops and the stream's length between the GL
  core's calls (`Arena::resume`, `Stream::resume`) and ends the frame at
  `flush`, `finish` or a draw into another target (`end_open`,
  backend.rs:1361). `kick_id` is the open frame (backend.rs:1619), so the
  GL core's `kick_pending` and `flush` + `finish` reach it before the CPU
  touches what it reads or writes.

### L3: gl_test under Armbian

gl_test (`driver/tools/pvrlab/gl_test.c`) is one C source for every GPU.
Until L3 it ran in Zephyr images and, on the host, with the recording
OSAL of `tools/hostharness`. On the T6 it runs under Armbian as the
binary `panthor-gltest` of `tools/mali/panthor`.

**Why not the CMake library.** `-DYAKOGL_OSAL=linux` builds `src/osal/linux.c`:
the host OSAL with no GPU (malloc'd memory, fake VAs, every submit complete
at once). It cannot submit to the Mali. The Mali's OSAL on Linux is the Rust
table in `tools/mali/panthor/src/osal.rs` over panthor, installed with
`osal::install()` (`osal_set_ops`). CMake has no `mali` GPU, and the
library it builds has a C OSAL compiled in.

**The form.** The crate's `build.rs` compiles `gl_test.c` (the host's
`cc`, or `aarch64-linux-gnu-gcc` for the board) with
`-DGL_TEST_USER -DYAKOGL_GPU_MALI -DGL_TEST_ES100 -Wall -Wextra -Werror`,
into a static archive the crate links. It runs the compiler itself, not
through the `cc` crate: a new dependency changes `Cargo.lock`, and a
`Cargo.lock` change selects every row of `tools/check.sh`. `panthor-gltest` is a Rust `main`.
The GL entry points `gl_test.c` calls are the `#[no_mangle]` functions of
the `yakogl` crate the binary already links (features `gpu-mali`,
`shaderc-online`, `no-shader-table`). It is one static binary, built and
run by `run-on-board.sh` like `panthor-gl`; the build of the crate is also
the gate's compile of the Mali arm (`tools/mali/check.sh` builds the crate).

**One process per case.** The parent runs each case in a child process of
the same binary (`--one <case>`), one after the other, so there is one GL
client at a time. The child installs the OSAL, makes C stdout
line-buffered (C and Rust lines then come out in order), and runs the one
case through gl_test's own walker (`gl_test_set_list`, `gl_test_run`; the
walker is the gl_test runner crate, `driver/tools/pvrlab/runner`, which the
binary links). A
process per case is needed for the measurement: the library logs only the
first refused draw of a process (`note_draw_refused`) and a `glog_once!`
line once per process, so in one long process a later case would inherit
the silence of the case that refused first. It also keeps a case that
faults the group from deciding the verdicts of the cases after it. The
case list is gl_test's own: `--set` reads the regression set from
`gl_test_set_cases` (the list is in the runner crate once), `--sweep` tries
every number 0..=255.

Alternatives, and why not:
- A staticlib of the crate, linked by a C `main`. It needs a second
  artefact and a C link line for one binary. The OSAL is already Rust.
- A `mali` arm in CMake with the panthor OSAL as a C library. It puts a
  development OSAL into the product build system, and the OSAL would have to
  be C (our code is Rust).
- Porting gl_test to Rust. It would be a second suite. The point is one
  source for four GPUs.

**The Mali arm of gl_test.c.** `YAKOGL_GPU_MALI` selects `GL_TEST_MALI`.
The OSAL takes no argument (`g_osal->init(NULL)`, like the GC7000). A site
that meant "the PowerVR" by "neither the GC7000 nor the V3D" now says
`GL_TEST_PVR`, so the Mali does not fall into the PowerVR's knobs. A case
the Mali build does not have is `-2` (no such case), as on the other GPUs.

**The verdict of a case.** The child defines `gl_test_case_result` (a weak
symbol in `gl_test.c`, made for harnesses like this one) to get the case's
rc, and prints one `result` line: the rc, the library's counters
`yakogl_draws_refused` and `yakogl_gpu_work_lost`, the panthor group state
and gl_test's first bad check.

**Refusal strings are whole.** The OSAL's log line is 255 bytes and the
library's info logs are 256, and a cut string can hide the real refuser.
So the child takes each string from its source:
- A refused draw: the Mali backend keeps the first refusal of the process
  as the `&'static str` itself (`mali::backend::first_refusal`). A draw
  the GL core refuses before the backend has only the library's log line;
  that one is marked truncated when it fills the line.
- A refused compile: `build.rs` wraps gl_test's `glCompileShader` calls
  (`-Wl,--wrap`). After a failed compile the child compiles the same
  source again through the same shader back end
  (`shaderc::backend().compile`) into an 8 KiB log.
- A refused link: `glLinkProgram` is wrapped the same way, and the
  program's info log is read whole. It has no second source, so it is
  marked truncated when it fills the library's 256 bytes.

The parent reads the `result` line and these strings and gives one of:
- `NOT-STARTED`: the child printed nothing, not even its build line (for
  example qemu-user with no binfmt for aarch64: exit status 127). This is
  the harness's environment, never a verdict on a case.
- `FAULT`: work lost, the group not in state 0, the VM unusable, the child
  killed by a signal or gone before its `result` line, or no case ran (the
  OSAL, the backend or the context did not come up).
- `HANG`: the child did not finish within the timeout (60 s) and was
  killed.
- `REFUSED`: a draw was refused, or rc is not 0 and a shader or a program
  was refused. The row carries the first string, whole; the histogram
  groups by the whole string.
- `PASS`: rc 0, nothing of the above.
- `FAIL`: rc -1 with nothing refused. The row carries gl_test's first bad
  check, with got and want.
- `SKIP`: gl_test skipped the case by name (rc -3).
- `NO-CASE`: rc -2 (no such case on this build).

Each child ends with the OSAL's buffer high-water line, next to the
backend's arena high-water lines.

### Z1: the image, its watchdog and the way back

Written on the host 2026-10-18, run on the board 2026-10-19 (the jumps
below). The T6's debug UART is wired to the build host since 2026-10-19
(below); before that, a kexec without a console would have said nothing. The board facts
below were read under Armbian, read-only.

**Zephyr.** The pinned workspace (`west.yml`, zephyr af17c0c8, 2026-07-28)
already has `soc/rockchip/rk35/rk3588`, `dts/arm64/rockchip/rk3588.dtsi`
(GICv3, the arch timer, PSCI over SMC, uart2/uart3 as ns16550) and
`roc_rk3588_pc`; Z1 needs no newer revision. The board is
`driver/boards/friendlyarm/nanopc_t6` (target `nanopc_t6`, qualifier
`rk3588`): CPU 0 only; uart2 at 24 MHz and 1500000 baud (the Zephyr dtsi
says 12 MHz; the board's `clk_summary` reads `sclk_uart2` 24000000); 32-bit
UART access (`UART_NS16550_ACCESS_WORD_ONLY`, Linux `reg-io-width = <4>`,
rk3588-base.dtsi:2558); the AArch64 Image header; and the `wdt0` node.

**Memory map** (board `/proc/iomem` and dmesg; the device bases are
rk3588-base.dtsi's):

| Range | What | Across the kexec |
|---|---|---|
| 0x00000000-0x001fffff | firmware: TF-A, the SCMI shared memory at 0x0010f000; not System RAM | live |
| 0x02000000-0x0432ffff | Armbian's kernel image | dead |
| **0x10000000-0x10ffffff** | **the Zephyr image** (`zephyr,sram`, 16 MiB; `t6_hello` uses 148 KiB) | ours |
| 0xcd400000-0xe13fffff | reserved by Linux | dead |
| 0xe1440000-0xeb43ffff | hdmi-receiver CMA (nomap) | not ours |
| 0x1_0022_0000-0x1_002f_ffff | the GIC's ITS and LPI tables (13 of 64 KiB, in Linux's log) | live: the redistributors keep them |
| 0xfb000000 | GPU | |
| 0xfd7c0000 | CRU | |
| 0xfe600000, 0xfe680000 | GICD, GICR | |
| 0xfeaf0000 | WDT0 | |
| 0xfeb50000 | uart2 | |

Why 0x10000000: it is inside the System RAM bank 0x00200000-0xe143ffff and
under no entry of it. The kernel copies the segment into place at the jump
through a copy of its linear map (linux:`machine_kexec.c:131-138`,
`relocate_kernel.S:55-65`), so the destination must be System RAM, and what
Linux kept there before does not matter. What stays live is outside it: the
firmware below 0x00200000, the GIC tables above 4 GiB. It is 2 MiB aligned
and below 4 GiB. `nanopc-t6.sh` checks all of this on the live board before
a jump (one System RAM range, no `/proc/iomem` entry, no GIC table in
Linux's log).

**Watchdog: WDT0, a DesignWare APB watchdog.** The facts, with sources:

- Base 0xfeaf0000, 0x100 bytes, SPI 315 level high,
  `"rockchip,rk3588-wdt", "snps,dw-wdt"`, clocks TCLK_WDT0 and PCLK_WDT0
  (linux:`rk3588-base.dtsi:2461-2467`). The gates are `CLKGATE_CON(15)`
  bits 1 and 0, and the counter clock is xin24m (`clk-rk3588.c:1050-1053`).
  Board: `CLKGATE_CON(15)` 0x00002ff8 (both open), `tclk_wdt0` 24000000 Hz.
- Registers (zephyr `drivers/watchdog/wdt_dw.h`, Apache-2.0): `WDT_CR` 0x00
  (`WDT_EN`, `RMOD`, `RPL`), `WDT_TORR` 0x04, `WDT_CCVR` 0x08, `WDT_CRR` 0x0c
  (0x76 restarts), `WDT_COMP_PARAM_1` 0xf4, `WDT_COMP_TYPE` 0xfc. Board:
  `COMP_TYPE` 0x44570120, `COMP_VERSION` 0x3131302a (1.10a), `COMP_PARAM_1`
  0x10000ac0: a 32-bit counter, fixed periods, `TOP` writable, not always
  enabled.
- The period is a power of two. `TOP` = i counts 2^(16+i) cycles of
  24 MHz: 11 is 5.6 s, 12 is 11.2 s, **13 is 22.4 s** (the default,
  `CONFIG_YAKOGL_NANOPC_T6_WDT_TOP`), 14 is 44.7 s, 15 is 89.5 s.
- Reset behaviour: with `RMOD` = 0 the first timeout resets (with 1 it would
  interrupt first and reset on the second). Once `WDT_EN` is set, only a reset
  of the block clears it; the RK3588 node has no `resets`, so Linux cannot
  stop it either (linux:`dw_wdt.c:285-297`).
- Under Armbian it is stopped: `/sys/class/watchdog/watchdog0` state
  "inactive", timeout 44 s, nobody has `/dev/watchdog` open, systemd's
  `RuntimeWatchdogSec` is off; `WDT_CR` 0x00000008, `WDT_TORR` 0x0000000e.
  So, unlike on the AM62P, no watchdog that Linux armed carries over: the
  image must arm it.
- What a WDT0 reset resets is in no source on this host: the RK3588 TRM
  (the CRU's global reset configuration, `GLB_RST_CON` 0xfd7c0c10) is not
  here (TRM not on this host: searched `~/org` and `~/com` for an RK3588 TRM,
  2026-10-19, none). So the bits are read values only, with no meaning
  assigned: board, under Armbian, `GLB_RST_CON` 0x0000ffdf, `GLB_RST_ST`
  (0xfd7c0c04) 0x00000002. A WDT0 timeout does bring the SoC back through
  the DDR loader, SPL and U-Boot into Armbian: measured twice on
  2026-10-19, below ("The watchdog proof and the trap"). After it Linux
  reads `GLB_RST_ST` 0x0 and watchdog0's bootstatus 0, as after a PSCI
  reset, so neither tells the two apart. `nanopc-t6.sh --go` records both
  registers before the jump and after the return.

The glue (`driver/zephyr/lib/board/nanopc_t6/nanopc_t6.c` over
`ip::dw_wdt`, `soc::rk3588`, `aarch64`):

- At EARLY it puts the GIC CPU interface back into EOImode 0. Armbian runs it
  in split mode (dmesg: "GICv3: GIC: Using split EOI/Deactivate mode").
- At PRE_KERNEL_1 it opens the two gates if they are closed and arms WDT0:
  `TORR` = top | top << 4, `CR` = RPL as found | `WDT_EN`, a restart, then
  `CR`, `TORR` and `CCVR` are read back. Seven refusals, one number each. A
  WDT0 that is already running (Linux's, under `--linux-wdt`) is only given
  the new `TOP` and restarted: every `CR` write keeps `WDT_EN` (host test
  `a_watchdog_linux_left_running_is_re_topped_and_restarted_never_stopped`).
- After the early console it prints the marker and the arming. If the arming
  refused, it resets through PSCI at once. If PSCI returns, it arms WDT0 once
  more with `TOP` 15 (89.5 s) and waits for it.
- A 1 s timer feeds WDT0 until `CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S` (60 s),
  and only while the sample's `t6_watchdog_feed()` calls come at most
  `..._ALIVE_S` (5 s) apart (the rule is `board::nanopc_t6::feed_verdict`
  in the drivers crate, with host tests at its edges). Once WDT0 is armed,
  the board resets at most 60 s + 22.4 s = 82.4 s after the arming,
  whatever the image does next.
- **The pre-arm window.** Before the arming, nothing of the image's own can
  reset the board: reset.S (EL2 to EL1), `z_prep_c` (BSS, data, the MMU
  tables), `z_cstart` up to PRE_KERNEL_1 (the EARLY hook that resets the
  GIC's EOI mode, `arch_kernel_init`, the SoC and board early hooks), and
  `t6_wdt_arm`'s own `device_map()` calls. `t6_wdt_arm` is the first
  PRE_KERNEL_1 hook (the gate row checks this in the ELF). This is
  straight-line code with no wait on a device, and its length is not
  measured. A fault in it halts the CPU for good: an exception, or the
  `k_panic()` of the arm64 MMU code when its `CONFIG_MAX_XLAT_TABLES` (8)
  tables run out, which `device_map()` can reach. With `--linux-wdt` the
  recipe covers it: `/dev/watchdog0` is opened in the jump's own ssh
  command, so WDT0 is already counting Linux's 44.7 s when the image
  starts, and nothing in Linux feeds or stops it across the kexec (linux
  v6.18 `watchdog_core.c:158-176`: the reboot notifier's `dw_wdt_stop`
  only marks it running, `dw_wdt.c:285-297`). This is the recipe's default
  since the first jump (below) stopped in this window; `--no-linux-wdt`
  turns it off, and then a hang here holds the board until its power is cut.
  The recipe reads `WDT_CR` back after the open and jumps only when
  `WDT_EN` is 1, and the record keeps that line. A hang in this window
  is reset by Linux's WDT0 back into Armbian: measured with the
  `VHE_EXIT=n` image, which halts here (68 s from the jump to Armbian).
- `t6_reset_to_linux()` ends a run with PSCI `SYSTEM_RESET` (0x84000009 over
  SMC; PSCI 1.1 on the board). Linux reboots the T6 the same way:
  psci_sys_reset has priority 129, above dw_wdt's 128
  (linux:`psci.c:327-330`, `dw_wdt.c:653`).
- `CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF` arms WDT0 and never feeds it, with a
  PSCI backstop 10 s after one period.

**The image.** `t6_hello` prints its build (`git describe`), the board and
the console, the watchdog's period and deadline, counts 10 s with `k_sleep`
(each second needs the timer interrupt, which a GIC left in split mode
delivers once), feeds the watchdog, prints PASS and resets:

```sh
# from the west workspace, by its real path (with a symlinked workspace the
# relative include in snippets-rodata.ld does not resolve)
west build -p always -b nanopc_t6 -d build/t6_hello yakogl/tools/probes/nanopc_t6/t6_hello
```

`zephyr.bin` is 57552 bytes, its RAM footprint 159744 bytes (the Image
header's `image_size`), at 0x10000000 (with the early banner, 2026-10-19). The marker it carries, which
`nanopc-t6.sh` requires:
`yakogl-wdt: nanopc_t6 WDT0 armed at boot; top 13; lifetime_s 60; alive_s 5; proof 0; load 0x10000000`.

**The kexec.** Armbian 26.8.10 has `CONFIG_KEXEC=y`, `CONFIG_KEXEC_FILE=y`,
`kexec_load_disabled` 0, and no kexec-tools. kexec-tools would not work
anyway: its arm64 loaders place the kernel at the first 2 MiB aligned hole
that fits (kexec-tools v2.0.31 `kexec-arm64.c:699-701`,
`kexec-image-arm64.c:70, 101-102`), and a Zephyr image runs only at its link
address. `driver/tools/kexec-zephyr/nanopc-t6.sh` uses the AM62P's
`kexec_zephyr` instead: one `kexec_load` segment at the link address, the
entry there, the MMU off. Linux enters it at EL2 (dmesg: "CPU: All CPU(s)
started at EL2"). Without `--go` the script checks the image (the Image
magic, the marker, the link address, the footprint) and the board (the
compatible, root, kexec allowed and nothing loaded, the window, the GIC
tables, the watchdog's Linux state, GPU users, remote sessions) and prints
the plan. On the board today every check passes and it prints the plan.

**The first jump (2026-10-19 00:26, `t6_hello` at 4a9b40c46, without
`--linux-wdt`).** The recipe read `GLB_RST_ST` 0x2, `GLB_RST_CON` 0xffdf,
WDT0 `CR` 0x8 (stopped), `TORR` 0xe and jumped; `kexec_zephyr` printed
"jumping to it". Then nothing: 0 bytes on the UART for 263 s, no ping, no
reset, until the user cut the power at about 00:35. The same capture
recorded U-Boot and Armbian's boot on that power cycle, so the UART path
works. Armbian's command line has `loglevel=1`, which is why not even
Linux's "Starting new kernel" reached the UART. Nothing of the image's own
printed, and WDT0 was never armed (it was stopped, and nothing had opened
`/dev/watchdog0`).

The candidates, read from the sources and ranked; each names what the next
jump shows (the early banner below):

1. **Linux's VHE state at EL2 (confirmed by the second jump, below).** Armbian runs its kernel at EL2
   with VHE (dmesg "CPU features: detected: Virtualization Host
   Extensions"), and a kexec from a VHE host enters the image at EL2 with
   `HCR_EL2` = RW | TGE | E2H | AMO | IMO | FMO (linux v6.18
   `kvm_arm.h:106`, `machine_kexec.c:123-126`, `relocate_kernel.S:84-99`).
   Zephyr's `z_arm64_el2_init` (reset.c) clears only TGE, IMO and AMO, so
   E2H stays 1; it then writes `CNTHCTL_EL2` in the E2H=0 layout, where
   under E2H=1 EL1's physical timer access is bits 10 and 11 (KVM:
   `arch_timer.c:829-846`). The first `msr cntp_cval_el0` in
   `z_arm64_el1_init` (in the image's ELF: right before the call of
   `z_arm64_el1_plat_init`) traps to EL2, whose vector base is still
   Linux's (a kernel virtual address, MMU off): the CPU stops with nothing
   printed. All before `z_prep_c`. Linux itself survives the same entry
   because its `init_el2` writes a whole `HCR_EL2` (head.S:302); U-Boot
   leaves E2H clear, which is why roc_rk3588_pc (`go`) never meets it. The
   fix is Linux's: the image's first code writes `HCR_EL2` = RW only
   (`CONFIG_YAKOGL_NANOPC_T6_VHE_EXIT`, default y; `aarch64.rs` has the
   whole account). Next jump: `T6:A` shows `HCR_EL2` found with E2H (bit 34)
   set, expected 0x0000000488000038, `-> 0x0000000080000000`, and, with the
   fix, `T6:A1` follows. With `VHE_EXIT=n` (the A/B arm)
   `T6:A` and then `T6:X` with the trap's syndrome are expected instead.
2. **Something else before the UART driver** (`z_prep_c`: BSS, the MMU
   tables and their `CONFIG_MAX_XLAT_TABLES`; the GIC EOI reset at EARLY;
   the `device_map()` calls): `T6:A1` present and `T6:A2` absent. The GIC
   reset (candidate (b) of the brief) needs `ICC_SRE_EL1.SRE`, which the
   Cortex-A55's CPU interface, having no legacy mode, holds at 1; with FMO
   left at 1 its write would go to the virtual interface (one more reason
   for the fix), not fault.
3. **The UART driver path** (uart2's 24 MHz, `reg-shift` 2 with 32-bit
   access as Linux's rk3588-base.dtsi, the divisor (24 MHz + 12 MHz) /
   24 MHz = 1, which ns16550 can write): `T6:A3` present, `T6:B` absent.
   The raw lines use the state Linux left (1.5 Mbaud, `DLAB` 0, checked
   before each line), so they do not depend on it.
4. **WDT0** (never armed because 1 happened first; whether its reset is
   routed to the whole SoC is unknown): `T6:A3`/`T6:C` show the arming;
   `GLB_RST_ST` after a WDT0 reset says what reset the SoC.

**Z1 PASS: the second jump (2026-10-19 00:56:31, `t6_hello` at 576065151,
zephyr.bin sha256 f7db69a2…, Linux's watchdog open; record
`0dce654a8:tools/mali/board-ref/2026-10-19-z1/20261019-005631-t6/`).** The board read `WDT0 CR
0x00000009 TORR 0x0000000e after the open: WDT_EN 1` and jumped. On the
UART: `T6:A EL2 HCR_EL2 0x000000048800003a -> 0x0000000080000002`
(E2H, TGE, AMO, IMO, FMO set as found, plus bit 1, which reads back as 1
after the write of RW only), `SCTLR_EL2 0x30400800`, `VBAR_EL2
0xffffcd30f6810800` (Linux's, a kernel virtual address) `-> 0x10001000`;
then `T6:A1`, `T6:A2`, `T6:A3 WDT0 arm rc 0 CR 0x00000009`, `T6:B`, the
marker, "ICC_CTLR_EL1 0x8402 had EOImode 1 ... now 0", "WDT0 armed: CR
0x00000009 -> 0x00000009 (was running), TORR 0x0000000e -> 0x0000000d",
`T6:C`, ten count lines 1000 ms apart, "t6_hello: PASS: counted 10 s in
10004 ms", "PSCI SYSTEM_RESET at 10004 ms (10 feeds)", then the DDR
loader. Armbian was back 45 s after the jump. That was PSCI, not a
watchdog: the reset came at 10 s, before either WDT0 period could expire.
After the return, `GLB_RST_ST` was 0x0, as before the jump, and
watchdog0's bootstatus was 0. No `T6:X`.

This confirms cause 1 below. E2H read set, as predicted. In the boot path
the two images differ only by the `HCR_EL2` write, the banner, and the
`VBAR_EL2` reporter, which acts only on a trap. The recipe differs by
Linux's watchdog, which takes no part before the image's output. The
first jump's
record is `0dce654a8:tools/mali/board-ref/2026-10-19-z1/20261019-002640-t6/` (0-byte
`uart.log`), next to the power-on capture `uart-powercycle.log`.

**The watchdog proof and the trap (2026-10-19 03:01 and 03:03, both
`t6_hello` at 37b4c0ff2 with Linux's watchdog open; records
`0dce654a8:tools/mali/board-ref/2026-10-19-z1-gpu/`).**

- The proof image (`WDT_PROOF=y`): every tag to `T6:C`, `WDT0 armed ...
  TORR 0x0000000e -> 0x0000000d ... never fed (proof)`, the ten counts,
  `proof: waiting for WDT0`, then the DDR loader. The backstop's `t6: proof
  FAILED` line, printed before its PSCI reset at 32.4 s, is absent, so WDT0
  reset the SoC. Back in Armbian 52 s after the jump. `GLB_RST_ST` 0x0,
  bootstatus 0 after.
- The `VHE_EXIT=n` image: `T6:A EL2 HCR_EL2 0x000000048800003a kept (VHE
  exit off)`, then `T6:X ... ESR_EL2 0x000000006234f804 ELR_EL2
  0x00000000100029bc ... HCR_EL2 0x000000048000000a; halted`. The ESR is
  EC 0x18 (trapped system register access), Op0 3, Op1 3, CRn 14, CRm 2,
  Op2 2, a write: `MSR CNTP_CVAL_EL0`, and the ELR is that instruction in
  this image's `z_arm64_el1_init`, right before `bl z_arm64_el1_plat_init`.
  That is cause 1 exactly as predicted, with the trap printed this time.
  Zephyr had cleared TGE, IMO and AMO and kept E2H and FMO. The CPU
  halted before the image armed WDT0; Linux's WDT0 (TOP 14, 44.7 s from the
  open) reset it, back in Armbian 68 s after the jump.

**Where an image dies: the early banner.** Raw lines on uart2, written by
direct register access before anything else can fail
(`board::nanopc_t6_early` over `ip::ns16550_tx`: read `LCR` once and write
nothing if `DLAB` is set; per byte poll `LSR.THRE` at most 100000 times,
then one 32-bit store to `THR`; nothing else is touched). A missing tag
says where the image stopped:

| Tag | Where | Printed by |
|---|---|---|
| `T6:A` | the image's first code: reset.S's weak `z_arm64_el2_plat_prep_c` (and the EL1 one), MMU off, before any C code | `yd_t6_early_entry`: the EL; at EL2 `HCR_EL2` found and written, `SCTLR_EL2`, `VBAR_EL2` found and set |
| `T6:A1` | the end of `z_arm64_el1_init` (`z_arm64_el1_plat_init`, weak), MMU off | the glue over the crate |
| `T6:A2` | the first PRE_KERNEL_1 hook, MMU on, before the WDT0 arming | through a `device_map()` of uart2 |
| `T6:A3` | right after the arming, with rc, `CR` and the period | the same |
| `T6:B` | the first console line (ns16550 driver) | `printk` |
| `T6:C` | WDT0 armed, before "board: watchdog armed" | `printk` |
| `T6:X` | any exception taken to EL2 after `T6:A` | the EL2 vector table `T6:A` installs: `ESR_EL2`, `ELR_EL2`, `FAR_EL2`, `SPSR_EL2`, `HCR_EL2`, then halt |

The hooks are reset.S's, the earliest a board can own in the pinned Zephyr
(af17c0c8 `arch/arm64/core/reset.S`: `WTEXT(z_arm64_el2_plat_prep_c)`, called
from `__reset_prep_c` "Stack is not yet available"; the entry takes the
stack `__start` already loaded into `x24`). `check_early_hooks.cmake`
fails every nanopc_t6 build in which Zephyr's weak ones win or the vector
table is not 2 KiB aligned. The register sequence of each line is pinned by
host tests against a UART model.

**Unknown until the first run with the UART:**

- The UART path to the host (2026-10-19): an FT232R on the debug header,
  `/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_<serial>-if00-port0` on the
  build host, 1500000 8N1. Both directions checked 2026-10-19: the board's
  output arrived, and bytes from the host raised UART2's rx count. uart2 is
  the debug header (Armbian's console, `console=ttyS2,1500000`, with an
  autologin root getty on it). **Anything written to that tty runs on the
  board as root**, so the host only reads it: `nanopc-t6.sh` sets it raw
  with no echo and reads it with `cat`, and never writes.
- Zephyr's console on uart2 as configured: **works** (second jump, `T6:B`
  onward).
- Whether a WDT0 timeout resets the whole SoC back through U-Boot into
  Armbian: **it does** (the proof image, and Linux's WDT0 after the trap).
  `GLB_RST_ST` reads 0x0 after it, as after a cold power-on and after the
  PSCI reset, so it is no reset-cause record as Linux sees it.
- PSCI `SYSTEM_RESET` from our EL1: **returns the board** to Armbian, via
  the DDR loader, SPL and U-Boot (second jump, 45 s).
- GPU power at hand-over: measured in the second half (below, "The GPU
  power-on").

**The first board procedure.** The UART (`T6_UART`, above) is captured,
read only, into the run's record (`uart.log`) from before the jump until
the board is back in Armbian; `--go` refuses when the tty is absent or
already open by another reader. Someone who can power-cycle the
T6 is named first (`T6_HAND`: `--go` refuses without it, and the record
carries the name). Their switch is the only way out in these cases: a hang
in the pre-arm window without `--linux-wdt`; an arming that refused twice
(the second time with `TOP` 15) while PSCI `SYSTEM_RESET` returned. A
WDT0 reset brings the SoC back (step 3, measured 2026-10-19).

1. Ask who uses the board. Run `nanopc-t6.sh <t6_hello zephyr.bin>` (the dry
   run) and read the plan. Read `GLB_RST_ST` (0xfd7c0c04) under Armbian.
2. `T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=/dev/serial/by-id/usb-FTDI_FT232R_USB_UART_<serial>-if00-port0 nanopc-t6.sh <t6_hello zephyr.bin> --go`
   (Linux's watchdog is opened by default; the record must hold "linux-wdt:
   WDT_EN proof"). Expected on the UART: `T6:A` .. `T6:A3`, `T6:B`, the
   marker; "t6: GICv3 CPU interface: ICC_CTLR_EL1 ... had EOImode 1";
   "t6: WDT0 armed: CR 0x00000008 -> 0x00000009 (was stopped), TORR
   0x0000000e -> 0x000000dd ... period 22369 ms" (with Linux's watchdog
   open, "was running" and Linux's values as found: expected `CR` 0x9 and
   `TORR` 0xee, TOP 14); `T6:C`; "board: watchdog armed";
   the build line; ten count lines about 1000 ms apart; "t6_hello: PASS";
   "t6: t6_hello done: PSCI SYSTEM_RESET"; then U-Boot. `nanopc-t6.sh`
   prints "back in Armbian N s after the jump". Read `GLB_RST_ST` again.
3. The proof image (`-DCONFIG_YAKOGL_NANOPC_T6_WDT_PROOF=y`) with `--go`:
   after the arming line nothing feeds, and U-Boot must appear about 22.4 s
   after it. "t6: proof FAILED" at about 32.4 s means WDT0 did not reset the
   SoC and the backstop did: record it; WDT0 is then not the image's way back
   until the reset configuration is understood. Read `GLB_RST_ST`.
4. Only then the second half of Z1: GPU power-on and `GPU_ID`.

### The GPU power-on (Z1's second half)

`tools/probes/nanopc_t6/t6_gpu` calls `yd_t6_gpu_up`
(`board::nanopc_t6_gpu` over `soc::rk3588`, `ip::rockchip_spi`,
`ip::rk806` and `gpu::mali` in driver/rust, each with host tests over a
mock of its registers; a composed test runs the whole sequence over the
board's values). The order:

1. Read what Linux handed over, writing nothing: PD_GPU in the PMU, the GPU
   clock in the CRU, SPI2's state, then the RK806 over SPI2 (read frames
   only: `ip::rk806` cannot build a write, and a test checks it). The image
   prints all of it before its first PMU or GPU write.
2. Stop when vdd_gpu_s0 (RK806 DCDC1) is off, or below the GPU OPP table's
   voltage for the rate the CRU gives the GPU (rk3588-opp.dtsi:118-153),
   or when the CRU's rate is unknown (the PVTPLL selected).
3. PD_GPU on in Linux's order (pm-domains.c): the four GPU clock gates,
   `PWR_GATE` bit 0 cleared, wait for `REPAIR_STATUS` bit 1, the bus idle
   request released, wait for its acknowledge and status. The SIP call that
   tells the firmware about the domain is left out (it configures system
   suspend only). The QoS words are read, not written.
4. `GPU_ID`; soft reset; `GPU_COHERENCY_PROTOCOL` = none; L2 on
   (panthor_gpu.c, under its MIT option). The L2 wait needs `L2_READY` to
   show the L2: panthor's condition (`(mask & val) == val`,
   panthor_gpu.c:210) also accepts 0.

**Measured, 2026-10-19 (records `0dce654a8:tools/mali/board-ref/2026-10-19-z1-gpu/`,
three jumps; the recipe now records the GPU's power under Linux before
the jump and after the return).**

- **PD_GPU at hand-over: off.** Under Armbian panthor runtime-suspends an
  idle GPU, and genpd turns the domain off: `PWR_GATE` 0x0000ffff (bit 0
  set), `REPAIR_STATUS` 0xfad50001 (bit 1 clear), bus idle requested and
  acknowledged (`BUS_IDLE_REQ` 0x83cf, `_ACK`/`_ST` 0x000a9fff). With the GPU
  forced active (`power/control` on) exactly those bits move (0xfffe,
  0xfad50003, 0x83ce, 0x000a9ffe). The image reads the GPU bits as Linux
  left them; other domains' bits differ (`BUS_IDLE_REQ` 0xe3cf,
  `REPAIR_STATUS` 0xfad40001): Linux's shutdown before the kexec idles
  more. The recipe refuses to jump while a process holds the GPU, so a
  jump always starts from the suspended state.
- **vdd_gpu_s0 at hand-over: on, 675 mV.** RK806 `POWER_EN0` 0x0f (DCDC1-4
  on), `POWER_SLP_EN0` 0xe0, `BUCK1_CONFIG` 0xa4, `BUCK1_ON_VSEL` 0x1c
  (500 mV + 28 · 6.25 mV), the same under Linux (regmap debugfs; Linux keeps
  the supply enabled through panthor's `mali-supply` even with the domain
  off: `regulator_summary` use count 1) and from the image. Nothing in
  Linux's kexec path turns it off (rk8xx_shutdown does nothing for the
  RK806, rk8xx-core.c:667-693).
- **The RK806 read.** Read as Linux reads it (a 3-byte command, then a
  receive-only phase, one chip select; `spi_write_then_read`), the image
  gets Linux's values. The same command and one more frame in a single
  full-duplex transfer gives 0x00 for every register: the first `t6_gpu`
  jump took that for "supply off" and stopped, before any write. SPI2 as
  the image finds it is as Linux leaves it (`CTRLR0` 0x2c01, `BAUDR` 0xc6,
  `VERSION` 0x00110002, gates open).
- **The GPU clock: the CRU's, never SCMI.** `CLKSEL_CON(158)` 0x00001f24:
  `clk_gpu_src` from CPLL (1500 MHz: `PLL_CON` 0xfa, 0x42) over 5, 300 MHz;
  bit 14 (the PVTPLL) clear; the stacks divider 0. That is devfreq's last
  rate under Linux, which sets the CRU's `clk_gpu` directly. SCMI clock 5
  is only `assigned-clock-rates` at boot (panthor's probe prints 198 MHz:
  GPLL 1188 MHz / 6). The T6's BL31 is Rockchip's closed binary (v1.48,
  "v2.3-868-g040d2de11" on the UART), so what clock 5 does is not readable;
  upstream TF-A's rk3588 SCMI clock writes the same `CLKSEL_CON(158)` and
  switches it to the PVTPLL for 300-1000 MHz. So the image relies on the
  CRU as it reads it, rates it itself, and refuses the PVTPLL.
- **Power-on to L2.** PD_GPU came on (`REPAIR_STATUS` 0xfad40003, bus out of
  idle) in a few µs. `GPU_ID` 0xa8670005, `L2_FEATURES` 0x07120306, `TILER`
  0x809, `MEM` 0x301, `MMU` 0x2830, `AS_PRESENT` 0xff, `SHADER_PRESENT`
  0x50005, `CSF_ID` 0x040a0412: panthor's probe line, value for value.
  `GPU_INT_RAWSTAT` read 0x100 before the clear (the power-on reset), 0
  after it, and `RESET_COMPLETED` again one 1 µs poll after the soft reset
  command. `L2_READY` read 0 before `L2_PWRON` and 1 at the first poll
  after it.
- **QoS after the image's power-on:** `PRIORITY` 0x80000000, `MODE` 3,
  `BANDWIDTH` 0x12d, `SATURATION` 0x40, `EXTCONTROL` 0 in all four blocks.
  Linux, GPU active, reads 0x80000000, 0, 0, 0, 0 (it restores what it
  saved); read again on 2026-10-26 under Armbian with `power/control` on
  (restored to `auto`), a `/dev/mem` read of `0xfdf35000 + {0, 0x200, 0x400, 0x600}
  + 0x08..0x18` (`regread.py qos`, record
  `e3d9b7470:tools/mali/board-ref/2026-10-26-open-questions/regread.out`): `PRIORITY`
  0x80000000, `MODE` 0, `BANDWIDTH` 0, `SATURATION` 0, `EXTCONTROL` 0 in all four
  blocks. The image leaves them (under "Open").

### Z2: AS0 and a cache flush

Written on the host 2026-10-26, before the code; **not run on the board**
(a kexec reboots the board's Armbian; the owner schedules it, procedure at
the end). What Z2 shows: our kernel side builds the MCU address space's
page tables in the image, enables AS0 on them the way panthor enables the
MCU's VM, and gets a GPU cache flush back with its completion bit. It is
the step before Z3, whose firmware is loaded into exactly these tables.

Sources. Linux v6.18 `drivers/gpu/drm/panthor/` under its MIT option
(decision 1): `panthor_regs.h` (the `MMU_*`, `AS_*`, `GPU_CMD` and
`GPU_INT_*` definitions), `panthor_mmu.c` (`panthor_vm_create`,
`panthor_vm_active`, `mair_to_memattr`, `lock_region`, `wait_ready`,
`write_cmd`, `mmu_hw_do_operation_locked`, `panthor_mmu_as_enable`,
`flags_to_prot`, `panthor_vm_map_pages`), `panthor_gpu.c`
(`panthor_gpu_flush_caches`, `panthor_gpu_irq_handler`). io-pgtable-arm.c
is GPL only: facts with their function, never its code
(`arm_lpae_prot_to_pte`, `arm_lpae_install_table`, its stage-1 `MAIR`, the
PTE sync of a non-coherent walk). The Arm ARM's VMSAv8-64 descriptor fields
are the architecture. **These citations name the function or macro, not a
line:** the hosts that wrote this subsection had no Linux tree and could not
fetch one, so the reading is from these functions as named; where this page
already had line numbers (`panthor_mmu.c:506-621, 785-790, 1990-2030`)
they are the earlier reading's. A line-numbered pass against v6.18 is a
review item before the board run.

**Where the tables live.** In the image, at no fixed address: two arrays in
the `t6_mmu` probe's `.bss`, inside the Z1 memory map's "ours" window.

| Range (Z1's map) | What | Size |
|---|---|---|
| 0x10000000-0x10ffffff | the Zephyr image, as in Z1 | 16 MiB |
| ... `.bss`, `t6_mmu_pool` | AS0's table pages, 4 KiB aligned; Z2 uses 4, Z3's sections need 9 | 16 pages, 64 KiB |
| ... `.bss`, `t6_mmu_backing` | what the MCU's shared-interface section (VA 0x04000000) maps to | 64 KiB |

Why there:
- 0x10000000-0x10ffffff is the only range of the Z1 map that is ours after
  the kexec. The Image header's `image_size` covers `.bss`, so
  `nanopc-t6.sh` loads the segment over all of it, and nothing else needs
  reserving.
- Zephyr maps the kernel image as normal cacheable memory at its link
  address (`zephyr,sram`), so `k_mem_phys_addr()` gives the physical
  address the descriptors and `AS_TRANSTAB` need. The image prints it.
- Not a fixed window outside the image, and not an uncached mapping: either
  is a second mapping of the same RAM with other attributes, which the Arm
  ARM calls mismatched. Linux keeps cacheable tables and cleans them for a
  non-coherent walker (io-pgtable's PTE sync when `coherent_walk` is false;
  panthor sets `coherent_walk` from `ptdev->coherent`, false on the T6).
  The image does the same: after building, `sys_cache_data_flush_range`
  over the used pages (Zephyr's "flush" is a clean to the point of
  coherency, `dc cvac` on arm64). Then the enable's own whole-VA flush
  cleans and invalidates the GPU's L2 before `UPDATE`, so the GPU cannot hold
  an old line of those pages.
- The code that takes the pool (`gpu::mali::pgtable::Tables`) allocates
  nothing: it is given the pages and their physical address, as the GC7000's
  MMU is given its pool.

**The format.** panthor gives a VM's tables to io-pgtable as
`ARM_64_LPAE_S1` (`panthor_vm_create`: 4 KiB and 2 MiB page sizes, `ias`
the GPU's VA bits, `oas` its PA bits). That is the Arm VMSAv8-64 stage-1
format. It is **not** `ARM_MALI_LPAE`, panfrost's format for the older
job-manager GPUs; the CSF GPUs select the AArch64 mode in `AS_TRANSCFG`
(`ADRMODE_AARCH64_4K`, `panthor_vm_active`). `MMU_FEATURES` 0x2830 gives
48 VA bits and 40 PA bits (Z1's read, panthor's probe line), so: four
levels (VA bits 47:39, 38:30, 29:21, 20:12), each table one 4 KiB page of
512 descriptors, the root's PA is `AS_TRANSTAB` as it is (io-pgtable's
`ttbr`). Z2 builds level-3 pages only, no 2 MiB blocks.

| Descriptor | Bits | Source |
|---|---|---|
| table (levels 0-2) | next table's PA (47:12) \| 0b11 | `arm_lpae_install_table` (no `NSTable`: only with a quirk panthor does not set) |
| page (level 3) | PA (47:12) \| 0b11 | `arm_lpae_init_pte` |
| `AttrIndx` 4:2 | 1 (write-back) for a cached mapping, 0 (non-cacheable) otherwise | `arm_lpae_prot_to_pte`, `IOMMU_CACHE` |
| `AP[1]` bit 6 | always (unprivileged; panthor never asks `IOMMU_PRIV`) | `arm_lpae_prot_to_pte`, `flags_to_prot` |
| `AP[2]` bit 7 | read only (`MAP_READONLY`) | the same |
| `SH` 9:8 | inner shareable (3) cached, outer (2) otherwise | `arm_lpae_prot_to_pte` |
| `AF` bit 10, `nG` bit 11 | always (stage 1) | the same |
| `PXN`/`UXN` 54:53 | both, unless executable (`MAP_NOEXEC` clear) | the same |

The section VA 0x04000000, 64 KiB, writable, cached, not executable (as
panthor maps a section without the execute flag): its page `n` is
`backing + n·0x1000 | 0x0060000000000f47`.

**The registers** (panthor_regs.h; AS n at 0x2400 + 0x40·n, AS0 the
MCU's: `panthor_vm_active` always gives the MCU VM slot 0):

| Register | Offset | Z2 writes |
|---|---|---|
| `AS_TRANSTAB` | +0x00 (64) | the pool's PA |
| `AS_MEMATTR` | +0x08 (64) | 0x8f8f8f8f8c4c8f4c (panthor's live word) |
| `AS_LOCKADDR` | +0x10 (64) | 0x3f (all of VA), then 0x000000000400000f |
| `AS_COMMAND` | +0x18 | `LOCK` 2, `UNLOCK` 3, `UPDATE` 1 (`FLUSH_PT` 4, `FLUSH_MEM` 5 are not written: they name the operation only) |
| `AS_FAULTSTATUS`, `AS_FAULTADDRESS` | +0x1c, +0x20 | read |
| `AS_STATUS` | +0x28 | read; bit 0 `AS_ACTIVE` |
| `AS_TRANSCFG` | +0x30 (64) | 0x00000000420001c6 |
| `MMU_INT_RAWSTAT`, `_CLEAR`, `_MASK` | 0x2000, 0x2004, 0x2008 | read (bit n a fault of AS n, bit 16+n its bus error) |
| `GPU_CMD` | 0x30 | 0x3304, 0x0304 |
| `GPU_INT_RAWSTAT`, `_CLEAR` | 0x20, 0x24 | bit 17 `CLEAN_CACHES_COMPLETED` |

- `AS_TRANSCFG` (`panthor_vm_active`): `PTW_MEMATTR_WB` (2 << 24) |
  `PTW_RA` (1 << 30) | `ADRMODE_AARCH64_4K` (6) | `INA_BITS(55 - 48)`
  (7 << 6) = 0x420001c6, the live value; `PTW_SH_OS` only when coherent.
- `AS_MEMATTR` (`mair_to_memattr` over io-pgtable's stage-1 `MAIR`
  0xf404ff44: index 0 normal non-cacheable 0x44, 1 write-back 0xff, 2
  device 0x04, 3 inner non-cacheable outer write-back 0xf4): a byte is
  write-back only when inner and outer both are, else non-cacheable
  (0x4c: `INNER_OUTER_NC` | `INNER_ALLOC_EXPL(0, 0)`); write-back takes the
  inner allocation hints and, non-coherent, `SH_MIDGARD_INNER`: 0xff gives
  0x8f, 0xf4 gives 0x4c (its inner is non-cacheable), 0x04 and the unused
  bytes 0x4c. So 0x4c4c4c4c8c4c8f4c derived; the board's live word
  ("MMU (Z2)" above) is 0x8f8f8f8f8c4c8f4c, the same in bytes 0-3, the
  ones a PTE selects, and 0x8f in bytes 4-7, which none does. The image
  writes the live word (`MEMATTR_LIVE`; a host test holds its bytes 0-3 to
  `mair_to_memattr`); the page's `AttrIndx` 1 is the 0x8f byte.
- `AS_LOCKADDR` (`lock_region`): the naturally aligned power-of-two block
  holding the range, at least `AS_LOCK_REGION_MIN_SIZE` (32 KiB), as its
  base with log2(size) - 1 in the low bits. All of VA: 63; the section's
  64 KiB: 0x0400000f.
- `GPU_FLUSH_CACHES(l2, lsc, other)` = `GPU_CMD_DEF(4, l2 | lsc << 4 |
  other << 8)`, `CACHE_CLEAN` 1, `CACHE_INV` 2: L2 and LSC cleaned and
  invalidated 0x3304 (`FLUSH_MEM`), the L2 only 0x0304 (`FLUSH_PT`).

**The sequences.**

1. The round trip alone (`panthor_gpu_flush_caches`): clear bit 17, read it
   clear (else refuse: a completion could not be told from an old one),
   write `GPU_CMD` 0x3304, poll `GPU_INT_RAWSTAT` bit 17 for at most 100 ms
   (panthor's `wait_event_timeout`), clear it. panthor waits for the IRQ and
   its handler clears the bit; Zephyr has no handler on SPI 94 here, so the
   image polls the raw status, as Z1's soft reset does.
2. AS0 on (`panthor_mmu_as_enable`): `mmu_hw_do_operation_locked` over the
   whole VA with `FLUSH_MEM`: `AS_LOCKADDR` = 63, `wait_ready`, `LOCK`,
   `wait_ready`, the flush (0x3304, bit 17), `UNLOCK` (a `GPU_CMD` flush does
   not unlock the region, panthor's comment there), `wait_ready`; then
   `AS_TRANSTAB`, `AS_MEMATTR`, `AS_TRANSCFG` (each 64-bit, low word first),
   `wait_ready`, `UPDATE`. `wait_ready` polls `AS_STATUS` bit 0 every 10 µs
   for at most 100 ms (`gpu_read_relaxed_poll_timeout_atomic(..., 10,
   100000)`).
3. The flush after a map (`panthor_vm_map_pages` ends with the range's
   `FLUSH_PT`): `AS_LOCKADDR` = 0x0400000f, `LOCK`, the L2-only flush
   0x0304, `UNLOCK`.

One difference from panthor, on purpose: after each `AS_COMMAND` the image
waits for `AS_ACTIVE` to clear (panthor only waits before the next one), so
each command's completion is a measured count. AS0's registers are read
back after `UPDATE`; the run passes when they equal what was written,
`AS_STATUS`, `AS_FAULTSTATUS` and `MMU_INT_RAWSTAT` read 0, and the three
flushes came back. Interrupt masks are read, never written.

**What Z2 does not show.** That the GPU walks these tables right: nothing
reads through AS0 before the MCU runs, and the MMU has no translate
command. A CPU walk of VA 0x04001234 over the same pages (after the build,
following each descriptor's PA back into the pool) is printed instead; the
GPU's first walk is the MCU's first fetch at Z3, and a wrong table shows
there as `MMU_INT_RAWSTAT` bit 0 with `AS_FAULTSTATUS`/`AS_FAULTADDRESS`.
Nor the MMU's IRQ line (SPI 93).

**The code.** Following "Where the code goes" and Z1's split:
`driver/rust/src/gpu/mali/pgtable.rs` (the builder and the walk),
`gpu/mali/mmu.rs` (the AS registers and sequences, `mair_to_memattr`,
`lock_region`), `gpu::mali::flush_caches` in `gpu/mali/mod.rs`,
`board/nanopc_t6_mmu.rs` (the Z2 run over them),
`ffi/rk3588.rs` `yd_t6_mmu_up` with `struct yd_t6_mmu_up` in
`include/yakogl_drivers.h`, and the probe `tools/probes/nanopc_t6/t6_mmu`
beside `t6_gpu` (Z1 split `t6_hello` and `t6_gpu` the same way; a probe of
its own keeps `t6_gpu`'s record reproducible). `t6_mmu` runs Z1's
`yd_t6_gpu_up`, prints its outcome in one line, then Z2. Host tests: the
descriptors, the indices, the section mapped and walked back, shared
tables, refusals (pool full, unaligned, out of range, remap, a foreign
table); `AS_TRANSCFG`, `AS_MEMATTR` and the lock regions; the enable's
register writes in panthor's order against a register model that refuses an
`AS_COMMAND` while `AS_ACTIVE` is set and a flush while one is pending; a
stuck `AS_ACTIVE`; the whole run, a failed clean, a register that does not
keep its value, a fault after the update; the C header's numbers and layout.
The gate row is `zephyr  nanopc_t6 t6_mmu image (Z2, AS0 and a cache
flush)`, built as the `t6_gpu` row is.

```sh
west build -p always -b nanopc_t6 -d build/t6_mmu yakogl/tools/probes/nanopc_t6/t6_mmu
```

**The board procedure (not run).** As Z1's ("The first board
procedure"), from a detached checkout of main with the image built from it:
someone who can power-cycle the T6 named, the UART captured read only from
before the jump, Linux's watchdog open by default.

1. Ask who uses the board; the dry run `nanopc-t6.sh <t6_mmu zephyr.bin>`
   and its plan. Note the PAs the ELF gives: `nm zephyr.elf | grep
   't6_mmu_\(pool\|backing\)'` (the image is identity mapped, so the UART's
   PAs must equal these).
2. `T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=$BOARD_UART nanopc-t6.sh
   <t6_mmu zephyr.bin> --go`.

Predicted on uart2, written before any run (`P` is `t6_mmu_pool`'s address,
`B` is `t6_mmu_backing`'s, from step 1):

- the Z1 banner and marker as for `t6_gpu` (`T6:A` .. `T6:C`), then
  `t6_mmu: build <describe>`, `t6_mmu: WDT0 period 22369 ms, fed until
  60000 ms of uptime`;
- `t6_mmu: Z1: rc 0 stop 0 refusal 0: GPU_ID 0xa8670005 MMU_FEATURES
  0x00002830 AS_PRESENT 0x000000ff L2_READY 0x1`;
- `t6_mmu: pool ... at PA P (16 pages), backing ... at PA B`;
- `AS0 as found`: `STATUS` 0, `FAULTSTATUS` 0, `TRANSTAB`, `MEMATTR`,
  `LOCKADDR` 0 after Z1's soft reset; `TRANSCFG` as found is not predicted
  (recorded). `IRQ as found`: `MMU_INT_RAWSTAT` 0; `GPU_INT_RAWSTAT` bit 17
  clear (bits 9/10, `POWER_CHANGED`, may be set by Z1's L2 power-on); the
  masks as the reset left them (expected 0, recorded, not asserted);
- `flush round trip: GPU_CMD 0x00003304`, bit 17 clear after the clear,
  then set within the 100 ms (Z1's soft reset completed in one 1 µs poll);
- `tables at PA P: 4 of 16 pages used`;
- `walk VA 0x0000000004001234: L0 P+0x1003 L1 P+0x2003 L2 P+0x3003 L3
  (B+0x1000) | 0x0060000000000f47 (4 levels): PA B+0x1234`;
- `tables cleaned to the point of coherency: rc 0`;
- `AS0 enable, the whole-VA flush: LOCKADDR 0x000000000000003f`, the flush
  `GPU_CMD 0x00003304`; `TRANSTAB P MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG
  0x00000000420001c6 written`;
- `AS0 after`: the same three values read back, `LOCKADDR` 0x3f,
  `STATUS` 0, `FAULTSTATUS` 0, `FAULTADDRESS` 0; `IRQ after`:
  `MMU_INT_RAWSTAT` 0;
- `the flush after the map: LOCKADDR 0x000000000400000f`, `GPU_CMD
  0x00000304`;
- `t6_mmu: PASS: AS0 on TRANSTAB P, three cache flush round trips, N ms`,
  then `t6: t6_mmu done: PSCI SYSTEM_RESET`, the DDR loader, U-Boot;
  `nanopc-t6.sh` prints "back in Armbian N s after the jump" (45 s for
  `t6_hello`).

A refusal prints `t6_mmu: FAIL: stop <n> (<step>), refusal <n>, detail
<value>` and resets the same way; the stop and refusal numbers are
`YD_T6_MMU_STOP_*` and `YD_MALI_*` in `yakogl_drivers.h`.

**The bounded way back.** Every wait of the run is bounded: Z1's (100 ms
reset, 20 ms L2, the PMU and SPI budgets), three flushes of at most 100 ms
and ten `AS_STATUS` waits of at most 100 ms each, so Z2 adds at most
about 1.3 s, under the 5 s the feeder allows between `t6_watchdog_feed()`
calls (one before and one after each of `yd_t6_gpu_up` and
`yd_t6_mmu_up`). A hang anywhere after the arming is reset by WDT0 at
most 82.4 s after it (Z1: 60 s lifetime + 22.4 s period); before it, by
Linux's watchdog (44.7 s from the open). If PSCI `SYSTEM_RESET` returns,
WDT0 resets the board within one period. A stuck GPU register is a
bounded refusal, not a hang: a read of the GPU with PD_GPU off would be
an external abort, but Z2 runs only after Z1 returned 0 (PD_GPU on, L2
ready).

## Decisions

Decided by the user on 2026-10-18:

1. **panthor is used under its MIT option.** All 20 files of
   `drivers/gpu/drm/panthor/` in Linux v6.18 carry `GPL-2.0 or MIT` (read in the
   tree, not from a summary), and `panthor_drm.h` is MIT. So its firmware interface
   and scheduler sequences may be transcribed with attribution, as galcore's dual
   MIT / GPL-2.0-or-later source is used under MIT for the GC7000
   (`docs/source-form.md`). The RK3588 power-domain, clock, page-table and RK806
   drivers are GPL only: facts and file:line only.
2. **Linux first.** The back end and the compiler are brought up against panthor's
   uAPI on Armbian (L1-L3), then the submit path moves to our own kernel side on
   Zephyr (Z1-Z4). This separates back-end defects from driver defects: when Z4
   differs from L1, the back end is already known good on the same bytes.

## Open

Measured under Armbian over ssh on 2026-10-26 and moved to their sections: the live
`AS_MEMATTR` ("MMU (Z2)"), `TIMESTAMP_OFFSET` and the runtime CS/CSG values ("The
firmware (Z3)"), Mesa 25.0.7 against 26.2.1 ("The command stream and descriptors
(L1)"), the Linux QoS words ("The GPU power-on"). What is left needs U-Boot or a job
from Zephyr; a UART / U-Boot session reads exactly this (issue #1281 names them as
Z-prerequisites), each with the board in U-Boot's prompt, before `bootm` / `booti`,
and nothing written:

- **PD_GPU and vdd_gpu_s0 at the U-Boot hand-over** (the supply is boot-on but not
  always-on). After Linux's kexec: PD_GPU off, vdd_gpu_s0 on at 675 mV, the clock
  300 MHz from CPLL (measured; "The GPU power-on"). Read in U-Boot, `md.l` each:
  the PMU (0xfd8d8000) `PWR_GATE`,
  `REPAIR_STATUS`, `BUS_IDLE_REQ`, `BUS_IDLE_ACK`, `BUS_IDLE_ST` (the registers
  and the Linux values are in `0dce654a8:tools/mali/board-ref/2026-10-19-z1-gpu/README.md`:
  off is 0xffff / 0xfad50001); and the RK806 (SPI2) `POWER_EN0`, `POWER_SLP_EN0`,
  `BUCK1_CONFIG`, `BUCK1_ON_VSEL` (Linux and the image: 0x0f, 0xe0, 0xa4, 0x1c =
  675 mV), through U-Boot's `regulator dev vdd_gpu_s0` / `regulator status` or
  `pmic dump` when its driver has the supply, else the same SPI2 read the image does.
  What decides: whether the image can power the domain on U-Boot's state without a
  step Linux took first.
- **The GPU clock without Linux**: SCMI clock 5 against the CRU. In U-Boot read the CRU
  (0xfd7c0000) `CLKSEL_CON(158)` (0xfd7c0578; Linux leaves 0x1f24: `clk_gpu_src`
  CPLL / 5 = 300 MHz, PVTPLL bit 14 clear), `CLKGATE_CON(66)`, the CPLL and GPLL
  `PLL_CON` words, and `clk dump` (the rates U-Boot believes, SCMI clock 5
  included), at the prompt with nothing run before. The image refuses the PVTPLL and, when the CRU is not at a
  rate inside the OPP table, sets `CLKSEL_CON(158)` itself (a PLL and divider
  inside the OPP table); not written yet.
- **The QoS words' effect.** Linux reads 0 for `MODE`, `BANDWIDTH` and `SATURATION`
  in all four blocks with the GPU active (measured 2026-10-26, "The GPU power-on");
  the image leaves the reset values (`MODE` 3, `BANDWIDTH` 0x12d, `SATURATION`
  0x40). Whether the difference matters is unmeasurable until a job runs on the GPU
  from Zephyr (Z3 and later): the first Z4 frame is run once with each setting.
