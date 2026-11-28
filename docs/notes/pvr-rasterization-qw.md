<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 9 dEQP-GLES3 `rasterization` QualityWarnings (QW-RAST)

Plan #400, item `cause-rastqw`. This is an investigation only. No file under
`src/` or `driver/` changed and nothing ran on a board. Sources read: dEQP at
VK-GL-CTS `93bca01` (the revision RESULTS.md uses), Mesa 26.2.1's PowerVR
(Rogue) Vulkan driver, and yakogl at the integration tip `ad2b6cc9`. The
cases come from `docs/es3/am62p-gles3-notpass.tsv`, tree row 14, build
`b25997e4`, run 2026-11-27.

**Verdict: all 9 are acceptable hardware behaviour. None is a defect, and
the driver sets no wrong state.** The Rogue ISP draws a GL line as a
parallelogram: it pushes the line out along the minor axis and
interpolates the varyings over that quad. dEQP asks for a different
geometry in two places:

- For a 1-pixel line, dEQP wants the spec's perpendicular projection for the
  interpolation weight (5 cases).
- For a wide line in a multisampled target, dEQP wants a rectangle that is
  perpendicular to the line (4 cases).

dEQP knows that hardware may do neither and does not fail these cases. It
reports QualityWarning for the first and CompatibilityWarning for the
second. The TSV writes the CompatibilityWarnings as QualityWarning
(section 1). Mesa's own driver for this GPU reports the same: no strict
lines, no rectangular lines, parallelogram wide lines (section 3). The rule
row can go from `open` to `known`.

## 1. The cases

| # | case | TSV message | dEQP result word | group |
| ---: | --- | --- | --- | --- |
| 1 | `rasterization.interpolation.basic.lines` | Interpolation was calculated using projected coordinateds | QualityWarning | A |
| 2 | `rasterization.interpolation.basic.line_strip` | same | QualityWarning | A |
| 3 | `rasterization.interpolation.projected.lines` | same | QualityWarning | A |
| 4 | `rasterization.fbo.rbo_singlesample.interpolation.lines` | same | QualityWarning | A |
| 5 | `rasterization.fbo.texture_2d.interpolation.lines` | same | QualityWarning | A |
| 6 | `rasterization.fbo.rbo_multisample_4.interpolation.lines_wide` | Interpolation of multisampled wide lines failed | CompatibilityWarning | B |
| 7 | `rasterization.fbo.rbo_multisample_max.interpolation.lines_wide` | same | CompatibilityWarning | B |
| 8 | `rasterization.fbo.rbo_multisample_4.primitives.lines_wide` | Rasterization of multisampled wide lines failed | CompatibilityWarning | B |
| 9 | `rasterization.fbo.rbo_multisample_max.primitives.lines_wide` | same | CompatibilityWarning | B |

("coordinateds" is dEQP's own spelling.)

- **The result words.** dEQP issues the group-B messages only as
  `QP_TEST_RESULT_COMPATIBILITY_WARNING` (`es3fRasterizationTests.cpp:650`,
  `:2154`). The TSV shows them as QualityWarning because
  `gles3-notpass.py` applies `am62p-gles3-merge.py`'s `norm()`, which counts
  CompatibilityWarning as QualityWarning (`tools/cts/gles3-notpass.py`
  header). Both words count as a pass in a conformance submission.
- **Change since ad2b5ed.** At `ad2b5ed` the 12 `rbo_multisample_4` cases
  were NotSupported ("Test requires 4x msaa rbo"), and cases 7 and 9 were
  Fail ("Found invalid pixel values" / "Incorrect rasterization"). Then
  `GL_MAX_SAMPLES` was 1, so dEQP's multisample relaxation (it needs
  `m_numSamples > 1`) could not apply. `d5231f8a` (in `b25997e4`) gave the
  PowerVR table `max_samples` 4. Now all 12 `_4` cases run: 10 pass and 2
  give the group-B warning. Cases 7 and 9 also moved from Fail to the
  warning. This settles the question that
  `docs/notes/es3-msaa-renderbuffers.md` (section (a)) left open about their
  Fail at 1 sample.
