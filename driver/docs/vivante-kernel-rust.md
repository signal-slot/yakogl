<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The GC7000 kernel side in Rust

Status 2026-10-03: **the only kernel side; the C is deleted.** On the
Verdin i.MX8M Plus every sample and dEQP group read byte-identical SNVS
records with the C and with the Rust kernel side: gc_gl_triangle (parity),
the gc_gl_test regression set (35/38), gc_slint (1666 frames), dEQP
fragment_ops.blend 1060/0, shaders.texture_functions 20/0 and fbo 711/0
(tools/cts/RESULTS.md, "GC7000 kernel side in Rust"). The staged smaller
steps (gc_identify, gc_fence, gc_clear, gc_triangle) and the X2 adopt run
were not run with the Rust before the switch. Source-form group C
(docs/source-form.md): `driver/src/vivante/gc_bringup.c`, `gc_hw.c`,
`gc_mmu.c` and `gc_cmdbuf.c` followed the Linux etnaviv kernel driver (GPL).
They are rewritten as original Rust, `gpu::vivante` in the drivers crate
(`driver/rust/src/gpu/vivante/`, feature `vivante`), from permissive sources
only, and `CONFIG_YAKOGL_DRV_GPU_VIVANTE` builds it; the four C files, the switch
`CONFIG_YAKOGL_DRV_GC_KERNEL_RUST` and `driver/tools/gclab/kerneltest` are gone.
The Linux module `driver/tools/gclab` (`gcgpu.ko`) was built from the C and
is retired with it (its README).

Linux etnaviv (`drivers/gpu/drm/etnaviv`) was not read for this work. Every
fact the C took from it is derived again below from a permissive source or
from a measurement on the board.

## Sources

| short name | what | licence | how used |
| --- | --- | --- | --- |
| rnndb | etna_viv rules-ng-ng headers in `driver/src/vivante/hw/*.xml.h` | MIT | register names, offsets, fields, command words; `regs.rs` is checked against them by a test |
| galcore | NXP's Vivante kernel driver, `drivers/mxc/gpu-viv` of [nxp-imx/linux-imx](https://github.com/nxp-imx/linux-imx) branch `lf-6.12.y` (commit `df24f942`, 2025-11-21) | dual MIT / GPL-2.0-or-later, used under MIT | documentation of sequences and of the part's database entry; cited by file and function; nothing copied or vendored |
| RM | i.MX 8M Plus Applications Processor Reference Manual, IMX8MPRM Rev. 3, 08/2024 | NXP document | GPC, CCM, the GPU register block, the interrupt |
| board | measurements on the Verdin iMX8M Plus ("Hardware findings that still hold" below, RESULTS; the bring-up record `docs/notes/imx8mp-gc-driver-notes.md`) | test data | values that differ from galcore and have run on the board since 2026-09-20 |

galcore's licence: every `.c` and `.h` of the 112 under `drivers/mxc/gpu-viv`
carries the same header. It begins "The MIT License (MIT) / Copyright (c) 2014 -
2024 Vivante Corporation", follows with "The GPL License (GPL)", and ends:
"Note: This software is released under dual MIT and GPL licenses. A recipient
may use this file under the terms of either the MIT license or GPL License."

galcore files cited below (paths under `drivers/mxc/gpu-viv/hal/`):
`kernel/arch/gc_hal_kernel_hardware.c` (HW), `kernel/arch/gc_hal_kernel_hardware_waitlink_fe.c`
(WLFE), `kernel/arch/gc_hal_kernel_hardware_func.c` (FUNC), `kernel/gc_hal_kernel_mmu.c`
(MMU), `kernel/gc_hal_kernel_command.c` (CMD), `kernel/gc_hal_kernel_event.c` (EVT),
`kernel/inc/gc_feature_database.h` (DB).

## Layout

```
driver/rust/src/gpu/vivante/
  mod.rs      GcCore (the struct gc_dev prefix), GcIdentity, GcSubmit, the carveout layout, Mem, dmb_sy
  regs.rs     rnndb constants (test: equal to hw/*.xml.h by name)
  power.rs    GPU clock roots/gates and power domains over soc::imx8mp::{ccm,gpc}
  hw.rs       identify (galcore DB entry), reset, init after reset, idle decode, FE double read
  mmu.rs      MMU v2 tables, map, the secure-mode enable, the fault dump
  cmd.rs      command words (LOAD_STATE, LINK, WAIT, END, EVENT, semaphore, stall)
  cmdbuf.rs   FE start, wait idle, the WAIT/LINK ring, one submit, the stall dump
  irq.rs      event claim, the interrupt's bookkeeping
  tests.rs    host tests on the model bus; c_golden.rs: the C's record
driver/rust/src/ffi/vivante.rs           yd_gc_* entry points, layout asserts
driver/rust/include/yakogl_drivers.h     their C declarations
driver/drivers/gpu/gpu_vivante_gc_rust.c the Zephyr side
```

