<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# NanoPC-T6 glmark2-es2 at main c74c2964 (2026-11-25)

One NanoPC-T6 (RK3588, Mali-G610, back end gpu-mali through panthor) glmark2-es2
session on the board's own Armbian 26.8.10 trixie (kernel
6.18.55-current-rockchip64). Plan #409, issues #33 and #7 (ROADMAP stage 5,
the benchmark criterion). It replaces the 2026-11-11 record at main a57d8104.

## What ran, against what

- Built from main `c74c2964` (c74c29643f769d7ccc716a8b566b55d1deae1979), the
  `origin/main` tip when the item started, with
  `tools/cts/mali-glmark2-build.sh` (`dev=no`): glmark2 22c527cb with
  `tools/cts/mali-glmark2-pbuffer.patch` (sha256 2035a5d4...), the
  `tools/mali/panthor-gles` pair, Debian trixie's g++-14 (MALI_DEQP_TOOLCHAIN).
  `--check` passed: needs GLIBC_2.38, GLIBCXX_3.4.32, CXXABI_1.3.9 (board
  floors 2.41, 3.4.33, 1.3.15).
- sha256: `lib/libGLESv2.so.2` 7f8fcd55d3912c57..., `lib/libEGL.so.1`
  b7cb2d36423e9d3b..., `bin/glmark2-es2-mali` 9f8fb256a89de919...
- Command: `tools/cts/mali-glmark2-session.sh <builddir> <outdir>` with
  `BOARD_HOST` (from the lease) set; no runner options: the 33 default
  scenes, one glmark2-es2 process a scene, 800x600 EGL pbuffer, 180 s bound.
- Span: 2026-11-25 16:18:22 to 16:22:36 UTC (board clock).

## Result

- 33 of 33 ran: **16 ok**, 16 refused by the online compiler at
  glCompileShader / glLinkProgram (set-up failed, nothing drawn), 1 crash
  (terrain).
- **No score**: not every scene has an FPS. ms/frame summed over the 16 clean
  scenes: 11.20.
- 16 GL error lines, all in terrain: 0x501 (`GL_INVALID_VALUE`) from
  glGetAttribLocation and glGetUniformLocation, 8 each. glmark2's
  `Program::release()` (`src/libmatrix/program.cc`) sets a program whose
  shader was refused to 0, and its renderers still look names up in it;
  program 0 is not a program object, so this is ES 2.0's answer, not a
  defect of the GL core. 0 in the 16 clean scenes.
- Terrain: `TerrainRenderer::init_program()`
  (`src/scene-terrain/terrain-renderer.cpp:91`) returns on the refusal
  before it sets the mesh's attribute locations (line 136), and
  `Mesh::render_vbo` (`src/mesh.cpp:575`) indexes the empty vector:
  glmark2 aborts on libstdc++'s assertion (`stl_vector.h:1130
  '__n < this->size()'`, no yakogl-mali-exit line). The probe after it
  (`run/probes.tsv`) passed.
- Pictures against the `mali` column of `tools/cts/glmark2-pictures.txt`:
  absent 17, no reference 15, mismatch 1 (desktop shadow: probe tr `b6b6b6`
  against `c0c0c0`, 10 levels over a tolerance of 8, mean within 1; the
  desktop scenes move their windows with time, and the 94566c1a run read the
  mismatch at probe bl instead). Against the `am62p` column (another board,
  context only, `tally-table-am62p.txt`): 8 match, 8 mismatch, 17 absent.
- Board left as found: panthor kernel lines 0 in every scene (resets 0,
  timeouts 0, faults 0); /boot unchanged (419 files), modules unchanged (100),
  U-Boot env not compared (no fw_printenv), boot_id unchanged, board reachable
  (`state-compare.txt`). Session exit 0.

## Against the host forecast

`tally-host-verdicts.txt` (`--host-verdicts
tests/fixtures/mali_glmark2_verdicts.txt --strict`, exit 0): host predicts 16
clean, 17 refused; the board agrees scene for scene, 0 flagged. Each refused
scene's board text is the first refusal the fixture names
(`docs/notes/mali-glmark2-forecast.md`, section 1).

## Against the earlier runs

- main a57d8104 (2026-11-11): 1 of 33 clean, 31 refused, terrain abort, 80 GL
  error lines. Now 15 more scenes run clean and the GL error lines are 16.
- main 94566c1a and 038b7d7d (2026-11-25, runs of closed pull requests, not
  merged): the same verdict per scene.

## Files

- `session.txt`: the session's own output; `preflight.txt`,
  `state-before.txt`, `state-after.txt`, `state-compare.txt`.
- `run/`: the runner's directory (`meta.txt`, `scenes.tsv`, `pictures.txt`,
  `probes.tsv`, `summary.txt`, per scene `log/<n>.out|err` and
  `kmsg/<n>.log|class`).
- `tally-*.txt`: `tools/cts/glmark2-mali-tally.py` on `run/` (table, results
  row, status row, picture lines, causes), `tally-host-verdicts.txt` and
  `tally-table-am62p.txt` as above.

Redaction: the board login's home and this host's paths are written `<home>`
and `<repo>`; no address, user, UART path or MAC. Each file without a comment
syntax has a `.license` sidecar.
