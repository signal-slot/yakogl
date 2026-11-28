<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# A 4x default framebuffer for the AM62P dEQP-GLES3 image (NS-MSAA-DEFAULT-FB)

Read at commit `5f781c491` (origin/main tip of plan #400's item). Docs only;
nothing here was run on a board.

## 1. The 16 cases and what they say

`dEQP-GLES3.functional.multisample.default_framebuffer.*`, 16 cases of
`tools/cts/gles3-mustpass.txt:8398-8413` (common_edge_{big_quad,small_quads,
viewport_quad}, constancy_{alpha_to_coverage,both,both_inverted,
sample_coverage,sample_coverage_inverted}, depth, num_samples_{line,polygon},
proportionality_{alpha_to_coverage,sample_coverage,
sample_coverage_inverted}, sample_coverage_invert, stencil). The AM62P rows
are `NotSupported  No multisample buffers`
(`docs/es3/am62p-gles3-notpass.tsv:764-779`; family `ns-default-fb-no-multisample`,
`tools/cts/am62p-gles3-families.txt:34`; cause row `NS-MSAA-DEFAULT-FB`,
`tools/cts/am62p-notpass-causes.py:165`).

These are the ES 2.0 multisample cases run on the *default* framebuffer of
the context. dEQP throws `NotSupported("No multisample buffers")` when the
render target it was given has no samples (the same text the GLES2 image logs,
`tools/cts/log/2026-11-12-t6-gles2-S1-a50d87b5/results.tsv:1722`). The render
target is whatever `--deqp-gl-config-name` selects: dEQP's default config is
single-sampled, so every case stops at its first line. (The dEQP source is
not in this workspace's dependency cache, so the throw's file:line is not
quoted; the message and the cause are what the GLES2 runs of the same group
already showed, and the GLES2 image fixed by the name below.)

## 2. Cause, with file:line

The EGL of the library already lists a 4x config on the PowerVR. The dEQP
image just never asks for it.

* **Where the configs are listed.** `src/gl/egl.rs:425-437`: `CONFIG_1X` (id
  1), `CONFIG_4X` (id 2), `CONFIG_565` (id 3), `ALL_CONFIGS`.
  `offered_configs()` (`src/gl/egl.rs:443-469`) removes the 4x one only for
  the V3D and GC7000 builds while their backend MSAA switch is off; the
  PowerVR and the Mali get `ALL_CONFIGS`. Attributes of config 2:
  `EGL_SAMPLES 4`, `EGL_SAMPLE_BUFFERS 1` (`src/gl/egl.rs:933-946`), RGBA8888,
  depth 24, stencil 8, `EGL_RENDERABLE_TYPE` ES2|ES3 (`src/gl/egl.rs:957`).
  `config_samples()` (`src/gl/egl.rs:480`) gives a surface its sample count
  (`src/gl/egl.rs:1424`, `1599`); `gl_context_bind_surface(..., samples)`
  (`src/gl/egl.rs:2114`, `2340`) makes it the default framebuffer's
  `default_samples`. No zephyr/ or driver/ file lists EGL configs: there is
  one EGL (`src/gl/egl.rs`) for all boards.
* **Where the dEQP image picks its config.**
  `driver/samples/zephyr/deqp/Kconfig:52-67`, `DEQP_GL_CONFIG_NAME`: passed as
  `--deqp-gl-config-name=<name>` when not empty; empty keeps dEQP's
  single-sampled default. `tools/cts/am62p-deqp-images.sh:112-114` reads a row
  marker `(with DEQP_GL_CONFIG_NAME=<name>)` from a `# image <LABEL>:` line,
  and `:171-183` builds the row with `-DCONFIG_DEQP_GL_CONFIG_NAME`; every
  other row gets an explicit empty name. `check-deqp-images.py:145-148`
  requires exactly `rgba8888d24s8ms4` when `--ms4` and an empty name
  otherwise.
* **Why the GLES3 table never sets it.** `tools/cts/am62p-gles3-tree.txt` is
  written by `tools/cts/gen-am62p-gles3-tree.py` with no labelled row and no
  config marker; the `multisample.*` glob sits in the row at line 13 of
  `tools/cts/am62p-gles3-tree.txt`, with `negative_api`, `occlusion_query`, `pbo`, ...), built on the
  default config. The GLES2 tree solved the same thing with a second table,
  `tools/cts/am62p-ms4-tree.txt` (one `# image MS4: ... (with
  DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4)` row; 16 GLES2 cases pass on it).
  `board-stage.sh am62p gles3` builds only the GLES3 table
  (`tools/cts/board-stage.sh:421-427` is the GLES2 `ms4` set).

