<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 glmark2-es2 at main 22c5fc5f (2026-11-25)

One glmark2-es2 session on the NanoPC-T6 (RK3588, Mali-G610, back end
gpu-mali through panthor), on the board's own Armbian 26.8.10 trixie (kernel
6.18.55-current-rockchip64). Plan #409, issues #33 and #7 (ROADMAP stage 5,
the benchmark criterion). It replaces the 2026-11-11 record at main a57d8104.

## What ran, against what

- Built from main `22c5fc5f` (22c5fc5f45c0d790973043ceda1e54a09dd04543), the
  `origin/main` tip when the item started, with
  `tools/cts/mali-glmark2-build.sh` (`dev=no`). The build used glmark2
  22c527cb with `tools/cts/mali-glmark2-pbuffer.patch` (sha256
  2035a5d461ab64f9...), the `tools/mali/panthor-gles` pair and Debian
  trixie's g++-14 (MALI_DEQP_TOOLCHAIN). `--check` passed. The build needs
  GLIBC_2.38, GLIBCXX_3.4.32 and CXXABI_1.3.9; the board floors are 2.41,
  3.4.33 and 1.3.15.
- sha256: `lib/libGLESv2.so.2` d18d7ad875618396..., `lib/libEGL.so.1`
  b7cb2d36423e9d3b..., `bin/glmark2-es2-mali` 9f8fb256a89de919...
- Command: `tools/cts/mali-glmark2-session.sh <builddir> <outdir>`, with
  `BOARD_HOST` (from the lease) and `MALI_GLMARK2_MAIN_REF` set. The second
  names this clone's copy of `origin/main`, because the build host could not
  fetch. No runner options were given, so it ran the 33 default scenes, one
  glmark2-es2 process a scene, on an 800x600 EGL pbuffer with a 180 s bound.
- Span: 2026-11-25 20:46:23 to 20:50:41 UTC (board clock). One attempt.

## Result

- 33 of 33 ran: **16 ok**, 16 refused by the online compiler at
  glCompileShader / glLinkProgram (set-up failed, nothing drawn), 1 crash
  (terrain).
- **No score**: not every scene has an FPS. The 16 clean scenes take 11.41
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
  absent 17, no reference 15, match 1 (desktop shadow). Against the `am62p`
  column (another board, context only, `tally-table-am62p.txt`): 9 match,
  7 mismatch, 17 absent.
- Board left as found: 0 panthor kernel lines in every scene (resets 0,
  timeouts 0, faults 0). /boot unchanged (419 files), modules unchanged
  (100), U-Boot env not compared (no fw_printenv), boot_id unchanged, board
  reachable (`state-compare.txt`). Session exit 0.

## Against the host forecast

`tally-host-verdicts.txt` (`--host-verdicts
tests/fixtures/mali_glmark2_verdicts.txt --strict`, exit 0): the host
predicts 16 clean and 17 refused, and the board agrees scene for scene, with
0 flagged. Each refused scene's board text is the first refusal the fixture
names (`docs/notes/mali-glmark2-forecast.md`, section 1).

## Against the earlier runs

- main a57d8104 (2026-11-11): 1 of 33 clean, 31 refused, terrain abort, 80 GL
  error lines. Now 15 more scenes run clean, and there are 16 GL error lines.
- main d4f95c29 (2026-11-25, the run of a closed pull request, not merged):
  the same verdict per scene. Between d4f95c29 and 22c5fc5f only
  `src/mali/frame_uniarray_tests.rs` (a test) moved under `src/mali`,
  `src/shaderc/online/mali`, `src/gl`, `tools/mali/panthor-gles` and the
  fixture. Desktop shadow's picture, a mismatch on that run, matches here:
  its probes sit on a window edge that moves with time.

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