`c_golden.rs` was written by `driver/tools/gclab/kerneltest`, which compiled
the C on the host; both were deleted with the C (4c25beb0). The record stays
as the replay test's data.

The shim keeps the names and signatures of `gc.h` and `gc_internal.h`, so the
composition code (`gc_render.c`, `gc_draw.c`, `gc_cmd.c`) built unchanged
against both implementations while both existed. The shim is the Zephyr device
(`DEVICE_DT_INST_DEFINE`, `device_map` of the GPU, GPC and CCM ranges,
`gc_read`/`gc_write`/`gc_identity`/`gc_alive`, `gc_bringup_attach`). It owns
`IRQ_CONNECT` and the ISR, and the kernel objects of `struct gc_dev`: it takes
the spinlock around `yd_gc_event_claim` and `yd_gc_irq_handle`, gives the event
semaphore once per freed event, and sleeps on it in `gc_event_alloc`. It also
provides `gc_now_ns`, and the log under the `gpu_gc` module and level.
`struct gc_dev` stays C's. Rust's `GcCore` mirrors its fields up to
`dump_seen`, the part before the kernel objects. The offsets are pinned three
ways: `BUILD_ASSERT`s in the shim, `const` asserts in `ffi/vivante.rs`, and a
host test against the layout kerneltest recorded from the C header.

`gc.h` and `gc_internal.h` are ours. Their etnaviv references were removed in
this step. They stay C, as the declarations shared by the composition code and
the shim.

## Provenance, per C function