## 3. Not the same thing as `YAKOGL_MSAA_FBO`

`YAKOGL_MSAA_FBO` (`CMakeLists.txt:110`, `zephyr/Kconfig:756`,
`zephyr/CMakeLists.txt:181`) turns on the crate's `msaa-fbo` feature: 4x
*framebuffer objects* and `GL_MAX_SAMPLES 4`. The 4x *window* config exists
without it (the GLES2 ms4 image and gl_test 16/198 use it on the AM62P).
`tools/cts/am62p-deqp-images.sh:153` passes `CONFIG_YAKOGL_MSAA_FBO=y` for
every gles3 image already, so the plan's items `msaa-arm-fix` and
`msaa-arm-build` (making that reach the Zephyr build) are about
`multisample.fbo_4_samples.*`, `fbo.msaa.*` and `GL_MAX_SAMPLES`, not about
these 16. Two consequences:

1. The 16 need **no src/gl or src/pvr change** to get a 4x window; the knob
   is only the dEQP config name.
2. With both on, a 4x window and 4x FBOs coexist in one context: the blit
   rules of the window against FBOs are already in the GL core and host-tested
   (`tests/es3_msaa_gl_window.rs`; `src/gl/blit.rs:60`, `:119`).
   `fbo.blit` / `fbo.msaa` rows must stay on the 1x config (the 4x window
   changes `GL_SAMPLE_BUFFERS` of the default framebuffer, which the blit
   rules read), so the 4x config must not be applied to the whole table.

## 4. What a 4x window surface needs on the PowerVR

Nothing new in the back end; this is what it already does and the evidence:

* A 4x render target resolved on the job's end: `src/pvr/backend.rs:979`
  (resolve on emit for a multisampled frame), `:1924`/`:5992` (`MsStore`, the
  samples across a split frame), `:6013` (the window as the one multisampled
  target of ES 2.0), `src/pvr/spm.rs:51`/`:234` (4x at 800x600 sizes the
  scratch), `ISP_CTL_MSAA4` (`src/pvr/backend_tests.rs:5486`).
* Measured on the AM62P: the ES 2.0 ms4 image passes all 16
  `dEQP-GLES2.functional.multisample.*` (`tools/cts/am62p-ms4-tree.txt:3-5`);
  constancy needed a4f64950 and is pinned by
  `tests/es2_pvr_ms4_constancy.rs` and gl_test 198.
* The ES 3.0 build differs from the ES 2.0 one only by the context version
  and `YAKOGL_MSAA_FBO`. The 16 GLES3 cases use the same draw code as the
  GLES2 group; they should pass wherever the GLES2 ones did, but this is a
  prediction, not a measurement. Cases that are not like the GLES2 ones:
  `depth` and `stencil` (4x depth/stencil of the window with an ES 3 context;
  `src/pvr/ds_stage.rs:10`) and `num_samples_*` (reads `GL_SAMPLES`/
  `GL_SAMPLE_BUFFERS` of the default framebuffer: 4 and 1).

## 5. Design (one)

A second, tiny GLES3 table, as the GLES2 tree did, so the main table and its
generator checks stay as they are:

1. **New table `tools/cts/am62p-gles3-ms4-tree.txt`** with one row:

   ```
   # image GMS4: multisample.default_framebuffer.* (with DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4)
   dEQP-GLES3.functional.multisample.default_framebuffer.*
   ```

   The label form is what the builder and `board.sh tree_rows`
   (`tools/board.sh:3223-3235`) already read; a table of labelled rows only
   avoids the mixed-table wait described in `am62p-ms4-tree.txt:7-10`.
   Build: `DEQP_MODULE=gles3 tools/cts/am62p-deqp-images.sh
   tools/cts/am62p-gles3-ms4-tree.txt <dir> gms4`; the image gets
   `CONFIG_YAKOGL_MSAA_FBO=y` like every gles3 image
   (`am62p-deqp-images.sh:153`).
