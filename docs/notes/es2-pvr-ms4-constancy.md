<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P ms4: `multisample.constancy_alpha_to_coverage` and `constancy_both_inverted`

**Status (2026-10-12): superseded; the root cause was found on main.**
Re-landed from integration (`cdb28c04c`, ccfarm
`01a12a25-0edc-7124-8113-2885a46b96a4`) as the host record that ruled the
program out. The cause is the frame: each of the case's 1,024 draws
uploaded 768 bytes of PDS blocks, the 768 KiB PDS arena held 1,014 draws,
and the 1,015th opened a second kick whose load quad wrote one resolved
colour to all four samples. Fixed by `a4f64950` (a frame uploads a PDS
program it already holds once; gl_test 198 `a2cconst`). Neither hypothesis
below is it. Of this item's tests, the three program-level ones stay in
`tests/es2_pvr_ms4_constancy.rs`; the hypothesis test, the board
placeholder and the hash printer were not re-landed.

Item `pub-redaction-audit` of plan 20 (host diagnosis, no `src/` change).
Host test: `tests/es2_pvr_ms4_constancy.rs`.

**Result in one paragraph.** The root cause is **not found**. On the host,
the program each draw runs is right: for every sample mask, every alpha of
the case and both colour masks, the samples that stay are exactly the ones
ES 2.0 4.1.3 names, and they do not depend on the quad's colour. So the
break is not in the compiled shader. Two hypotheses remain, ranked below.
The first one fits *which* cases fail, but a counter-example (a passing case
with the same kind of draw) keeps it from being proven. The second one is a
real code path that can break constancy and needs a log line from the board
to confirm. The next step is a short board check (section 6) that decides
between them and also tells whether the failure repeats.

## 1. What the two cases check

Source: VK-GL-CTS `modules/gles2/functional/es2fMultisampleTests.cpp`,
`MaskConstancyCase` (line 1257, `iterate` at line 1304). All five
`constancy_*` cases are this one class with two flags.

| case | `SAMPLE_ALPHA_TO_COVERAGE` + colour mask (T,T,T,F) | `SAMPLE_COVERAGE` | `glSampleCoverage(value, invert)` |
|---|---|---|---|
| `constancy_alpha_to_coverage` | on | off | not called |
| `constancy_sample_coverage` | off | on | `(v, FALSE)` |
| `constancy_sample_coverage_inverted` | off | on | `(1-v, TRUE)` |
| `constancy_both` | on | on | `(v, FALSE)` |
| `constancy_both_inverted` | on | on | `(1-v, TRUE)` |

Setup: a 256x256 viewport, clear to black, no depth test, no blend. The
fragment shader is `gl_FragColor = v_color` (line 326). The case draws
`4*samples = 16` by `16` pairs of quads. For each cell (row, col) it draws
a **green** quad and then a **red** quad on exactly the same geometry:

* vertex alpha is `col/15` on the left pair of vertices and `row/15` on the
  right pair (1.0 when alpha to coverage is off);
* `v = (row*16+col)/255` when sample coverage is on.

Both quads of a pair therefore have the same alpha at every pixel and the
same glSampleCoverage. If the coverage mask depends only on those values
(the *constancy* property) the red quad covers every sample the green one
did. The case reads the image back and fails (`Fail (Failed)`) if any pixel
has **green > 0**.

So a failure means: some sample was written by the green draw and *not* by
the red draw with the same alpha, mask and geometry.

### What the two failing cases share, and the passing ones lack

Not "alpha to coverage": `constancy_both` uses it and passes. Not the
colour mask: all three alpha-to-coverage cases set it. What is different
is the **sample coverage mask that alpha to coverage meets**:

* `constancy_alpha_to_coverage`: mask `0xf` (coverage off), prefix mask
  from alpha.
* `constancy_both`: the glSampleCoverage mask is a *low-bit prefix*
  (`0001, 0011, 0111, 1111`), and so is the alpha mask. A prefix and a
  prefix always intersect in a prefix. For the cells with `idx < 64`
  (the ones with small alpha) the mask is empty, and the GL core drops
  those draws (`covers_no_sample`, `src/gl/draw.rs:1122`).
* `constancy_both_inverted`: the glSampleCoverage mask is a *high-bit
  suffix* (`1000, 1100, 1110, 1111`) and alpha gives a low-bit prefix. They
  meet only at large alpha. Most draws (95 of 256 in the host model) end
  with **no sample at all** kept by alpha to coverage, and the core does
  not drop them (its test looks at the glSampleCoverage mask only).
* `constancy_alpha_to_coverage`: the cells (0,0), (0,1), (1,0), (1,1) have
  alpha below 1/8 over the whole quad: alpha to coverage covers no sample.
  In `constancy_both` the same four cells have an empty glSampleCoverage
  mask and are dropped by the core.

