<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL on the NanoPC-T6 (Mali-G610, RK3588): design

**Z1 PASS 2026-10-19 00:57 JST, build 576065151**: `t6_hello`, kexec'd out of
Armbian, ran on the board (console, WDT0 armed, 10 s on the system timer) and
returned through PSCI `SYSTEM_RESET` in 45 s (records:
`0dce654a8:tools/mali/board-ref/2026-10-19-z1/`). Open: the watchdog reset path has not
been exercised (no hang has been left to WDT0 yet).

**Z2 PASS 2026-11-11 21:01 JST, build v0.1.0-913-g1f19113a3**: `t6_mmu`
(main `1f19113a3`), kexec'd out of Armbian, enabled AS0 on its own page
tables, made three cache flush round trips and returned through PSCI
`SYSTEM_RESET` in 30 s (record `20261111-210127-t6`, `VERDICT t6_mmu
PASS`). Two runs of the image of main `3aa24a68d` the same day stopped,
once in Z1's power-on and once on the image's own reading of a status
word; both were corrected before this run ("Z2 board procedure", "The
runs of 2026-11-11"). The power-on's PLL step ran the same day: `t6_gpu`
of main `1bac19568`, started from a powered-down NPLL, powered it and
passed (record `20261111-213650-t6`, `VERDICT t6_gpu PASS`).

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
| Clock now | 300 MHz from cpll at Z1's run of 2026-10-19, 850 MHz from npll on 2026-11-11 (devfreq simple_ondemand); Linux powers npll down while the GPU is suspended ("The GPU power-on") | board debugfs, `/dev/mem` |
| Console | uart2, 1500000 baud | linux:`rk3588-nanopc-t6.dtsi:40-42` |

**GPU power-on (kernel side, Z1)**, from linux:`pm-domains.c` (GPL: facts only):
supply on; domain clocks on (the clock framework first powers their PLL and waits
for its lock, `clk-pll.c`); PMU+0x14c = 0x00010000; poll PMU+0x290 bit 1; SIP SMC
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

**Host side, written 2026-10-28 (no board run yet).** What is code:

- **The file.** The board's `/lib/firmware/arm/mali/arch10.8/mali_csffw.bin`
  (Armbian 26.8.10's armbian-firmware), read-only: 282624
  bytes, sha256 `a27847ea11f8efb3136340c3ba8aab413ae25145eeb4a7f64ff5edd829a2405b`,
  header 0.3, version hash 0x01050000, build `git_sha:
  95a25d71030715381f33105394285e1dcc860a65`. It is byte for byte
  linux-firmware's `arm/mali/arch10.8/mali_csffw.bin` at commit 930ef904
  ("panthor: Add firmware for more Mali GPUs", 2025-08-08; checked
  2026-10-28), so a host gets it without the board:
  `tools/cts/deps-env.sh --build MALI_CSFFW` fetches that file and checks
  the sha256 pinned in `tools/cts/deps.toml`. It lives outside the tree
  (`MALI_CSFFW`) and is never committed (Arm's licence,
  `LICENCE.mali_csffw`). `driver/tools/mkmalifw.sh` makes
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
  file (`MALI_CSFFW`), and is a gap naming the file without it (never a
  failure). The gate workflow fetches the file on a runner that lacks it.

**Predicted for the board run** (each value is what the code above writes or
expects):

| Item | Predicted |
|---|---|
| `MCU_STATUS` before the start | 0 (disabled, after Z1's soft reset) |
| `JOB_INT_RAWSTAT` bit 31 | rises within 1 s of `MCU_CONTROL` = 2; `MCU_STATUS` then 1 |
| GLB `version` | 0x01050000 = **1.5.0**, the image header's version hash; `features` 0, `instr_features` 0x71 (the board's dmesg under panthor, read 2026-10-26: "CSF FW using interface v1.5.0, Features 0x0 Instrumentation features 0x71"; the image's control block holds the same 0 and 0x71 before boot) |
| GLB input / output | 0x04002000 / 0x04006000; 8 groups, stride 0xa0 (the image's values; the firmware may rewrite them) |
| CSG 0 | input 0x04003000, output 0x04007000, 8 streams of stride 0xc; `suspend_size` and `protm_suspend_size` 25600 each (measured by Z4's first run, issue #4515; the 7-page buffers panthor allocates, above) |
| CS 0 of CSG 0 | input 0x04003400, output 0x04007200; `features`: work registers 96 (`cs_reg_count`, measured under panthor), scoreboards 8 |
| Written | `core_en_mask` 0x50005; `poweroff_timer` 235, `idle_timer` 19 (24 MHz ticks of 1024); `progress_timer` 0x271000; `ack_irq_mask` 0x0400050e; `req` 0x40e |
| Acknowledged | `CFG_ALLOC_EN`, `CFG_POWEROFF_TIMER`, `CFG_PROGRESS_TIMER` within 100 ms; then **`PING`** (req bit 8 toggled, ack bit 8 follows) within 100 ms |

After Z3 the kernel side must ping at least every 12 s (`PING_INTERVAL_MS`).

**Open, before the board run:**

- **Done: the all-section AS0 map** (8c8b8581):
  `board::nanopc_t6_sections` and `board::nanopc_t6_mmu`, in the next list.
- **Done: the Zephyr probe and its FFI entry.**
  `tools/probes/nanopc_t6/t6_fw/src/main.c` (ceb971bb scaffold, 29756d0e
  the call) embeds `mali_csffw_blob.h`, owns the backing of every loaded
  section inside the image and the `__nocache` shared section, and prints
  the record; `yd_t6_fw_up` (c022cbfc, over `board::nanopc_t6_fw`,
  c76d6bca) runs Z1's power-on, Z2's AS0 and `csf::boot`. The expected
  uart2 lines are in "Z3 board procedure". `McuMem::cpu` hands out
  whatever CPU address the image gives a backing, and `McuMem::clean` is
  the CPU clean of the loaded bytes.
- **Done: the cache flush after the load** — answered, in the paragraph
  "Settled: no GPU cache flush after the load, beyond the CPU clean"
  below (7f35d633).
- **Done: the gate row for the `t6_fw` image** (370dd06b), under
  "Closed on main, host side" below.

**Closed on main, host side:**

- **The `t6_fw` gate row** (370dd06b): `zephyr  nanopc_t6 t6_fw image
  (Z3, the firmware boot), -Werror on our C files` in `tools/check.sh`,
  after the `t6_mmu` row. It builds `tools/probes/nanopc_t6/t6_fw` with
  `-Werror` on our C files (`zephyr_nanopc_t6_fw_image`); with the board's
  file at `MALI_CSFFW` it first makes `mali_csffw_blob.h` from it with
  `driver/tools/mkmalifw.sh` and replaces `driver/src/fw/mali_csffw_blob.h`
  only when the bytes differ. The header is never committed. A gap, not a
  pass: without a west workspace at `ZEPHYR_WS` (the one gap line for every
  Zephyr image), or with neither the file at `MALI_CSFFW` nor a header
  (`needs` names the file; `tools/cts/deps-env.sh --build MALI_CSFFW`
  fetches it). Locally: `ZEPHYR_WS=<ws> MALI_CSFFW=<mali_csffw.bin>
  tools/check.sh --changed origin/main`.

- **Z2's map, extended to every section** (8c8b8581):
  `board::nanopc_t6_sections` maps each section `fw::parse` keeps at its
  MCU address onto its own backing, with the protection panthor derives
  from the section's own flags (`section_prot`: writable only with `WR`,
  executable only with `EX`, GPU-cached only for `CACHED`, from
  `panthor_fw_load_section_entry`'s flags, panthor_fw.c:584-592, and
  `flags_to_prot`). On the board's file that is the 7 sections, 896 KiB:
  the data and code read-only, 0x02000000 and the shared section
  writable, only the code executable, all GPU-cached but the shared
  section (`CACHED_COHERENT`, uncached on the non-coherent T6).
  `Placement::new` checks the backings before anything is mapped (one per
  section, not smaller, 4 KiB aligned, no overlap, none protected);
  `mmu_up_sections` runs Z2's sequence (`nanopc_t6_mmu::as0_up`, shared
  with `mmu_up`) with one `map` per section, a CPU walk of each section
  back to its backing, the clean of the 9 used table pages (of the pool's
  16: 1 L0, 1 L1, 1 L2 and 6 L3, since 0x00400000 and 0x00410000 share
  one), the enable, then one flush after the map over each section's own
  range (each section is its own kernel BO and map in panthor).
  `McuMem` is the board's `csf::McuMemory` over the placement: `cpu` is
  the backing's CPU address inside one section only, `clean` calls the
  cleaner the Zephyr side supplies (`sys_cache_data_flush_range`), and
  `clean_rc` keeps the first rc that failed. Host tests:
  `the_protection_is_the_sections_own_flags_as_panthor_reads_them`,
  `every_page_descriptor_follows_panthors_rules` (every page descriptor
  of every section, against the descriptor rules of "Z2: AS0 and a cache
  flush": 0xfc7 read-only cached, 0xf47 writable cached, 0xe43 writable
  uncached, `XN` on everything but the code),
  `every_section_comes_up_in_as0_on_nine_table_pages`,
  `the_cpu_side_is_the_backing_inside_one_section_only`,
  `a_bad_backing_or_a_small_pool_is_refused_before_as0_is_touched`,
  `load_sections_through_the_board_memory_cleans_every_loaded_byte`, and,
  on the board's file, `the_boards_file_maps_into_as0_section_by_section`
  (`MALI_CSFFW`).
- **Whether the load needs a GPU cache flush beyond the clean**
  (7f35d633): it does not — the reading, line by line against Linux
  v6.18, is the next paragraph.

**Settled: no GPU cache flush after the load, beyond the CPU clean** (read
in Linux v6.18, 7d0a66e4; settled on main 7f35d633). panthor's first
boot, `panthor_fw_init`
(`panthor_fw.c:1343-1403`), runs: `panthor_gpu_l2_power_on` (1367;
`GPU_COHERENCY_PROTOCOL` then the L2 power-on, `panthor_gpu.c:227-248`);
`panthor_vm_create` for the MCU (1371); `panthor_fw_load` (1381) and, per
section, `panthor_fw_load_section_entry` (489-633): the kernel buffer is
created and mapped in the not yet enabled VM (603-607, `panthor_gem.c:173`);
that map's `FLUSH_PT` is skipped because the VM holds no address space yet
(`panthor_vm_flush_range` returns at `panthor_mmu.c:903-904`; the map ends
there at 1005); then `panthor_fw_init_section_mem` copies the data and
zeroes the rest (619, 398-420; the CPU's mapping of the buffer is
write-combined on a non-coherent GPU, `panthor_gem.c:247`) and
`dma_sync_sgtable_for_device(..., DMA_TO_DEVICE)` over the buffer's pages
(624-626), a clean and nothing else; `panthor_vm_active` (1385;
`panthor_mmu.c:724-819`), whose `panthor_mmu_as_enable` (636-650) does a
whole-VA `FLUSH_MEM` first (`mmu_hw_do_operation_locked`, 572-621: `LOCK`,
`GPU_CMD` flush of L2 clean+invalidate and LSC clean+invalidate at 608,
`UNLOCK`), then writes `AS_TRANSTAB`/`AS_MEMATTR`/`AS_TRANSCFG` and `UPDATE`;
`panthor_fw_start` (1389; 1028-1059): `MCU_CONTROL` = `MCU_CONTROL_AUTO` (2,
`panthor_regs.h:121-124`) at 1034, with nothing in between. So between the
section copy and the MCU start there is exactly one GPU-side flush: the
enable's whole-VA clean+invalidate, which `panthor_vm_active` brings for the
address space, not for the sections. `panthor_gpu_flush_caches` has only two
callers in the driver, that one (`panthor_mmu.c:608`) and the scheduler's
suspend (`panthor_sched.c:2764`). panthor's own reload after a slow reset
(`panthor_fw_post_reset`, 1112-1148) runs the other way round: `panthor_vm_active`
(1117), then `panthor_reload_fw_sections` (1126; the copy and the same sync,
675-690), then `panthor_fw_start` (1138): the copy follows the enable's
flush there, and no flush follows the copy.

Ours, `csf::boot` after Z2: Z1's power-on has powered the L2 (PD_GPU was off
at hand-over); the flush round trip alone; the tables built and cleaned;
`mmu::as_enable` with its whole-VA `FLUSH_MEM`; the map's `FLUSH_PT` (Z2's
step 3); then `csf::load_sections` copies, zeroes and cleans each section
(`McuMemory::clean`, `sys_cache_data_flush_range`) and `start_mcu` writes
`MCU_CONTROL` = 2. The order of the clean and the start is panthor's, the
enable and its flush come before the copy as in panthor's slow reset, and
nothing flushes the GPU between the clean and the start, as in panthor. The
sequences match. Why it is enough (our reading, not a panthor statement):
the MCU is disabled until the write, so the GPU's L2 cannot allocate a line
of any section between the enable's invalidate and the start (and the L2 was
powered up after the PD_GPU-off hand-over, so it held nothing to begin with);
what the MCU must see is only what the CPU's caches still hold, and the clean
to the point of coherency covers that.

