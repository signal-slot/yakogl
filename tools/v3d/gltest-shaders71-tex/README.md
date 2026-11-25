<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Our V3D 7.1 programs of gl_test's texture, mipmap and cube cases (P10)

The programs that gl_test's first 2D-texture, `glGenerateMipmap` and cube-map
cases link, compiled by our online compiler's 7.1 back end
(`src/shaderc/online/v3d/chain71.rs`, `compile_program71`) for the Raspberry
Pi 5 (BCM2712 D0, V3D 7.1.10). The six P9 programs are in
`../gltest-shaders71/`; Mesa's offline binaries of the same three programs are
in `../gltest-mesa71-tex/` (there named `texture`, `texture2` and `cube`).

`tests/v3d71_gltest_tex_shaders.rs` compiles each program again and compares
`ours_<name>.h` and `ours_<name>.json` byte for byte (regeneration: that
test's header). It also checks this file: the three rows of the case table,
each program's section with its two source symbols and its `* state:` line
(`pinned` with the file names, or `refused:` with the compiler's error text),
that the cases are the first of their kind in the dispatch table, and that the
programs are exactly the ones the two cases link. The `gl_test.c` line numbers
below are a reading at commit `ab919063`; the test does not compare them, so
they move when `gl_test.c` is edited above them.

## Selection rule

Shared with the Mesa-side item `mesa71tex`. In `driver/tools/pvrlab/gl_test.c`'s
dispatch table (`switch (c)`, in order) take the first case that samples a 2D
texture filled by `glTexImage2D`, the first that calls `glGenerateMipmap`, and
the first that samples a `GL_TEXTURE_CUBE_MAP`. Sources: the ES 1.00 set,
`driver/tools/pvrlab/gl_test_shaders_es100.h` (generated from
`tools/shaders/gl_test_es100/`; what a V3D build takes). Every distinct
program those cases link is pinned.

## Cases

| shape | case | dispatch | function | what it does |
|---|---|---|---|---|
| 2D texture | 2 `texture` | gl_test.c:24149 | `case_texture`, gl_test.c:1837 | `glTexImage2D` at gl_test.c:1884 (an 8x8 RGBA texture, NEAREST, CLAMP_TO_EDGE), sampled by `uv_texture` (`link_program` at gl_test.c:1843) |
| mipmap | 2 `texture` (the first with `glGenerateMipmap`) | gl_test.c:24149 | `case_texture`, gl_test.c:1837 | levels 0..2 by `glTexImage2D` at gl_test.c:1923-1925, then `glGenerateMipmap(GL_TEXTURE_2D)` at gl_test.c:1976, sampled by `uv_texture`; NEAREST_MIPMAP_NEAREST and LINEAR_MIPMAP_LINEAR |
| cube map | 41 `es3cube` | gl_test.c:24209 | `case_es3cube`, gl_test.c:6538 | six 8x8 faces by `glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f)` at gl_test.c:6556, NEAREST, CLAMP_TO_EDGE; sampled by `cube` (`link_program` at gl_test.c:6549) per face axis, then +Y as a framebuffer attachment |

Case 2 is both the first 2D-texture case and the first `glGenerateMipmap`
case, so the 2D and the mipmap shapes share their programs; its two-unit draws
(`link_program` at gl_test.c:2000) add `uv_texture2`. So there are two cases
and three programs.

The cube case is an ES 3.00 one (`#version 300 es`, `samplerCube`). The
ES 1.00 set carries its two sources verbatim (`gl_test_es300_cube_*_src` in
`gl_test_shaders_es100.h`, from `tools/shaders/es300/cube.{vert,frag}`), but
that set's `GL_TEST_SHADERS_SKIP_REASON` skips case 41 ("ES 3.00 source,
outside the ES 2.0 compiler"), so a build that takes the ES 1.00 set does not
run it. The program is pinned because the rule names the case. The first
ES 1.00 cube case in dispatch order is 115 (`case_texcube`, the one Mesa's
`tools/v3d/pi5/gl_cube` reference follows); the rule does not select it and it
is not pinned here.

## Programs

Each pinned program has `ours_<name>.h` (C prefix `ours71_<name>`: the QPU
words, the uniform streams, the prog_data fields and the `GL Shader State
Record Draw Index` fields that come from the programs) and `ours_<name>.json`
(the same, in `v3d_shc`'s JSON shape with `gpu.ver` 71 and `record71`), as
`../gltest-shaders71/` has them. A program the compiler cannot build has no
file and a `* state: refused: <error>` line.

### `uv_texture`

* vertex stage `gl_test_uv_vs_src`, fragment stage `gl_test_texture_fs_src` (`gl_test_shaders_es100.h`)
* sources: `tools/shaders/gl_test_es100/uv.vert` + `texture.frag`
* cases: 2 `texture` (`case_texture`: the 2D, the mipmap and the `glGenerateMipmap` draws)
* state: pinned: `ours_uv_texture.h` and `ours_uv_texture.json` (compiled by `compile_program71`)

### `uv_texture2`

* vertex stage `gl_test_uv_vs_src`, fragment stage `gl_test_texture2_fs_src` (`gl_test_shaders_es100.h`)
* sources: `tools/shaders/gl_test_es100/uv.vert` + `texture2.frag`
* cases: 2 `texture` (`case_texture`: the two-texture-unit draws)
* state: pinned: `ours_uv_texture2.h` and `ours_uv_texture2.json` (compiled by `compile_program71`)

### `cube`

* vertex stage `gl_test_es300_cube_vs_src`, fragment stage `gl_test_es300_cube_fs_src` (`gl_test_shaders_es100.h`)
* sources: `tools/shaders/es300/cube.vert` + `cube.frag`
* cases: 41 `es3cube` (`case_es3cube`)
* state: pinned: `ours_cube.h` and `ours_cube.json` (compiled by `compile_program71`)

## Refusals

None: the 7.1 back end builds all three programs, so the table needs Mesa's
offline binary (`../gltest-mesa71-tex/`) for none of them.
