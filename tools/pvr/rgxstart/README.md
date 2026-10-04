<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The firmware start before and after the DDK's RGXStart

docs/source-form.md step 6 replaced the driver's own copy of the DDK's start
sequence (`pvr_fw_start()` and its helpers in `driver/src/pvr_fw_boot.c`) by
the DDK's `RGXStart()` itself (`third_party/ti-img-rogue-ddk-25.2/services/
server/devices/rogue/rgxstartstop.c`, unmodified) on our rgxlayer
(`driver/rust/src/gpu/pvr/rgxlayer.rs`). This directory holds the evidence of
what that changed on the register bus.

| file | what |
|---|---|
| `main-38438f4d.trace` | the old sequence, recorded by `record_main.sh` (the functions cut out of origin/main 38438f4d, run against a recording `sys_io`) |
| `rgxstart.diff` | `diff -u` from it to the sequence `RGXStart` produces on our rgxlayer (`tools/pvr/rgxlayer_check.c`) |
| `rgxstop.trace` | `RGXStop` on an idle GPU (no former counterpart: the driver never stopped the firmware) |

`tools/check.sh` (row "pvr rgxlayer_check") rebuilds the new traces and
requires the diff and the stop trace to be these, byte for byte. Same inputs
on both sides: page catalogue 0xe0300000, firmware heap at
`RGX_FIRMWARE_RAW_HEAP_BASE` (0xe1c0000000), `SLC_CTRL_MISC` found as
PSG-hazard-check | 0xf00000, core clock 800 MHz. Trace lines: `W off val`,
`R off val` (32-bit; a 64-bit register is two accesses, low word first, on both
sides), `B off` write barrier, `D us` busy wait.

## Every difference, explained

Same on both sides: the order of the blocks (SYS_BUS_SECURE, FWCORE_BOOT 0,
the three SOFT_RESET steps, SLC_CTRL_MISC read and write with the same value
0x00200003, SAFETY_EVENT_ENABLE 0xff, FWCORE_RESET_ADDR / NMI_ADDR, the
Garten wrapper config, AXI_ACE_LITE_CONFIGURATION 0x8882af00, BIF_CAT_BASE0 and
FWCORE_MEM_CAT_BASE0, SOFT_RESET 0, DMI_DMCONTROL.DMACTIVE, FWCORE_BOOT 1) and
the three 3 us waits around the core's reset release. The differences:

1. **Readbacks become barriers** (SYS_BUS_SECURE, the four SOFT_RESET writes,
   DMI_DMCONTROL). The old code read each register back after writing it;
   `RGXStart` calls `RGXWriteRegMemoryBarrier32/64` instead, and the DDK's own
   implementation of that is `OSWriteMemoryBarrier()`, `wmb()` (`dsb st`) on
   Linux, with a readback only under `PVRSRV_ENABLE_READBACK_ON_WRITES`, which
   the AM62P configuration (`config/am62p_linux_release/config_kernel.h`) does
   not set. Ours issues `dsb st`. The DMCONTROL write has no barrier at all in
   `RGXStart`.
2. **Boot code remap (region 3, 0x3018)**: the size field goes from 64 KiB
   (`0x100100e1_c0000000`) to `RGX_FIRMWARE_RAW_HEAP_SIZE`, 32 MiB
   (`0x120000e1_c0000000`). Same base (the firmware code, at the bottom of the
   firmware heap), same FETCH enable. 64 KiB was the size read back from a
   running DDK, after the firmware had reprogrammed the window.
3. **Boot data remap (region 0xA, 0x3050)**: base heap+0x80000, 132 KiB,
   LOAD_STORE (`0x200210e1_c0080000`) becomes base = the firmware *code*
   address, 32 MiB, LOAD_STORE (`0x220000e1_c0000000`). That is what
   `RGXInitRiscvProcWrapper` writes: both boot windows from
   `RGXAcquireBootCodeAddr`, each the whole raw heap. The old values were
   again the ones read back from a running DDK.
4. **Nine remap writes are gone** (regions 0xb, 4, 0xc, 0, 2, 8, 9 and 1:
   0x3058, 0x3020, 0x3060, 0x3000, 0x3010, 0x3040, 0x3048, 0x3008). The old code
   wrote the values a running DDK showed in them (`riscv_wrapper_init`'s
   comment said so: "RGXStart writes the boot code/data ones, the firmware the
   rest"); the DDK's host code writes none of them (in the DDK 25.2 kernel
   source, services/server, only `rgxstartstop.c` writes a FWCORE remap
   register, `rgxdebug.c` reads some for its dump; searched 2026-10-01 in a
   clone of the UPSTREAM commit), so the firmware programs them itself. The
   board confirms it: with these writes gone the firmware boots and renders
   as before (tools/cts/RESULTS.md, "AM62P: the DDK's RGXStart").
5. **The 1 us wait between DMCONTROL and FWCORE_BOOT is gone**: `RGXStart`
   has none.

The three 3 us waits are `RGXWaitCycles(32, 3)`: our rgxlayer waits the
longer of 32 core cycles and 3 us (3 us at 800 MHz).

## The image load

Not a trace: `tools/pvr/rgxlayer_check.c` runs `RGXGetFWImageAllocSize` and
`RGXProcessFWImage` on a synthetic image with the shape of
rgx.fw.36.53.104.796 (and on the real image when one is given; it is TI/Img
property and never committed) and compares the heap byte for byte with the old
loader's placement (code at heap 0 + the entry's allocation offset, RW and RO
data at heap 0x80000 + (vaddr - 0xa0000000)). Identical, with the real image
too (2026-10-01). The one rule that differs: the old loader zeroed the whole
`RGXRISCVFW_BOOT_DATA` block; `RGXProcessFWImage` writes every field but
`ui32Reserved`, which the image has as zero.
