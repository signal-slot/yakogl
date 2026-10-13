<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Pi 4 dEQP-GLES2 final tree, rows 0-14, at origin/main 20a48791 (pi4-es2-final-a)

Date: 2026-10-14. Item `pi4-es2-final-a` (plan #20, roadmap stage 1).
Rows 15-28 are item `pi4-es2-final-b`, at the same commit.

**Result: rows 0-14 ran in full on the Pi 4 (V3D). 46 Fail, all of them
BGRA renderbuffer cases. The cause is a regression since d56e79e5:
`glRenderbufferStorage(GL_BGRA_EXT)` now returns GL_INVALID_ENUM in a
report-es2 build (section 5).** Every other case has the same result as in
`pi4-es2-final-r2.md`, except one more CompatibilityWarning in
`flush_finish` (section 4). Stage 1 (0 Fail on the Pi 4) is not met at this
commit.

- Commit: `origin/main` after `git fetch origin`,
  `20a487918c74845d9bcd966a66e2b862ad6d0292` ("ccfarm: merge
  ccfarm/01a11e01-7a31-72fb-a648-c3861169a3ab (#489)", 2026-10-13). The
  images report `V3D_BUILD=20a48791`, zephyr `af17c0c8aa2a`.
- Rows: 0-14 of `tools/cts/pi4-full-tree.txt` (AL D E F1 FB1 FB2 FF H LA
  MISC NS RT S1 S2 S3), 11953 mustpass names.
- Records: under the gitignored `records/` of the build checkout, not
  committed.

## 1. Console probe (before anything was built)

The procedure of `pi4-console-probe.md`: open the Pi 4's serial bridge with
`tools/am62p_netboot_uart.py capture` for 40 s, Pi idle in Linux. After
about 10 s, `echo PROBE_MARKER_FA14 > /dev/serial0` over ssh (ttyAMA0, the
`console=` UART of the kernel command line).

- Connection accepted, not busy.
- Captured: 19 bytes, `PROBE_MARKER_FA14\r\n`, nothing else. The stamped
  file has one line, `PROBE_MARKER_FA14`.
- Boot output: row AL's console then shows the image's banner through the
  same bridge (`*** Booting Zephyr OS build af17c0c8aa2a ***`,
  `v3d_deqp: build 20a48791`). Every row's console has its banner and its
  end line.

## 2. Image build

`tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <outdir> <labels>`
with the cached `rpi_4b` dEQP stage 1 as `DEQP_WORK`. Only the 15 labels of
rows 0-14 were built. The first call was stopped after MISC (the shell's
background time limit); the half-built NS directory was removed and NS-S3
were built by a second call. Both calls printed the same first line:

```
pi4-deqp-images: build from 20a48791 (20a487918c74845d9bcd966a66e2b862ad6d0292)
AL rc=0 sha256=e6d0ba49f02b7d05f63e4df25c7d2b133dec534c7cfa033f025b5bdd65b3eea6 build=20a48791
D rc=0 sha256=6a53eb507a501cadd0dd0e91e6c441fb066bf4a7ce50af30787d5bfabe849e0f build=20a48791
E rc=0 sha256=59821eac8a4cb946f95c3e110231044fe3ceae5d5e96374b0164b43838c841b5 build=20a48791
F1 rc=0 sha256=986880b5236fbb156887c7d755990dd0196612b3ca0aec7239688309b90d83c0 build=20a48791
FB1 rc=0 sha256=601164b6019346ce170406c5d7e039898088b8f6c12029664710672ada30d5f2 build=20a48791
FB2 rc=0 sha256=33189f3d9f47dea14a9bbe191f51bf151f1c9583e09ff6acec77ae7bcc848ac1 build=20a48791
FF rc=0 sha256=87f7b0b67b959803b9809e4e3e68e2d46d4e580aa2064707a997d7a87f3222df build=20a48791
H rc=0 sha256=7624b162a4d6ebe73389c54b5a1462101f2363dcddde1e9805d2ef2d1fe30127 build=20a48791
LA rc=0 sha256=ede59df421721da382d3dfdb8ac8ac56980dd9d3c8b8ec94fc02c7d380d19dce build=20a48791
MISC rc=0 sha256=f362d170f5b41c6d06d8478aa1b664ee2f70fc9aaad4495b59abb167bed74b3a build=20a48791
NS rc=0 sha256=f9e9705a278890acf73d1aa656e9acadd509eb6457f69732b2a382f5f2751766 build=20a48791
RT rc=0 sha256=087f60c01e6b20a1523aefa0aaf5b62ae01f4ccc825fdf74eb4d836fc5dc93b7 build=20a48791
S1 rc=0 sha256=f8c94ce7ff4b375eb530769045f1437fe2a465d5a95327af6b1d7a89f3a7663c build=20a48791
S2 rc=0 sha256=d0bff1788c141bbf72d5dcd54a50f9fd8525aaa29b99d115c28eba6b3772c0ac build=20a48791
S3 rc=0 sha256=6b9ffebccad4c9a612f2d238f1447e7053df15ac7e736382e86bcc06248aebd1 build=20a48791
```

## 3. Row verdicts and tally

`tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt <outdir>` with
`TREE_ROWS=0-14`, `TREE_IMAGE_DIR=<the image dir>`, `BOARD_BOOT=tryboot`,
then `tools/board.sh pi4 tally <outdir>`. The per-run lines of the tally,
verbatim:

```
board.sh pi4 AL rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=299s
board.sh pi4 D rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 E rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 F1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=298s
board.sh pi4 FB1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 FB2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 FF rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_2 resets=0 record=absent back_after=298s
board.sh pi4 H rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_3 resets=0 record=absent back_after=296s
board.sh pi4 LA rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 MISC rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=301s
board.sh pi4 NS rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=297s
board.sh pi4 RT rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=307s
board.sh pi4 S1 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=302s
board.sh pi4 S2 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=298s
board.sh pi4 S3 rc=0 verdict=done boot=tryboot row boot_verdict=complete result=legacy:v3d_deqp_0 resets=0 record=absent back_after=300s
```

(`result=legacy:v3d_deqp_N` counts warnings, not failures: FB1 and FB2
print `v3d_deqp_0` with 3 and 43 Fail. The per-case lines below are the
result.) Every record: recoveries 0, GPU reset lines 0.

The tally (the Pi 4 console form prints a mustpass table, not a
`TALLY (n entries)` line; this is its total), verbatim:

```
== mustpass: 11953 cases in these records' case filters (5212 outside them, not run, left out); 11953 with a result (12090 case lines seen)
TOTAL                          11877     46     25      3      2      0
                               (pass   fail   n.s.     QW  other   none)
```

The 5212 names outside are rows 15-28 (item `pi4-es2-final-b`).

### Per row, against d56e79e5

The image's own `deqp: done` counter. All rows `complete=1`, every case
begun has a result.

| Row | Name | Pass | Fail | NotSupported | Warnings | d56e79e5 (r2) |
|---|---|---|---|---|---|---|
| 0 | AL | 57 | 0 | 0 | 0 | same |
| 1 | D | 305 | 0 | 0 | 0 | same |
| 2 | E | 453 | 0 | 0 | 0 | same |
| 3 | F1 | 1923 | 0 | 0 | 0 | same |
| 4 | FB1 | 389 | **3** | 0 | 0 | 392 Pass, 0 Fail |
| 5 | FB2 | 276 | **43** | 0 | 0 | 319 Pass, 0 Fail |
| 6 | FF | 3 | 0 | 0 | 2 (CompatibilityWarning) | 4 Pass, 1 CompatibilityWarning |
| 7 | H | 651 | 0 | 0 | 3 (QualityWarning) | same |
| 8 | LA | 19 | 0 | 0 | 0 | same |
| 9 | MISC | 58 | 0 | 16 | 0 | same |
| 10 | NS | 633 | 0 | 9 | 0 | same |
| 11 | RT | 200 | 0 | 0 | 0 | same |
| 12 | S1 | 2949 | 0 | 0 | 0 | same |
| 13 | S2 | 1910 | 0 | 0 | 0 | same |
| 14 | S3 | 2188 | 0 | 0 | 0 | same |
| | rows 0-14 | 11877 | 46 | 25 | 3 + 2 | 11924, 0, 25, 3 + 1 |

## 4. Failures by case name, against pi4-es2-final-r2.md

`pi4-es2-final-r2.md` (d56e79e5) has no Fail in any row. All 46 below are
new. Names are prefixed `dEQP-GLES2.functional.`.

FB1, 3 cases, each `Fail (Expected no error but got GL_INVALID_ENUM)`:

- `fbo.completeness.renderable.renderbuffer.color0.bgra`
- `fbo.completeness.renderable.renderbuffer.depth.bgra`
- `fbo.completeness.renderable.renderbuffer.stencil.bgra`

FB2, 43 cases, each `Framebuffer is not complete:
'GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT'` (es2fFboRenderTest.cpp:554; :1472
for `shared_colorbuffer_clear.rbo_bgra`):

- `fbo.render.color.`: `blend_npot_rbo_bgra`,
  `blend_npot_rbo_bgra_depth_component16`, `blend_rbo_bgra`,
  `blend_rbo_bgra_depth_component16`
- `fbo.render.color_clear.`: `rbo_bgra`, `rbo_bgra_depth_component16`,
  `rbo_bgra_depth_component16_stencil_index8`, `rbo_bgra_stencil_index8`
- `fbo.render.depth.`: `npot_rbo_bgra_depth_component16`,
  `rbo_bgra_depth_component16`
- `fbo.render.recreate_colorbuffer.`: `no_rebind_rbo_bgra`,
  `no_rebind_rbo_bgra_depth_component16`,
  `no_rebind_rbo_bgra_depth_component16_stencil_index8`,
  `no_rebind_rbo_bgra_stencil_index8`, `rebind_rbo_bgra`,
  `rebind_rbo_bgra_depth_component16`,
  `rebind_rbo_bgra_depth_component16_stencil_index8`,
  `rebind_rbo_bgra_stencil_index8`
- `fbo.render.recreate_depthbuffer.`:
  `no_rebind_rbo_bgra_depth_component16`,
  `no_rebind_rbo_bgra_depth_component16_stencil_index8`,
  `rebind_rbo_bgra_depth_component16`,
  `rebind_rbo_bgra_depth_component16_stencil_index8`
- `fbo.render.recreate_stencilbuffer.`:
  `no_rebind_rbo_bgra_depth_component16_stencil_index8`,
  `no_rebind_rbo_bgra_stencil_index8`,
  `rebind_rbo_bgra_depth_component16_stencil_index8`,
  `rebind_rbo_bgra_stencil_index8`
- `fbo.render.resize.`: `rbo_bgra`, `rbo_bgra_depth_component16`,
  `rbo_bgra_depth_component16_stencil_index8`, `rbo_bgra_stencil_index8`
- `fbo.render.shared_colorbuffer.`: `rbo_bgra`,
  `rbo_bgra_depth_component16`, `rbo_bgra_depth_component16_stencil_index8`,
  `rbo_bgra_stencil_index8`
- `fbo.render.shared_colorbuffer_clear.rbo_bgra`
- `fbo.render.shared_depthbuffer.`: `rbo_bgra_depth_component16`,
  `rbo_bgra_depth_component16_stencil_index8`
- `fbo.render.stencil.`: `npot_rbo_bgra_depth_component16_stencil_index8`,
  `npot_rbo_bgra_stencil_index8`, `rbo_bgra_depth_component16_stencil_index8`,
  `rbo_bgra_stencil_index8`
- `fbo.render.stencil_clear.`: `rbo_bgra_depth_component16_stencil_index8`,
  `rbo_bgra_stencil_index8`

Not Pass, not a failure (30 cases): the same 25 NotSupported as in r2
(`multisample.*` 16, `negative_api.texture.compressedtexsubimage2d_*` 9),
the same 3 QualityWarning (`rasterization.interpolation.basic.lines`,
`.projected.lines`, `.projected.lines_wide`), and 2 CompatibilityWarning:
`flush_finish.flush` (also in r2) and `flush_finish.flush_wait` (new; a
timing warning, Pass in r2).

## 5. Cause of the 46 failures

All 46 use a renderbuffer of the unsized format `GL_BGRA_EXT` (0x80E1).

1. What dEQP expects. In the pinned VK-GL-CTS,
   `modules/glshared/glsFboCompletenessTests.cpp` lines 191-194 and 295-297:
   when GL_EXTENSIONS lists `GL_EXT_texture_format_BGRA8888`, both
   `GL_BGRA` and `GL_BGRA8_EXT` are `RENDERBUFFER_VALID` and
   `COLOR_RENDERABLE`. yakogl lists that extension (`src/gl/state.rs:64`).
2. What yakogl does. Commit `8bbf9ba2` ("report-es2: buffer and framebuffer
   entry points refuse ES 3.0-only values", 2026-10-12, not in d56e79e5)
   made `renderbuffer_format_advertised` (`src/gl/framebuffer.rs:2264-2284`)
   decide each renderbuffer format from the extension list in a report-es2
   build. It names `GL_BGRA8_EXT` but not `GL_BGRA_EXT`, so the unsized
   name falls to `_ => false`, and `glRenderbufferStorage` answers
   GL_INVALID_ENUM (FB1). The renderbuffer then has no storage, so every
   framebuffer built on it is `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` (FB2).
3. Host reproduction (this session, then removed): a test in
   `tests/es2_report_guard.rs` that calls `glRenderbufferStorage` with each
   name, run with
   `cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_report_guard`:
   `GL_BGRA8_EXT` gives GL_NO_ERROR, `GL_BGRA_EXT` gives
   `left: "GL_INVALID_ENUM"`, `right: "GL_NO_ERROR"`. The rule is not per
   GPU, so the AM62P and i.MX8MP report-es2 builds should fail the same
   cases.
4. Why the gate did not see it. The unit test that checks both names,
   `the_sized_bgra_name_is_taken_wherever_the_unsized_one_is`
   (`src/gl/framebuffer.rs:7561`), is `#[cfg(not(feature = "report-es2"))]`.
   The report-es2 pin `the_bgra8_renderbuffer_is_accepted_for_now`
   (`tests/es2_report_guard.rs:587`) checks only `GL_BGRA8_EXT`.

The fix is not part of this item (a board session). It belongs to the ES 2.0
error-rules work: add `GL_BGRA_EXT` beside `GL_BGRA8_EXT` in
`renderbuffer_format_advertised`, and pin both names in a report-es2 test.
Then rows FB1 and FB2 need a re-run on the Pi 4.

## 6. How the run went

- The tree's first start stopped at once with `no image` for AL (rc 3):
  `board.sh`'s `builder_ok` waits for a `<label>.built` file only when some
  `.built` file already exists, and the first image was still building.
  Nothing was staged. The tree was started again after `AL.built` appeared.
- At 02:37, after LA, row MISC was refused: `BOARD_UART bridge is busy
  (another client holds it)`. Another ccfarm task on the same worker was
  running `tools/board.sh am62p tree tools/cts/common-tree.txt` with its
  `BOARD_UART` set to this bridge, which carries the Pi 4's console. That
  AM62P run read the Pi 4's console, not the AM62P's. It was not
  interrupted. The tree was resumed (finished rows are skipped) with
  `TREE_RETRIES=120 TREE_RETRY_S=20`. MISC got the bridge at 03:05, once the
  other run's rows ended. The refused attempts staged nothing and rebooted
  nothing; their run directories (only `meta.txt`) were moved out of the
  tree before the tally.
- Rows took about 5 minutes each (`back_after` 296-307 s). The Pi was back
  in Linux at the end, and `tryboot.txt` was put back by `board.sh`.
