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

## Limits

- Built, not run on the board. Its first run also answers what the plan
  leaves to the board: whether the firmware has the V3D powered and clocked
  before the image (`M1 before power-on` prints `PM_GRAFX`), whether the
  GPU sees ARM physical addresses as they are (U4/U10: the PA width is in the
  identity line; the buffers are in `sram0`, below 34 MiB), and whether the
  7.1 interrupt bits and waits are the 4.2 ones (`v3d_hw.c`).
- `get_throttled` is recorded by `tools/board.sh` in `meta.txt`, not by the
  image (the image cannot read it);
  check it before reading a GPU fault as a hardware one.