- **The same picture on other GPUs.** dEQP draws the same lines everywhere,
  because each iteration seeds `de::Random` with
  `123 + iteration * 1000 + primitive` (`es3fRasterizationTests.cpp:2194`).
  Any rasterizer that interpolates along the major axis therefore warns on
  the same cases. The i.MX8MP GC7000UL's row 24
  (`tools/cts/log/2026-11-imx8mp-gles3-9bc82915-rows17-34/results.tsv`) has
  exactly cases 1-5, 7 and 9 among its non-Pass cases. The Pi 4's V3D 4.2
  gives QualityWarning for cases 1 and 3-5
  (`tools/cts/log/2026-11-pi4-gles3-fad0558a-rerun/R1.tally.txt`;
  `docs/notes/v3d-notpass-rest.md` G9).
- **What passes.** The same family passes on the AM62P:
  `interpolation.basic.line_loop`, all `*_wide` single-sample interpolation
  cases, `interpolation.projected.line_strip` / `line_loop`, all `flatshading`
  cases and `primitives.lines` / `lines_wide` (single-sample), and in 4x
  `primitives.lines` and `interpolation.lines` (narrow). Section 2 explains
  why.

## 2. What dEQP checks

### Group A: narrow-line interpolation (`LineInterpolationTest`)

Each case runs 3 iterations (`es3fRasterizationTests.cpp:2036`). Each
iteration draws 6 random vertices, `x, y` in [-0.9, 0.9] of a 256x256 target
(`:113`, `:2194-2218`). In `projected` the coordinates are also multiplied by
a random `w` in [0.2, 4.0]. The test sets `allowNonProjectedInterpolation =
true` and calls `verifyLineGroupInterpolation` (`:2129-2131`). With 0 samples
(window, single-sample renderbuffer, 2D texture) the verifier works in this
order (`tcuRasterizationVerifier.cpp:3259-3298`):

1. If the line is 1 pixel wide, it tries the spec's weight
   (`verifySinglesampleNarrowLineGroupInterpolation`, `:2180`).
   `calcLineInterpolationWeights` (`:441-501`) computes
   `t = ((p - a) . (b - a)) / |b - a|^2`. This is the fragment centre's
   **perpendicular** projection onto the segment, then perspective-divided
   by `wa`, `wb`. The tolerance is 1-3 ULP on each step, plus the 2x2-subpixel
   test square around the pixel centre (`:573-607`).
2. If that fails, it tries `verifyLineGroupInterpolationWithProjectedWeights`
   (`:2193`, `calcLineInterpolationWeightsAxisProjected` `:503-571`). For an
   x-major line this computes `t = (x - xa) / (xb - xa)`: the projection
   **along the minor axis**. If this matches, the result is
   `LINEINTERPOLATION_PROJECTED`.
3. `PROJECTED` adds QualityWarning "Interpolation was calculated using
   projected coordinateds" (`es3fRasterizationTests.cpp:2139-2148`).
   `INCORRECT` at width 1 would be a Fail ("Found invalid pixel values",
   `:2160`). The AM62P gives no such Fail.

The warning is therefore a measurement: **every pixel of the case matched
the axis-projected weight, and at least one pixel did not match the
perpendicular one.** For a fragment at a minor-axis distance `d` from the
line (`|d| <= 0.5` for a 1-pixel line), the two weights differ by
`d * sin(theta) / L`. Here `theta` is the line's angle to its major axis
(at most 45 degrees) and `L` is its length in pixels. So a long line or a
nearly axis-aligned one stays inside the tolerance, and a short, slanted one
falls outside. That is why `basic.line_loop` and `projected.line_strip` pass
while `basic.line_strip` warns. The random geometry decides which case
warns, and the hardware's rule is the same in all of them.

For wide lines (5, 10 and `ALIASED_LINE_WIDTH_RANGE` max = 16 px,
`:2054-2072`; yakogl answers [1, 16], `src/gl/state.rs:198-203`) dEQP's own
reference moves the line along the minor direction
(`verifySinglesampleWideLineGroupInterpolation`, `:2241` on, "wide line
interpolations are calculated for a line moved in minor direction", and
`getLineCoordAtAxisCoord` `:2218-2230`). The hardware's axis-projected
result is therefore *strictly correct* for single-sample wide lines, and all
those cases pass.

### Group B: multisampled wide lines

