<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D Mesa pin programs

GLSL ES 3.00 programs that pin Mesa's V3D 4.2 code for the ES 3.0 look-up
forms the online compiler selects: the shadow look-ups (`sampler2DShadow`,
`samplerCubeShadow`, `sampler2DArrayShadow`, with a level, an offset, a
projection), `textureGrad` (2D, cube, 3D with an offset), the 3D texel
offset, and vertex-stage look-ups of the 32-bit texture classes (the
`--vs-ret32` key of `v3d_shc`, docs/notes/v3d-vs-ret32.md).

They live here and not under `tools/shaders` because every file there is a
row of several pinned corpora (`tests/online_corpus.rs`,
`tests/v3d_online_corpus.rs`, `tests/gc_const_map.rs`, the Mali and Vivante
corpus verdicts), and a V3D pin should move none of them.

`pairs.txt` lists the programs: `<vs> <fs> [v3d_shc flags...]`, one a line.
`tools/v3d/sim_fixture.py --pins` compiles each with Mesa's `v3d_shc`
(`tools/v3d/README.md`) and writes `tests/fixtures/v3d_mesa_pins.txt`, in the
format of `tests/fixtures/v3d_mesa_programs.txt` plus a `flags` line per pair.
`tests/v3d_sim_mesa.rs` (`the_pin_pairs_...`) runs Mesa's code on the QPU
interpreter against the IR evaluator where the harness can, pins the
population, and prints our compiler's word counts beside Mesa's for each.
Regenerate the fixture in the same commit as a change here.