| C function (file) | source the C named | source now (Rust) |
| --- | --- | --- |
| `gc_clocks_on` (bringup) | Linux clk-imx8mp.c | RM §5.1.2 Table 5-1: slices 3 GPU3D_CORE (0x8180) and 4 GPU3D_SHADER (0x8200), mux 2 = SYSTEM_PLL1; 24 GPU_AXI (0x8C00) and 25 GPU_AHB (0x8C80), mux 1 = SYSTEM_PLL1; maxima 1000/1000/800/400 MHz. §5.1.7.10: no pre divider on core slices. Table 5-9: CCGR87 "GPU", CCGR70 "GPU3D". `power::power_on` over `soc::imx8mp::ccm` |
| `gpc_power_up` (bringup) | Linux gpcv2.c | RM §5.2.10.37/.72, §5.2.11, §5.2.10.57 through `soc::imx8mp::gpc` (GPU_SHARE_LOGIC PGC 7, GPU_3D PGC 9, GPUMIX handshake bits 9/25) |
| `gc_hwdb` (bringup) | etnaviv_hwdb.c | DB entry `gc7000UL_6200` (chip 0x7000, rev 0x6202, product 0x70003, informal: matched on rev & 0xfff0): TempRegisters 64, ThreadCount 512, NumShaderCores 2, InstructionCount 512, NumberOfConstants 320, NumPixelPipes 1, **Streams 8**, VaryingCount 16, VertexOutputBufferSize 1024, VertexCacheSize 16, REG_MMU, SECURITY, SECURITY_AHB. `hw::DB` |
| `gc_identify` (bringup) | etnaviv_hw_identify | HW `_IdentifyHardwareByDatabase`: CHIP_IDENTITY, MODEL, REV, DATE, CUSTOMER_ID, PRODUCT_ID, ECO_ID, FEATURE, MINOR_FEATURE_0..6 read as registers; capabilities from the DB (`gcQueryFeatureDB`). Offsets from rnndb. `hw::identify` |
| `gc_bringup_reset` (bringup) | etnaviv_hw_reset | HW `_ResetGPU`: clock gating off, pulse eater 0x015B0880 with DISABLE pulsed, FSCALE 64 loaded, isolate, soft reset through MMUv2_AHB_CONTROL on a SECURITY_AHB part ("secure in normal" mode, HW `_SetHardwareOptions`), release, de-isolate, IDLE_STATE, CLOCK_CONTROL IDLE_3D/2D, MMU enable bit clear (SEC_CONTROL 0x388 / CONTROL 0x18C). `hw::reset`. Board values: see below |
| `gc_bringup_attach`, `gc_init`, accessors (bringup) | ours | shim C |
| `gc_rd_fe` (hw) | etnaviv gpu_read | HW reads the FE debug registers 0x660..0x66C twice (HW:718, 3600, 4992, 7468, 9680). `hw::rd_fe` |
| `gc_wr_sync` (hw) | etnaviv gpu_write_power_sync | ours (a posted write read back before what follows) |
| `gc_enable_mlcg` (hw) | etnaviv_gpu_enable_mlcg | HW `gckHARDWARE_InitializeHardware`: PM_POWER_CONTROLS bit 0 set, PM_MODULE_CONTROLS RA_HZ (bit 17) and RA_EZ (bit 16) unconditionally. Board values: see below. `hw::init` |
| `gc_setup_pulse_eater` (hw) | etnaviv_gpu_setup_pulse_eater | HW `gckHARDWARE_InitializeHardware`: read-modify-write of 0x10C when rev > 0x5420 and a 3D pipe. Board value (bit 18): see below |
| `gc_reg_absent` (hw) | ours | board: MMUv2_AHB_CONTROL and undecoded addresses read 0xdeadde.. ("Hardware findings that still hold", 2026-09-22); RM silent |
| `gc_hw_init` (hw) | etnaviv_gpu_hw_init | HW `gckHARDWARE_InitializeHardware`: AHB_CONTROL NONSEC_ACCESS (0x2) on a secure-AHB part; INTR_ENBL 0xffffffff. AXI_CONFIG 0x2200: board value, see below |
| `gc_idle_state_decode` (hw) | ours | ours (rnndb bit names) |
| `gc_event_alloc` (hw) | etnaviv event_alloc | ours: `irq::claim` under the shim's lock, the wait in the shim |
| `gc_irq_handle` (hw) | etnaviv irq_handler | HW `gckHARDWARE_Interrupt` (INTR_ACKNOWLEDGE read to clear, zero = not ours), EVT `gckEVENT_Notify` (bit 31 AXI bus error, bit 30 MMU exception, bits 29:0 events). Fence bookkeeping ours. `irq::handle` |
| `gc_isr`, `gc_irq_connect` (hw) | Zephyr | shim C |
| PTE bits, shifts (mmu) | etnaviv_iommu_v2.c | MMU `_SetPage` (phys 31:12, phys 39:32 in 11:4, bit 2 writable, bit 0 present, bit 1 exception), `_ConstructDynamicStlb` (master entry = slave table \| present); gc_hal_kernel_mmu.h 4 KiB mode (shift 22, 1024 x 1024). `mmu.rs` |
| safe page pattern (mmu) | etnaviv_mmu.c | ours (galcore: 256 zero bytes); any content works |
| `stlb_for`, `gc_mmu_map`, `gc_mmu_init` (mmu) | ours, over the formats | ours, over galcore's formats; carveout layout and identity mapping ours |
| `gc_mmu_enable` (mmu) | etnaviv_iommuv2_restore_sec | HW `_InitPageTableArray` (PTA_ADDRESS_LOW/HIGH 0x38C/0x390), FUNC `_FuncExecute_MMU_CMD` (PTA_CONTROL 0x394 = 1, safe addresses 0x398/0x39C, SAFE_ADDRESS_CONFIG 0x3A0 high bits), FUNC `_ProgramMMUStates` (PTA entry = MTLB phys \| mode, the stream LOAD_STATE MMUv2_PTA_CONFIG INDEX(0) + END run with the MMU off), HW `gckHARDWARE_SetMMU` (then SEC_CONTROL 0x388 = ENABLE) |
| `gc_mmu_dump` (mmu) | etnaviv dump_mmu_fault | EVT / HW `gckHARDWARE_DumpMMUException`: SEC_STATUS 0x384, SEC_EXCEPTION_ADDR 0x380, codes 1..6 |
| `wait_cycles` (cmdbuf) | etnaviv (core_hz >> 15) | board value (galcore: fixed 200, HW `gckHARDWARE_Construct`) |
| `gc_fe_start` (cmdbuf) | etnaviv_gpu_start_fe | WLFE `gckWLFE_Execute`: FE_COMMAND_ADDRESS 0x654, FE_COMMAND_CONTROL 0x658 = ENABLE \| (bytes+7)>>3, then with the secure AHB SEC_COMMAND_CONTROL 0x3A4 = the same, in that order |
| `gc_fe_wait_idle` (cmdbuf) | ours | ours (galcore polls IDLE_STATE too, FUNC `_FuncExecute_MMU_CMD`) |
| `gc_cmd_build_pta_config` (cmdbuf) | etnaviv | FUNC `_ProgramMMUStates` (first state and END) |
| `emit_idle_loop`, `replace_wait` (cmdbuf) | etnaviv_buffer.c | WLFE `gckWLFE_WaitLink` (WAIT, LINK prefetch 2 back to the WAIT), `gckWLFE_Link` (address word, barrier, header word, barrier), CMD `_CommitWaitLinkOnce` |
| `gc_ring_init`, `gc_ring_submit` (cmdbuf) | etnaviv_buffer_init/queue | HW `gckHARDWARE_PipeSelect` (flush, FE->PE semaphore + stall, PIPE_SELECT 0), `gckHARDWARE_Flush` (GL_FLUSH_CACHE bits, TS_FLUSH_CACHE 0x1650 = 1), WLFE `gckWLFE_Event` (GL_EVENT id \| FROM_PE; REG_BltEngine 0 on this part). Ring arithmetic, link-not-copy, flush sets ours/board |
| `gc_ring_dump` (cmdbuf) | ours | ours |

