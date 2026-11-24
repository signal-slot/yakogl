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

## Verdicts

`sh verdicts.sh <console.log>` (host selftest: `sh verdicts_selftest.sh`)
reads a console log of either image and prints one line per group (build
shaders, M1, M2, M5, pixels) and a last `verdicts: PASS` or `verdicts: FAIL
(<reason>)`. PASS needs the end line to say PASS, the M1 identity line
`V3D 7.1 ... : 0`, `M1 mailbox`, `M2 mmu_init` and `M5 caches_clean` at 0,
`M5 bin` and `M5 render` with rc 0, five `M5 pixel` lines, a read-back with
rc 0, 0 wrong pixels and 0 guard bytes changed, and the same shaders name
(`mesa` or `ours`) in the build line and the end line: a PASS end line beside
a non-zero rc or a missing identity line is a FAIL. Exit 0 PASS, 1 no end
line, 2 usage, 3 FAIL.

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

## On the board (M5, 2026-11-20)

Both images built from `0c58c457` with `-DYAKOGL_WERROR=ON` (zephyr
`af17c0c8aa2a`, 82,208 bytes each; `rpi5_triangle` sha256
`1a8697651604f873...`, `rpi5_triangle_ours` sha256 `0fb81874ead08a6f...`) and
run once each through `tools/board.sh pi5 run`, Mesa's shaders first, with no
change to the probe or the driver:

- `rpi5_triangle`: `rc=0 verdict=done boot=tryboot row
  boot_verdict=complete result=legacy:v3d_triangle_pass resets=0
  back_after=227s`
- `rpi5_triangle_ours`: `rc=0 verdict=done boot=tryboot row
  boot_verdict=complete result=legacy:v3d_triangle_pass resets=0
  back_after=215s`

`get_throttled` read 0x0 before and after both. The console of the Mesa run
after Zephyr's banner:

```
v3d_triangle: build 0c58c457, shaders mesa (FS 9, VS 23, CS 15 instructions)
v3d_triangle: cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1 sys_cache_data_flush_and_invd_range, data line 64 bytes
v3d_triangle: rpi_5, V3D 7.1, target 64x64 raster RGBA8888, watchdog armed at boot
M1 mailbox: 0
M1 before power-on: PM_GRAFX 0x00001040
M1 power_on: 0, PM_GRAFX 0x00001040, SMS REE_CS 0x00000000 TEE_CS 0x00000050
M1 V3D clock: rate 0 (500000000 Hz)
[00:00:00.051,000] <inf> gpu_v3d: V3D 7.1: 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes
[00:00:00.061,000] <inf> gpu_v3d: V3D raw: HUB_IDENT0..3 42554856 00081117 00001900 00020a10 CTL_IDENT0..2 07443356 81001431 c0078101 MMU_DEBUG_INFO 20804664
M1 identity: HUB_IDENT0..3 0x42554856 0x00081117 0x00001900 0x00020a10
M1 identity: CTL_IDENT0..2 0x07443356 0x81001431 0xc0078101 MMU_DEBUG_INFO 0x20804664
M1 identity: V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes: 0
M2 page table: PA 0x280000, 4096 entries, scratch PA 0x27f000, VA 0x00100000..0x0016a000 mapped, 108 pages used
M2 mmu_init: 0
frame 64x64: tile 64x64, 1x1 tiles, BCL 126 / RCL 96 / indirect 187 bytes (record +0x60, attributes +0x80, tile list +0xa0..+0xbb), tile alloc 16384 (draws 1), state 256
frame uniforms: FS +0x0, VS +0x0, CS +0x40; code FS 0x00103000 VS 0x00103200 CS 0x00103400
frame record: first dword 0x04208002, 0 field(s) differ from the mesa header
M5 before bin: MISCCFG 0x00000006 UIFCFG 0x00000045 ERR_STAT 0x00001000 FDBGS 0x00000007 core INT_STS 0x00000000 hub INT_STS 0x00000000
M5 bin: 0, CT0CA 0x0010007e CT0EA 0x0010007e (end 0x0010007e), BPCA 0x00108000 BPCS 0x00001000, overflow blocks used 0
M5 render: 0, CT1CA 0x00101060 (end 0x00101060), ERR_STAT 0x00001000
M5 caches_clean: 0
M5 vertices (window): (8,8) (56,8) (32,56), clear 00 00 ff ff, tol 3, margin 1 px
M5 pixel centre (32,24): 51 56 58 ff
M5 pixel near v0 (12,10): e0 11 0d ff
M5 pixel near v1 (51,10): 11 e0 0d ff
M5 pixel near v2 (32,50): 0c 11 e2 ff
M5 pixel corner (1,62): 00 00 ff ff
M5 read-back: 0, 1012 inside (0 wrong, max diff 1), 2796 outside (0 wrong), 288 edge (0 wrong), guard rows 0 of 512 bytes changed
v3d_triangle: PASS (64x64 read back, 0 of 4096 px wrong: 1012 inside, 2796 outside, 288 edge, tol 3; shaders mesa)
```

then the watchdog's reset and Linux. Our shaders' run prints the same lines
except these:

```
v3d_triangle: build 0c58c457, shaders ours (FS 12, VS 28, CS 20 instructions)
frame 64x64: tile 64x64, 1x1 tiles, BCL 126 / RCL 96 / indirect 219 bytes (record +0x80, attributes +0xa0, tile list +0xc0..+0xdb), tile alloc 16384 (draws 1), state 256
frame uniforms: FS +0x0, VS +0x0, CS +0x50; code FS 0x00103000 VS 0x00103200 CS 0x00103400
frame record: first dword 0x04208002, 0 field(s) differ from the ours header
v3d_triangle: PASS (64x64 read back, 0 of 4096 px wrong: 1012 inside, 2796 outside, 288 edge, tol 3; shaders ours)
```

The five sample pixels and the read-back counts are the same in both runs.

What it answers:
- The driver's 7.1 clear-then-triangle job (`v3d_frame_build_triangle`,
  equal to the board's `triangle.clif` on the host) bins and renders on
  the 7.1.10 core under our MMU. Both waits return within the 1 s polls,
  no overflow block is used (Mesa's 16384-byte tile allocation is enough),
  and `ERR_STAT` stays at its idle 0x1000.
- The D0 shader record and the attribute records are right as built for
  both programs' shapes.
- The window orientation is the frame's: no wrong pixel without flipping
  the rows.
- Our 7.1 compiler's pass-through VS, CS and FS run on the hardware and
  give the same pixels as Mesa's.

## Limits

- The interrupts are not used (the waits poll), as in `rpi5_clear`.
- One overflow block: a bin that needs a second one is a FAIL, not a retry
  (this job's tile allocation is Mesa's, which needed none on the board).
- The window orientation (y not flipped, row = window y) is the frame's
  `v3d_viewport_gl(.., y0_top = false, ..)`, as `frame_check --triangle71`
  builds it; the flipped-rows line tells a frame drawn the other way up.
- `get_throttled` is recorded by `tools/board.sh` in `meta.txt`, not by the
  image; check it before reading a GPU fault as a hardware one.
