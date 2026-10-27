<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Host replay: what the samples' replays share

The Slint sample has a replay (`tools/slint/replay/`; the AGL cluster
sample's, `cluster/host/replay/`, went with that sample on 2026-10-19) that
drives the sample's Rust
archive against the **real** library on the host: the GL core, the in-process
compiler and the back end's command generation of `libyakogl.a`, with the
Linux OSAL (`src/osal/linux.c`: GPU memory is malloc'd, every submit completes
at once, nothing is rendered). This directory holds its shared parts.

| file | what it is |
|---|---|
| `replay.h`, `replay_common.c` | EGL up as the samples' `main.c` does it (RGBA8888 + stencil 8, ES 2.0, a window over a malloc'd buffer), the loader the archives get (`eglGetProcAddress`, which logs every name YakoGL does not answer), the per-frame check (`eglSwapBuffers`, `glGetError`, the draws the library refused, the draws that sampled an incomplete texture) and the summary line |
| `common.sh` | builds `libyakogl.a` per GPU (the top-level CMake, as `tools/hostharness/glmark2/run.sh` does: `YAKOGL_OSAL=linux`, `YAKOGL_SHADERC=online`, `YAKOGL_REPORT_ES2=ON`, release), links a replay program and runs it with `YAKOGL_ERROR_LOG=1000000` so every GL error is logged with its entry point |
| `report.py` | per GPU: the GL errors, refused draws, incomplete-texture lines, unresolved names and unexpected lines of one run |
| `selftest/` | `selftest.sh <out>`: the replay program against a stand-in archive (`fake_sample.c`, one triangle per frame) per GPU. A clean run must pass; with `FAKE_BAD=1` (a bad `glEnable`, an unknown name) the report must name both |

## GPUs

All three run: `powervr`, `vivante` and `v3d` (the default is all three).
`src/osal/linux.c` drives the V3D as the host model of the Pi 4's OSAL (its
section "the V3D": one carve-out, the board's allocator and free rule, the
`YAKOGL_V3D_*` settings), as for `tools/hostharness/glmark2/run.sh`.

Each GPU runs with its board's settings, read from the board layer's
`driver/zephyr/lib/board/<board>/app.conf` (`hr_board`, `hr_settings` in
`common.sh`): AM62P for
`powervr`, i.MX8MP for `vivante`, Pi 4 for `v3d`. Every
`CONFIG_YAKOGL_V3D_<NAME>` becomes `YAKOGL_V3D_<NAME>` (so the V3D runs in
the sample's own carve-out: 48 MiB for Slint). A value already in
the environment wins. `<out>/<gpu>-settings.txt` lists what a run used.

## What a replay reports

* **GL errors**: every `GL error 0x… in gl…` line the library logs (the entry
  point that raised it), and `glGetError` after each frame.
* **Refused draws**: the library's refusal lines and `yakogl_draws_refused()`
  per frame.
* **Incomplete textures**: the draws that sampled an incomplete texture, per
  frame and in the summary (`yakogl_incomplete_texture_draws`, read as a
  difference like the refused draws), and the library's incomplete lines. The
  counter is what finds them: such a draw reads (0,0,0,1) with no GL error
  (ES 2.0 3.7.10), and the library names only the first incomplete texture of
  a process (`src/gl/draw.rs`, "a texture on unit N is incomplete"), so a
  frame's count can be above 0 with no line of its own. A framebuffer that is
  incomplete at a draw raises GL_INVALID_FRAMEBUFFER_OPERATION and shows as a
  GL error (and as the library's "framebuffer N incomplete" line). One case
  the counter does not see: a bound texture that was never given any image.
  The draw path skips a texture without storage before its completeness check
  (`src/gl/draw.rs`, "no texture: the backend binds a black 1x1"), so such a
  draw samples (0,0,0,1) uncounted and unlogged. The selftest therefore uses
  a one-level texture under a mipmap filter. A follow-up in `src/gl/draw.rs`
  would count that case too.
* **GPU memory**: an allocation the V3D carve-out refused (`out of GPU
  memory`). The V3D's tiled-copy high-water and sampler-variant lines are
  listed for information.
* **Unresolved names**: GL names the archive asks `eglGetProcAddress` for and
  YakoGL does not answer (glow resolves them all at context creation and
  panics on a call to one).

The replay program's exit status is 1 when any frame failed, ended with a GL
error, or a name did not resolve; `report.py` also fails on any logged
line of the classes above.

## Limits

* The stream is the sample's own deterministic frame sequence, run live. It is
  not stored as a call trace: the archive is the recording, and each run
  regenerates the calls (this keeps Slint's generated UI out of the
  repository).
* Nothing is rendered, so a finding that shows only in pixels is invisible.
  The library's own lines about it (errors, refusals, incomplete textures) are
  what these runs see.
* The PowerVR and GC7000 fake GPUs never fail and never run out of memory, so
  those boards' pool limits are not modelled. The V3D's carve-out is
  modelled.
