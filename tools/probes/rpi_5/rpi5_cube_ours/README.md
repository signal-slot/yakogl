<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_cube_ours

The first Pi 5 cube-map case (plan #384 acceptance c7, roadmap P10,
question U8 of `docs/notes/pre-study-rpi5.md`) through the driver's V3D 7.1
job builder `v3d_job71.c`, and not through the clear-then-triangle frame
that `rpi5_tex` uses. One seamless cube-map draw (C1) and the level 1 of a
cube map made by the 7.1 TFU (C2), each read back. The probe also prints the
core's `IDENT1.VPM_SIZE`, which `vpmsrc71` left open. ROADMAP stage 5
(issue #7).

```sh
west build -p always -b rpi_5 -d build/rpi5_cube_ours yakogl/tools/probes/rpi_5/rpi5_cube_ours -- -DYAKOGL_WERROR=ON
```

`tools/check.sh` builds it in its `zephyr_rpi5_probe_images` row, as every
directory under `tools/probes/rpi_5/` with a `CMakeLists.txt` and a
`prj.conf`. It runs `west build -p always -b rpi_5 -d <dir> <probe> --
-DZEPHYR_MODULES=<tools/cts/zephyr-modules.sh> -DYAKOGL_DIR=<checkout>
-DYAKOGL_WERROR=ON` from the west workspace.

Its configuration and board overlay are `rpi5_clear`'s: `CMakeLists.txt`
merges `../rpi5_clear/prj.conf` before this directory's empty `prj.conf`,
and uses `../rpi5_clear/boards/rpi_5.overlay`. The driver files it compiles
are used unchanged:

- `v3d_hw.c`: power-on, identity, MMU, bin and render submission.
- `v3d_job71.c` with `v3d_frame.c`: the 7.1 job. That is the binning and
  render lists, the D0's draw-index shader record, the attribute records
  without a default-attribute buffer, `Clear Render Targets`, and the
  uniform streams. The builder writes the FS stream's TMU config P0 and P1
  words (contents 40 and 41) from the draw's sampler binding
  (`struct v3d_job71_tmu`). The probe does not patch them.
- `v3d_tex71.c`: the 7.1 texture and sampler records.
- `v3d_tfu71.c`: the 7.1 TFU register values (`v3d_tfu71_pack`, IOC
  included).
- `v3d_tex.c` and `v3d_tiling_mesa.c`: Mesa's level layout and texel
  tiling, and `v3d_tex_p0`/`v3d_tex_p1`, which the probe uses only to check
  the words the builder wrote.

`main.c`, `v3d_hw.c`, `v3d_frame.c` and `v3d_job71.c` are built at
`V3D_CL_VER` 71. `v3d_tex71.c` is its own 7.1 unit. `v3d_tfu71.c`,
`v3d_tex.c` and `v3d_tiling_mesa.c` are built at the default.

## The program: our 7.1 compiler

The program comes from **our V3D 7.1 online compiler**
(`src/shaderc/online/v3d/chain71.rs`, `compile_program71`, triangles,
VPM 65536). No Mesa binary is used.

| Sources | FS / VS / CS instructions | FS uniforms |
|---|---|---|
| `shaders/cube.vert`, `shaders/cube.frag` (`textureCube(u_s, v_dir)`, `gl_test_texcube_fs_src`'s shape) | 20 / 21 / 18 | TMU config P0 (data 3), P1 (data 4), binding 0 |

The program is the same as `rpi5_tex`'s C1 program. The generated code
words, uniform streams and prog_data are identical, under the names
`rpi5_cube_cube_*`. `src/shaders_ours.h` is generated. Regenerate or check
it with:

```sh
tools/probes/rpi_5/rpi5_cube_ours/gen/gen_shaders.sh           # write src/shaders_ours.h
tools/probes/rpi_5/rpi5_cube_ours/gen/gen_shaders.sh --check   # compare only
```

The script builds `gen/gen_shaders.rs` as a throwaway crate under
`target/rpi5_cube_ours_gen/`. The crate has a path dependency on yakogl,
with the features `host-test,gpu-v3d,shaderc-online,no-shader-table`. If
the compiler refuses the program, the script prints the refusal's name,
exits 1 and writes no header.

## What it does

### M1 and M2

As in `rpi5_tex`:

- **M1:** the mailbox switches the V3D clock, then power-on and identity
  run. After the identity the probe prints this line:

  ```
  rpi5_cube_ours: IDENT1 VPM_SIZE <v> (<v * 8192> bytes) CTL_IDENT1 0x... HUB_IDENT1 0x...
  ```

  `VPM_SIZE` is bits 31:28 of the core's `IDENT1` (`V3D_CTL_0_IDENT1`,
  `driver/docs/rpi5-v3d71-facts.md`), counted in 8 KiB units as Mesa reads
  it. The hub's `IDENT1` has no VPM field, so its word is printed beside it
  only for reference.
- **M2:** one page table covers the probe's statics, and the MMU is turned
  on.

### C1: the seamless cube through v3d_job71

- **The cube map.** It has `gl_cube`'s shape and colours, as `rpi5_tex`'s
  C1: six 4x4 RGBA8 faces as layers, one level, one colour per face (+X
  `ff0000ff`, -X `ffffff00`, +Y `ff00ff00`, -Y `ffff00ff`, +Z `ffff0000`,
  -Z `ff00ffff`). The records come from `v3d_tex71_pack_state` (depth 6) and
  `v3d_tex71_pack_sampler`. The sampler is LINEAR/LINEAR with no mip filter
  and wrap CLAMP in S, T and R, which is st/mesa's seamless rule. Both
  records are printed next to Mesa's `gl_cube` words.
- **The job.** One `v3d_job71_begin`, one `v3d_job71_draw` and one
  `v3d_job71_end`:
  - The target is a 64x64 raster RGBA8 render target 0, cleared to 0, with
    a guard row above and one below.
  - The draw is GL_TRIANGLES with 324 vertices: 54 quads, one per sample,
    each filling one 8x8 block of the target.
  - Sampler binding 0 holds the two records' addresses.
  - The viewport is `glViewport(0, 0, 64, 64)`, not flipped.

  The probe prints the TMU config words that the builder wrote into the FS
  stream, each next to `v3d_tex_p0`/`v3d_tex_p1` of the same record (equal
  on the host for this draw).
- **The samples.** Each face has 9 samples (54 in all), each with a major
  component of exactly ±1:
  - the centre;
  - four edge samples, a quarter texel inside each edge (minor component
    ±0.875);
  - four corner samples, an eighth texel inside each corner in both
    directions (minor components ±0.8125).

  The seamless and non-seamless predictions are `rpi5_tex`'s
  (`../rpi5_tex/README.md`, "C1"):
  - **Seamless:** an edge sample is 0.75 A + 0.25 B. A corner sample is
    A 0.766, B and C 0.109 each, and a corner texel that no face has is the
    mean of A, B and C.
  - **Non-seamless:** the result is A.
- **The comparison.**
  - **Every pixel** of each sample's 8x8 block is compared with the
    seamless prediction. A pixel is wrong when a channel differs by more
    than 8. This gives 576 px per face and 3456 in all.
  - Each block's centre pixel also decides `seamless=yes|no|neither`, as
    in `rpi5_tex`: all 48 edge and corner centres match seamless, all match
    non-seamless, or neither.
  - The 640 px of the 10 blocks that no quad covers must keep the clear.
  - A changed px there, or a changed guard row, makes the answer `neither`
    and adds one wrong px for each kind.

### C2: level 1 of a cube by the 7.1 TFU

- **The cube map.** A second 4x4 RGBA8 cube map with two levels, in
  Mesa's layout: per layer, level 0 is LINEARTILE at +0x1000, level 1 at
  +0xfc0, and the layer stride is 4160. Level 0 of face f holds texel
  (x, y) = R 64x+16, G 64y+16, B 40f+20, A 255-16x-4y. Every other byte is
  `0xa5`.
- **The TFU jobs.** One TFU job per face, as Mesa's glGenerateMipmap does
  per layer. Each job is `v3d_tfu71_pack` in place (source = destination =
  the face's level 0), with NUMMM 1 and DIMTW. The registers are written in
  `rpi5_tfu`'s order (facts file section 7): IIA, IIS, ICA, IUA, IOA,
  **IOC**, IOS, COEF0..3, then ICFG with its IOC bit, which starts the job.
  Completion is the hub's TFUC bit or a TFU_CS conversion count, 1 s at
  most per job.
- **The comparison.** All six faces are read back and untiled. Level 0
  must be unchanged. Level 1 (2x2 per face, 24 px) must be within 2 per
  channel of the 2x2 box of level 0, (a+b+c+d+2)/4. A changed level 0 adds
  one wrong px.

On the host (gcc, the same driver files), `v3d_tfu71_pack` accepts each of
these jobs (`ICFG 0x01840000 IOS 0x00040004 IOC 0x00003011`), and
`v3d_job71` records the C1 draw (tile 64x64, BCL 126, RCL 96, indirect 219
bytes). Whether the 7.1.10 TFU takes a 4x4 LINEARTILE base is the board's
answer. `rpi5_tfu`'s PASS was for a 64x64 job from a raster source.

The caches are handled as in `rpi5_tex`. Every GPU buffer is cleaned and
invalidated before its job, `v3d_caches_invalidate` runs before the bin and
before the TFU jobs, the GPU caches are cleaned after each part, and the
CPU's copy is invalidated before each read-back.

## The console

Before the verdicts, the probe prints `rpi5_cube_ours: build ...`,
`rpi5_cube_ours: cache: ...`, `rpi5_cube_ours: rpi_5, ...`, the `M1` and
`M2` lines, and the `IDENT1 VPM_SIZE` line. Then come the step lines:

- `C1 cube: ...`, with the packed records and `equal` or `differ from`
  Mesa's words;
- `C1 FS uniform N: TMU config P0|P1 0x... (equals|differs from
  v3d_tex_pN 0x...)`;
- `C1 job: ...`, then `C1 bin: ...` and `C1 render: ...`;
- one `C1 sample N face F centre|edge|corner dir/10000 (x y z): centre ...,
  seamless ... (diff D), non-seamless ... (diff D); block n of 64 px off
  seamless (max M)` line per sample, then `C1 read-back: ...`;
- `C2 cube: ...`, then one `C2 face F TFU: ...` line per job, one
  `C2 face F level 1 (0,0): ...` line per face, and `C2 read-back: ...`.

Then come exactly these lines, in this order:

```
v3d_cube71 face 0 +X: PASS|FAIL <n> of 576 px wrong
v3d_cube71 face 1 -X: ...
v3d_cube71 face 2 +Y: ...
v3d_cube71 face 3 -Y: ...
v3d_cube71 face 4 +Z: ...
v3d_cube71 face 5 -Z: ...
v3d_cube71: PASS seamless=yes <n> of 3456 px wrong
                                         or  v3d_cube71: FAIL seamless=<yes|no|neither> <n> of 3456 px wrong
v3d_cube71_tfu: PASS|FAIL <n> of 24 px wrong (<what>)
rpi5_cube_ours: done (C1 and C2 ran)     or  rpi5_cube_ours: FAIL (<step>)
```

The verdicts:

- A face passes with 0 px wrong.
- `v3d_cube71` passes only with `seamless=yes` and 0 px wrong. The face
  counts add up to the total, apart from the stray and guard px.
- `v3d_cube71_tfu` passes with 0 px wrong.

Every verdict line is printed on every path. A step that stops the probe
before a part runs prints that part's lines as FAIL with every px counted
wrong (`seamless=neither` for C1). The TFU line then carries the step as
its `<what>`, or `did not run` when C1 stopped the probe. The end line
becomes `rpi5_cube_ours: FAIL (<step>)`, where the step is one of:

- `M1 mailbox`, `M1 power-on`, `M1 identity`, `M2 page table`,
  `M2 cache clean` or `M2 MMU`;
- a C1 step: `layout`, `v3d_tex71 refused the records`, `program`,
  `v3d_job71_begin refused the target`, `v3d_job71_draw full|refused`,
  `v3d_job71_end refused`, `job sizes`, `cache clean`,
  `bin: timeout|MMU fault|out of memory`, `render: timeout|MMU fault`,
  `GPU cache clean` or `target cache invalidate`;
- a C2 step: `layout`, `face F: v3d_tfu71_pack refused the job`,
  `face F: TFU CS ... before the job`, `face F: TFU MMU fault`,
  `face F: TFU timeout`, `GPU cache clean`, `mip cube cache invalidate`
  or `level 0|1 untile refused`.

A GPU stop in C1 ends the run before C2. `rpi5_cube_ours: done` means that
both parts ran to their verdicts, whatever those verdicts are.

Nothing runs afterwards. The board layer's PM watchdog
(`CONFIG_YAKOGL_RPI5_WDOG`, armed in the first `PRE_KERNEL_1` hook) resets
the SoC, and tryboot brings Linux back.

## On the board