2. **`tools/cts/check-deqp-images.py`**: the builder passes `--ms4` and
   `--gles3` together (`am62p-deqp-images.sh:188-190`); the two flags are
   independent in `conformance_rules` (`:124-148`), so nothing changes there.
   Its table self-test (`:398-401`) names only `am62p-ms4-tree.txt`; add the
   new table to the same check.
3. **`tools/cts/gen-am62p-gles3-tree.py`**: keep `multisample.*` in the main
   table's glob (its 16 `default_framebuffer` cases stay NotSupported there
   and are *replaced by* the GMS4 rows when the two records are tallied), or
   narrow the unit with `NARROW` to `multisample.fbo_*`, `multisample.common*`
   and the other groups, so the cases are run once. The second form is cleaner;
   `--check` (every mustpass case matched by exactly one filter) must then
   count the GMS4 filter too. Either form is a tools/cts change for a later
   item, not for this note.
4. **`tools/cts/board-stage.sh`**: a set `am62p gles3-ms4`, cloned from the
   `ms4` set (`:421-427`), and the tally merge that replaces the 16 rows in
   `docs/es3/am62p-gles3-notpass.tsv`.
5. **The 16 rows' time**: 16 cases at the GLES2 ms4 rate is far inside the
   600 s watchdog; one image.

No change under `src/gl/**` and none under `src/pvr/**` is needed for the
config. A change is only needed if the board shows a failing case (a
`depth`/`stencil` or `num_samples_*` Fail), and that goes to its own item.
`src/gl` changes named: **none**.

## 6. The host test to add

`tests/egl_pvr_ms4_es3.rs` (not under `src/gl`; same harness as
`tests/egl_bench_configs.rs`):

```text
cargo test --no-default-features --features host-test,gpu-powervr --test egl_pvr_ms4_es3
```

1. Pick the config as dEQP does for `rgba8888d24s8ms4`:
   `eglChooseConfig` with `EGL_RED/GREEN/BLUE/ALPHA_SIZE 8`, `EGL_DEPTH_SIZE
   24`, `EGL_STENCIL_SIZE 8`, `EGL_SAMPLES 4`, `EGL_SAMPLE_BUFFERS 1`,
   `EGL_SURFACE_TYPE EGL_PBUFFER_BIT`, `EGL_RENDERABLE_TYPE
   EGL_OPENGL_ES3_BIT`; assert exactly config id 2 comes back, and that with
   no sample attributes config 1 or 3 sorts first (the default image stays
   1x).
2. Create the pbuffer and an ES 3 context
   (`EGL_CONTEXT_CLIENT_VERSION 3`) on it, make current, and read
   `GL_SAMPLE_BUFFERS` (1) and `GL_SAMPLES` (4) of the default framebuffer.
3. Draw a quad with `glSampleCoverage` and read back; the resolved edge pixel
   must be a mix, as `tests/es2_pvr_ms4_constancy.rs`' model says, to pin
   that the ES 3 context uses the same multisampled target.
4. Check the other direction: the same ES 3 context on config 1 reads
   `GL_SAMPLE_BUFFERS` 0, so the 16 cases' guard is what the 1x image meets
   today.

The test fails today only if EGL stopped offering the 4x config or an ES 3
context stopped inheriting `default_samples`; its job is to pin the premise of
the GMS4 row, since the cause is the image, not the library. The board run of
the GMS4 row is the real check (prediction: 16 pass; the GLES2 ms4 image
passed 16).

## 7. For the planner

* The 16 are one small, low-risk board row once `tools/cts` can build it
  (items touching `tools/cts/**` are held by board-a/board-b in this plan, so
  the table and the `board-stage.sh` set wait for them).
* If the GMS4 row is not wanted, the cheapest alternative is setting
  `DEQP_GL_CONFIG_NAME` for the whole `multisample.*` image row; it also puts
  `fbo_4_samples` and `fbo_max_samples` on a 4x window, which no result
  depends on today, so it is not recommended.
* Nothing in this note was measured; the counts are from the TSV at
  origin/main.