Pinned by `only_the_two_failing_cases_draw_quads_that_kill_every_sample`:
the count of draws that kill every sample although the core did not drop
them is 4 (`constancy_alpha_to_coverage`), 95 (`constancy_both_inverted`)
and 0 for the other three. That matches the 2 fail + 3 pass split. The
model of alpha to coverage there is the one `usc_exec::cov_model_of` uses
(`round(alpha*4)` low samples). The real `pck.cov` may differ, so the
numbers are a model; the pattern (prefix against suffix) does not depend on
the exact rounding.

**Counter-evidence.** `proportionality_alpha_to_coverage` iteration 0 draws
one quad with alpha 0.0: also a draw that kills every sample, not dropped.
It passes at the tip (and in the same r1c record). So a kill-every-sample
draw alone does not fail a case. The difference may be that it is the first
draw after the clear there, and that constancy has hundreds of such draws
in one frame. This is why hypothesis A below is not a proven cause.

## 2. Commits since `d60b9d76`

`d60b9d76` (RESULTS 1z2, 2026-10-02) is **not in this repository**: history
starts at the import `afb1633` (2026-10-04), see `docs/es2-cts-failures.md`
section D. So the host cannot build the `d60b9d76` tree, and the 2 days
between `d60b9d76` and the import are one lump (`afb1633`). What can be
listed is the 42 non-merge commits in `afb1633..89d518e` that touch
`src/pvr/**`, `src/shaderc/online/powervr/**` or the draw path
`src/gl/draw.rs` (`git log --no-merges afb1633..89d518e -- src/pvr
src/shaderc/online/powervr src/gl/draw.rs`). Every one of the 42 is an
ancestor of the failing tree `f013dbbb` (`git merge-base --is-ancestor`),
so membership in that tree does not separate them: only the bisect of
section 6 can.

Did I try to build an older tree? Yes, and it was refused: `git worktree`,
`git archive` and `git checkout <old> -- src` need approval in this worker.
The comparison "program bytes at `afb1633` and at the tip" is therefore a
board-session step (section 6, step 4) with a ready tool: the ignored test
`program_hashes` prints the size and a hash of the four programs; copy
`tests/es2_pvr_ms4_constancy.rs` and `tests/usc_exec/` into the old tree.
Tip values for comparison:

```
PROGRAM kill replace:   142 dwords, temps 33, fbfetch 0, hash 56d8cecf2a3496e5
PROGRAM kill alpha_off: 146 dwords, temps 33, fbfetch 1, hash 677669c84d20b341
```

## 3. Ranked suspects

Only commits that can change what the two cases draw, or how the draws
are built, are ranked. Every other commit of the 42 is in the "excluded" rows below
with a reason (all 42 are accounted for).