With `numSamples != 0`, both `verifyLineGroupRasterization` (`:3161-3170`,
`strictMode = true`) and `verifyMultisampleLineGroupInterpolation` (`:1376`,
`strictMode = true` by default) model each line as two triangles. These form
the rectangle `a +- n * w/2`, `b +- n * w/2`, where `n` is the line's
**perpendicular** normal: the minor axis is taken only in non-strict mode
(`:1050`, `:1272`). That is the multisample line of ES 3.0
section 3.5 (a rectangle of width `w` centred on the segment). At width 1
the parallelogram and the rectangle cover nearly the same samples, so 4x
`primitives.lines` and `interpolation.lines` pass. At 5, 10 and 16 px the
two shapes differ by up to `w/2 * tan(theta)` pixels at each end, so the
strict check fails. dEQP then relaxes the result:

- `BaseLineCase::iterate` (`es3fRasterizationTests.cpp:631-636`): "multisampled
  wide lines might not be supported". It sets
  `m_multisampleRelaxationRequired`, treats the iteration as passed, and ends
  with CompatibilityWarning "Rasterization of multisampled wide lines failed"
  (`:648-651`).
- `LineInterpolationTest::iterate` (`:2150-2156`): an INCORRECT result at
  `lineWidth != 1` with `m_numSamples > 1` gives CompatibilityWarning
  "Interpolation of multisampled wide lines failed".

`rbo_multisample_max` asks `glGetInternalformativ(GL_SAMPLES)` for its
sample count (`es3fRasterizationTests.cpp:191-201`). The PowerVR answers 4 there, so `_4` and `_max`
draw the same 4x image. That explains why the four group-B cases come in
identical pairs.

## 3. What the PowerVR does, and why that is its only mode

- **yakogl draws GL lines as native Rogue lines.** `GL_LINES` maps to
  `PVR_VDM_TOPOLOGY_LINE_LIST` with `PVR_TA_OBJTYPE_LINE`
  (`src/pvr/backend.rs:9197-9205`; `GL_LINE_STRIP` and `GL_LINE_LOOP` use
  `PVR_VDM_TOPOLOGY_LINE_STRIP` / `LINE_LOOP` with the same object type). The TA expands each line into a quad. The source comments
  call it "the quad it expands them into" (`src/pvr/backend.rs:10219`), and
  the ISP sets up the varyings over that quad. The only line state in
  `STATE_ISPA` is `pointlinewidth` (4.4 fixed point minus 1, max 255 = 16 px)
  and `linefilllastpixel` (Mesa `src/imagination/csbgen/rogue/ppp.xml:153-154`).
  yakogl writes `pointlinewidth` from `glLineWidth`
  (`src/pvr/backend.rs:9979-9992`) and leaves `linefilllastpixel` at 0. Leaving
  it at 0 is right for GL, because the diamond-exit rule does not draw a
  segment's last pixel, and it is why `primitives.lines` passes. It also sets
  `pds_tri_merge_disable` for lines (`:9999`), as Mesa does.
- **Plane-equation interpolation over a minor-axis quad is exactly dEQP's
  "projected" weight.** Take an x-major line `a -> b` expanded to `a +- (0, h)`,
  `b +- (0, h)`. The two vertices at each end share the attribute and `w`. So
  the screen-space planes of `1/w` and `attr/w` through them have zero
  y-gradient, and both depend on `x` only. The perspective-correct value is
  then `((1-s)/wa * A + s/wb * B) / ((1-s)/wa + s/wb)` with
  `s = (x - xa) / (xb - xa)`. That is `calcLineInterpolationWeightsAxisProjected`
  term for term, including the `w` division that `projected.lines` tests.
  The same quad, `w` pixels across the minor axis, is the parallelogram that
  fails dEQP's strict rectangle in group B.