## Where galcore differs, and what is kept

The rewrite reproduces the C's register and memory traffic exactly (see
"Host identity"). Where galcore does something else for this part, the C's
value is kept: it has run on every board run since 2026-09-20 (gc_identify,
gc_fence, gc_clear, gc_triangle, gc_gl_triangle, gc_gl_test, dEQP-GLES2,
gc_slint at 60 fps). The galcore variants are candidates for board
experiments, not changes.

| what | the board's value (kept) | galcore |
| --- | --- | --- |
| reset: clock gating off | PM_POWER_CONTROLS = 0 | PM_MODULE_CONTROLS = 0 |
| reset: pulse eater writes | 0x015B0880, then \| DISABLE | the same two, then 0x015B0880 again |
| reset: CLOCK_CONTROL value | FSCALE 64 alone (0x100) | 0x00070900 (adds DISABLE_DEBUG_REGISTERS and the read-only idle bits) |
| reset: waits | 20 us after the soft reset; 10 ms between attempts | 1 ms after FSCALE, 1 ms after release |
| reset: idle check | every module but AXI_LP, one clean pass | IDLE_STATE bit 0 (FE) only, two clean passes |
| init: module clock gating | on (POWER_CONTROLS bit 0), MODULE_CONTROLS \|= 0x00030000 | for rev 0x6202: off again, MODULE_CONTROLS 0x004300E0 |
| init: AXI_CONFIG | AWCACHE 2 \| ARCACHE 2 (0x2200) | written only for older parts |
| init: pulse eater | \| INTERNAL_DFS (bit 18) | clear bit 16, set bit 17; bit 18 only with gcdDVFS |
| init: MC_L2_CACHE_CONFIG 0x55C | not written | 0x00FFFFFF on parts without the 64K L2 (HW `gckHARDWARE_Construct`) |
| MMU mode | 4 KiB (1024 x 1024) | 1 KiB by default (gcdENABLE_MMU_1KMODE) |
| unused master entries | exception bit | 0 |
| PTA stream | PTA_CONFIG, END | also states 0x3848, 0x3878 and a semaphore/stall |
| WAIT delay | core clock >> 15 (24414 at 800 MHz) | 200 |
| stream count | 8 (galcore's DB, now) | 8. The C's etnaviv-derived table said 16; nothing in the driver reads it |

## Identity: what changed

The C took the feature words from etnaviv's table. Galcore reads them as
registers and takes capabilities from its database, and so does the Rust:
`features` and `minor_features[0..5]` are the registers (minor 6 at 0xDC is
read by galcore but has no rnndb name, so it is not read). The database's
REG_MMU, SECURITY and SECURITY_AHB are folded in as the rnndb bits of words
1, 7 and 10, so `secure_ahb` and the reset's MMU check come out as before. The
table matches on model 0x7000, revision & 0xfff0 = 0x6200 and product 0x70003
(galcore's rule), and also requires revision 0x6204, the one revision the
init has run on. Model 0, all ones, or the absent pattern is -ENODEV. Another
part is identified from its registers alone, with a warning (the C did the
same).

## Host identity with the C

`driver/tools/gclab/kerneltest` (deleted with the C) compiled the shipped C
files on the host, against a recording register file and a 64 MiB carveout at
the board's 0x58000000. It ran one scenario and wrote `c_golden.rs`, and the
gate row `cc GC7000 kernel C == c_golden.rs (kerneltest)` kept that record
equal to the C while the C existed. The host test
`the_c_scenario_comes_out_identical` replays the scenario on the Rust and
asserts:

- identify + reset: the C's ten identity reads in its order, then the five
  minor-feature reads (new), then every reset access as the C; the reset
  alone (recovery) and the init after reset: every register access identical;
- the MMU: `gc_mmu_init`, a 3-page mapping of physical 0x1_2345_6000 (above
  4 GiB), a 2-page mapping across a 4 MiB master boundary, then
  `gc_mmu_enable`. Its register writes are identical, and so is every byte
  of MTLB, PTA, safe page, PTA stream and the 18 slave tables (FNV-1a over
  0x16000 bytes, plus the words around each mapping);
- the ring: `gc_ring_init`'s writes, then 1000 submits cycling five flag sets,
  eight buffers and seven sizes. The ring wraps at submit 786. Every
  (ring_pos, wait_pos), the 64 KiB ring and the LINKs written back into the
  buffers are byte-identical; a submit touches no register; misaligned
  buffers give -EINVAL and buffers outside the carveout -EFAULT;
- events and the interrupt: ids 0..4 for fences 1..5. Then an interrupt with
  events 0, 2, 29 and both error bits, and a late event 1: fence_done,
  event_free, irq_errors, the perf counters and the semaphore count all
  identical;
- refusals: a misaligned map (-EINVAL), and running out of slave tables
  (-ENOMEM, 64 used, tables byte-identical).

The other tests cover the sequences' own properties and their failures. The
power order: roots, then gates, then GPUMIX, its handshake, then GPU3D, with
the model failing any GPU access before that. Failures: a stuck or refused
GPC trigger, a missing handshake acknowledge, identify refusals, a GPU that
never idles (bounded at 100 attempts), an MMU left on, the non-secure reset
path, an FE timeout, MMU enable when already on or when the PTA stream hangs,
and a carveout above 4 GiB or too small. Also: entry formats, events running
out, a corrupt bitmap, fence_done across the u32 wrap, the error bits, the
idle decode, the WAIT clamp, and the rnndb constants against the headers.

## What only the board can confirm

The board runs above passed with records identical to the C's. Whether each
item below was read in the board's log is not recorded; they stay listed as
what to look at first if a GPU bring-up problem appears.

1. The GPUMIX handshake acknowledge (PU_PWRHSK bit 25) after power-up.
   `soc::imx8mp::gpc::adb_wait_up` waits for it. The C did not, and the RM
   does not describe the handshake on the way up. A timeout is logged as a
   warning and the identity read decides.
2. The GPC status checks and the PSR clear `gpc::power_up` adds, and
   `ccm::set_root` keeping the bits it does not own (the C overwrote
   TARGET_ROOT whole).
3. Reading MINOR_FEATURE_1..5 (galcore reads them on this SoC; the C did not).
   The values are only logged, but they fill the exported words, so the
   gc_identify log changes.
4. The product id 0x70003 match (measured once, RESULTS 2026-09; customer
   and eco ids not recorded).
5. The ISR now holds the spinlock across the INTR_ACKNOWLEDGE read (the C took
   it per event).
6. That inline `dmb sy` and the volatile stores give the front end the same
   ordering as Zephyr's `barrier_dmem_fence_full()` and plain C stores did.
   The disassembly of `yd_gc_ring_submit` has the four `dmb sy`.

Everything else is the same traffic as the C, by the host record.

## Hardware findings that still hold

Measured on the Verdin iMX8M Plus in September 2026, with the C kernel side
and the Linux harness that are now gone. They are facts about the part, so
they hold for the Rust. The full accounts are in
`docs/notes/imx8mp-gc-driver-notes.md` (the old
`driver/src/vivante/NOTES.md`) and
`docs/notes/imx8mp-gc-bringup.md`.

Registers:

- **`MMUv2_AHB_CONTROL` (0x3a8) does not exist on this part.** It reads
  `0xdeaddeae`, the interconnect's pattern for an address no slave decodes.
  `hw::init` writes the non-secure grant alone and logs that the register is
  absent. Whether the grant is needed at all is open: the evidence says the
  AHB is already non-secure, since everything works.
- **`HI_AXI_CONFIG` (0x08) reads back 0** after the 0x2200 write. It is
  write-only or absent. The write stays; it is harmless either way.
- `HI_INTR_ENBL` does read back, so the register block itself is alive.
- **Never read the 0x0800+ state space from the CPU.** Two builds that read
  shader-state registers from the CPU rebooted the board, one of them after
  a draw that completed. Dozens of runs without such reads, many with
  deliberate GPU hangs, never did. Neither etnaviv nor the vendor driver
  reads at or above 0x800 from the CPU. The stall dump reads only the HI,
  PM and MC registers, and that is safe.

Clock gating:

- Module-level clock gating is on (`PM_POWER_CONTROLS` bit 0), with RA_HZ
  and RA_EZ excluded. The idea that a unit gated while stalled causes the
  GPU hang was **refuted** on 2026-09-22: in the stall dumps the gated set
  (PA, SE, RA, TX) and the busy set (FE, PE, SH) do not intersect. The
  dumps were from one build and one hang shape, and say nothing about gating
  during the approach to the hang.
- The pulse eater after init reads 0x015d0880: the reset constant with
  `INTERNAL_DFS` (bit 18) set, as the init writes it.

Shaders and hangs:

- `VS_TEMP_REGISTER_CONTROL` (0x080c) and `PS_TEMP_REGISTER_CONTROL`
  (0x100c) have a 6-bit `NUM_TEMPS` field, so 63 is the largest count either
  stage can program. A larger count is refused by name
  (`src/vivante/shader.rs`); it used to be masked (67 went out as 3).
- `VS/PS_START_PC` and `END_PC` are not programmed, and need not be: Mesa
  emits them only for parts without unified instruction memory, and this
  HALTI5 part has it. A program's extent comes from `RANGE_HIGH` and
  `ICACHE_COUNT`.
- A vertex shader hang is **correlated with** the temporary-register count
  but is **not a function of** it: on one ladder 26 temps pass and 27 hang,
  but a 28-temp, 95-instruction shader draws. So the ceiling the compiler
  uses is a mitigation, not a rule. Mesa's own bound (64 temps) would emit
  shaders that hang here. The measurement that would give a rule: run the
  same ladder at a second `num_out`, since the load-balancing word depends
  on `num_out` alone.
- **Recovery.** A fence wait that times out resets the GPU and restores it
  (`gc_render_recover()` in `driver/src/vivante/gc_render.c`: reset, init,
  MMU, ring). Work the reset threw away returns `-ECANCELED`; the deferred
  free treats it as done. On Zephyr the timeout is the OSAL's
  `GC_WAIT_FOREVER_MS` (2 s, `src/osal/zephyr_gc.c`), and recovery is always
  on. Measured on 2026-09-22: a dEQP sweep that used to stop the GPU for
  good at 1,431 cases completed 11,366 results with 2 recoveries. A reset is
  the floor, not the fix: the compiler must not emit a program that cannot
  terminate.

Still open (not measured):

1. Whether the GC7000 can bound shader execution in hardware (an
   instruction-count or watchdog limit).
2. Whether 24414 is the right idle-loop `WAIT` delay (core clock >> 15 at
   the assumed 800 MHz). It costs latency only.
3. Whether the tile-status cache flush in the epilogue is needed with no
   tile status.
4. Whether the instruction cache needs code padded or aligned beyond 64
   bytes.

## Board runs, and what is left

Done on the Verdin iMX8M Plus, C kernel against Rust kernel, byte-identical
SNVS records: `gc_gl_triangle` (parity), `gc_gl_test` (regression set),
`gc_slint`, dEQP `fragment_ops.blend`, `shaders.texture_functions` and `fbo`
(RESULTS). Then the default flipped, and the four C files, kerneltest and its
gate row, the switch and NOTICE's etnaviv mention were removed (2026-10-03).

Left:

- The Armadillo-X2 `gc_gl_triangle` adopt run with the Rust kernel side.
- `driver/tools/gclab` and `gcgpu.ko` are removed (`git ls-files
  driver/tools/gclab` is empty). The RESULTS Work list row that asks for
  this follow-up may now be closed.
- The galcore variants in the table above can be tried one at a time,
  measured, never folded in unmeasured.