| rank | suspect | evidence |
|---|---|---|
| A | **Not a commit: draws that kill every sample** (the residual risk written in RESULTS 1z, cause 3). | The failing set is exactly the cases that contain such draws (section 1, pinned on the host). RESULTS 1z: "a draw with an empty coverage mask hangs the next frame (HWR, dm=3), hardware cause unknown"; the core only drops the glSampleCoverage case. "Other draws that kill every sample (alpha to coverage with alpha 0 everywhere) may hang the same way; they are not caught." Against: it passed at `d60b9d76` (1z2) with the same draws, and `proportionality_alpha_to_coverage` iteration 0 passes at the tip. So if A is the cause, something made the board less tolerant since: a timing or memory-layout change of the 4x frame (see the commits at ranks 3 and 4). |
| B | `632687b` (2026-10-05) "a variant refused for room is asked for again; slots are least recently used". | Before it, a sample-kill variant refused for want of keep room was remembered as missing for good: every later draw took the plain shader (no kill), so green and red were both drawn in full and the case passed. After it, the slot is freed (`src/gl/draw.rs:1548` onward) and **the next draw asks again**. `draw.rs:2932` turns a refused variant into `None`, and the draw runs the program's own shader, which ignores the sample state. If the green draw is refused for room and the red draw gets the variant (something was freed in between), the red quad kills samples the green one drew: green stays. All five cases need the kill variant, and the three with alpha to coverage also need a blend variant built on it (colour mask (T,T,T,F)), the longest build chain. B does not explain by itself why `constancy_both` passes; it would have to be luck of the room left at that point of the run. The log line at `draw.rs:1555` ("no sampler-state variant ...") and the "no room for the blend key" line say if it happened. The host cannot tell: it needs the keep region of the board run. |
| C | `05f8f74` (10/6) "the back ends' own programs come from our compiler" and `17605c3` (10/6) "the precompiled tier in front of the compiler". | They change the internal clear and frame-load programs the 4x frame uses (the first draw of each case carries a fused clear with the clear's own colour mask, `src/pvr/backend.rs:1434`). Not shown to matter: the clear is before the first quad, and green leaking needs a difference between two draws, not in the clear. Keep as the bisect point if A and B are excluded. |
| D | `2b219c5`, `78b5119` (10/5) depth/stencil buffers: 32 slots, least recently used given up; `b67c89b`, `92939b0` (10/5) PVR kick options and refusals by name; `a138f73`, `b0a9c0c`, `068cb62` (10/5) logging and per-step timing in the draw path. | They change timing and memory of a 4x frame. Candidate only for hypothesis A (a hang or reset that depends on timing). No code path from them to the kill decision. |
| excluded | `7010bd2` blend factors computed once, `ecd341d` copies forwarded, `39bd698` varying product fused with its sum, `f4c41d0` clamp to one saturating add, `afa12e9`/`97ba03d`/`5f4d367`/`81c53c6` predicated `if`, `18972dc`/`6f96104` mul-add fusion, `c6aa6ba` attribute registers, `cc78859` NPOT mips | They change the compiled code in general. The case's program (a varying copied to the output, plus the kill epilogue, plus the alpha-off colour mask) was run in the byte model through the product entry points at the tip: the kept samples are right and colour-independent (`the_kept_samples_*`, `a_kept_sample_with_alpha_masked_*`). `discard_epilogue` (`src/shaderc/online/powervr/select.rs:2086`) has not changed since the import (`git log -L`). `fusion_pairs` (the clamp and product fusions) works on IR ops before the epilogue is built. `cc78859` is NPOT texture layout; the case uses no texture. |
| excluded | `b9b2ac0` (10/8), merge resolution `bb4a982` integer colour render targets | The one blend-path change: `d.blend = c.blend && !framebuffer::fb_target_is_integer(c)` and the same guard in `current_blend_key` (`src/gl/draw.rs`), and the clear's alpha quantise is skipped for an integer target. `fb_target_is_integer` is `gles_surf_is_integer(fb_target_format(c))`; the default framebuffer's draw buffer is RGBA8, fixed point, so both guards are `true`/unchanged: the blend key and `d.blend` equal the earlier values. It also adds `refuse_undrawable_targets`, which passes for RGBA8 (`rt_format_drawable`). Not a suspect; a board log with "draw refused" would reopen it. |
| excluded | `16ac864`, `116aca6`, `2a75787`, merge resolution `6009e62` transform feedback and rasterizer discard | Early returns in `draw_arrays`/`draw_elements`: `tf::pre_draw` returns `true` when no capture is active, and `c.rasterizer_discard` is false by default. The cases never set either. |
| excluded | `141dcfa`, `c86adc9`, `3985bee` clippy `needless_range_loop` clean-ups | "No behaviour change" by message; `141dcfa` rewrites the `ds_get` loop in `src/pvr/backend.rs` as `iter_mut().enumerate().take(PVR_BACKEND_MAX_DS)`, the same bounds and the same body as the depth/stencil slot code of rank D. Read, equivalent. |
| excluded | `888af5d` ES 3.0 sized internal formats, `2673197` OES_texture_npot claim, `ec5ba4f` V3D non-seamless cube | Texture formats and upload paths, or a V3D-only path. The cases bind no texture. |
| excluded | `72d1686`, `260711b` Vivante knobs, `dea95de` Mesa reference out of product, `78c96ea` remote shader back end removed, `080b8aa` `gpu-powervr` Cargo feature, `0fe778f` table blobs record | Other back ends, build or table-provenance changes. No PowerVR draw or kill path. `080b8aa` is the feature the ms4 image builds with; it selects the same back end. |
| excluded | `01260ee`, `58402fd` gl_test 193/194 | Measurement tests (gl_test cases), not driver code. |
| excluded | `4e9397c` ES2 context refuses a DEPTH_COMPONENT24 renderbuffer | The ms4 config's depth is the window surface's, not a renderbuffer. |

Checked in the current code and correct (not suspects):
`set_sample_state` (`src/gl/draw.rs:1755`), `sample_coverage_mask`
(`:1129`, Mesa's `(unsigned)(value*samples)` rule, inverted), `needs_sample_kill`
(`:1484`), `fs_meta_word` (`src/pvr/backend.rs:523`, mask in bits 9..24, alpha
to coverage bit 25), `fs_sample_rate` (`:502`, FULL rate when the program has
the kill epilogue or reads the pixel; used at `:4905`).

## 4. What the host test shows

`tests/es2_pvr_ms4_constancy.rs`, run with

```sh
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online \
    --test es2_pvr_ms4_constancy
```

* `the_kept_samples_do_not_depend_on_the_colour`: the case's shader for the
  4x sample state, with and without colour mask alpha off, by two routes
  (`compile_fragment_for` with the sample-kill key; and the product's
  `compile`, `sampler_variant(LOD_KEY_SAMPLE_KILL)`, `blend_variant`), run
  once per sample for all 16 masks, a2c on and off, the 16 alphas of the
  column series, green and red: the kept set is identical for green and red.
* `the_kept_samples_are_the_coverage_mask_and_alpha_to_coverage`: and it is
  `mask & alpha_mask` (ES 2.0 4.1.3).
* `a_kept_sample_with_alpha_masked_writes_rgb_and_keeps_the_pixel_alpha`:
  red over green leaves no green in a kept sample.
* `only_the_two_failing_cases_draw_quads_that_kill_every_sample`: section 1.
* `constancy_alpha_to_coverage_and_both_inverted_hold_on_the_board`
  (`#[ignore]`): the placeholder for the wrong behaviour, which the host
  cannot reproduce. The ignore reason names this document.
* `program_hashes` (`#[ignore]`): a measurement, section 2.

All pass at the tip: 4 passed, 2 ignored. The tests that pass do not
reproduce the board failure; they exclude the shader as the place.

What was **not** compared: the state words of the draws at `d60b9d76`. That
tree is not in the repository and an older tree could not be checked out
here (section 2). The GL-core fields the draw carries for the two cases
(`sample_mask`, `alpha_to_coverage`, colour mask) have unit tests already
(`src/gl/draw.rs` around `:6060`, `:6758`; `src/pvr/backend_tests.rs:1589`
to `:1630`) and were read, not changed.

## 5. Root cause and fix

Root cause: **not found.** No exact fix is claimed. The two candidate
changes, to be made only after the board check says which hypothesis holds,
are:

* If B (a refused variant gives an inconsistent plain draw): refuse the
  draw instead of drawing with the program's own shader when the wanted
  lod key carries `LOD_KEY_SAMPLE_KILL` — `src/gl/draw.rs:2932` (the `None`
  of `lod_variant_for`), the sample-kill bit is `shaderc::LOD_KEY_SAMPLE_KILL`.
  A plain draw on a 4x target with a coverage mask or alpha to coverage on is
  wrong by definition (it ignores both), so a refusal is the honest answer;
  the same rule already holds for blend variants ("never drawn with the plain
  shader", `fs_variant_for`, `:1237`). Also size the keep region for the
  kill and blend variants of the ms4 image.
* If A (a draw that kills every sample): `covers_no_sample` (`:1122`)
  cannot be widened to alpha to coverage, because alpha is per pixel and the
  core does not know it. The fix would be in the PowerVR job/PDS setup for
  such a draw (`src/pvr/backend.rs`, the fragment kick at `:4895`), and
  needs the console capture of step 3 below first.

## 6. Board check

This is a board session on the AM62P with the 4x image. Use the build
helpers as the other sessions do; no address, serial path or credential
belongs in the record.

1. Build the ms4 image at the tip with `tools/cts/am62p-deqp-images.sh
   tools/cts/am62p-ms4-tree.txt <img-dir> ms4 > <img-dir>/build.out`, run it
   with `TREE_IMAGE_DIR=<img-dir> TREE_TAG=ms4 tools/board.sh am62p tree
   tools/cts/am62p-ms4-tree.txt <outdir>` (`DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4`
   is set by the image script). Run it **three times**. Pass count per run
   tells whether the 2 failures repeat (the r1c record has one run only).
2. If a run passes 16/16: the failure is run-to-run noise or timing; keep
   all three console logs and stop the bisect. Hypothesis A with a timing
   trigger is then the working explanation; open a follow-up for the
   residual risk of RESULTS 1z cause 3.
3. If the 2 fail every time: run a one-row table with the filter
   `dEQP-GLES2.functional.multisample.constancy_alpha_to_coverage` and
   capture the whole console. Look for these lines before the case verdict:
   `yakogl: no sampler-state variant ...`, `yakogl: no room for the blend
   key ...` (hypothesis B), a hardware reset (`HWR`) or a kick re-post
   (hypothesis A). Their presence decides the hypothesis at once.
4. Build the same image at `afb1633` (the oldest tree in the repository),
   at `632687b^` and `632687b`, then at `05f8f74^` and `05f8f74`, and run
   step 1 on each. The first image that fails names the commit. Before
   building, run `program_hashes` (section 2) in each tree on the host: if
   the hashes equal the tip's, the program did not change and the cause is
   in the draw path; a different hash names a compiler commit directly.

Expected result if the fix lands: ms4 image 16 pass / 0 fail on three
consecutive runs; then the AM62P final tree (`am62p-es2-final-b`) takes its
ms4 row from it.

## 7. Notes for the next item

* The fix is a separate item. It touches `src/gl/draw.rs` and/or
  `src/pvr/**`, and must wait for the board check of section 6.
* This item changed no `src/**`. The files are this document and
  `tests/es2_pvr_ms4_constancy.rs`.