- **Mesa's Rogue driver reports the same hardware rule** (Mesa 26.2.1,
  `src/imagination/vulkan/pvr_physical_device.c`):
  - `wideLines = true` (`:248`) and line widths 1/16 to 16 at 1/16 steps
    (`:759-761`, `src/imagination/common/pvr_limits.h:43-45`).
  - `strictLines = false` (`:763`): lines are not rasterized as the
    perpendicular rectangle in multisample mode.
  - `nonStrictWideLinesUseParallelogram = true` (`:926`): wide lines are
    minor-axis parallelograms.
  - `nonStrictSinglePixelWideLinesUseParallelogram = false` (`:925`):
    1-pixel lines use Bresenham-style coverage, which is the coverage that
    passes `primitives.lines`.
  - `bresenhamLines = true` (`:523`). `rectangularLines` and `smoothLines`
    are absent, so the hardware has no rectangle mode to turn on.
- **The one other line bit, `PPP_CTRL.prim_msaa`** (`ppp.xml:329`). Mesa
  sets it only for `VK_LINE_RASTERIZATION_MODE_BRESENHAM_KHR` line
  topologies (`pvr_arch_cmd_buffer.c:7568-7577`). It selects the Bresenham
  (non-MSAA, pixel-coverage) line in a multisampled target, which is further
  from dEQP's rectangle, not closer. yakogl never sets it, which matches
  Mesa's default (non-strict) line mode. No ISP, PPP or TA state word in
  `src/pvr/**` is set wrongly for these cases.
- **Host reproduction is not possible with the host PowerVR path.** The linux
  OSAL keeps the command stream and every read-back is black
  (`tools/cts/README.md` 2b), and no host model of the ISP rasterizer exists
  (`tests/usc_exec` runs USC code only). The evidence is therefore dEQP's
  source, Mesa's description of the same GPU, the plane-equation argument
  above, and the board's result pattern: every narrow pixel matched the
  axis-projected weight (otherwise the case would be a Fail), and narrow lines
  pass in 4x while wide ones do not.

## 4. Verdict per case

| # | case | cause | verdict | fix |
| ---: | --- | --- | --- | --- |
| 1 | `interpolation.basic.lines` | minor-axis quad interpolation = axis-projected `t` | acceptable hardware behaviour | none |
| 2 | `interpolation.basic.line_strip` | same (this geometry has a short, slanted segment) | acceptable hardware behaviour | none |
| 3 | `interpolation.projected.lines` | same, with perspective division matching dEQP's projected formula | acceptable hardware behaviour | none |
| 4 | `fbo.rbo_singlesample.interpolation.lines` | same as 1 into an RGBA8 renderbuffer | acceptable hardware behaviour | none |
| 5 | `fbo.texture_2d.interpolation.lines` | same as 1 into an RGBA8 texture | acceptable hardware behaviour | none |
| 6 | `fbo.rbo_multisample_4.interpolation.lines_wide` | wide line is a minor-axis parallelogram, not the perpendicular rectangle | acceptable hardware behaviour (CompatibilityWarning, dEQP's sanctioned relaxation) | none |
| 7 | `fbo.rbo_multisample_max.interpolation.lines_wide` | as 6 (max = 4 samples) | as 6 | none |
| 8 | `fbo.rbo_multisample_4.primitives.lines_wide` | as 6, coverage instead of interpolation | as 6 | none |
| 9 | `fbo.rbo_multisample_max.primitives.lines_wide` | as 8 | as 6 | none |

**The only way to remove the warnings, which is not recommended.** You
would have to stop using the hardware line. The driver would expand every
line into two triangles along the perpendicular normal before the TA,
either in the GL core's draw path (`src/gl/draw.rs`) or as a PowerVR
back-end vertex rewrite in `src/pvr/backend.rs`. Triangle interpolation over
a perpendicular rectangle gives the spec's `t`, and in 4x its coverage is
dEQP's rectangle. But for 1-pixel single-sample lines the triangle pair
would replace the hardware's diamond-exit coverage with rectangle coverage.
That puts `primitives.lines`, `line_strip`, `line_loop` and the
`flatshading` line cases at risk, which pass today. It also costs vertex
work on every line draw, and its gain is only a change from a warning to a
Pass that conformance already accepts. This note proposes no item for it.

## 5. Rule row

For `tools/cts/am62p-notpass-causes.py` (item `causes-index`):

```python
    ("QW-RAST", {QW}, r"^rasterization\.", None, "known",
     "pvr-rasterization-qw.md", "minor-axis line quads: projected weights / multisampled wide parallelograms (hardware)"),
```

No issue is needed: there is no defect.
