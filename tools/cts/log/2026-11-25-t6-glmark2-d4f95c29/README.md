<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 glmark2-es2 at main d4f95c29 (2026-11-25)

One glmark2-es2 session on the NanoPC-T6 (RK3588, Mali-G610, back end
gpu-mali through panthor), on the board's own Armbian 26.8.10 trixie (kernel
6.18.55-current-rockchip64). Plan #409, issues #33 and #7 (ROADMAP stage 5,
the benchmark criterion). It replaces the 2026-11-11 record at main a57d8104.

## What ran, against what

- Built from main `d4f95c29` (d4f95c2943787df0d65b8f34d8ac478551015f7a), the
  `origin/main` tip when the item started, with
  `tools/cts/mali-glmark2-build.sh` (`dev=no`). The build used glmark2
  22c527cb with `tools/cts/mali-glmark2-pbuffer.patch` (sha256
  2035a5d461ab64f9...), the `tools/mali/panthor-gles` pair and Debian
  trixie's g++-14 (MALI_DEQP_TOOLCHAIN). `--check` passed. The build needs
  GLIBC_2.38, GLIBCXX_3.4.32 and CXXABI_1.3.9; the board floors are 2.41,
  3.4.33 and 1.3.15.
- sha256: `lib/libGLESv2.so.2` 793b5f06a031ce4e..., `lib/libEGL.so.1`
  b7cb2d36423e9d3b..., `bin/glmark2-es2-mali` 9f8fb256a89de919...
- Command: `tools/cts/mali-glmark2-session.sh <builddir> <outdir>`, with
  `BOARD_HOST` (from the lease) and `MALI_GLMARK2_MAIN_REF` set. The second
  names this clone's copy of `origin/main`, because the build host could not
  fetch. No runner options were given, so it ran the 33 default scenes, one
  glmark2-es2 process a scene, on an 800x600 EGL pbuffer with a 180 s bound.
- Span: 2026-11-25 19:33:22 to 19:37:42 UTC (board clock).
- An earlier attempt of this session (19:29 UTC) lost its ssh path at scene
  13 when the host's forwarding restarted. The board was not involved: the
  boot_id was unchanged and the board answered right after. The runner's
  stage was removed and the whole session was run again; this slice is that
  second run. The first attempt's output is in the gitignored record only.

## Result

- 33 of 33 ran: **16 ok**, 16 refused by the online compiler at
  glCompileShader / glLinkProgram (set-up failed, nothing drawn), 1 crash
  (terrain).
- **No score**: not every scene has an FPS. The 16 clean scenes take 11.00
  ms/frame summed.
- 16 GL error lines, all in terrain: 0x501 (`GL_INVALID_VALUE`) from
  glGetAttribLocation and glGetUniformLocation, 8 each. glmark2's
  `Program::release()` (`src/libmatrix/program.cc:163`) sets a program whose
  shader was refused to 0, and its renderers still look names up in it.
  Program 0 is not a program object, so this is the answer ES 2.0 requires,
  not a defect of the GL core. The 16 clean scenes have none.
- Terrain: `TerrainRenderer::init_program()`
  (`src/scene-terrain/terrain-renderer.cpp:91`) returns on the refusal
  before it sets the mesh's attribute locations (line 136), and
  `Mesh::render_vbo` (`src/mesh.cpp:575`) indexes the empty vector.
  glmark2 aborts on libstdc++'s assertion (`stl_vector.h:1130
  '__n < this->size()'`) and prints no yakogl-mali-exit line. The probe
  after it (`run/probes.tsv`) passed.
- Pictures against the `mali` column of `tools/cts/glmark2-pictures.txt`:
  absent 17, no reference 15, mismatch 1. The mismatch is desktop shadow:
  probe tr `adadad` against `c0c0c0` (19 levels off, tolerance 8) and probe bl
  `fcfcfc` against `f3f3f3` (9 off), with the mean within 1. The desktop
  scenes move their windows with time, so the probes on a window edge read
  differently from run to run: the c74c2964 run read tr 10 off, and the
  94566c1a run read bl. Against the `am62p` column (another board, context
  only, `tally-table-am62p.txt`): 8 match, 8 mismatch, 17 absent.
- Board left as found: 0 panthor kernel lines in every scene (resets 0,
  timeouts 0, faults 0). /boot unchanged (419 files), modules unchanged
  (100), U-Boot env not compared (no fw_printenv), boot_id unchanged, board
  reachable (`state-compare.txt`). Session exit 0.

## Against the host forecast

`tally-host-verdicts.txt` (`--host-verdicts
tests/fixtures/mali_glmark2_verdicts.txt --strict`, exit 0): the host
predicts 16 clean and 17 refused, and the board agrees scene for scene, with
0 flagged. Each refused scene's board text is the first refusal the fixture
names (`docs/notes/mali-glmark2-forecast.md`, section 1). The fixture,
`src/mali/**`, `src/shaderc/online/mali/**` and `tools/mali/panthor-gles`
are unchanged between main c74c2964 and d4f95c29.

## Against the earlier runs

- main a57d8104 (2026-11-11): 1 of 33 clean, 31 refused, terrain abort, 80 GL
  error lines. Now 15 more scenes run clean, and there are 16 GL error lines.
- main c74c2964, 94566c1a and 038b7d7d (2026-11-25, runs of closed pull
  requests, not merged): the same verdict per scene.

## Files

- `session.txt`: the session's own output; `preflight.txt`,
  `state-before.txt`, `state-after.txt`, `state-compare.txt`.
- `run/`: the runner's directory (`meta.txt`, `scenes.tsv`, `pictures.txt`,
  `probes.tsv`, `summary.txt`, and per scene `log/<n>.out|err` and
  `kmsg/<n>.log|class`).
- `tally-*.txt`: `tools/cts/glmark2-mali-tally.py` on `run/` (table, results
  row, status row, picture lines, causes), plus `tally-host-verdicts.txt`
  and `tally-table-am62p.txt` as above.

Redaction: the board login's home is written `<home>`, this host's checkout
`<repo>` and the cross toolchain's place `<deps>`. There is no address, user,
UART path or MAC. Each file without a comment syntax has a `.license`
sidecar.
