<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_clear

The first clear on the Raspberry Pi 5's V3D 7.1: milestones M1 to M4 of
`docs/notes/pre-study-rpi5.md` (section 6), the Pi 4's `v3d_clear` carried
over as `docs/notes/v3d71-clear-frame.md` section 3 says. Part of ROADMAP
stage 5 (issue #7).

```sh
west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_clear yakogl/tools/probes/rpi_5/rpi5_clear
```

## What it does

- **M1**: the firmware mailbox (`CONFIG_YAKOGL_RPI5_MBOX`) switches the V3D
  clock (firmware clock 5); `v3d_power_on` for the BCM2712 (pm's reset, the
  SMS powered up and reset); `v3d_identify`. The hub and core IDENT words and
  `MMU_DEBUG_INFO` are printed raw, before anything is mapped, to compare
  with Linux's debugfs `v3d_ident` (D3).
- **M2**: one page table (4096 entries, 16 MiB of GPU VA from 1 MiB; VA 0
  never mapped) over the probe's static buffers, `v3d_mmu_init`.
- **M3**: the clear-only frame of `v3d71-clear-frame.md` section 1, written by
  `src/frame71.c` (the 7.1 packers of H4 are not in the tree, so the probe
  puts each field at the bit Mesa's 7.1 packer uses; the tile size is
  `v3d_frame_tiling_gen` at 71). `tools/v3d/check71.d/clear71.sh` compares
  the three lists with Mesa's own 7.1 packers, byte for byte, this frame
  included. Binned and rendered; `FLDONE` and `FRDONE` polled for 1 s each.
- **M4**: the target, 128x64 RGBA8888 raster (two 64x64 tiles side by side),
  pre-filled with magenta `0xffff00ff`, the row above it red `0xff0000ff`;
  the clear is green `0xff00ff00`. Pass: 0 wrong pixels and the guard row
  unchanged.

Every GPU buffer is a static in the image, written through the data cache:
each is cleaned and invalidated (`sys_cache_data_flush_and_invd_range`)
before the GPU reads it, and the target is invalidated again before the CPU
reads it back. `prj.conf` turns `CONFIG_CACHE_MANAGEMENT` on (the BCM2712
SoC does not; without it every cache call is `-ENOTSUP` and does nothing),
and a cache call that fails ends the run.

## The console

`v3d_clear: build ...`, `v3d_clear: cache: ...` (which cache option ran:
`CONFIG_CACHE_MANAGEMENT`, `CONFIG_DCACHE`, the call used, the line size),
then one line per step with its raw result (`M1
mailbox`, `M1 power_on`, `M1 V3D clock`, three `M1 identity` lines, `M2 page
table`, `M2 mmu_init`, `frame 128x64: tile 64x64, ... lists 19 / 99 / 27
bytes` (the render list is 96 bytes for one 64x64 tile, 99 here: two
`SUPERTILE_COORDINATES`; `clear71.sh` checks the 64x64 frame at 19 / 96 / 27), `M3 bin`, `M3 render`, `M3 caches_clean`, `M4: ...`). A failed wait
prints the stall registers (`core_sts`, `hub_sts`, both control lists'
current and end address, `err_stat`, `mmu_ctl`, `vio_addr`, `vio_id`).

The last line is the verdict, which `tools/board.sh pi5` ends the run on:

- `v3d_clear: PASS (M1 to M4, 128x64 cleared; N pages used incl. page table
  and scratch, cache: ...)`
- `v3d_clear: FAIL (<reason>; N pages used ..., cache: ...)`, the reason one of `M1 mailbox`,
  `M1 power-on`, `M1 identity`, `M2 page table`, `M2 cache clean`,
  `M2 MMU`, `frame sizes`, `frame build`, `M3 cache clean`,
  `M3 bin: out of memory`, `M3 bin: MMU fault`, `M3 bin: timeout`,
  `M3 render: MMU fault`, `M3 render: timeout`, `M3 GPU cache clean`,
  `M4 cache invalidate`, `M4 pixels`, `M4 guard row`.

Nothing runs afterwards: the board layer's PM watchdog
(`CONFIG_YAKOGL_RPI5_WDOG`, armed in the first `PRE_KERNEL_1` hook) stops
being fed 30 s after boot and resets the SoC within 10 s more; tryboot brings
Linux back. A hang anywhere ends the same way, with the last step line on
the console.

## On the board (M1 to M4, 2026-11-19)

Built from `8eb2b3cf` with `-DYAKOGL_WERROR=ON` (zephyr `af17c0c8aa2a`,
image sha256 `457d1c7c945cd42e...`, 65,824 bytes) and run once through
`tools/board.sh pi5 run`, with no change to the probe or the driver:
`rc=0 verdict=done boot=tryboot row boot_verdict=complete
result=legacy:v3d_clear_pass resets=0 back_after=213s`, `get_throttled` 0x0
before and after. The console after Zephyr's banner:

```
v3d_clear: build 8eb2b3cf
v3d_clear: cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1 sys_cache_data_flush_and_invd_range, data line 64 bytes
v3d_clear: rpi_5, V3D 7.1, target 128x64 raster RGBA8888, watchdog armed at boot
M1 mailbox: 0
M1 before power-on: PM_GRAFX 0x00001040
M1 power_on: 0, PM_GRAFX 0x00001040, SMS REE_CS 0x00000000 TEE_CS 0x00000050
M1 V3D clock: rate 0 (500000000 Hz)
[00:00:00.046,000] <inf> gpu_v3d: V3D 7.1: 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes
[00:00:00.056,000] <inf> gpu_v3d: V3D raw: HUB_IDENT0..3 42554856 00081117 00001900 00020a10 CTL_IDENT0..2 07443356 81001431 c0078101 MMU_DEBUG_INFO 20804664
M1 identity: HUB_IDENT0..3 0x42554856 0x00081117 0x00001900 0x00020a10
M1 identity: CTL_IDENT0..2 0x07443356 0x81001431 0xc0078101 MMU_DEBUG_INFO 0x20804664
M1 identity: V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes: 0
M2 page table: PA 0x23e000, 4096 entries, scratch PA 0x23d000, VA 0x00100000..0x0012c000 mapped
M2 mmu_init: 0
frame 128x64: tile 64x64, 2x1 tiles, lists 19 / 99 / 27 bytes, tile alloc 12288, state 512
M3 bin: 0
M3 render: 0
M3 caches_clean: 0
M4: 0 of 8192 target pixels wrong, guard row 0 of 128 changed
v3d_clear: PASS (M1 to M4, 128x64 cleared; 46 pages used incl. page table and scratch, cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1 sys_cache_data_flush_and_invd_range)
```

then the watchdog's reset (`PM_RSTS 00000020`) and Linux. What it answers:
the firmware hands over the V3D out of reset (`PM_GRAFX_2712` bit 6 set
before any write) and at 500 MHz (U5); the identity is Linux's (D3,
`tools/v3d/pi5/v3d_regs.txt`); the GPU reaches the image's `sram0` buffers at
their ARM physical addresses through our page table (U10 for buffers below
34 MiB); the 7.1 bin and render done bits are the ones `v3d_hw.c` polls.

## Limits

- The interrupts are not used (the waits poll); the GIC's line count for
  SPIs 249 and 250 is still to be read (`driver/docs/rpi5-v3d71-facts.md`,
  section 8).
- `get_throttled` is recorded by `tools/board.sh` in `meta.txt`, not by the
  image (the image cannot read it);
  check it before reading a GPU fault as a hardware one.

## Verdicts

`tools/probes/rpi_5/rpi5_clear/verdicts.sh <console.log>` checks a captured
console against the lines `src/main.c` prints: `M1` (mailbox, power_on and
identity `V3D 7.1` all 0), `M2 mmu_init`, `M3` (bin, render, caches_clean),
the `M4:` read-back (0 pixels wrong, guard row 0 changed) and the end line.
It prints one `verdicts:` line per group, `end:` and `verdicts: PASS|FAIL`.
An end line of PASS beside a wrong read-back or a non-zero step is a FAIL.
Exit 0 done, 1 no end line, 2 usage, 3 FAIL. `verdicts_selftest.sh` runs it
on inline logs (good, image FAIL, wrong read-back, guard row, bad step,
truncated, CR and timestamp form, usage); the gate runs it on its own.
