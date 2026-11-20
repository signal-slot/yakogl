<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_triangle

The first triangle on the Raspberry Pi 5's V3D 7.1: milestone M5 of
`docs/notes/pre-study-rpi5.md` (section 6; P6 of section 8, done when
`v3d_triangle: PASS`), the Pi 4's `v3d_triangle`
(`docs/notes/pi4-v3d-bringup.md`) carried over. Part of ROADMAP stage 5
(issue #7). `rpi5_triangle_ours` (`../rpi5_triangle_ours/`) is the same image
with our compiler's shaders.

```sh
west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_triangle yakogl/tools/probes/rpi_5/rpi5_triangle
west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_triangle_ours yakogl/tools/probes/rpi_5/rpi5_triangle_ours
```

`tools/check.sh`'s `zephyr_rpi5_probe_images` row builds both, as it builds
every directory under `tools/probes/rpi_5/` with a `CMakeLists.txt` and a
`prj.conf` (with `-DZEPHYR_MODULES=$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")`
and `-DYAKOGL_WERROR=ON`).

## What it does

- **M1, M2**: as `rpi5_clear` (`../rpi5_clear/README.md`): the firmware
  mailbox switches the V3D clock, `v3d_power_on` for the BCM2712,
  `v3d_identify` (raw IDENT words and `MMU_DEBUG_INFO` printed), one page table
  (4096 entries, 16 MiB of GPU VA from 1 MiB; VA 0 never mapped) over the
  probe's static buffers, `v3d_mmu_init`. The firmware hands the V3D over
  powered, at 500 MHz and out of reset (U5).
- **The frame**: the driver's own 7.1 clear-then-triangle job,
  `v3d_frame_build_triangle()` (`driver/src/v3d/v3d_frame.c` and
  `v3d_frame71_tri.c`, the whole image built at `-DV3D_CL_VER=71`;
  `docs/notes/v3d71-triangle-frame.md`). It is the board's Linux reference
  `tools/v3d/pi5/triangle.clif` (Mesa 26.2.2, `tools/v3d/gl_ref.py triangle`):
  64x64 RGBA8 cleared to `0xffff0000` (blue), one Gouraud triangle at
  (-0.75,-0.75) red, (0.75,-0.75) green, (0,0.75) blue,
  `glViewport(0, 0, 64, 64)`, the colours `0x30` bytes behind the positions
  in one vertex buffer. Only the target differs: raster with a 256-byte
  stride where Mesa's is UIF. `tools/v3d/check71.d/triangle71.sh` checks the
  same builder with the same inputs against that dump (BCL 126, RCL 96,
  indirect 187 bytes, the record at `+0x60`).
  - The programs (`src/shaders.c`): Mesa's FS, VS and CS words, their uniform
    kinds and their prog_data from
    `tools/v3d/tri-shaders71/mesa_passthrough_interp_color.h` (default), or
    ours from `ours_passthrough_interp_color.h` (`RPI5_TRI_SHADERS=ours`, the
    `rpi5_triangle_ours` image). Each program is placed at its own 512-byte
    slot of one code page.
  - The indirect buffer holds, in this order, the FS, VS and CS uniform
    streams, the 32-byte-aligned D0 shader record ("GL Shader State Record
    Draw Index"), the two attribute records, then the generic tile list.
  - Before anything is submitted the record the builder wrote is checked
    field by field against the header's record values
    (`tri_shaders_rec_check`: Mesa's `MESA71_TRI_REC_*`, or our
    `OURS71_TRI_REC_*` with the frame code's clipping, point-size, NaN and
    minimum-segment values; `docs/notes/v3d71-tri-ours-record.md`). A builder
    refusal or a record field that differs is a FAIL, and nothing is
    submitted.
- **Bin and render** (`docs/notes/v3d71-bin-submit.md`): the tile allocation
  sized with one draw (`v3d_frame_alloc_sizes(.., 1, 4096, ..)`: 16384, Mesa's
  `CT0QMS` for this job); one 256 KiB overflow block mapped before the bin and
  handed to `v3d_wait_bin` (`OUTOMEM` gives it to the binner; a second
  `OUTOMEM` is "out of memory"); `FLDONE`, then `FRDONE`, polled for 1 s each;
  `v3d_caches_clean`.
- **The read-back**: the target, pre-filled with magenta `0xffff00ff`, a
  guard row of `0xa5` bytes above and below it, invalidated from the CPU
  cache and checked by `tools/probes/rpi_5/common/tri_check/tri_check.h`
  (host-tested, `common/tri_check/test/run.sh`): each pixel centre is
  inside (at least 1 px from every edge; the barycentric colour, within 3
  per channel), outside (at least 1 px outside an edge; the clear colour,
  within 3) or edge (either the clear colour or within the vertex colours'
  range, widened by 3). The window coordinates are the viewport applied to
  the positions, not flipped: window y is the buffer's row, (8,8), (56,8),
  (32,56). Pass: no wrong pixel and both guard rows unchanged.

Every GPU buffer (page table, scratch page, the three lists, code, vertices,
tile allocation, tile state, overflow block, target) is a static in the
image, written through the data cache and cleaned and invalidated
(`sys_cache_data_flush_and_invd_range`) before the GPU sees it; a cache call
that fails ends the run (`prj.conf` turns `CONFIG_CACHE_MANAGEMENT` on, as
`rpi5_clear`'s).

## The console

`v3d_triangle: build <commit>, shaders mesa (FS 9, VS 23, CS 15
instructions)` (ours: 12, 28, 20), `v3d_triangle: cache: ...`,
`v3d_triangle: rpi_5, V3D 7.1, target 64x64 raster RGBA8888, watchdog armed
at boot`, then one line per step:

- `M1 mailbox`, `M1 before power-on`, `M1 power_on`, `M1 V3D clock`, three
  `M1 identity` lines, `M2 page table`, `M2 mmu_init` (as `rpi5_clear`);
- `frame 64x64: tile 64x64, 1x1 tiles, BCL 126 / RCL 96 / indirect 187 bytes
  (record +0x60, attributes +0x80, tile list +0xa0..+0xbb), tile alloc 16384
  (draws 1), state ...` (ours: indirect 219, record `+0x80`), `frame
  uniforms: ...` (stream offsets and code addresses), `frame record: first
  dword 0x04208002, 0 field(s) differ from the mesa header`;
- `M5 before bin: MISCCFG ... UIFCFG ... ERR_STAT ... FDBGS ... core INT_STS
  ... hub INT_STS ...` (board idle under Linux: MISCCFG 0x6, UIFCFG 0x45,
  ERR_STAT 0x1000, FDBGS 0x7; `tools/v3d/pi5/v3d_regs.txt`);
- `M5 bin: <rc>, CT0CA ... CT0EA ... (end ...), BPCA ... BPCS ..., overflow
  blocks used N`; `M5 render: <rc>, CT1CA ... (end ...), ERR_STAT ...`;
  `M5 caches_clean: <rc>`. A failed wait prints the stall registers, the MMU
  fault address shifted to a 36-bit VA (`vio_addr << 4` on this board), the
  client (`vio_id & 0x7f`, Linux's 7.1 table) and the raw 7.1 GMP pair
  (`GMP_STATUS` 0x600, `GMP_VIO_ADDR` 0x608);
- `M5 vertices (window): (8,8) (56,8) (32,56), clear 00 00 ff ff, tol 3,
  margin 1 px`, five `M5 pixel <name> (x,y): R G B A` lines (centre, near
  each vertex, a corner outside), `M5 read-back: <rc>, N inside (N wrong, max
  diff N), N outside (N wrong), N edge (N wrong), guard rows N of 512 bytes
  changed`. With a wrong pixel also `M5 first wrong pixel (x,y): ...` and
  `M5 the same check with the rows flipped: N wrong` (a diagnosis: a frame
  drawn upside down passes that one).

The last line is the verdict, exactly one, at column 0, which
`tools/board.sh pi5` ends the run on (its `END_RE_DEFAULT` matches
`v3d_triangle: (PASS|FAIL)`):

- `v3d_triangle: PASS (64x64 read back, 0 of 4096 px wrong: <I> inside, <O>
  outside, <E> edge, tol 3; shaders mesa)`
- `v3d_triangle: FAIL (<reason>; shaders mesa)`, the reason one of `M1
  mailbox`, `M1 power-on`, `M1 identity`, `M2 page table`, `M2 cache clean`,
  `M2 MMU`, `frame build (a program is larger than its code slot)`, `frame
  sizes`, `frame build`, `frame record: <n> field(s) differ from the header,
  first <field>`, `M5 cache clean`, `M5 bin: out of memory`, `M5 bin: MMU
  fault`, `M5 bin: timeout`, `M5 render: MMU fault`, `M5 render: timeout`,
  `M5 GPU cache clean`, `M5 cache invalidate`, `M5 read-back: the check
  refused its arguments`, `<n> of 4096 px differ by more than 3: <i> inside,
  <o> outside, <e> edge, first at <x>,<y>`, `guard rows: <n> of 512 bytes
  changed`.

`rpi5_triangle_ours` prints the same lines with `shaders ours`.

Nothing runs afterwards: the board layer's PM watchdog
(`CONFIG_YAKOGL_RPI5_WDOG`, armed in the first `PRE_KERNEL_1` hook) stops
being fed 30 s after boot and resets the SoC within 10 s more; tryboot brings
Linux back. A hang anywhere ends the same way, with the last step line on
the console.

## Running it

From one commit of origin/main, built as above, through the pi5 runner
(tryboot: `kernel=` names the staged `zephyr.bin`, config.txt is restored
after the run). `BOARD_HOST` and `BOARD_UART` come from the environment (the
leases), never from a file:

```sh
tools/board.sh pi5 run build/rpi5_triangle/zephyr/zephyr.bin <logdir outside the checkout> rpi5_triangle
tools/board.sh pi5 run build/rpi5_triangle_ours/zephyr/zephyr.bin <logdir outside the checkout> rpi5_triangle_ours
```

Mesa's shaders first; ours only after Mesa's passed. If the board does not
come back within the runner's limit, stop: no retry
(`tools/probes/rpi_5/README.md`).

## On the board

<!-- the board session (M5) fills this: build, image, the board.sh result
line, get_throttled, the console, what it answers -->

## Limits

- The interrupts are not used (the waits poll), as in `rpi5_clear`.
- One overflow block: a bin that needs a second one is a FAIL, not a retry
  (this job's tile allocation is Mesa's, which needed none on the board).
- The window orientation (y not flipped, row = window y) is the frame's
  `v3d_viewport_gl(.., y0_top = false, ..)`, as `frame_check --triangle71`
  builds it; the flipped-rows line tells a frame drawn the other way up.
- `get_throttled` is recorded by `tools/board.sh` in `meta.txt`, not by the
  image; check it before reading a GPU fault as a hardware one.