The image must add nothing on the GPU side. What the code items keep to:
the clean covers each section's whole `va_size`, the zero-filled tail
included (`load_section` does; panthor syncs the whole buffer); every clean,
and the table clean, is complete (a barrier after the last `dc cvac`) before
the `MCU_CONTROL` write, so `clean` should be Zephyr's range flush and
nothing weaker; and the loaded sections are not touched again by the CPU
between the clean and the start. An extra GPU flush before `MCU_CONTROL`
would be harmless but is not panthor's, and would hide a missing clean: add
none; if the board run shows an MCU fault on its first fetch with the tables
walking right, adding one is the first experiment, recorded as such.

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
on main; S6 remains (#31).
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
  gate row; below, "Z2: AS0 and a cache flush"). **PASS on the board
  2026-11-11** (the image of main `1f19113a3`, by kexec; below, "Z2 board
  procedure", "The runs of 2026-11-11"), after two FAIL runs of the image
  of main `3aa24a68d` the same day: the first stopped in Z1's power-on,
  the second ran every Z2 step and stopped on its own reading of
  `MMU_INT_RAWSTAT`. The sign-off form (U-Boot starting the image) has
  not run.
- **Z3**: firmware loaded; the MCU boots; GLB version reads 1.5.0; a `PING` is
  acknowledged. Host side written 2026-10-28 ("The firmware (Z3)": the
  generator, the parser, the interface, the boot over a model of the MCU);
  the map over every loaded section joined it on 2026-11-01 (8c8b8581);
  the Zephyr side joined it 2026-11-03 to 2026-11-06: the `t6_fw` probe
  (ceb971bb scaffold, 29756d0e the call), the C entry `yd_t6_fw_up`
  (c022cbfc, fbbd4d71, in `driver/rust/src/ffi/rk3588.rs` and
  `yakogl_drivers.h`), `t6_fw`'s lines in `t6-verdict.sh` (08163414) and
  the `t6_fw` gate row (370dd06b). **The board run passed on 2026-11-12**
  (`VERDICT t6_fw PASS`, the image of main `988639752`; below, "Z3 board
  procedure", and `tools/cts/RESULTS.md`): a development result, by kexec
  out of Armbian. The sign-off form (U-Boot starting the image) has not
  run.
- **Z4**: CSG 0 / CS 0 started from our kernel side; the L1 frame renders the same
  bytes as under panthor. **The board run passed on 2026-11-12**
  (`VERDICT t6_submit PASS`, the image of main `ef86672bf`; below, "Z4
  board procedure", and `tools/cts/RESULTS.md`), after a first run the
  same day whose frame was right and whose record showed three
  predictions wrong (issue #4515): a development result, by kexec out of
  Armbian. The sign-off form (U-Boot starting the image) has not run.
  From here the same back end and compiler run on Zephyr,
  and gl_test and dEQP move to the product form.

### The Z4 frame against the trace

What Z4 submits is the frame `l1::build_triangle_frame`
(tools/mali/panthor/src/l1.rs, 6dd3dc48) builds, so the frame is compared
here with the board trace once, word by word, before the Z4 record is
predicted. The reference is `tools/mali/testdata/board-25.0.7/tri.pandecode`
(Mesa 25.0.7, the default AFBC run, 1735 lines, text only: the decoded
descriptors and the CS words, no memory), frame 1 = lines 1-815 (6 vertices;
lines 816-1735 are a second frame of another program, not compared), and for
the raw bytes the linear run's dump `tools/mali/board-dump/tri-frame-2026-10-18.txt`,
which has the same VAs but for the descriptor base (`...d4000` there, `...d3000` in
`tri.pandecode`) and the same line numbers. **Mesa 26.2.1's trace
(`mesa-26.2.1.pandecode`) is under the gitignored `records/` and is not in the
tree, so nothing below was measured against 26.2.1**: the 26.2.1 column is the
25.0.7 result plus the table of choices in "The command stream and descriptors
(L1)", read from the source, never a byte equality.

The frame was built at the trace's VAs (descriptors `0x7ffffffd3000`, code
`0x7fffffffc000`, SPDs `0x7fffffffd020`, heap descriptor `0x7fffffffb000`,
geometry `0x7ffffffeb000`, target `0x7ffffffe3000`, vertices `0x7fffffecb000`,
first chunk `0x800000a00000` of 2 MiB, heap context `0x800000015000`, 92 user
registers). Results: **equal** (same bytes), **reloc** (only an address that
moves with the VA or with the layout, the value checked), **choice** (a
documented difference), **GPU** (written by the GPU before the dump), **not
built** (Mesa emits it, the frame does not). No difference is unexplained.

| Part | Size | Result | Rests on (`tri.pandecode` line, or `tri-frame-2026-10-18.txt`) |
|---|---|---|---|
| Queue-init stream | 2 words | equal: `MOVE d72, #0x800000015000`, `HEAP_SET d72` (the heap context is the kernel's) | 1-2 |
| OOM handler install and body | 3 + 40 words | not built (`SET_EXCEPTION_HANDLER`, the handler, `queue_init` doc above) | 3-45 |
| Draw stream, `REQ_RESOURCE` to `RUN_IDVS` | 33 words | 30 equal, 3 reloc: SRT `d0`/`d4` (`...3307`/`...3387` against `...3647`/`...36c7`, table count 7 kept; 26.2.1 would write 8, doc table) and the tiler `d40` (`...3280` against `...32c0`: no OOM context or incremental FBDs before it); `MOVE d30` (TSD_3) is in both, the 25.0.7 choice, 26.2.1 drops it | 46-79 |
| Not built in the draw stream | 5 words | `MOVE d76` (OOM context), `LOAD_MULTIPLE r78`, `WAIT #1`, `BRANCH.le r78`, and the `MOVE d40` the branch skips (not printed; raw CS buffer) | 48, 570-572 |
| `FINISH_TILING` to `RUN_FRAGMENT`, then `FINISH_FRAGMENT` to `FLUSH_CACHE2` | 18 words | 16 equal, 2 differ: `MOVE d40` (FBD pointer `...3001` against `...3003`, tag 3 = ZS/CRC extension: choice, linear and no CRC) and `MOVE d90` (tiler, reloc as above) | 562-573, 803-813 |
| Not built at the end | 2 words | `MOVE d90` error-state word, `STORE_STATE.ERROR_STATE` (`PAN_MESA_DEBUG=sync` only) | 809-810 |
| Whole stream | 51 words | 47 equal as dumped, 4 reloc; 7 of Mesa's 58 not emitted (the list above) | `frame_tests.rs`, the raw CS buffer |
| Framebuffer | 128 B | equal but the sample-locations pointer (`+0x10`, `...b040` against `...f000`: our context arena packs the heap descriptor and the samples together, reloc) and the tiler pointer (`+0x38`, reloc); AFBC run: `Has ZS CRC Extension: true` and `CRC Write Enable: true` (choice) | 654-728 |
| Render target | 64 B | equal but the target pointer (input VA); AFBC run has AFBC block format, header, `Row stride 64`, `Body offset 256` and `Surface stride 256` where the frame writes linear, `Row Stride 256` (choice: "Linear render targets, no AFBC, no CRC") | 752-801 |
| ZS/CRC extension | 64 B | not built (room only, `Has ZS CRC Extension: false`); the AFBC run has CRC row stride 32, base `...7100` (choice) | 730-750 |
| Shader programs (2 SPDs) and binaries (2 x 7 words) | 32 B x 2, 56 B x 2 | equal (the trace's own programs, `tri_shaders`) | 317-383 |
| TLS (position, varying, fragment: one block) | 32 B | equal | 388-411 |
| Depth/stencil, blend, DCD/primitive flags | 32 B, 16 B | equal (the decoded text of lines 459-558 is identical in both runs; the bytes are equal to the linear dump) | 459-561 |
| Attribute and varying descriptor, vertex buffer | 32 B, 32 B | attribute equal; buffer: address (input VA) and **size 48 against 1048576**: `l1.rs` passes the bytes it uploaded, Mesa the whole 1 MiB BO (`frame_tests.rs` passes `1 << 20`); the six vertices lie inside either | 93-123 |
| Samplers (VS, FS), UBO tables, sysval block, push | 32 B x 4, 24 B | equal but the pointers inside the UBO table and resource tables (reloc) and the sysval block's offset lane `+0x1c`, which Mesa fills from its stack (4 bytes, `frame_tests.rs`) | 80-306 |
| Resource tables (VS 112 B, FS 112 B) | 112 B x 2 | equal but the four VS and one FS entry pointers (reloc, `...e0/...60/...80/...a0` keep their low bytes) | 80-306 |
| Tiler context | 128 B | equal but the polygon list (`+0x0`) and private state word 0 (`+0x60`), GPU-written | 427-450, 705-728 |
| Tiler heap descriptor | 32 B | equal but Bottom/Top (`+0x10`, `+0x18`), GPU-written | 418-426 |
| Sample positions | 193 B | equal (at `...b040`, not `...f000`: reloc) | 620-653 |

The raw-byte rows rest on `src/mali/frame_tests.rs` ("24 compared, 20 equal as
dumped, 24 equal with the 6 named differences, 0 differing", and the stream
"51 ours, 47 equal as dumped, 4 relocated, 7 of Mesa's not emitted, 0
differing"); the AFBC-run rows rest on the line-by-line diff of the decoded
text of `tri.pandecode` against `tri-linear-2026-10-18.txt` (same line numbers),
which differs only at the VA (lines 68-69, 654), `Has ZS CRC Extension` (689)
and the extension and render target (730-781), and on the CS words printed in
the trace. The 26.2.1 reading of the table: the frame as built equals 26.2.1's
only after the three choices (TSD_3, table count 8, OOM slot 4 in the handler,
which the frame does not install) are made in `src/mali/frame.rs`; that change
is not part of Z4 and waits for the 26.2.1 trace on the board.

To reproduce: `cargo test --no-default-features --features
host-test,gpu-mali,shaderc-online,no-shader-table --lib -- mali::frame_tests
--nocapture` prints the two summary lines and the named differences. The
`tri.pandecode` VAs need a throwaway program that is not committed: it calls
`l1::build_context` (heap descriptor at `...b000`), `frame::queue_init` and
`l1::build_triangle_frame` with the arenas at the VAs above and the 92 user
registers, prints the words, and compares them with
`Trace::parse(tri.pandecode)` (`tools/mali/panthor/src/pandecode.rs`, the
depth-1 words of lines 1-815) and the descriptor bytes with the linear dump.

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

1. Read what the previous stage handed over, writing nothing: PD_GPU in the
   PMU, the GPU clock in the CRU, SPI2's state, then the RK806 over SPI2
   (`rockchip_spi::write_then_read`, read frames only). The image prints all
   of it before its first RK806, PMU or GPU write (`gpu_up`'s `on_handover`).
2. The GPU clock onto the OPP table (`soc::rk3588::gpu_clock_set`). Kept,
   nothing written, when `CLK_GPU_SRC` is a PLL the CRU reads in normal mode
   whose rate over the divider is an OPP rate (`gpu_opp_rate`) and the stacks
   divider (`CLKSEL_CON(159)` [4:0]) is 0: Armbian's 0x1f24 (CPLL / 5, 300
   MHz) and 0x1f60 (NPLL, 850 MHz) both stay as they are. Otherwise (the
   PVTPLL selected, SPLL, a mux value past the parents, a divider off the
   table, a PLL out of normal mode) it writes `CLKSEL_CON(158)` := 0x40ff0024
   (`SET_SEL158`: CPLL, divider 5, PVTPLL bit clear, one hiword-masked write)
   and, only if its divider field is not 0, `CLKSEL_CON(159)` := 0x001f0000
   (`SET_SEL159`), reads both back and rates the CRU again: 300 MHz or a
   refusal. It does this only while the CPLL reads 1500 MHz in normal mode
   (`SET_CPLL_HZ`); GPLL's 1188 MHz divides to no OPP rate and NPLL is the
   GPU's own PLL, which Linux powers down with the GPU. No `PLL_CON`,
   `MODE_CON0` or PVTPLL word is ever written. Stops: no table rate (stop 9,
   `ClockSetRefused`, `detail` the CPLL's rate, nothing written); a word that
   did not read back, or a rate other than 300 MHz afterwards (stop 10,
   `ClockReadBack`).
3. The supply, held against the rate of step 2's result (the OPP table's
   voltage, rk3588-opp.dtsi:118-153). The image never writes a voltage. By
   DCDC1 (`POWER_EN0` bit 0) and PD_GPU (`REPAIR_STATUS` bit 1):
   - **vdd_gpu_s0 on** (Linux's kexec state, 675 mV): a `BUCK1_ON_VSEL` below
     the OPP minimum stops the run (stop 4, `ClockNotCovered`, refusal 0,
     `detail` the rate); nothing is written to the RK806.
   - **vdd_gpu_s0 off, PD_GPU off** (possible after U-Boot, the supply being
     boot-on but not always-on): `BUCK1_ON_VSEL` as found must already cover
     the rate (else stop 11, `SupplyBelowOpp`, refusal 0, `detail`
     `BUCK1_ON_VSEL` << 32 | rate in Hz, nothing written). Then, after the
     hand-over report and the PLL (step 4), the one RK806 write,
     `ip::rk806::dcdc1_enable_frame`: `POWER_EN0` := 0x11 (DCDC1 and its
     write-enable twin, nothing else), sent by `rockchip_spi::write`
     (transmit only: `write_then_read` refuses an empty receive buffer).
     `POWER_EN0` is polled until DCDC1 reads on, at most
     `rk806::DCDC1_RAMP_POLL_BUDGET_US`, and `BUCK1_ON_VSEL` is read again and
     held against the rate again. Stops: the frame could not be sent (stop 13,
     `Rk806Write`, refusal the `SpiRefusal`; whether it reached the PMIC is
     unknown), DCDC1 not on in the budget (stop 12, `SupplyRampTimeout`,
     `detail` the last `POWER_EN0`), `BUCK1_ON_VSEL` below the rate after the
     enable (stop 11, refusal 1). Nothing is written to the PMU on any of
     them: the supply is on before the domain, as in Linux.
   - **vdd_gpu_s0 off, PD_GPU powered**: no state the facts cover; stop 3
     (`SupplyOff`), refusal 1, `detail` `POWER_EN0`, nothing written to the
     RK806.

   An RK806 read that fails is stop 2 (`Rk806Read`); an SPI2 gate that stays
   closed is stop 1.
4. The PLL the GPU clock runs from, on, as Linux's
   `rockchip_rk3588_pll_enable` does it (clk-pll.c): when `PLL_CON1` bit 13
   (power-down) is set, clear it through its write-enable bit; then wait
   for `PLL_CON6` bit 15 (lock), at most Linux's 1000 µs
   (`rockchip_rk3588_pll_wait_lock`). The rate words and `MODE_CON0` are
   left as found. A PLL that does not lock stops the run here (stop 4,
   refusal 1, `detail` the `PLL_CON6`), before the RK806 and the PMU are written.
5. PD_GPU on in Linux's order (pm-domains.c), `soc::rk3588::pd_gpu_on`: the
   four GPU clock gates, `PWR_GATE` bit 0 cleared, wait for `REPAIR_STATUS`
   bit 1, the bus idle request released, wait for its acknowledge and status.
   Its PMU words go through write masks and the waits read the result, so no
   step depends on the other domains' bits or, with the domain off, on
   whether the GPU's idle request was set. A domain already powered with its
   idle request released (`PdGpuRegs::usable`) is left as it is (only the
   gates are opened); one powered with the request still set runs the whole
   sequence. A refusal is stop 5 (`PowerDomain`, refusal the `PdRefusal`,
   `detail` the register read). The SIP call that tells the firmware about
   the domain is left out (it configures system suspend only). The QoS words
   are read, not written.
6. `GPU_ID`; soft reset; `GPU_COHERENCY_PROTOCOL` = none; L2 on
   (panthor_gpu.c, under its MIT option). The L2 wait needs `L2_READY` to
   show the L2: panthor's condition (`(mask & val) == val`,
   panthor_gpu.c:210) also accepts 0. Stops: a `GPU_ID` that is not the G610's
   is stop 6 (`NotG610`), a soft reset that does not complete stop 7
   (`Reset`), an L2 that does not come up stop 8 (`L2`).

The stop numbers are `board::nanopc_t6_gpu::Stop`; the C names of 9 and up
(`YD_T6_GPU_STOP_*`) and the clock-set and supply-enable fields of the record
are not in the header yet.

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

**Measured, 2026-11-11: the GPU clock on a PLL that Linux powered down.**
Step 3 above was added after this run; the image of 2026-10-19 opened the
clock gates and never looked at the PLL.

- **The run.** `t6_mmu`, build v0.1.0-897-g3aa24a68d, record
  `20261111-131354-t6`: `t6_mmu: Z1: rc -5 stop 5 refusal 3`, `detail`
  0x000a9fff. PD_GPU's power came on, the bus idle request was released,
  and the acknowledge (`BUS_IDLE_ACK` bit 0) never cleared in the wait's
  budget. The board came back through the image's PSCI reset.
- **The hand-over of that run** (the runner's "gpu power before" line):
  `CLKSEL_CON(158)` 0x00001f60, `clk_gpu_src` from NPLL over 1, and
  `BUCK1_ON_VSEL` 0x30 (800 mV): devfreq's last rate was 850 MHz, where
  on 2026-10-19 it was 300 MHz from CPLL. PD_GPU off as before
  (`PWR_GATE` 0x0000ffff, `BUS_IDLE_REQ` 0x000083cf, `_ACK`/`_ST`
  0x000a9fff, `REPAIR_STATUS` 0xfad50001).
- **NPLL under Armbian with the GPU suspended** (read through `/dev/mem`
  the same day, kernel 6.18.55): `PLL_CON0` 0x000001a9, `PLL_CON1`
  0x00002083 (bit 13 set: powered down), `PLL_CON6` 0x00004000 (bit 15
  clear: not locked), `MODE_CON0` 0x00000545; `clk_summary` shows
  `pll_npll` at 850 MHz with enable count 0. CPLL, which other clocks
  use, reads `PLL_CON1` 0x00000042 and `PLL_CON6` 0x00009c00 (powered,
  locked). The GPU is NPLL's only user, so Linux's clock framework
  powers NPLL down when panthor suspends (`rockchip_rk3588_pll_disable`
  sets bit 13 and nothing else).
- **What it means.** A domain's bus cannot leave idle without its clock.
  With the GPU's clock on CPLL the image got a running clock without
  doing anything; on NPLL it got none. The rate arithmetic was right
  (850 MHz, 800 mV covers it): the image rated a PLL that was off.
- **On the host.** The board model in `board::nanopc_t6_gpu`'s tests now
  lets the bus leave idle only when the selected PLL is powered and
  locked. With that and the values above, the old sequence stops exactly
  as the board did (stop 5, refusal 3, 0x000a9fff); with step 3 it runs
  to the end and its only CRU write before the gates is `PLL_CON1` :=
  0x20000000.
- **The record now shows it.** `t6_gpu` prints `GPU PLL on: PLL_CON6 …
  at hand-over, was on N, waited N us, PLL_CON6 …`; `nanopc-t6.sh`'s
  "gpu power" lines carry `MODE_CON0` and `PLL_CON1`/`PLL_CON6` of
  AUPLL, CPLL, GPLL and NPLL.
- **Run on the board** the same day (`t6_gpu`, build
  v0.1.0-915-g1bac19568, record `20261111-213650-t6`, `VERDICT t6_gpu
  PASS`), from the same hand-over (`CLKSEL_CON(158)` 0x00001f60,
  `PLL_CON1` 0x00002083, `PLL_CON6` 0x00004000, `BUCK1_ON_VSEL` 0x30):
  `t6_gpu: GPU PLL on: PLL_CON6 0x00004000 at hand-over, was on 0, waited
  10 us, PLL_CON6 0x0000a400`. NPLL locked within the first 10 µs wait;
  the bus then left idle (`PD_GPU on: gates opened 0xd2, was on 0, waited
  6/1/0 us`) and every later line is the predicted one ("The runs of
  2026-11-11", the fourth).

### Z2: AS0 and a cache flush

Written on the host 2026-10-26, before the code; run on the board on
2026-11-11, **PASS** on the third jump (below, "Z2 board procedure"). What Z2
shows: our kernel side builds the MCU address space's
page tables in the image, enables AS0 on them the way panthor enables the
MCU's VM, and gets a GPU cache flush back with its completion bit. It is
the step before Z3, whose firmware is loaded into exactly these tables.

Sources. Linux v6.18 (7d0a66e4), read line by line 2026-11-01;
`drivers/gpu/drm/panthor/` under its MIT option (decision 1), line numbers
below are that tree's:
`panthor_regs.h` (`GPU_MMU_FEATURES` 31-33, `GPU_INT_*`, `GPU_CMD` and
`GPU_FLUSH_CACHES` 37-56, `MCU_CONTROL` 121-124, `MMU_INT_*` 141-144, the
`AS_*` registers 148-201), `panthor_mmu.c` (`wait_ready` 506-524, `write_cmd`
526-536, `lock_region` 538-570, `mmu_hw_do_operation_locked` 572-621,
`panthor_mmu_as_enable` 636-650, `panthor_vm_active` 724-819,
`panthor_vm_flush_range` 898-914, `panthor_vm_map_pages` 945-1006,
`flags_to_prot` 1008-1024, `mair_to_memattr` 1990-2030, `panthor_vm_create`
2322-2441: the io-pgtable configuration 2381-2392, the memattr 2407-2408),
`panthor_gpu.c` (`panthor_gpu_irq_handler` 55-75, `panthor_gpu_flush_caches`
259-294). `drivers/iommu/io-pgtable-arm.c` is **GPL-2.0-only**: facts with
their line, never its code (`arm_lpae_prot_to_pte` 481-549,
`arm_lpae_install_table` 392-422, `__arm_lpae_init_pte` 339-358, the stage-1
`MAIR` and `ttbr` 1037-1058, the PTE sync of a non-coherent walk 318-322,
356-357 and 413-417). The Arm ARM's VMSAv8-64 descriptor fields are the
architecture. The `panthor_mmu.c` lines elsewhere on this page (`506-621,
785-790, 1990-2030`) were checked in the same pass and hold. Where the
reading contradicted this subsection it is corrected below: the `MAIR`
byte of `INC_OWBRWA` ("`AS_MEMATTR`"), and the function name in the
descriptor table's page row.

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
  panthor sets `coherent_walk` from `ptdev->coherent`, false on the T6: `panthor_mmu.c:2385`; the PTE sync is `__arm_lpae_sync_pte`, `io-pgtable-arm.c:318-322`).
  The image does the same: after building, `sys_cache_data_flush_range`
  over the used pages (Zephyr's "flush" is a clean to the point of
  coherency, `dc cvac` on arm64). Then the enable's own whole-VA flush
  cleans and invalidates the GPU's L2 before `UPDATE`, so the GPU cannot hold
  an old line of those pages.
- The code that takes the pool (`gpu::mali::pgtable::Tables`) allocates
  nothing: it is given the pages and their physical address, as the GC7000's
  MMU is given its pool.

**The format.** panthor gives a VM's tables to io-pgtable as
`ARM_64_LPAE_S1` (`panthor_vm_create`, `panthor_mmu.c:2381-2392`: 4 KiB and 2 MiB page sizes, `ias`
the GPU's VA bits, `oas` its PA bits). That is the Arm VMSAv8-64 stage-1
format. It is **not** `ARM_MALI_LPAE`, panfrost's format for the older
job-manager GPUs; the CSF GPUs select the AArch64 mode in `AS_TRANSCFG`
(`ADRMODE_AARCH64_4K`, `panthor_vm_active`, `panthor_mmu.c:785-788`). `MMU_FEATURES` 0x2830 gives
48 VA bits and 40 PA bits (Z1's read, panthor's probe line), so: four
levels (VA bits 47:39, 38:30, 29:21, 20:12), each table one 4 KiB page of
512 descriptors, the root's PA is `AS_TRANSTAB` as it is (io-pgtable's
`ttbr`). Z2 builds level-3 pages only, no 2 MiB blocks.

| Descriptor | Bits | Source |
|---|---|---|
| table (levels 0-2) | next table's PA (47:12) \| 0b11 | `arm_lpae_install_table`, `io-pgtable-arm.c:400-402` (no `NSTable`: only with a quirk panthor does not set; `IO_PGTABLE_QUIRK_ARM_NS`) |
| page (level 3) | PA (47:12) \| 0b11 | `__arm_lpae_init_pte`, `io-pgtable-arm.c:348-354` (called from `arm_lpae_init_pte`, 388) |
| `AttrIndx` 4:2 | 1 (write-back) for a cached mapping, 0 (non-cacheable) otherwise | `arm_lpae_prot_to_pte`, `io-pgtable-arm.c:519-526` (`IOMMU_CACHE` from `flags_to_prot`, `panthor_mmu.c:1015-1016`) |
| `AP[1]` bit 6 | always (unprivileged; panthor never asks `IOMMU_PRIV`) | `io-pgtable-arm.c:493-494`; `flags_to_prot` sets no `IOMMU_PRIV` (`panthor_mmu.c:1008-1024`) |
| `AP[2]` bit 7 | read only (`MAP_READONLY`) | `io-pgtable-arm.c:489-491` (`IOMMU_WRITE` clear, `IOMMU_READ` set), `panthor_mmu.c:1018-1021` |
| `SH` 9:8 | inner shareable (3) cached, outer (2) otherwise | `io-pgtable-arm.c:534-537` |
| `AF` bit 10, `nG` bit 11 | always (stage 1) | `io-pgtable-arm.c:488` (`nG`), 545-546 (`AF`) |
| `PXN`/`UXN` 54:53 | both, unless executable (`MAP_NOEXEC` clear) | `io-pgtable-arm.c:79, 539-540`; `panthor_mmu.c:1012-1013` |

The section VA 0x04000000, 64 KiB, writable, cached, not executable (as
panthor maps a section without the execute flag): its page `n` is
`backing + n·0x1000 | 0x0060000000000f47`.

**The registers** (panthor_regs.h; AS n at 0x2400 + 0x40·n, AS0 the
MCU's: `panthor_vm_active` always gives the MCU VM slot 0, `panthor_mmu.c:755-757`; AS register offsets `panthor_regs.h:148-201`):

| Register | Offset | Z2 writes |
|---|---|---|
| `AS_TRANSTAB` | +0x00 (64) | the pool's PA |
| `AS_MEMATTR` | +0x08 (64) | 0x8f8f8f8f8c4c8f4c (panthor's live word) |
| `AS_LOCKADDR` | +0x10 (64) | 0x3f (all of VA), then 0x000000000400000f |
| `AS_COMMAND` | +0x18 | `LOCK` 2, `UNLOCK` 3, `UPDATE` 1 (`FLUSH_PT` 4, `FLUSH_MEM` 5 are not written: they name the operation only) |
| `AS_FAULTSTATUS`, `AS_FAULTADDRESS` | +0x1c, +0x20 | read |
| `AS_STATUS` | +0x28 | read; bit 0 `AS_ACTIVE` |
| `AS_TRANSCFG` | +0x30 (64) | 0x00000000420001c6 |
| `MMU_INT_RAWSTAT`, `_CLEAR`, `_MASK` | 0x2000, 0x2004, 0x2008 | read (bit n a fault of AS n; bit 16+n a command of AS n completed, no fault) |
| `GPU_CMD` | 0x30 | 0x3304, 0x0304 |
| `GPU_INT_RAWSTAT`, `_CLEAR` | 0x20, 0x24 | bit 17 `CLEAN_CACHES_COMPLETED` |

- `AS_TRANSCFG` (`panthor_vm_active`, `panthor_mmu.c:785-790`): `PTW_MEMATTR_WB` (2 << 24) |
  `PTW_RA` (1 << 30) | `ADRMODE_AARCH64_4K` (6) | `INA_BITS(55 - 48)`
  (7 << 6) = 0x420001c6, the live value; `PTW_SH_OS` only when coherent.
- `AS_MEMATTR` (`mair_to_memattr`, `panthor_mmu.c:1990-2030`, over
  io-pgtable's stage-1 `MAIR` 0xf404ff44, `io-pgtable-arm.c:1037-1046` with
  the bytes at 131-138: index 0 normal non-cacheable 0x44, 1 write-back
  0xff, 2 device 0x04, 3 `INC_OWBRWA` 0xf4): a byte is non-cacheable
  (`INNER_OUTER_NC` | `SH_MIDGARD_INNER` | `INNER_ALLOC_EXPL(0, 0)` = 0x4c)
  when `outer & 3` is 0, `outer & 4` is 0 or `inner & 4` is 0 (2004-2007), write-back
  otherwise (2009-2024): the test is on those bits, not on a policy name. The
  write-back byte takes the inner allocation hints (`inner & 1`, `inner & 2`)
  and, non-coherent, `SH_MIDGARD_INNER` (2020-2021): 0xff gives 0x8f, **0xf4
  gives 0x8c** (its `inner` is 4: bit 2 is set, so panthor's test calls it
  write-back, with no allocation hints; this page first said 0x4c, from
  reading 0xf4 as inner non-cacheable, which the Arm ARM does but the
  driver's test does not), 0x44 gives 0x4c (`outer & 3` is 0), 0x04 and the
  unused zero bytes 0x4c. So 0x4c4c4c4c8c4c8f4c derived; the board's live word
  ("MMU (Z2)" above) is 0x8f8f8f8f8c4c8f4c, the same in bytes 0-3, the
  ones a PTE selects, and 0x8f in bytes 4-7, which none does. The image
  writes the live word (`MEMATTR_LIVE`; a host test holds its bytes 0-3 to
  `mair_to_memattr`); the page's `AttrIndx` 1 is the 0x8f byte.
- `AS_LOCKADDR` (`lock_region`, `panthor_mmu.c:538-570`): the naturally aligned power-of-two block
  holding the range, at least `AS_LOCK_REGION_MIN_SIZE` (32 KiB,
  `panthor_regs.h:173`; the width at 556-557, the base masked at 563), as its
  base with log2(size) - 1 in the low bits. All of VA: 63; the section's
  64 KiB: 0x0400000f.
- `GPU_FLUSH_CACHES(l2, lsc, other)` = `GPU_CMD_DEF(4, l2 | lsc << 4 |
  other << 8)`, `CACHE_CLEAN` 1, `CACHE_INV` 2: L2 and LSC cleaned and
  invalidated 0x3304 (`FLUSH_MEM`), the L2 only 0x0304 (`FLUSH_PT`): the L2
  always cleaned and invalidated, the LSC only for `FLUSH_MEM`
  (`panthor_mmu.c:574-590`; `panthor_regs.h:50-56`).

**The sequences.**

1. The round trip alone (`panthor_gpu_flush_caches`, `panthor_gpu.c:259-294`): clear bit 17, read it
   clear (else refuse: a completion could not be told from an old one),
   write `GPU_CMD` 0x3304, poll `GPU_INT_RAWSTAT` bit 17 for at most 100 ms
   (panthor's `wait_event_timeout`, 276-278), clear it. panthor waits for the IRQ and
   its handler (`panthor_gpu_irq_handler`, 55-75) clears the bit; Zephyr has no handler on SPI 94 here, so the
   image polls the raw status, as Z1's soft reset does.
2. AS0 on (`panthor_mmu_as_enable`, `panthor_mmu.c:636-650`): `mmu_hw_do_operation_locked` (572-621) over the
   whole VA with `FLUSH_MEM`: `AS_LOCKADDR` = 63 (568), `wait_ready`, `LOCK` (569),
   `wait_ready` (604), the flush (0x3304, bit 17, 608), `UNLOCK` (617; a `GPU_CMD` flush does
   not unlock the region, panthor's comment there, 611-615), `wait_ready` (620); then
   `AS_TRANSTAB`, `AS_MEMATTR`, `AS_TRANSCFG` (each 64-bit, low word first: `gpu_write64`, `panthor_device.h:473-477`; 645-647),
   `wait_ready`, `UPDATE` (649). `wait_ready` polls `AS_STATUS` bit 0 every 10 µs
   for at most 100 ms (`gpu_read_relaxed_poll_timeout_atomic(..., 10,
   100000)`, 514-515).
3. The flush after a map (`panthor_vm_map_pages` ends with the range's
   `FLUSH_PT`, 1005, via `panthor_vm_flush_range` 898-914, which does nothing for a VM without an address space, 903-904): `AS_LOCKADDR` = 0x0400000f, `LOCK`, the L2-only flush
   0x0304, `UNLOCK`.

One difference from panthor, on purpose: after each `AS_COMMAND` the image
waits for `AS_ACTIVE` to clear (panthor only waits before the next one), so
each command's completion is a measured count. AS0's registers are read
back after `UPDATE`; the run passes when they equal what was written,
`AS_STATUS`, `AS_FAULTSTATUS` and the fault bits of `MMU_INT_RAWSTAT`
(15:0) read 0, and the three flushes came back. Interrupt masks are read,
never written.

The high half of `MMU_INT_RAWSTAT` is no fault on a CSF GPU: bit 16 + n
says that a command of AS n completed. Sources: Linux v6.18
`panthor_mmu.c`, `panthor_mmu_fault_mask` ("Bits 16 to 31 mean
REQ_COMPLETE", it returns bits 15:0) and `panthor_mmu_as_fault_mask`
(`BIT(as)`); the Tyr driver's register definition (`[PATCH v5 4/6]
drm/tyr: Use register! macro for MMU_CONTROL`, 2026-04-09: `IRQ_RAWSTAT`
at 0x2000 with `page_fault` 15:0 and `command_completed` 31:16). The
"bit 16 + n is a bus error" meaning is the job-manager GPUs'. This page
and the image had the job-manager meaning until 2026-11-11; the board
showed it wrong ("Z2 board procedure", the run of 2026-11-11).

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

### Z2 board procedure

Run three times on 2026-11-11: two FAIL, then PASS ("The runs of
2026-11-11" below). A run needs the T6 and `resource:<board-uart>` and
starts when they are free: `T6_HAND` names who power-cycles the board if
it does not come back, and the run does not wait for that person. This
is Z2's run (the `t6_mmu` image) only; Z3 runs after it and its procedure and
predicted record are not here ("The firmware (Z3)", "Open, before the
board run"). Z1's procedure ("The first board procedure") applies
unchanged: the named hand, the UART captured read only into the run's
record (`uart.log`) from before the jump until the board is back in
Armbian, Linux's watchdog open by default (the record must hold its
WDT_EN proof), `GLB_RST_ST` read before and after; `--go` takes the
board lock (`board_lock.sh`; it refuses, never waits). The predictions
below were written 2026-11-02 from
`tools/probes/nanopc_t6/t6_mmu/src/main.c` and the host tests, before
any run. A run whose lines differ is a finding: record it as it came,
never by changing the prediction. Two predictions were wrong and the
run of 2026-11-11 showed it: `LOCKADDR` as found (predicted 0, read
0xe) and `MMU_INT_RAWSTAT` after the enable (predicted 0, read
0x00010000). That run stays a FAIL; the two lines below carry the
corrected prediction, and the run that passed read both as corrected.

**Build** — from a detached checkout of main, in the west workspace as
Z1 built (the checkout named `yakogl`):

```sh
west build -p always -b nanopc_t6 -d build/t6_mmu yakogl/tools/probes/nanopc_t6/t6_mmu
```

The file to jump is `build/t6_mmu/zephyr/zephyr.bin`: the board's
defconfig carries the AArch64 image header and the watchdog marker
`nanopc-t6.sh` insists on. The gate builds the same image as the row
`zephyr  nanopc_t6 t6_mmu image (Z2, AS0 and a cache flush)`, with
`-Werror` on our C files, and is a gap naming `ZEPHYR_WS` without the
workspace. The image prints its own checkout on its first line,
`t6_mmu: build <describe>`: a result row names that build.

**Run** — the board's host, the UART tty and the record directory come
from the environment (`$BOARD_HOST`, `$BOARD_UART`, `T6_RECORD_DIR`),
never from this page:

1. Ask who uses the board. `nanopc-t6.sh <t6_mmu zephyr.bin>` (the dry
   run) and read its plan. Note the two addresses the record will name:
   `nm build/t6_mmu/zephyr/zephyr.elf | grep 't6_mmu_\(pool\|backing\)'`
   — below, `P` is the pool's address and `B` the backing's. The image
   is identity mapped and both sit under 4 GiB (the image is at
   0x10000000), so the `%p` and the `PA` on a line must be the same
   number, and every 16-hex-digit field of the record prints as
   `0x00000000` followed by its 8 digits.
2. `T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=$BOARD_UART nanopc-t6.sh
   <t6_mmu zephyr.bin> --go`.

**The expected uart2 record**, a PASS run, each line exactly as
`main.c` prints it (`P` and `B` from step 1; `…` is recorded, not
predicted; each 64-bit register or descriptor field is one `0x` word —
the L3 word of the walk line is `0x0060000000000f47 | (B + 0x1000)`,
spelled as that sum below):

```
T6:A .. T6:C, the arming lines, the marker — as for t6_gpu
t6_mmu: build <describe>
t6_mmu: WDT0 period 22369 ms, fed until 60000 ms of uptime
t6_mmu: Z1: rc 0 stop 0 refusal 0: GPU_ID 0xa8670005 MMU_FEATURES 0x00002830 AS_PRESENT 0x000000ff L2_READY 0x1
t6_mmu: pool 0x{P} at PA 0x{P} (16 pages), backing 0x{B} at PA 0x{B}
t6_mmu: AS0 as found: TRANSTAB 0x0000000000000000 MEMATTR 0x0000000000000000 TRANSCFG 0x… LOCKADDR 0x… STATUS 0x00000000 FAULTSTATUS 0x00000000 FAULTADDRESS 0x0000000000000000
t6_mmu: IRQ as found: MMU_INT_RAWSTAT 0x00000000 MMU_INT_MASK … GPU_INT_RAWSTAT 0x… GPU_INT_MASK …
t6_mmu: flush round trip: GPU_CMD 0x00003304, GPU_INT_RAWSTAT 0x… before the clear, 0x… after; CLEAN_CACHES_COMPLETED after n polls of 1 us
t6_mmu: tables at PA 0x00000000{P}: 4 of 16 pages used; backing PA 0x00000000{B}
t6_mmu: walk VA 0x0000000004001234: L0 0x00000000{P+0x1003} L1 0x00000000{P+0x2003} L2 0x00000000{P+0x3003} L3 (B+0x1000) | 0x0060000000000f47 (4 levels): PA 0x00000000{B+0x1234}
t6_mmu: tables cleaned to the point of coherency: rc 0
t6_mmu: AS0 enable, the whole-VA flush: LOCKADDR 0x000000000000003f, LOCK polls n/n; GPU_CMD 0x00003304, GPU_INT_RAWSTAT 0x… before the clear, 0x… after, CLEAN_CACHES_COMPLETED after n polls of 1 us; UNLOCK polls n/n
t6_mmu: AS0 enable: TRANSTAB 0x00000000{P} MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG 0x00000000420001c6 written; UPDATE polls n/n
t6_mmu: AS0 after: TRANSTAB 0x00000000{P} MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG 0x00000000420001c6 LOCKADDR 0x000000000000003f STATUS 0x00000000 FAULTSTATUS 0x00000000 FAULTADDRESS 0x0000000000000000
t6_mmu: IRQ after: MMU_INT_RAWSTAT 0x00010000 MMU_INT_MASK … GPU_INT_RAWSTAT 0x… GPU_INT_MASK …
t6_mmu: the flush after the map: LOCKADDR 0x000000000400000f, LOCK polls n/n; GPU_CMD 0x00000304, GPU_INT_RAWSTAT 0x… before the clear, 0x… after, CLEAN_CACHES_COMPLETED after n polls of 1 us; UNLOCK polls n/n
t6_mmu: PASS: AS0 on TRANSTAB 0x00000000{P}, three cache flush round trips, N ms
t6: t6_mmu done: PSCI SYSTEM_RESET at N ms (F feeds)
```

then the DDR loader, U-Boot and Armbian; `nanopc-t6.sh` prints "back in
Armbian N s after the jump" (45 s for `t6_hello`).

What each value is, and what a wrong one would mean:

- **The two lead lines.** `build <describe>` names the checkout the
  result belongs to; the WDT0 line is Z1's arming — period 22369 ms as
  measured, fed until 60000 ms of uptime as the lifetime says.
- **`Z1: rc 0 …`.** As `t6_gpu` measured it. Anything else stops the
  run there: Z1's record follows, the lines `t6_gpu` prints (the
  hand-over, the clock set, the supply, each step reached) under
  `t6_mmu:`, then `t6_mmu: Z1 stopped: stop N (name), refusal R, detail
  0x…` and the FAIL line (`tools/probes/nanopc_t6/common/t6_z1.c`;
  `t6_fw` and `t6_submit` print the same record after their Z3 stop
  line). A Z1 or hand-over regression, not a Z2 one.
- **`AS0 as found`.** `TRANSTAB`, `MEMATTR`, `STATUS`, `FAULTSTATUS`
  and `FAULTADDRESS` 0 after Z1's soft reset; `TRANSCFG` and `LOCKADDR`
  recorded, not predicted (Armbian's panthor had AS0 enabled, and how
  far a soft reset reaches into the AS registers is not measured;
  `LOCKADDR` read 0xe on 2026-11-11, the smallest region, 2^15 bytes
  at base 0, and no source gives its reset value). A
  non-zero `TRANSTAB` or `MEMATTR` means the GPU is not in the state Z1
  reported: the hand-over, not the table builder.
- **The two IRQ lines.** `MMU_INT_RAWSTAT` 0 as found and 0x00010000
  after: bit 16 says AS0's commands completed (the enable's `LOCK`,
  `UPDATE` and `UNLOCK`), bits 15:0 are the faults (bit 0 AS0's) and
  stay 0. The image reads the word and never clears it. The masks 0 as
  the reset left them — recorded, never written. `GPU_INT_RAWSTAT` bit 17
  (`CLEAN_CACHES_COMPLETED`) must be clear before the first flush: a
  bit already set cannot be told from a completion, which is why the
  round trip refuses there. Bits 9/10 (`POWER_CHANGED`) may ride along
  in every raw-status field, set by Z1's L2 power-on. A raw status
  that never clears between the three flushes: the GPU does not take
  `GPU_CMD`.
- **The three flushes.** `GPU_CMD` 0x00003304 (L2 and LSC cleaned and
  invalidated) for the round trip and the enable, 0x00000304 (the L2
  only) for the flush after the map, in that order; the "after" raw
  status equals the "before" one, bit 17 clear. The poll counters are
  recorded; the prediction is a small count — every Z1 GPU wait
  completed in one 1 µs poll — always under 100 ms, after which the
  step refuses. Another `GPU_CMD` word is a wrong flush word.
- **`4 of 16 pages used`.** One 64 KiB section: one table page per
  level. The host test pins the 4
  (`from_a_reset_gpu_as0_comes_up_on_the_section_s_tables`); the
  16-page pool is sized for Z3's 9
  (`z3_s_firmware_sections_take_nine_table_pages`). A 4 that is not 4
  on a PASS is the builder or a different image, not the board.
- **The walk line.** `P+0x1003`, `P+0x2003`, `P+0x3003`: each level's
  table at the next pool page, its PA | 0b11. The L3 word is one entry
  of the descriptor table of "The format": write-back (`AttrIndx` 1),
  unprivileged, writable, inner shareable, `AF`, `nG`, both `XN`. A
  word differing in one bit field differs in exactly that row: 0xfc7
  would be the page read-only, an `…e43` word uncached and outer
  shareable, a high word of 0 a page left executable. A walk `PA` off
  `B+0x1234`, or fewer than 4 levels, never reaches the PASS record:
  the run stops at `stop 4` with the walked PA in `detail`.
- **`tables cleaned to the point of coherency: rc 0`.** One
  `sys_cache_data_flush_range` over the 4 used pages. An rc that is
  not 0 is `stop 5` with the rc as `detail`.
- **The enable's lines.** `LOCKADDR` 0x3f is `lock_region` over all of
  VA (log2 of its size, minus 1), 0x0400000f the section's 64 KiB;
  `TRANSTAB` = P, `MEMATTR` 0x8f8f8f8f8c4c8f4c (the board's live word;
  bytes 4-7 no PTE selects), `TRANSCFG` 0x420001c6
  (`ADRMODE_AARCH64_4K`, 48 VA bits, write-back table walks, non-
  coherent so no `SH`). A register that reads back other than written
  is `stop 7` naming it — `refusal` 1 `TRANSTAB`, 2 `MEMATTR`, 3
  `TRANSCFG`, `detail` the value read: a dropped write, clocks or
  reset state, not the tables.
- **`AS0 after`.** The three written values again, `LOCKADDR` still
  0x3f (the enable's; the flush after the map rewrites it after this
  snapshot), `STATUS` 0 (bit 0 `AS_ACTIVE` is the busy bit every
  `AS_COMMAND` waits clear — panthor's `wait_ready`), `FAULTSTATUS`
  and `FAULTADDRESS` 0 and no fault bit in `MMU_INT_RAWSTAT`: nothing
  walked AS0. `stop 8` (Fault) refuses those same fields: refusal 1
  `AS_ACTIVE` stuck after the `UPDATE`, 2 a `FAULTSTATUS` set though
  nothing walks AS0 before the MCU, 3 a fault bit of `MMU_INT_RAWSTAT`
  (`detail` its bits 15:0; bit n with n > 0 is another address space's
  — Linux's AS1 state surviving the jump).
- **`PASS` and the reset.** `TRANSTAB` = P read once more; `N ms` is
  the wall time from before `yd_t6_mmu_up`, and the budgets of "the
  way back" below keep it in single seconds. `F feeds` counts the
  watchdog feeds before the PSCI reset.

A stopped run prints, between the same banner and reset:

```
t6_mmu: FAIL: stop N (<step>), refusal R, detail 0x…
```

with the stop names of `main.c` (`YD_T6_MMU_STOP_*` and `YD_MALI_*` in
`yakogl_drivers.h`). Each stop means:

- **1 Features** — Z1's `MMU_FEATURES` (`detail`) is not 48 VA / 40 PA
  bits. Nothing was written (a host test holds the run to that): the
  GPU is not the one these tables were sized for.
- **2 Flush** — `detail` the `GPU_INT_RAWSTAT` at the stop: bit 17
  found set before the clear (the refusal exists exactly because a
  completion could then not be told from an old one) or never raised
  within the 100 ms (`GPU_CMD` not executed at all: clocks, PD_GPU,
  L2).
- **3 Tables** — `refusal` the `pgtable::PtRefusal`, `detail` the
  refused VA. With one section in a 16-page pool it cannot happen; it
  would mean the addresses handed over are not the ELF's of step 1.
- **4 Walk** — `detail` the PA the walk reached, 0 if unmapped: the
  pool does not hold the tables of the walk line (alignment, the
  pool's PA, the identity mapping).
- **5 Clean** — `detail` the rc of the table clean.
- **6 Enable** — the enable's sequence stopped: a `wait_ready` over its
  100 ms or a flush refused; `refusal` the mali refusal, `detail` the
  register value it timed on — the MMU is not accepting commands.
- **7 ReadBack**, **8 Fault** — as read in the two lines above.
- **9 FlushPt** — the flush after the map stopped like 2 or 6, in its
  own step.

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

**The runs of 2026-11-11.** Four jumps by kexec out of Armbian (kernel
6.18.55), records in `T6_RECORD_DIR`. The first two are the same image,
`t6_mmu` build v0.1.0-897-g3aa24a68d (main `3aa24a68d`, sha256
bd5fc04c…f69431e): both are FAIL and stay FAIL. The third is the image
with both corrections, build v0.1.0-913-g1f19113a3 (main `1f19113a3`,
sha256 1c2a83d6…fa178f5): PASS. The fourth is Z1's image, `t6_gpu` build
v0.1.0-915-g1bac19568 (main `1bac19568`, sha256 6d8eef52…c5f1cfc),
started from the first run's hand-over: PASS. The board came back to
Armbian through the image's PSCI reset every time (31 s, 30 s, 30 s and
30 s after the jump, `GLB_RST_ST` 0 before and after).

1. **13:13 JST, record `20261111-131354-t6`: stopped in Z1's power-on.**
   `VERDICT t6_mmu FAIL (14 of 27)`. The image's lines:

   ```
   t6_mmu: Z1: rc -5 stop 5 refusal 3: GPU_ID 0x00000000 MMU_FEATURES 0x00000000 AS_PRESENT 0x00000000 L2_READY 0x0
   t6_mmu: FAIL: Z1's power-on stopped (stop 5, refusal 3, detail 0x00000000000a9fff); run t6_gpu for its lines
   ```

   No Z2 step ran. The cause is a hand-over this image did not handle:
   Linux had left the GPU clock on NPLL and NPLL powered down ("The GPU
   power-on", "Measured, 2026-11-11"). The power-on now powers the PLL
   and waits for its lock (step 3 there).
2. **13:20 JST, record `20261111-132001-t6`: every Z2 step ran, the image
   stopped on its own judgement.** `VERDICT t6_mmu FAIL (2 of 27)`
   against the corrected prediction above: the flush after the map and
   the PASS line are missing. This jump started from the reboot of the
   first one, with the GPU clock back on CPLL over 5
   (`CLKSEL_CON(158)` 0x00001f24), and Z1 passed. The image's lines:

   ```
   t6_mmu: Z1: rc 0 stop 0 refusal 0: GPU_ID 0xa8670005 MMU_FEATURES 0x00002830 AS_PRESENT 0x000000ff L2_READY 0x1
   t6_mmu: pool 0x10023000 at PA 0x10023000 (16 pages), backing 0x10013000 at PA 0x10013000
   t6_mmu: AS0 as found: TRANSTAB 0x0000000000000000 MEMATTR 0x0000000000000000 TRANSCFG 0x0000000000000001 LOCKADDR 0x000000000000000e STATUS 0x00000000 FAULTSTATUS 0x00000000 FAULTADDRESS 0x0000000000000000
   t6_mmu: IRQ as found: MMU_INT_RAWSTAT 0x00000000 MMU_INT_MASK 0x00000000 GPU_INT_RAWSTAT 0x00000600 GPU_INT_MASK 0x00000000
   t6_mmu: flush round trip: GPU_CMD 0x00003304, GPU_INT_RAWSTAT 0x00000600 before the clear, 0x00000600 after; CLEAN_CACHES_COMPLETED after 2 polls of 1 us
   t6_mmu: tables at PA 0x0000000010023000: 4 of 16 pages used; backing PA 0x0000000010013000
   t6_mmu: walk VA 0x0000000004001234: L0 0x0000000010024003 L1 0x0000000010025003 L2 0x0000000010026003 L3 0x0060000010014f47 (4 levels): PA 0x0000000010014234
   t6_mmu: tables cleaned to the point of coherency: rc 0
   t6_mmu: AS0 enable, the whole-VA flush: LOCKADDR 0x000000000000003f, LOCK polls 0/1; GPU_CMD 0x00003304, GPU_INT_RAWSTAT 0x00000600 before the clear, 0x00000600 after, CLEAN_CACHES_COMPLETED after 2 polls of 1 us; UNLOCK polls 0/1
   t6_mmu: AS0 enable: TRANSTAB 0x0000000010023000 MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG 0x00000000420001c6 written; UPDATE polls 0/1
   t6_mmu: AS0 after: TRANSTAB 0x0000000010023000 MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG 0x00000000420001c6 LOCKADDR 0x000000000000003f STATUS 0x00000000 FAULTSTATUS 0x00000000 FAULTADDRESS 0x0000000000000000
   t6_mmu: IRQ after: MMU_INT_RAWSTAT 0x00010000 MMU_INT_MASK 0x00000000 GPU_INT_RAWSTAT 0x00000600 GPU_INT_MASK 0x00000000
   t6_mmu: FAIL: stop 8 (AS0 is active or faulted after the update), refusal 3, detail 0x0000000000010000
   ```

   Measured for the first time on the board: the cache flush completes
   (2 polls of 1 µs), the tables take 4 pages and walk to `B+0x1234`,
   AS0 takes `LOCK`, `UPDATE` and `UNLOCK` without a wait and reads back
   what was written, with no fault recorded. The stop is the image's:
   it took `MMU_INT_RAWSTAT` 0x00010000 for a fault of AS0. On a CSF
   GPU bits 15:0 of that word are the faults and bits 31:16 say "command
   completed" per address space (panthor_mmu.c,
   `panthor_mmu_fault_mask`); bit 16 is AS0's `LOCK`/`UPDATE`/`UNLOCK`
   done. `board::nanopc_t6_mmu` now refuses only bits 15:0, and the two
   wrong predictions (`LOCKADDR` as found, `MMU_INT_RAWSTAT` after) are
   corrected above for the next run.
3. **21:01 JST, record `20261111-210127-t6`: PASS.** `VERDICT t6_mmu
   PASS`: every line of the prediction above was seen. The hand-over
   was as in the second run: the GPU clock on CPLL over 5
   (`CLKSEL_CON(158)` 0x00001f24); NPLL was powered down (`PLL_CON1`
   0x00002083) and not selected. The image's lines up to `IRQ after` read as in the second
   run, value for value (`LOCKADDR` as found 0xe, each flush done after
   2 polls of 1 µs, `MMU_INT_RAWSTAT` 0x00010000 after the enable). Then,
   in place of the stop:

   ```
   t6_mmu: the flush after the map: LOCKADDR 0x000000000400000f, LOCK polls 0/1; GPU_CMD 0x00000304, GPU_INT_RAWSTAT 0x00000600 before the clear, 0x00000600 after, CLEAN_CACHES_COMPLETED after 2 polls of 1 us; UNLOCK polls 0/1
   t6_mmu: PASS: AS0 on TRANSTAB 0x0000000010023000, three cache flush round trips, 13 ms
   t6: t6_mmu done: PSCI SYSTEM_RESET at 16 ms (0 feeds)
   ```

   Measured for the first time on the board: with AS0 enabled, the
   `LOCK` of the mapped 64 KiB, the L2-only flush and the `UNLOCK` take
   no wait, and Z2's sequence with its prints takes 13 ms. Not shown by
   this run: Z1's PLL power-on (the clock's PLL was running at
   hand-over; the fourth run shows it), and what Z2 never shows ("What
   Z2 does not show": the GPU's own walk of these tables is Z3's).
4. **21:36 JST, record `20261111-213650-t6`: Z1 from a powered-down
   NPLL, PASS.** `VERDICT t6_gpu PASS`. The hand-over of the first run
   was made on purpose. Under Armbian, with the GPU runtime suspended
   and no user of the GPU device, `1000000000` was written to the GPU
   devfreq's `min_freq` (`/sys/class/devfreq/fb000000.gpu/min_freq`);
   `clk_summary` then showed `clk_gpu_src` at 850 MHz from `npll` and
   `pll_npll` with enable count 0. It is a sysfs write, not a GPU load:
   no load was run. The runner's "gpu power before" line read
   `CLKSEL_CON(158)` 0x00001f60, NPLL `PLL_CON1` 0x00002083 and
   `PLL_CON6` 0x00004000, `MODE_CON0` 0x00000545. The image's lines of
   the power-on:

   ```
   t6_gpu: PD_GPU at hand-over: PWR_GATE 0x0000ffff STATUS 0x2457ffff BUS_IDLE_REQ 0x0000e3cf ACK 0x000affff ST 0x000affff REPAIR_STATUS 0xfad40001: off or idle
   t6_gpu: CRU at hand-over: CLKSEL_CON(158) 0x00001f60 CLKSEL_CON(159) 0x00000000 CLKGATE_CON(66) 0x000000d6 MODE_CON0 0x00000545 PLL_CON 0x000001a9 0x00002083 0x00000000: clk_gpu_src 850000000 Hz
   t6_gpu: RK806 at hand-over: POWER_EN0 0x0f POWER_SLP_EN0 0xe0 BUCK1_CONFIG 0xa4 BUCK1_ON_VSEL 0x30: vdd_gpu_s0 on, 800000 uV
   t6_gpu: GPU PLL on: PLL_CON6 0x00004000 at hand-over, was on 0, waited 10 us, PLL_CON6 0x0000a400
   t6_gpu: PD_GPU on: gates opened 0xd2, was on 0, waited 6/1/0 us
   t6_gpu: PD_GPU after: PWR_GATE 0x0000fffe STATUS 0x2457fffe BUS_IDLE_REQ 0x0000e3ce ACK 0x000afffe ST 0x000afffe REPAIR_STATUS 0xfad40003: on
   t6_gpu: soft reset: GPU_INT_RAWSTAT 0x00000100 before the clear, 0x00000000 after; RESET_COMPLETED after 1 polls of 1 us
   t6_gpu: L2 mask 0x1: L2_READY 0x0 before L2_PWRON, 0x1 after; waited 0/0 polls
   t6_gpu: PASS: GPU_ID 0xa8670005, L2 ready, 16 ms
   ```

   Measured for the first time on the board: after the image's write
   that powers NPLL (`PLL_CON1` := 0x20000000), the lock (`PLL_CON6`
   bit 15) reads set after the first 10 µs wait; with the PLL running,
   PD_GPU's bus leaves idle where the first run stopped. After the
   reboot the GPU clock is
   back on CPLL (`CLKSEL_CON(158)` 0x00001f24) and devfreq's `min_freq`
   is its default. Not shown by this run: a hand-over reached by a real
   GPU load under Linux (the clock, PLL and supply words are the first
   run's, which devfreq's own choice had produced), and the sign-off
   form.

The reset line reads `PSCI SYSTEM_RESET at 14 ms (0 feeds)` in the first
two runs, `at 16 ms (0 feeds)` in the third and `at 18 ms (0 feeds)` in
the fourth: the count is the 1 s timer's feeds of WDT0 (`nanopc_t6.c`),
and the image was done before the first tick.

**Recording the result.** A kexec run is a development result, as Z1's
was (its sign-off form — U-Boot starting the image — has not run). The
build line, the PASS or FAIL line and the record reference go into Z2's
row in `tools/cts/RESULTS.md` (current results only: the run replaces
the row, no run log) and the summary in `tools/cts/STATUS.md`;
`uart.log` and the record directory stay out of the tree, in
`T6_RECORD_DIR`. The run is reported to #7 as a comment naming the
build and the record, and every new defect opens an issue, both posted
with the run. A run that did not happen is not a row: a gap is never a
pass.

### Z3 board procedure

Run on 2026-11-12 11:41 JST: **PASS** (`VERDICT t6_fw PASS`, 31 of 31
lines of the predicted record seen; the image of main `988639752`,
`t6_fw: build v0.1.0-986-g988639752`). The values the prediction left
to the board: `boot_waited_us` 43, `init_ack_us` 32, `ping_ack_us` 5,
`job_raw` 0x80000000, `cs0` features 0x0007085f (96 work registers, 8
scoreboards), `glb_req` and `glb_ack` 0x0000050e; the firmware did not
move the control block's offsets or strides. The row and the reading
are in `tools/cts/RESULTS.md` ("Mali: NanoPC-T6 on Zephyr").

The procedure below was written 2026-11-05 from `board::nanopc_t6_fw`
and the host tests over the MCU model, before any run, and is kept as
the way to run it again. A run whose record differs is a finding:
record it as it came, never by changing the prediction. Z3 runs only
after Z2 has passed, and reports to #7 (milestones: #26).

**Preconditions** — all of them, or no run:

- The T6 and `resource:<board-uart>`, free; the named hand (`T6_HAND`:
  who power-cycles the board if it does not come back; the run does not
  wait for that person), the read-only UART capture,
  the WDT_EN proof, `GLB_RST_ST` before and after, and `--go` taking the
  board lock are Z1's procedure as the Z2 one above states them.
- Z2's board run has passed and is recorded (its row in
  `tools/cts/RESULTS.md`): AS0 is the step Z3 stands on.
- The `t6_fw` image built from a detached checkout of a commit that is on
  `origin/main` (the image prints it on its first line, `t6_fw: build
  <describe>`; a result row names that build), in the west workspace as
  for Z2:

  ```sh
  west build -p always -b nanopc_t6 -d build/t6_fw yakogl/tools/probes/nanopc_t6/t6_fw
  ```

- The firmware header made on the building host and never committed:
  `tools/cts/deps-env.sh --build MALI_CSFFW` fetches the board's
  `mali_csffw.bin` (sha256 `a27847ea…a2405b`, "The firmware (Z3)"),
  `driver/tools/mkmalifw.sh <file>` writes the gitignored
  `driver/src/fw/mali_csffw_blob.h`. The probe prints the blob's size,
  sha256 and magic before it touches the GPU: they must be 282624 bytes
  and the digest above, or the result is not Z3's. The gate's `t6_fw`
  row is a gap without `ZEPHYR_WS` or the header, and a gap is not the
  precondition met.

**Run** — as Z2's, with the `t6_fw` image: ask who uses the board;
`nanopc-t6.sh <t6_fw zephyr.bin>` (the dry run) and read its plan;
`nm build/t6_fw/zephyr/zephyr.elf | grep 't6_fw_'` for the pool, the
section backings and the `__nocache` shared section (the lines
`t6_fw: section n: VA …, backing … at PA …` print the same, and the
addresses are identity mapped under 4 GiB, as for Z2); then
`T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=$BOARD_UART nanopc-t6.sh
<t6_fw zephyr.bin> --go`. The board's host, tty and record directory
come from the environment, never from this page. The WDT0 feeder
bounds every wait as for Z2; the MCU's boot is the longest new one,
1 s, and `csf::boot` adds at most the 100 ms of the two acknowledge
waits, so the run stays inside the feeder's 5 s between feeds when the
probe feeds around `yd_t6_fw_up`.

**Predicted record.** Z1's and Z2's lines are as the Z2 procedure and
"Closed on main, host side" say, except that Z2's map is the 7 sections
of the firmware and not Z2's one 64 KiB section. The values Z3 adds are
the fields of `FwUp` (`board::nanopc_t6_fw`, `struct yd_t6_fw_up`); a
`PASS` run holds `step` 0 (`STEP_NONE`), `stop` 0, `refusal` 0 and:

| `FwUp` field | Predicted | From |
|---|---|---|
| `image_version`, `sections`, `protected_skipped` | 0x01050000, 7, 1 | the header and `fw::parse` ("Host side, written 2026-10-28") |
| `tables_used`, `flushed` | 9 of 16 pool pages, 7 flushes (one per section: 896 KiB) | `every_section_comes_up_in_as0_on_nine_table_pages` |
| `clean_rc` | 0 for every clean | `McuMem::clean_rc` |
| `mcu_status_before`, `mcu_status` | 0 (disabled), then 1 (enabled) | "Predicted for the board run" |
| `job_raw` | `JOB_INT_RAWSTAT` bit 31 set (`JOB_INT_GLOBAL_IF`) | `start_mcu` |
| `boot_waited_us` | recorded, not predicted; far under the 1 s (1,000,000 µs) budget | the MCU's own boot time is not known from the model |
| `version` | 0x01050000 = **1.5.0**, equal to `image_version` | the board's dmesg under panthor; `the_boards_file_boots_over_the_model` |
| `features`, `instr_features` | 0, 0x71 | the image's own words |
| `group_num`, `group_stride` | 8, 0xa0 | the image's control block |
| `glb_input`, `glb_output` | 0x04002000, 0x04006000 | as above |
| `csg0` (input, output, `stream_num`, `stream_stride`) | 0x04003000, 0x04007000, 8, 0xc | as above |
| `cs0` (input, output, `features`) | 0x04003400, 0x04007200, features recorded (work registers 96, scoreboards 8 under panthor) | as above |
| `core_en_mask` | 0x50005 (`SHADER_PRESENT` of Z1's reset) | `global_init` |
| `poweroff_timer`, `progress_timer`, `idle_timer`, `ack_irq_mask`, `init_req` | 235, 0x271000, 19, 0x0400050e, 0x40e | "Predicted for the board run" |
| `acked` | 0x10e: `GLB_INIT_TOGGLES` (0xe) and `GLB_PING` (0x100), all acknowledged | `csf::acked` |
| `init_ack_us`, `ping_ack_us` | recorded, not predicted; each under the 100 ms budget (100,000 µs) | the firmware's latency is not known from the model |
| `ping_acked` | 1 | `csf::ping` |
| `glb_req`, `glb_ack` | agree on the bits of `acked` | the toggle protocol |
| `gpu_id`, `shader_present`, `mmu_features` | as Z1 measured: 0xa8670005, 0x50005, 0x00002830 | `t6_gpu` |

Over the MCU model with the real file
(`cargo test -p yakogl-drivers --features mali --lib board::nanopc_t6_fw`,
`MALI_CSFFW` set; run 2026-11-05, 9 tests ok) the run asserts 7 sections,
1 protected skipped, 9 table pages, 7 flushes, GLB version 1.5.0 equal
to the header's and the PING acknowledged: those rows are host-measured; the rest
are the model's pinned values; the timings and the MCU's own `features` word are
the ones only the board can give. The board can differ from the model in
exactly these: the MCU's boot time, the acknowledge latencies, and
whether the firmware rewrites the control block's pages and strides.
The probe's own `t6_fw:` lines carry them.

**Expected uart2 record**, typed from the printk formats of
`tools/probes/nanopc_t6/t6_fw/src/main.c` (29756d0e), in the order the
probe prints them. `<…>` is a value the board gives (not predicted),
`…` the rest of a value that is not predicted; `%p` addresses are the
image's. Z1's and Z2's own lines are not repeated: `yd_t6_fw_up` runs them
inside and prints nothing until it returns.

```
t6_fw: build <describe>
t6_fw: WDT0 period <ms> ms, fed until <ms> ms of uptime
t6_fw: image 282624 bytes, 7 loaded sections, sha256 a27847ea…a2405b, magic 0x…
t6_fw: section 0: VA 0x00400000, 64 KiB, backing <p> at PA 0x<pa>
t6_fw: section 1: VA 0x00410000, 64 KiB, backing <p> at PA 0x<pa>
t6_fw: section 2: VA 0x00000000, 64 KiB, backing <p> at PA 0x<pa>
t6_fw: section 3: VA 0x00800000, 128 KiB, backing <p> at PA 0x<pa>
t6_fw: section 4: VA 0x02000000, 256 KiB, backing <p> at PA 0x<pa>
t6_fw: section 5: VA 0x01000000, 256 KiB, backing <p> at PA 0x<pa>
t6_fw: section 6: VA 0x04000000, 64 KiB, backing <p> at PA 0x<pa>
t6_fw: pool <p> at PA 0x<pa> (16 pages)
t6_fw: Z3: rc 0 step 0, <ms> ms
t6_fw: Z3: image version 0x01050000, 7 sections loaded, 1 protected skipped, 9 table pages, 7 flushes
t6_fw: Z3: GPU_ID 0xa8670005 SHADER_PRESENT 0x0000000000050005 MMU_FEATURES 0x00002830 shared PA 0x…<pa> clean rc 0
t6_fw: Z3: MCU_STATUS 0 before, 1 after, booted in <us> us, JOB_INT_RAWSTAT 0x8…
t6_fw: Z3: GLB version 1.5.0 (0x01050000), features 0x00000000, instr 0x00000071, 8 groups stride 0xa0
t6_fw: Z3: GLB input 0x04002000 output 0x04006000, CSG0 in 0x04003000 out 0x04007000 8 streams stride 0xc, CS0 in 0x04003400 out 0x04007200 features 0x<recorded>
t6_fw: Z3: CORE_EN 0x0000000000050005, poweroff 0x000000eb progress 0x00271000 idle 0x00000013, ack_irq_mask 0x0400050e
t6_fw: Z3: init req 0x0000040e acked 0x0000010e in <us> us, PING acked in <us> us, GLB req 0x… ack 0x…
t6_fw: PASS
```

(Reading the format strings: `shader_present` and `core_en_mask` print
as two 32-bit words, high first, so 0x50005 is `0x0000000000050005`;
`poweroff_timer` 235 prints as `0x000000eb`; `job_raw` has bit 31 set, so
it reads `0x8…`; the section lines list the backings in the order of
`fw::Image::sections()`, not by VA, which is why VA 0x00400000 is
section 0. The model's counts, not the board's: 7 sections, 896 KiB, 9
table pages, 7 flushes.) `PASS` needs `rc` 0, `ping_acked` and
`version` 0x01050000 together (`main.c`); the same run on the host over
the MCU model is `cargo test -p yakogl-drivers --features mali --lib
board::nanopc_t6_fw`: `test result: ok. 9 passed; 0 failed; 0 ignored;
0 measured; 566 filtered out` (2026-11-05).

**What the probe prints instead of the record when the run fails.** The
`t6_fw: Z3: rc <rc> step <n>, <ms> ms` line is always printed; then:

| Failure | Lines that replace the record and PASS |
|---|---|
| `yd_t6_fw_up` refused its arguments (rc -EINVAL) | `t6_fw: FAIL: yd_t6_fw_up refused its arguments` |
| Step 1 to 4 (the image was refused, the backings do not fit, Z1's power-on stopped, Z2's AS0 stopped) | `t6_fw: Z3: stop <n> refusal <n> detail 0x<hi><lo> acked 0x<acked>`, then `t6_fw: FAIL: Z3 step <n> (<name>)` with `<name>` one of `the image was refused`, `the backings do not fit the sections`, `Z1's power-on stopped`, `Z2's AS0 stopped` |
| Step 5, `csf::boot` refused (the table above) | the same `stop … refusal … detail … acked …` line, then the seven `t6_fw: Z3:` record lines (as far as filled), then `t6_fw: FAIL: Z3 step 5 (csf::boot refused)` |
| Step 6, a cache clean of the load failed | as step 5, ending `t6_fw: FAIL: Z3 step 6 (a cache clean of the load failed)` |
| Step 7, GLB version is not the header's | the `stop …` line, the record lines, then `t6_fw: FAIL: Z3 step 7 (the GLB version is not the image's)` |
| `step` 0 but not PASS (no PING ack or a version other than 0x01050000) | the record lines, then `t6_fw: FAIL: Z3 step 0 (none): rc <rc>, PING not acked, GLB version 0x<version>` (`PING acked` when only the version is wrong) |

A run that prints no `t6_fw: Z3: rc` line at all stopped inside
`yd_t6_fw_up`: the watchdog's PSCI reset follows, and the capture ends
at the `t6_fw: pool` line.

**Failure signatures.** A refused run prints `step` (`STEP_*` of
`nanopc_t6_fw.rs`), `stop`, `refusal` and `detail` (lo, hi), and the PSCI
reset follows as for Z2. Steps 1 to 4 (`STEP_PARSE`, `STEP_PLACE`,
`STEP_GPU`, `STEP_MMU`) are the file, the image's backings, Z1 and Z2
refusing: read them with the Z1 and Z2 procedures. What Z3 adds is
`STEP_BOOT` (5) with `refusal` a `csf::CsfRefusal`, and the two after it.
panthor v6.18 `drivers/gpu/drm/panthor/`, lines as cited elsewhere on
this page:

| Signature in the record | `FwUp` step, refusal (`detail`) | What it means; panthor v6.18 |
|---|---|---|
| `mcu_status_before` not 0 | `STEP_BOOT`, `McuNotDisabled` (2): the status read | The MCU was not disabled after Z1's reset; panthor starts only a disabled MCU (`panthor_fw_start`, `panthor_fw.c:1028-1059`). |
| `JOB_INT` raw bit 31 set before the start | `STEP_BOOT`, `BootIrqStuck` (3): the raw word | An old event; the start's acknowledge could not be told from it (`panthor_fw.c:1013` clears what the handler saw). |
| MCU never halts nor boots: no `JOB_INT` bit 31 within 1 s, `mcu_status` still 0 | `STEP_BOOT`, `BootTimeout` (4): `MCU_STATUS` (`mcu_status` repeats it) | The MCU did not start: the sections it fetched are not what the CPU wrote (the clean, `clean_rc`, the AS0 map's attributes, the uncached shared section), or its clocks/power. panthor: "Failed to boot MCU" and `MCU_STATUS` read after a timeout, `panthor_fw.c:1039-1054`. |
| MCU halts: `mcu_status` 2 (halt) or 3 (fatal) with bit 31 set | not refused: `step` 0 or `STEP_VERSION`; read `mcu_status` | The MCU ran and stopped; the record carries the status for the verdict (`fault_mcu_status_not_enabled_is_reported_not_refused`). panthor reads the status only after a timeout (`panthor_fw.c:1039-1054`). A fault the MCU took on AS0 shows in Z2's fault fields: `FAULTSTATUS`, `MMU_INT_RAWSTAT`. |
| GLB version 0 | `STEP_BOOT`, `VersionZero` (5): 0 | The MCU did not run far enough to write its version, or the shared section is not coherent for the CPU: "Firmware version is 0. Firmware may have failed to boot" (`panthor_fw.c:944-947`). |
| Wrong GLB version (not 0x01050000) | `STEP_VERSION` (7): `detail` the version read, `image_version` the header's | A different firmware build or the CPU reading a stale shared section. panthor refuses only version 0 and logs the others (`panthor_fw.c:969-974`); ours refuses a version that is not the header's. |
| Interface page or block outside the shared section | `STEP_BOOT`, `IfaceOutsideShared` (6): the MCU address | A control block read from a section that is not coherent, or a rewritten page (`panthor_fw.c:886`, `:835`). |
| `group_num` or `stream_num` out of panthor's counts, or slots differing | `STEP_BOOT`, `GroupCount` (7), `StreamCount` (8), `CsgSlotsDiffer` (11), `CsSlotsDiffer` (12): the count, the group, or the group · 256 + stream | The control block is not the image's (the model's values: 8 groups, 8 streams); panthor: "Invalid number of control groups", "Expecting identical CSG/CS slots" (`panthor_fw.c:202-203`, `:886`). |
| CFG ack missing: `acked` without some of 0xe, `init_ack_us` unset | `STEP_BOOT`, `GlbConfigAckTimeout` (9): the bits that were acknowledged (`acked`) | The firmware is up (version read) but does not take the global configuration, or the CPU's write of `req` is not seen by the MCU (the shared section cached for the CPU). panthor does not wait here; its next request would time out (`panthor_fw.c:978-1009`). |
| PING ack missing: `acked` 0xe, `ping_acked` 0 | `STEP_BOOT`, `PingTimeout` (10): 0 | The firmware took the configuration but does not answer `PING` within 100 ms, or the doorbell did not reach it. panthor: "FW ping timeout, scheduling a reset" (`panthor_fw.c:1313-1335`). |
| A section's clean failed | `STEP_CLEAN` (6): the first rc (`clean_rc`) | The boot ran, but `sys_cache_data_flush_range` failed over a section; the run's other fields stand. panthor's equivalent is the sync to the device, `dma_sync_sgtable_for_device` (`panthor_fw.c:624-626`). |
| `SectionUnmapped` | `STEP_BOOT`, `SectionUnmapped` (1): the MCU address | AS0 does not cover a section: Z2's map is missing or short (the placement is checked before, so this is the image, not the run). |

A refusal not in the table is read from `refusal` against
`csf::CsfRefusal`; a `step` not in the table against `STEP_*`. The 100 ms
waits are the host's budgets (the 1 s boot wait is panthor's and the
configuration wait is ours; panthor does not wait there), so a timeout
here is a finding, not a defect of the prediction.

**Recording the result.** As Z2's: the build line and the PASS or FAIL
line into Z3's row in `tools/cts/RESULTS.md` and the summary in
`tools/cts/STATUS.md`, `uart.log` and the record directory out of the
tree, one comment for #7 naming the build and the record, a new issue
for each defect, posted with the run. A run that did not happen is not a
row.

### Z4 board procedure

**Run twice on 2026-11-12; the second run passed.** Written 2026-11-09
from the Z3 procedure above, before any run. The first run (the image of
main `4f618d77e`) drew the frame and was recorded as it came: FAIL, 3 of
its 40 predicted lines differed. In all three the prediction was wrong,
not the driver (issue #4515), and the model, the record and the verdict
were corrected from that record. The second run (the image of main
`ef86672bf`) passed: `VERDICT t6_submit PASS`, 42 of 42 lines
(`tools/cts/RESULTS.md`). Below, a value says "measured" where a run
gave it. A run whose record differs is a finding: record it as it came,
never by changing the prediction. Z4 runs only after Z3 has passed, and
reports to #7 (milestones: #26).

**What Z4 does.** It brings up Z3 (the firmware boot, AS0, the MCU
acknowledging `PING`), starts CSG 0 / CS 0 from our kernel side
(`gpu::mali::sched`), creates the tiler heap (`gpu::mali::heap`), submits
the L1 frame that `l1::build_triangle_frame` (`src/mali/frame.rs`) builds,
at the Z4 VAs, and waits on the sync object the frame signals; it returns
one `#[repr(C)]` record. [The Z4 frame against the
trace](#the-z4-frame-against-the-trace) compares the frame's bytes with
the panthor / Mesa trace: read it first, it names the three choices that
are not part of Z4.

**Preconditions** — all of them, or no run:

- The T6 and `resource:<board-uart>`, free; the named hand (`T6_HAND`:
  who power-cycles the board if it does not come back; the run does not
  wait for that person), the read-only UART capture,
  the WDT_EN proof, `GLB_RST_ST` before and after, and `--go` taking the
  board lock are Z1's procedure as the Z2 one above states them.
- Z3's board run has passed and is recorded (its row in
  `tools/cts/RESULTS.md`): the booted firmware is the step Z4 stands on.
- The `t6_submit` image built from a detached checkout of a commit that
  is on `origin/main` (the image prints it on its first line, `t6_submit:
  build <describe>`; a result row names that build), in the west
  workspace as for Z3:

  ```sh
  west build -p always -b nanopc_t6 -d build/t6_submit yakogl/tools/probes/nanopc_t6/t6_submit
  ```

- The firmware header made on the building host and never committed,
  exactly as for Z3: `tools/cts/deps-env.sh --build MALI_CSFFW`, then
  `driver/tools/mkmalifw.sh <file>` writes the gitignored
  `driver/src/fw/mali_csffw_blob.h`. The probe prints the blob's size,
  sha256 and magic before it touches the GPU: 282624 bytes and sha256
  `a27847ea…a2405b`, or the result is not Z4's.
- The gate's row `zephyr  nanopc_t6 t6_submit image (Z4), -Werror on our
  C files` in `tools/check.sh` is green for that commit, not a gap. It
  is a gap without `ZEPHYR_WS` or the firmware header, and a gap is not
  the precondition met.

**Run** — as Z3's, with the `t6_submit` image: ask who uses the board;
`nanopc-t6.sh <t6_submit zephyr.bin>` (the dry run) and read its plan;
`nm build/t6_submit/zephyr/zephyr.elf | grep 't6_submit_'` for the pool,
the section backings and the `__nocache` shared section (the probe prints
the same, identity mapped under 4 GiB as for Z3); then
`T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=$BOARD_UART nanopc-t6.sh
<t6_submit zephyr.bin> --go`. The board's host, tty and record directory
come from the environment, never from this page. The WDT0 feeder bounds
every wait as for Z2 and Z3: the probe feeds around the Z4 call, and a
wait that does not finish inside the feeder's window ends in the
watchdog's PSCI reset, not in a hang.

**The runner's commands** (every one is in
`driver/tools/kexec-zephyr/`; the runs of 2026-11-12 made steps 3 and 4
through `tools/board.sh nanopc-t6 run`, which calls the same runner and
verdict and keeps the record). The runner is `nanopc-t6.sh`; it is
image-agnostic and has no `t6_submit` case: it kexecs any `zephyr.bin`
that carries the `yakogl-wdt: nanopc_t6` marker. `BOARD_HOST`,
`BOARD_UART`, `T6_RECORD_DIR` and the hand's name come from the
environment, never from this page.

```sh
# 1. build, as the gate does (zephyr_werror_image in tools/check.sh), in
#    the west workspace; the firmware header is in place (above)
mods=$(PATH="$ZEPHYR_WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$ZEPHYR_WS")
(cd "$ZEPHYR_WS" && PATH="$ZEPHYR_WS/.venv/bin:$PATH" ZEPHYR_BASE="$ZEPHYR_WS/zephyr" \
  west build -p always -b nanopc_t6 -d "$PWD_YAKOGL/build/t6_submit" \
  "$PWD_YAKOGL/tools/probes/nanopc_t6/t6_submit" -- \
  "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$PWD_YAKOGL" -DYAKOGL_WERROR=ON)

# 2. dry run: checks the image on this host and the board read only,
#    prints the plan, changes nothing anywhere
BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/nanopc-t6.sh build/t6_submit/zephyr/zephyr.bin

# 3. the jump, once the preconditions hold
BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=$BOARD_UART \
  driver/tools/kexec-zephyr/nanopc-t6.sh build/t6_submit/zephyr/zephyr.bin --go

# 4. judge the capture the run wrote
driver/tools/kexec-zephyr/t6-verdict.sh t6_submit <dir>/<date>-t6/uart.log
```

(`$PWD_YAKOGL` is the detached checkout's root.) The `--go` run captures
`uart2` itself: `T6_UART` is only ever read, as `uart.log` in the run's
record directory `<dir>/<date>-t6/`, from before the jump until the board
is back in Armbian, next to `record.txt` and the two `dmesg` files; there
is nothing to capture separately. The image is copied to
`/tmp/yakogl-kexec` on the board, which the next boot or run wipes. Nothing
else is staged on the board, no boot-loader variable is set and the runner
has no per-probe variable: the flow changes no persistent board state, and the
runner prints no undo because there is none to print.

**The safety net**, as the scripts implement it, the same for `t6_submit`
as for `t6_fw`:

- The image is refused (exit 3, nothing copied) without exactly one
  `yakogl-wdt: nanopc_t6` marker and the `board: watchdog armed` line, with
  a TOP outside 11..15, or linked outside the window; the dry run
  shows it. `--go` is refused without `T6_HAND`, `T6_RECORD_DIR` and an
  unopened `T6_UART`, and refuses (it does not wait) while anyone holds the
  board lock, a kexec image is loaded, a process holds the GPU device or
  `/dev/watchdog0`, or remote sessions exist (unless `T6_ALLOW_SESSIONS=1`).
- Before the jump the runner opens `/dev/watchdog0`, reads WDT0's `WDT_CR`
  and jumps only when `WDT_EN` (bit 0) reads 1; otherwise it writes the
  magic close, does not jump and exits 98 (the run ends with exit 1).
  That open covers the image's pre-arm window; the image then re-arms WDT0
  itself and feeds it only while the probe's feeder window lasts.
- A hang, a stop or a wait on the sync object that never ends is ended by
  the image's PSCI reset or, if that fails, WDT0's own reset, no later than
  the bound the dry run prints; the runner waits that long plus 180 s for
  ssh and a new `boot_id`, then writes the registers (`GLB_RST_ST`, WDT0)
  and the new Linux's `dmesg` to the record.
- A board that does not come back is not retried: the runner records "no
  Armbian … s after the jump: the board needs `<T6_HAND>` at its power
  switch" and exits 1, and the named hand cuts the power. The same Linux
  still running 60 s after the jump is reported as "the jump did not
  happen", also exit 1.

**Predicted record.** Z1's, Z2's and Z3's lines are as the Z3 procedure
says; Z4 adds the fields of its own record (`board::nanopc_t6_submit`,
C entry `yd_t6_submit`), and a `PASS` run holds `step` 0, `stop` 0 and
`refusal` 0.

These are the **MCU model's values**: `board::nanopc_t6_submit`'s test
`the_generated_frame_s_record_is_as_predicted` runs the generated frame
over the host MCU model and asserts the record below. Where the first
board run (2026-11-12, issue #4515) showed the model wrong, the model now
does what the board's firmware did, and the row says "measured"; the
second run (the same day, PASS) measured the drain's rows. Every other
board-dependent value is the model's. The "pinned" ones are fixed by the frame and the module's
constants and must read the same on the board; the "board-dependent" ones
are the model's own (its register values, lags and microseconds, the
synthetic firmware's sizes): the model's value is given for orientation
and a different board value is not a finding. The record is
`struct yd_t6_submit_up` (432 bytes, offsets asserted in
`yakogl_drivers.h`).

| field | predicted value | pinned or board-dependent |
|---|---|---|
| `step`, `refusal`, `detail` | 0, 0, 0 | pinned (a pass) |
| `fw` | Z3's record whole (`tables_used` 7 on the model, 9 on the board's file; `ping_acked` 1) | as the Z3 procedure |
| `as_present` | 0xff | board-dependent (the board's `AS_PRESENT`; AS1 must be present) |
| `l2_features` | 0x07120306 | board-dependent |
| `as0_tables_used` | 8 (Z3's tables and one L3 for the queue's pages) | board-dependent (Z3's count plus 1: 10 on the board's file) |
| `vm_tables_used` | 10 | pinned (the group VM's L0; L1, L2 and three L3 for the 4 GiB block (queue, arenas, target); L1, L2 and two L3 for the kernel range (pool, chunk)) |
| `suspend_size`, `protm_suspend_size` | 25600, 25600 | pinned (measured: the board's firmware reports them, issue #4515; the model reports the same, so the host test maps both buffers, 7 pages each, after the user pages) |
| `heap_ctx` | 0x0000800000015000 | pinned (`HEAP_CTX_VA`, `HEAP_SET`'s operand) |
| `heap_chunk` | 0x0000800000a00000 | pinned |
| `heap_chunk_size` | 2097152 (2 MiB) | pinned |
| `heap_stride` | 64 | pinned (the G610's L2 line) |
| `csg_req`, `csg_ack` | 0x11, 0x11 | pinned |
| `cs_req`, `cs_ack` | 0x0511, 0x0011 | pinned (measured: `ack` 0x00000011 for `req` 0x00000511, so the firmware's `ack` carries the state field and bit 4 but not the idle bits 0x0500 of the request; issue #4515. Not decided: whether bit 4 is a copy of the request's `CS_EXTRACT_EVENT` or an extract event's toggle, both read 1; the model copies it) |
| `csg_ack_us`, `cs_ack_us` | 1, 0 | board-dependent (the model's acknowledge lag) |
| `latest_flush` | 0 | board-dependent (the GPU's flush id) |
| `sync_updates` | 0 | board-dependent (the `CSG_SYNC_UPDATE` events the wait acknowledged. The model's firmware raises the event after the stream wrote the `seqno`. Measured: 0 on the board's first run, 1 on the second: the event comes about when the wait ends, before or after its last look) |
| `insert`, `extract_at_done` | 128, 0 | `insert` pinned; `extract_at_done` board-dependent (the output page's `extract` right after the wait saw the `seqno`: the firmware publishes it later, measured 0 on both board runs) |
| `seqno`, `sync_status` | 1, 0 | pinned |
| `wait_us` | 20 | board-dependent (the model's job lag; bounded by the 5 s wait) |
| `status_cmd_ptr`, `info` | 0, 0 | pinned (a refused wait fills them) |
| `stop_us`, `terminate_us` | 3, 1 | board-dependent |
| `stop_refusal`, `terminate_refusal` | 0, 0 | pinned |
| `cleared[3]` | 0, 0, 0 | board-dependent (what the terminate found pending: CSG 0's events, CS 0's `cs_irq` bit, CS 0's events. On the model the drain has acknowledged the late `CSG_SYNC_UPDATE` and the `cs_irq` bit. Measured: 268435456 1 0 on the first run (`CSG_SYNC_UPDATE`; no drain in that image), 536870912 1 0 on the second (`CSG_IDLE`; the wait had taken the sync update, the drain took nothing)) |
| `red`, `blue` | 2048, 2048 | pinned (the model paints the frame's halves; the board's GPU must too) |
| `fill`, `other` | 0, 0 | pinned |
| `pixels` | 4096 | pinned |
| `digest` | 0xa4db81a9d39bfdcc | pinned (FNV-1a 64 of the blocks and the 53 stream words at `CS_VA`; the probe prints the same) |
| `extract_drained`, `drain_us` | 128, 10 | `extract_drained` pinned (measured: 128, `insert`'s value, on the second run); `drain_us` board-dependent (`sched::drained` after the wait: the model's one poll; measured 10 too, `extract` was there on the drain's second look) |
| `drain_refusal`, `drain_sync_updates` | 0, 1 | `drain_refusal` pinned (measured 0; 27 `DrainTimeout` is recorded, not a step; any other refusal is step 17, `STEP_DRAIN`); `drain_sync_updates` board-dependent (measured 0: the wait had taken the run's one sync update) |
| `drain_cs_ack` | 0x11 | pinned (measured: CS 0's `ack` right after the drain is 0x00000011, as before it: the firmware did not toggle `CS_EXTRACT_EVENT` when it published `extract`) |
| `extract_at_end` | 128 | pinned (measured: `extract` after the group's stop and terminate is still 128) |
| `drain_cs_irq_acks`, `drain_idle_acks` | 1, 0 | board-dependent (the drain's looks that acknowledged CS 0's `cs_irq` bit, and the `CSG_IDLE` events it acknowledged. Measured 0, 0: neither was pending during the drain's two looks; the terminate found both, `cleared` above) |

A record whose pinned field differs is a finding, recorded as it came.

**The firmware's publish of `extract`.** Measured on the first run: the
sync object's `seqno` is the stream's own write (its `SYNC_ADD64`), and
the wait saw it 220 µs after the doorbell with the output page's
`extract` still 0 and no `CSG_SYNC_UPDATE` (`sync_updates` 0); the
terminate then found `CSG_SYNC_UPDATE` and CS 0's `cs_irq` bit pending
(`cleared` 268435456 1 0). That record did not read `extract` again.
The second run did: after the wait the
probe runs `sched::drained` (polls until `extract` equals `insert`,
within `CS_ACK_WAIT_US`, acknowledging the events as the wait does) and
records the microseconds, the `extract` it ended with, its refusal, the
sync updates, `cs_irq` bits and `CSG_IDLE` events it acknowledged, CS 0's
`ack` after it, and `extract` once more after the group's end (Z4j,
Z4k). Measured on the second run: the wait saw the `seqno` 230 µs after
the doorbell, again with `extract` 0, and this time acknowledged one
`CSG_SYNC_UPDATE`; the drain's first look did not yet read `insert`'s
value, its second look, one `JOB_POLL_US` (10 µs) later, read 0x80,
equal to `insert`; the drain acknowledged nothing, CS 0's `ack` stayed
0x00000011, and `extract` read 0x80 again after the group's end, where
the terminate found `CSG_IDLE` and CS 0's `cs_irq` bit pending
(`cleared` 536870912 1 0). So the MCU firmware does publish `extract`,
a few microseconds after the stream's `seqno` (within one 10 µs poll in
this run), and `CSG_SYNC_UPDATE` is not tied to that publish: it came
after the wait in the first run and before the publish in the second.
The model publishes both `PUBLISH_LAG` reads after the `seqno` (the
instant publish is its other setting, tested too). panthor has no such
wait: it
takes completion from the sync object and ring space from its
scheduler's credits, and reads `output->extract` only when it programs
a slot and for its idle check.

**Time limit.** The wait on the sync object is bounded by `sched::wait`'s
`JOB_TIMEOUT_US`, panthor's `JOB_TIMEOUT_MS`: 5 s (5000000 µs), counted
from the call; on expiry the wait refuses (`step` 14, `refusal` 19,
`JobTimeout`) and the group is stopped and terminated. The model's job
takes 20 µs; a board job that needs more than a few milliseconds is a
finding before it is a timeout. The probe feeds WDT0 around the call
(`t6_watchdog_feed()` before and after `yd_t6_submit`, and in the
handover callback); the feeder's rule is as for Z2 and Z3 (`TOP` 13,
22.4 s period; the timer feeds only while the calls come at most
`ALIVE_S` = 5 s apart, and until `LIFETIME_S` = 60 s of uptime). Z3's
boot, the 5 s wait and the stop and terminate waits can together pass 5 s
between calls: the feeding then stops and WDT0 resets the board within
22.4 s of its last feed, at most 82.4 s after the arming, whatever the
image does. Whether the probe also feeds inside the wait is not
assumed here.

**Verdict.** `driver/tools/kexec-zephyr/t6-verdict.sh t6_submit
<uart.log>` judges the capture with the table `table_t6_submit` of that
script (it judged both board runs of 2026-11-12; its self-test,
`t6-verdict-test.sh`, runs it over fixtures). It looks for each line in order, each after the one
before it, and prints `ok`, `differs` or `missing` per line, then
`VERDICT t6_submit PASS` (exit 0) or `VERDICT t6_submit FAIL (<n> of <m>)`
(exit 1). The lines, as the table writes them, in the order `uart2` must
carry them:

1. Z1's lines as for `t6_fw`: the early banner `T6:A` to `T6:A3`, `T6:B`,
   the `yakogl-wdt: nanopc_t6` marker, the GIC line, the arming line,
   `T6:C` and `board: watchdog armed` (`table_boot`).
2. Z2's and Z3's lines as for `t6_fw`, with the prefix `t6_submit:`:
   build, WDT0, image, the 7 sections and the pool (`table_fw_image`),
   then `Z3: rc` and its seven record lines (`table_z3_record`). The Z3
   procedure above gives them; they are not repeated here.
3. The eleven Z4 lines (`SubmitUp`, 432 bytes; 64-bit words print as
   `%016llx`, 32-bit words as `%08x`, counts as `%u`), then `PASS`:

```
t6_submit: Z4a: step 0 refusal 0 detail 0x0000000000000000
t6_submit: Z4b: AS_PRESENT 0x000000ff L2_FEATURES 0x07120306 as0_tables_used {d} vm_tables_used 10 suspend_size 25600 protm_suspend_size 25600
t6_submit: Z4c: heap ctx 0x0000800000015000 chunk 0x0000800000a00000 chunk_size 2097152 stride 64
t6_submit: Z4d: CSG0 req 0x00000011 ack 0x00000011 CS0 req 0x00000511 ack 0x00000011, acked in {d}/{d} us
t6_submit: Z4e: insert 0x0000000000000080 extract 0x{x16} seqno 0x0000000000000001 sync_status 0x00000000, wait {d} us
t6_submit: Z4f: latest_flush 0x{x8} sync_updates {d} status_cmd_ptr 0x0000000000000000 info 0x0000000000000000
t6_submit: Z4g: stop {d} us refusal 0, terminate {d} us refusal 0, cleared {d} {d} {d}
t6_submit: Z4h: target red {@R} blue {@B} fill {@F} other {@O} of {@P} pixels
t6_submit: Z4i: frame digest 0xa4db81a9d39bfdcc
t6_submit: Z4j: drain {d} us refusal 0 extract 0x0000000000000080 sync_updates {d} cs_ack 0x00000011 cs_irq_acks {d} idle_acks {d}
t6_submit: Z4k: extract after the end 0x0000000000000080
t6_submit: PASS
```

The glue's `t6: t6_submit done: PSCI SYSTEM_RESET at {d} ms ({d} feeds)`
closes the capture. Tokens, explained once: `{d}` a decimal number
(optionally negative); `{x8}` exactly 8 lower-case hex digits (`{x16}`:
16: the early banner, `extract`); `{@R}`, `{@B}`, `{@F}`, `{@O}` and `{@P}` the
target's red, blue, fill, other and total pixel counts (`@` captures the value), each a decimal
number: the verdict adds the first four and compares the sum with `{@P}`.

**Pinned**, written out above and compared exactly: `step`, `refusal`
and `detail` 0 (Z4a: a stopped run differs here); `vm_tables_used` 10;
`suspend_size` and `protm_suspend_size` 25600;
`heap_ctx` 0x0000800000015000, the heap chunk 0x0000800000a00000,
`chunk_size` 2097152, stride 64; the CSG 0 request and acknowledge words
0x00000011, CS 0's request 0x00000511 and acknowledge 0x00000011;
`insert` 0x80, `seqno` 1, `sync_status` 0; `status_cmd_ptr` and `info` 0; the stop's and the
terminate's `refusal` 0; the frame digest 0xa4db81a9d39bfdcc; and, from
the second run, the drain's `refusal` 0, the `extract` it ended with
0x80, CS 0's `ack` 0x00000011 after it (Z4j) and `extract` 0x80 after
the group's end (Z4k). **Any value
of its form**, checked for presence and form only: `as0_tables_used` (the
board's file sizes Z3's tables: the model's 8 is not the board's, 10 is
predicted above), the timings (`acked in`, `wait`, `stop` and `terminate`
µs), `latest_flush`, `extract` after the wait, `sync_updates`, the three
`cleared` words and the drain's microseconds and event counts
(`sync_updates`, `cs_irq_acks`, `idle_acks`): these move with when the
firmware raises its events and publishes `extract` against our looks,
and the two runs read them differently; and the four pixel counts (only their sum is checked
against the total).

Where the verdict and the prediction table above differ in strength
(reported, the table is not changed): the table pins the red and blue
counts at 2048 each, while the verdict checks them for form only, so
read them against the table by eye; the verdict writes `AS_PRESENT` 0x000000ff and `L2_FEATURES`
0x07120306 as Z1 measured them: a board that reports other values makes
Z4b differ, a finding to record, not a defect of the verdict. Every pinned value in the list above agrees with the
table.

**A stopped Z4 run.** The probe prints its record anyway, with the
fields after the stop at 0, and in place of `t6_submit: PASS` the line
`t6_submit: FAIL: Z4 step N (name)`. Z4a then differs (step, refusal or
detail not 0), as do the fields the run left at 0, and the `PASS` line is
reported as differing; a `t6_submit: Z3: stop …` line counts as one more
differing line. A watchdog reset during Z4 is not a PASS either: the
capture ends after the last Z3 line with a fresh boot banner, and Z4a to
Z4k, `PASS` and the reset line are `missing`, the first of them Z4a (the
fixture `testdata/t6_submit-z4-reset.uart` of the self-test).

To judge a captured log, as step 4 of the runner's commands above (it
only reads the file):

```sh
driver/tools/kexec-zephyr/t6-verdict.sh t6_submit <dir>/<date>-t6/uart.log
```

**What a pass and a stop look like.** A pass is the probe's `t6_submit:
PASS` line with the verdict script reporting no failed line: the sync
object reached its value and the record's exception words are clear.
A stop is any of: a `t6_submit: FAIL: …` line naming the Z4 step; a
refusal of Z3's steps (read with the Z3 procedure); a record that ends
before the Z4 line, which means the call stopped inside and the
watchdog's PSCI reset follows (the capture ends at the last line printed
before it); or a wait on the sync object that times out. Every stop is
a finding. Whether the render itself is right is not Z4's pass: the
pixels are read back separately; a difference between the frame and the
trace is read against "The Z4 frame against the trace".

**Recording the result.** As Z3's: the build line and the PASS or FAIL
line into Z4's row in `tools/cts/RESULTS.md` and the summary in
`tools/cts/STATUS.md`, `uart.log` and the record directory out of the
tree, one comment for #7 naming the build and the record, a new issue
for each defect, posted with the run. A run that did not happen is not a
row.

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
from Zephyr; a UART / U-Boot session reads exactly this (issue #26 names them as
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
  The image already handles the states U-Boot could plausibly leave (PD_GPU off with
  vdd_gpu_s0 off or on, or PD_GPU powered with its idle request set; `board::nanopc_t6_gpu::gpu_up`,
  "The GPU power-on" step 3 and 5), each host-tested with a stop for every refusal.
  What decides: whether U-Boot's real state is one of those, whether it leaves PD_GPU
  powered with the idle request set, and how fast DCDC1 ramps (the image polls up to
  `rk806::DCDC1_RAMP_POLL_BUDGET_US`, a budget and not a fact). The design is in
  docs/notes/nanopc-t6-uboot-handover.md.
- **The GPU clock without Linux**: SCMI clock 5 against the CRU. In U-Boot read the CRU
  (0xfd7c0000) `CLKSEL_CON(158)` (0xfd7c0578; Linux leaves 0x1f24: `clk_gpu_src`
  CPLL / 5 = 300 MHz, PVTPLL bit 14 clear), `CLKGATE_CON(66)`, the CPLL and GPLL
  `PLL_CON` words, and `clk dump` (the rates U-Boot believes, SCMI clock 5
  included), at the prompt with nothing run before. The image refuses the PVTPLL
  and, when the CRU is not at a rate inside the OPP table, sets `CLKSEL_CON(158)`
  (and (159)) itself: `soc::rk3588::gpu_clock_set`, CPLL / 5 = 300 MHz while the
  CPLL reads 1500 MHz in normal mode, host-tested over the register model for
  Armbian's 0x1f24 (kept), a PVTPLL state, an out-of-table divider and an unknown
  mux ("The GPU power-on", step 2). What the prompt read decides is whether U-Boot's
  CRU is in one of those states, and whether the CPLL reads 1500 MHz there.
- **The QoS words' effect.** Linux reads 0 for `MODE`, `BANDWIDTH` and `SATURATION`
  in all four blocks with the GPU active (measured 2026-10-26, "The GPU power-on");
  the image leaves the reset values (`MODE` 3, `BANDWIDTH` 0x12d, `SATURATION`
  0x40). Whether the difference matters is unmeasurable until a job runs on the GPU
  from Zephyr (Z3 and later): the first Z4 frame is run once with each setting.
