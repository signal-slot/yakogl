<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tests/fixtures/deqp_random_dead_cube/`

GLSL ES 1.00 sources that dEQP-GLES2 (VK-GL-CTS 93bca01) generates for eleven
`functional.shaders.random.*` cases, copied from the shader sources a host run
logged (`tools/cts/host-deqp-run.sh` with `GPU=powervr`, filter
`shaders.random`, at 2c87c2e4). Each one is the stage that has the
samplerCube; the file name is the case name after `shaders.random.`.

Every one has a `textureCube` whose result nothing reads. With the native cube
on (`pvr_native_cube`, the default since step C) the PowerVR online compiler
refused all of them with `Usc(Build(Unsupported("a cube coordinate the chain did
not consume")))`: dead-code removal in selection deleted the cube sample and
kept its `SelOp::SampleZ`. `tests/usc_limits.rs`
`a_dead_cube_sample_takes_its_z_with_it` compiles them.

`texture.vertex.41.board.vert` is the eleventh: the source dEQP generated for
`texture.vertex.41` ON THE BOARD (1,110 bytes, a dead `textureCube`), read from
the AM62P with the deqp sample's CONFIG_DEQP_ECHO_QPA (image ce920150,
2026-09-25). The x86 host generates a different vertex shader for that case
(1,033 bytes, a `sampler2D` and no cube; the two differ from their third
line on, and the board's carries `float(2.38419e-07)`), so the host run could
not show it; why dEQP's generator diverges between the two builds was not
measured.
