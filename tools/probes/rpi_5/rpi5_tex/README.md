<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# rpi5_tex

The first texture look-ups on the Raspberry Pi 5's V3D 7.1.10, before the
7.1 GL table exists: one textured quad (T1) and one set of seamless cube-map
samples (C1), each read back. Roadmap P10 and question U8 of
`docs/notes/pre-study-rpi5.md` (the host half of U8 is
`docs/notes/v3d71-cube-seamless.md`). ROADMAP stage 5 (issue #7).

```sh
west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_tex yakogl/tools/probes/rpi_5/rpi5_tex
```

(West 1.5 takes the CMake option only after `--`:
`west build -p always -b rpi_5 -d build/rpi5_tex yakogl/tools/probes/rpi_5/rpi5_tex -- -DYAKOGL_WERROR=ON`.)

Its configuration and board overlay are `rpi5_clear`'s: `CMakeLists.txt`
merges `../rpi5_clear/prj.conf` before this directory's empty `prj.conf`,
and uses `../rpi5_clear/boards/rpi_5.overlay`. The driver files it compiles,
unchanged:

- `v3d_hw.c`: power-on, identity, MMU, bin and render submission.
- `v3d_frame.c` and `v3d_frame71_tri.c`: the 7.1 clear-then-triangle frame
  (`v3d_frame_build_triangle`), as `rpi5_triangle` uses it.
- `v3d_tex71.c`: the 7.1 texture and sampler records (`v3d_tex71_pack_state`,
  `v3d_tex71_pack_sampler`, checked by `tools/v3d/check71.d/tex71.sh`).
- `v3d_tex.c` and `v3d_tiling_mesa.c`: Mesa's level layout and texel tiling,
  and the P0/P1 uniform words (`v3d_tex_p0`, `v3d_tex_p1`; 7.1 keeps 4.2's
  P0 and P1).

`main.c`, `v3d_hw.c` and the two frame files are built at `V3D_CL_VER` 71
(as in `rpi5_triangle`); `v3d_tex71.c` is its own 7.1 unit; `v3d_tex.c` and
`v3d_tiling_mesa.c` are built at the default, as in `rpi5_tfu`.

## The programs: our 7.1 compiler

Both programs come from **our V3D 7.1 online compiler**
(`src/shaderc/online/v3d/chain71.rs`, `compile_program71`, triangles,
VPM 65536). No Mesa binary is used.

| Part | Sources | FS / VS / CS instructions | FS uniforms |
|---|---|---|---|
| T1 | `shaders/tex2d.vert`, `shaders/tex2d.frag` (`texture2D(u_tex, v_uv)`, `gl_test_texture_fs_src`'s shape) | 11 / 19 / 18 | TMU config P0 (data 3), P1 (data 4) |
| C1 | `shaders/cube.vert`, `shaders/cube.frag` (`textureCube(u_s, v_dir)`, `gl_test_texcube_fs_src`'s shape) | 20 / 21 / 18 | TMU config P0 (data 3), P1 (data 4) |

`src/shaders_ours.h` is generated, with the code words, the uniform streams
and the prog_data as `v3d_frame.h`'s structs. Regenerate or check it with:

```sh
tools/probes/rpi_5/rpi5_tex/gen/gen_shaders.sh           # write src/shaders_ours.h
tools/probes/rpi_5/rpi5_tex/gen/gen_shaders.sh --check   # compare only
```

The script builds `gen/gen_shaders.rs` as a throwaway crate under
`target/rpi5_tex_gen/` with a path dependency on yakogl (features
`host-test,gpu-v3d,shaderc-online,no-shader-table`, those of
`tests/v3d71_gltest_shaders.rs`). If the compiler refuses a program, the
script prints the refusal's name and exits 1. It writes no header then.

The frame builder (`v3d_frame71_tri.c`) writes no texture uniform. So
before the frame is built, the probe turns each FS `QUNIFORM_TMU_CONFIG_P0`
or `P1` word (contents 40 or 41) into a constant: `v3d_tex_p0(texture
record) | data` or `v3d_tex_p1(sampler record) | data`, as `v3d_job.c` does
for the 4.2 table. Any other binding than 0 is refused.

## What it does

- **M1, M2** (as `rpi5_triangle`): the mailbox switches the V3D clock, then
  power-on and identity run. One page table covers the probe's statics, and
  the MMU is turned on.
- **T1**:
  - **The texture.** A 64x64 RGBA8 texture whose texels follow `rpi5_tfu`'s
    pattern: R = 4x+2, G = 4y+1, B = 4(x^y), A = 255-2(x+y). Every channel
    moves with x or y, so a transposed, shifted or channel-swapped look-up
    shows. It is laid out by `v3d_tex_layout(64, 64, 1, 4)` (Mesa's
    layout, level 0 UIF) and tiled by `v3d_tex_store_level`.
  - **The records.** The texture record is `v3d_tex71_pack_state` with type
    RGBA8, swizzle RGBA, transfer function none, and base, max level and
    depth 0, 0 and 1. The sampler is NEAREST/NEAREST with no mip filter,
    wrap REPEAT. The probe prints both records next to Mesa's `gl_tex2d`
    words (`tools/v3d/pi5/README.md`, the texture record table): texture
    words 1 to 3 (the others hold the base address) and sampler words 0
    and 1.
  - **The draw.** One full-screen quad (two triangles) into a 64x64 raster
    RGBA8888 target, cleared to 0 first. The uv is (0,0) at NDC (-1,-1) and
    (1,1) at (1,1), and the viewport is `glViewport(0, 0, 64, 64)`, not
    flipped. So pixel (x, row y) samples the centre of texel (x, y).
  - **The comparison.** Every one of the 4096 pixels is compared with its
    texel. A pixel is wrong when a channel differs by more than 1 (the
    tolerance covers the TMU's 16-bit return and the RGBA8 store). A guard
    row above and one below the target must stay untouched; a changed guard
    row adds one to the wrong count.
- **C1**:
  - **The cube map.** It has `gl_cube`'s shape: six 4x4 RGBA8 faces as
    layers (`v3d_tex_layout(4, 4, 1, 4)`, each layer
    `v3d_tex_layer_stride` apart, depth 6). Each face is one colour (R in
    the low byte): +X `ff0000ff`, -X `ffffff00`, +Y `ff00ff00`, -Y
    `ffff00ff`, +Z `ffff0000`, -Z `ff00ffff`. Any two faces that share an
    edge differ by 255 in at least one channel.
  - **The sampler.** It is `gl_cube`'s LINEAR sampler: mag and min LINEAR,
    no mip filter, wrap CLAMP in S, T and R. That last part is what st/mesa
    forces for a seamless cube (`v3d71-cube-seamless.md` section 3). Both
    records are printed next to Mesa's `gl_cube` words.
  - **The samples.** There are 54 directions, each with a major component
    of exactly ±1, so no normalisation is in play. Per face there are:
    - the centre;
    - four edge samples, one a quarter texel inside each edge: the minor
      component is ±0.875, so a quarter of the bilinear footprint lies
      beyond the edge;
    - four corner samples, one an eighth texel inside each corner in both
      directions: both minor components are ±0.8125.

    Each direction fills one 8x8 block of the 64x64 target (two triangles
    with the direction at all three vertices). The probe reads the block's
    centre pixel.
  - **The two predictions.** Each sample is compared with both:
    - **Seamless.** The texel beyond an edge comes from the neighbouring
      face. For an edge sample that gives 0.75 A + 0.25 B, with A the
      sampled face and B the neighbour. For a corner sample the weights are
      A 0.766, B and C 0.109 each, and the corner texel that no face has
      takes 0.016, as the mean of A, B and C. The corner convention can
      move a channel by at most about 4.
    - **Non-seamless.** The face is clamped to its own edge texels, which
      gives A.

    A sample matches a prediction when every channel is within 8. Every
    edge and corner sample differs from A by at least 28 in some channel,
    so the two predictions never both match the same sample. The centre
    sample is A under both.
  - **The answer.** `seamless=yes` when all 48 edge and corner samples
    match the seamless prediction, `no` when all match the non-seamless
    one, and `neither` otherwise. The wrong count is the samples (centres
    included) that are not within 8 of the seamless prediction. A changed
    guard row adds one and makes the answer `neither`.

The caches are handled as in `rpi5_triangle`. Every GPU buffer is cleaned
and invalidated before each frame, `v3d_caches_invalidate` runs before each
bin (the L2T, so that the TMU does not read stale texels), the GPU caches
are cleaned after the render, and the target is invalidated before the
read-back.

## The console

Before the verdicts, the probe prints `rpi5_tex: build ...`, `rpi5_tex:
cache: ...` and `rpi5_tex: rpi_5, ...`. Then come the `M1` and `M2` step
lines and, per part:

- `T1 texture: ...` or `C1 cube: ...`, with the packed records and
  `equal` or `differ from` Mesa's words;
- `tex2d: FS uniform N: TMU config P0|P1 0x...` (or `cube: ...`);
- `T1 frame: ...` or `C1 frame: ...`, then the `bin` and `render` lines;
- `T1 read-back: ...` with the first wrong pixel if there is one, or one
  `C1 sample N face F centre|edge|corner dir/10000 (x y z): got ...,
  seamless ... (diff D), non-seamless ... (diff D)` line per sample, then
  `C1 read-back: ...`.

Then come exactly these three lines, in this order:

```
v3d_tex: PASS <n> of <m> px wrong        or  v3d_tex: FAIL <n> of <m> px wrong
v3d_cube: PASS seamless=yes <n> of <m> samples wrong
                                         or  v3d_cube: FAIL seamless=<yes|no|neither> <n> of <m> samples wrong
rpi5_tex: done (both parts ran)          or  rpi5_tex: FAIL (<step>)
```

`m` is 4096 for T1 and 54 for C1. `v3d_tex` passes with 0 px wrong.
`v3d_cube` passes only with `seamless=yes` and 0 samples wrong. A 4.2-like
TMU that does not filter across faces gives `v3d_cube: FAIL seamless=no
48 of 54 samples wrong`.

Both verdict lines are printed on every path. A step that stops the probe
before a part runs prints that part's line as FAIL with every px or sample
counted wrong (`seamless=neither` for C1), and the end line becomes
`rpi5_tex: FAIL (<step>)`. The step is one of `M1 mailbox`, `M1 power-on`,
`M1 identity`, `M2 page table`, `M2 cache clean`, `M2 MMU`, or a T1 or C1
step: `layout`, `v3d_tex71 refused the records`, `program`, `frame build
refused`, `cache clean`, `bin: timeout|MMU fault|out of memory`,
`render: timeout|MMU fault`, `GPU cache clean`, `target cache invalidate`.
A GPU stop in T1 ends the run before C1. `rpi5_tex: done` means that both
parts ran to their verdicts, whatever those verdicts are.

Nothing runs afterwards. The board layer's PM watchdog
(`CONFIG_YAKOGL_RPI5_WDOG`, armed in the first `PRE_KERNEL_1` hook) resets
the SoC, and tryboot brings Linux back.

## On the board
