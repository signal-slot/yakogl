<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# HOST-ONLY GC7000UL dEQP-GLES3 at 05769bf2

**HOST-ONLY.** Every number here comes from runs on a host with the Linux
OSAL, which keeps the command stream instead of executing it: every
`glReadPixels` reads black, so every render verdict (Pass, or Fail by image
comparison or verification) is meaningless and must not be quoted as a board
result. What is real is each case's compile and link outcome, its GL errors
and its NotSupported reason, which is what the counting not-pass list holds
(render Fails are left out of it).

This is the HOST-ONLY GC7000UL (vivante) column of plan #372, the host side
of the first i.MX8MP dEQP-GLES3 board runs (issue #24). It is a newer
column than the vivante rows of `docs/es3/host-gles3-tip.tsv` and
`docs/es3/host-gles3-notpass.tsv` (26d1c4a2), which the board compare at
9bc82915 (`docs/es3/imx8mp-gles3-compare-9bc82915.tsv`) used.

- Commit (library and harness): `05769bf2c24a00a8c69c608c2310c0edab3f807c`
- GPU vivante, module gles3 (the library reports OpenGL ES 3.0)
- VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` (`93bca018`), named in
  the commit messages of all six hgc items
- One `tools/cts/host-deqp-run.sh` run per filter (`GPU=vivante
  MODULE=gles3`), each run directory written to a part file with
  `tools/cts/host-gles3-tally.py --part`; the commit messages named in the
  table below hold the runs' own notes

## Coverage

The commit column names each hgc item's commit on the integration branch.

74 part files, all in this directory as `hgc<N>/<group>.tsv` and all in the
`--part` format (`tools/cts/host-gles3-tally.py` `load_part` reads every one).
Each part's `cases=` equals the number of `tools/cts/gles3-mustpass.txt`
cases under its filter, no case is under two filters, and together they are
the whole mustpass: 42,810 cases. Crash is 0 in every part: every case
began and ended with a result.

Every part here is a `<group>.tsv`. No part is missing or short.

| dir | parts | filter | commit |
|---|---|---|---|
| hgc1 | 1 (6,402 cases) | `dEQP-GLES3.functional.shaders.operator.` | c3cb5e6c |
| hgc2 | 3 (6,008 cases) | `dEQP-GLES3.functional.shaders.builtin_functions.`, `dEQP-GLES3.functional.shaders.matrix.`, `dEQP-GLES3.functional.shaders.random.` | cb36d3c7 |
| hgc3 | 33 (8,206 cases) | `dEQP-GLES3.functional.shaders.<group>.` per file | b7b8cab5 |
| hgc4 | 1 (5,105 cases) | `dEQP-GLES3.functional.texture.` | 17011ab1 |
| hgc5 | 3 (6,138 cases) | `dEQP-GLES3.functional.fbo.`, `dEQP-GLES3.functional.fragment_ops.`, `dEQP-GLES3.functional.fragment_out.` | 7b804f8b |
| hgc6 | 33 (10,951 cases) | `dEQP-GLES3.functional.<group>.` per file (`info.tsv`: `dEQP-GLES3.info.`) | 7dcce6d2 |

Per part, the tip counts and the not-pass rows by status and kind:

| part | cases | pass | fail | notsupported | other | Fail compile | Fail gl-error | NotSupported refusal | NotSupported gl-error |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| hgc1/operator.tsv | 6402 | 6 | 6396 | 0 | 0 |  |  |  |  |
| hgc2/builtin_functions.tsv | 1539 | 92 | 1010 | 0 | 437 |  | 824 |  |  |
| hgc2/matrix.tsv | 2646 | 0 | 2646 | 0 | 0 |  |  |  |  |
| hgc2/random.tsv | 1823 | 63 | 1760 | 0 | 0 | 26 |  |  |  |
| hgc3/arrays.tsv | 308 | 18 | 290 | 0 | 0 |  |  |  |  |
| hgc3/builtin_variable.tsv | 29 | 0 | 29 | 0 | 0 |  |  |  |  |
| hgc3/conditionals.tsv | 41 | 20 | 21 | 0 | 0 |  |  |  |  |
| hgc3/constant_expressions.tsv | 340 | 0 | 340 | 0 | 0 |  |  |  |  |
| hgc3/constants.tsv | 143 | 43 | 100 | 0 | 0 |  |  |  |  |
| hgc3/conversions.tsv | 1156 | 96 | 1060 | 0 | 0 |  |  |  |  |
| hgc3/declarations.tsv | 50 | 50 | 0 | 0 | 0 |  |  |  |  |
| hgc3/derivate.tsv | 282 | 54 | 228 | 0 | 0 |  | 88 |  |  |
| hgc3/discard.tsv | 25 | 0 | 25 | 0 | 0 |  |  |  |  |
| hgc3/fragdata.tsv | 7 | 4 | 3 | 0 | 0 |  |  |  |  |
| hgc3/fragdepth.tsv | 17 | 0 | 17 | 0 | 0 |  |  |  |  |
| hgc3/function.tsv | 4 | 0 | 4 | 0 | 0 |  |  |  |  |
| hgc3/functions.tsv | 305 | 108 | 197 | 0 | 0 |  |  |  |  |
| hgc3/indexing.tsv | 756 | 0 | 756 | 0 | 0 |  |  |  |  |
| hgc3/invalid_implicit_conversions.tsv | 768 | 768 | 0 | 0 | 0 |  |  |  |  |
| hgc3/invariance.tsv | 36 | 36 | 0 | 0 | 0 |  |  |  |  |
| hgc3/keywords.tsv | 282 | 274 | 8 | 0 | 0 |  |  |  |  |
| hgc3/large_constant_arrays.tsv | 16 | 0 | 16 | 0 | 0 |  |  |  |  |
| hgc3/linkage.tsv | 114 | 29 | 85 | 0 | 0 |  |  |  |  |
| hgc3/loops.tsv | 579 | 0 | 579 | 0 | 0 |  |  |  |  |
| hgc3/metamorphic.tsv | 6 | 0 | 6 | 0 | 0 |  |  |  |  |
| hgc3/negative.tsv | 12 | 12 | 0 | 0 | 0 |  |  |  |  |
| hgc3/precision.tsv | 64 | 16 | 48 | 0 | 0 |  |  |  |  |
| hgc3/preprocessor.tsv | 546 | 270 | 276 | 0 | 0 |  |  |  |  |
| hgc3/qualification_order.tsv | 79 | 64 | 15 | 0 | 0 |  |  |  |  |
| hgc3/return.tsv | 38 | 0 | 38 | 0 | 0 |  |  |  |  |
| hgc3/scoping.tsv | 71 | 40 | 31 | 0 | 0 |  |  |  |  |
| hgc3/struct.tsv | 89 | 0 | 89 | 0 | 0 |  |  |  |  |
| hgc3/switch.tsv | 173 | 52 | 121 | 0 | 0 |  |  |  |  |
| hgc3/swizzle_math_operations.tsv | 544 | 0 | 544 | 0 | 0 |  |  |  |  |
| hgc3/swizzles.tsv | 648 | 0 | 648 | 0 | 0 |  |  |  |  |
| hgc3/texture_functions.tsv | 560 | 118 | 442 | 0 | 0 | 304 |  |  |  |
| hgc3/uniform_block.tsv | 118 | 118 | 0 | 0 | 0 |  |  |  |  |
| hgc4/texture.tsv | 5105 | 19 | 3748 | 1338 | 0 | 432 | 65 | 1338 |  |
| hgc5/fbo.tsv | 2028 | 551 | 1222 | 255 | 0 |  | 128 | 248 | 7 |
| hgc5/fragment_ops.tsv | 3170 | 91 | 3079 | 0 | 0 |  |  |  |  |
| hgc5/fragment_out.tsv | 940 | 0 | 825 | 115 | 0 |  | 150 | 115 |  |
| hgc6/attribute_location.tsv | 274 | 274 | 0 | 0 | 0 |  |  |  |  |
| hgc6/buffer.tsv | 695 | 168 | 527 | 0 | 0 |  |  |  |  |
| hgc6/clipping.tsv | 602 | 0 | 602 | 0 | 0 |  |  |  |  |
| hgc6/color_clear.tsv | 19 | 0 | 19 | 0 | 0 |  |  |  |  |
| hgc6/default_vertex_array_object.tsv | 1 | 1 | 0 | 0 | 0 |  |  |  |  |
| hgc6/default_vertex_attrib.tsv | 80 | 0 | 80 | 0 | 0 |  |  |  |  |
| hgc6/depth_stencil_clear.tsv | 10 | 0 | 10 | 0 | 0 |  |  |  |  |
| hgc6/dither.tsv | 20 | 0 | 20 | 0 | 0 |  |  |  |  |
| hgc6/draw.tsv | 322 | 2 | 320 | 0 | 0 |  |  |  |  |
| hgc6/draw_buffers_indexed.tsv | 76 | 0 | 0 | 76 | 0 |  |  | 76 |  |
| hgc6/fence_sync.tsv | 10 | 10 | 0 | 0 | 0 |  |  |  |  |
| hgc6/implementation_limits.tsv | 49 | 43 | 6 | 0 | 0 |  |  |  |  |
| hgc6/info.tsv | 6 | 6 | 0 | 0 | 0 |  |  |  |  |
| hgc6/instanced.tsv | 45 | 0 | 45 | 0 | 0 |  |  |  |  |
| hgc6/lifetime.tsv | 56 | 56 | 0 | 0 | 0 |  |  |  |  |
| hgc6/multisample.tsv | 64 | 9 | 7 | 48 | 0 |  |  | 48 |  |
| hgc6/negative_api.tsv | 312 | 304 | 4 | 4 | 0 |  | 4 | 4 |  |
| hgc6/occlusion_query.tsv | 62 | 62 | 0 | 0 | 0 |  |  |  |  |
| hgc6/pbo.tsv | 49 | 39 | 4 | 6 | 0 |  | 4 | 6 |  |
| hgc6/polygon_offset.tsv | 20 | 0 | 20 | 0 | 0 |  |  |  |  |
| hgc6/prerequisite.tsv | 3 | 2 | 1 | 0 | 0 |  |  |  |  |
| hgc6/primitive_restart.tsv | 504 | 504 | 0 | 0 | 0 |  |  |  |  |
| hgc6/rasterization.tsv | 100 | 34 | 54 | 12 | 0 |  |  | 12 |  |
| hgc6/rasterizer_discard.tsv | 51 | 0 | 51 | 0 | 0 |  |  |  |  |
| hgc6/read_pixels.tsv | 48 | 0 | 48 | 0 | 0 |  | 24 |  |  |
| hgc6/samplers.tsv | 42 | 42 | 0 | 0 | 0 |  |  |  |  |
| hgc6/shader_api.tsv | 52 | 38 | 0 | 14 | 0 |  |  | 14 |  |
| hgc6/state_query.tsv | 946 | 921 | 16 | 9 | 0 |  | 1 | 9 |  |
| hgc6/transform_feedback.tsv | 882 | 880 | 2 | 0 | 0 | 2 |  |  |  |
| hgc6/ubo.tsv | 2355 | 0 | 2355 | 0 | 0 |  |  |  |  |
| hgc6/uniform_api.tsv | 2181 | 1605 | 576 | 0 | 0 | 1 |  |  |  |
| hgc6/vertex_array_objects.tsv | 10 | 10 | 0 | 0 | 0 |  |  |  |  |
| hgc6/vertex_arrays.tsv | 1005 | 40 | 965 | 0 | 0 |  |  |  |  |

## Tip (HOST-ONLY)

`host-gc-gles3-tip.tsv`, 43 groups: Pass 8,062, Fail 32,434, NotSupported 1,877,
Crash 0, other 437 (8,062 + 32,434 + 1,877 + 0 + 437 = 42,810).
The 437 other are all in `functional.shaders.builtin_functions` (hgc2):
QualityWarning, as the message of 1bacfc9a says (a part file counts them
only as other). Pass and Fail are render verdicts and mean nothing here.

## Counting not-pass list

`host-gc-gles3-notpass.tsv`: 3,930 rows, every one vivante, rc and owner empty
(no triage yet):

| status | kind | rows |
|---|---|---:|
| Fail | compile | 765 |
| Fail | gl-error | 1,288 |
| NotSupported | refusal | 1,870 |
| NotSupported | gl-error | 7 |
| | total | 3,930 |

So 2,053 Fail (765 compile + 1,288 gl-error) and 1,877 NotSupported
(1,870 refusal + 7 gl-error). The NotSupported rows are the tip's
NotSupported count, part by part. The 7 NotSupported gl-error rows are
`functional.fbo.completeness.samples.*` cases whose NotSupported message
(`Unsupported number of samples: 'builder.getError() != GL_INVALID_OPERATION'`)
names a GL error, which is what the tally's kind rule matches first; they are
NotSupported, not Fails.

By group (the first three name parts after `dEQP-GLES3`):

| group | Fail compile | Fail gl-error | NotSupported refusal | NotSupported gl-error |
|---|---:|---:|---:|---:|
| functional.draw_buffers_indexed.overwrite_common |  |  | 18 |  |
| functional.draw_buffers_indexed.overwrite_indexed |  |  | 18 |  |
| functional.draw_buffers_indexed.random |  |  | 40 |  |
| functional.fbo.blit |  | 52 | 146 |  |
| functional.fbo.color |  | 16 | 33 |  |
| functional.fbo.completeness |  | 12 |  | 7 |
| functional.fbo.invalidate |  | 6 | 4 |  |
| functional.fbo.msaa |  | 34 | 32 |  |
| functional.fbo.multiview |  |  | 3 |  |
| functional.fbo.render |  | 8 | 30 |  |
| functional.fragment_out.array |  | 72 | 48 |  |
| functional.fragment_out.basic |  | 24 | 48 |  |
| functional.fragment_out.random |  | 54 | 19 |  |
| functional.multisample.default_framebuffer |  |  | 16 |  |
| functional.multisample.fbo_4_samples |  |  | 16 |  |
| functional.multisample.fbo_8_samples |  |  | 16 |  |
| functional.negative_api.buffer |  | 4 |  |  |
| functional.negative_api.fragment |  |  | 4 |  |
| functional.pbo.renderbuffer |  | 4 | 6 |  |
| functional.rasterization.fbo |  |  | 12 |  |
| functional.read_pixels.alignment |  | 8 |  |  |
| functional.read_pixels.rowlength |  | 8 |  |  |
| functional.read_pixels.skip |  | 8 |  |  |
| functional.shader_api.program_binary |  |  | 14 |  |
| functional.shaders.builtin_functions |  | 824 |  |  |
| functional.shaders.derivate |  | 88 |  |  |
| functional.shaders.random | 26 |  |  |  |
| functional.shaders.texture_functions | 304 |  |  |  |
| functional.state_query.fbo |  | 1 |  |  |
| functional.state_query.indexed |  |  | 9 |  |
| functional.texture.compressed |  |  | 266 |  |
| functional.texture.filtering |  |  | 48 |  |
| functional.texture.format |  | 32 | 16 |  |
| functional.texture.shadow | 432 |  |  |  |
| functional.texture.units |  | 33 |  |  |
| functional.texture.wrap |  |  | 1008 |  |
| functional.transform_feedback.random_full_array_capture | 2 |  |  |  |
| functional.uniform_api.random | 1 |  |  |  |

## Against the 26d1c4a2 vivante column

Every one of the 3,930 rows is also a vivante row of
`docs/es3/host-gles3-notpass.tsv` (26d1c4a2, 4,171 vivante rows), with the same
status, kind and message. The 241 rows that list has and this one does not:

| status | kind | group | rows |
|---|---|---|---:|
| NotSupported | refusal | functional.texture.wrap | 216 |
| NotSupported | refusal | functional.texture.format | 24 |
| Fail | compile | functional.uniform_api.random | 1 |

The tip totals moved from 8,056 / 32,200 / 2,117 / 0 / 437
(pass / fail / notsupported / crash / other) at 26d1c4a2 to the ones above;
the groups that changed:

| group | 26d1c4a2 | 05769bf2 (as at 5454be1b) |
|---|---|---|
| functional.implementation_limits | 41 / 8 / 0 / 0 / 0 | 43 / 6 / 0 / 0 / 0 |
| functional.state_query | 918 / 19 / 9 / 0 / 0 | 921 / 16 / 9 / 0 / 0 |
| functional.texture | 19 / 3508 / 1578 / 0 / 0 | 19 / 3748 / 1338 / 0 / 0 |
| functional.uniform_api | 1604 / 577 / 0 / 0 / 0 | 1605 / 576 / 0 / 0 / 0 |

## Change against 5454be1b

The 65 commits `5454be1b..05769bf2` changed nothing under `src/vivante`
(`git diff --stat 5454be1b 05769bf2 -- src/vivante` is empty). They did change
shared code the vivante back end runs through (`src/gl`, `src/backend.rs`)
and other back ends, but no GC7000UL host verdict moved. Each of the 74 part
files here differs from its 5454be1b twin only in its first line (the commit),
so every tip count and every not-pass row is the same, group by group:

| not-pass kind | rows at 5454be1b | rows at 05769bf2 | stopped | started |
|---|---:|---:|---:|---:|
| Fail compile | 765 | 765 | 0 | 0 |
| Fail gl-error | 1,288 | 1,288 | 0 | 0 |
| NotSupported refusal | 1,870 | 1,870 | 0 | 0 |
| NotSupported gl-error | 7 | 7 | 0 | 0 |

- Cases that stopped failing to compile: none. In particular
  `functional.shaders.texture_functions` (304), `functional.texture.shadow`
  (432), `functional.shaders.random` (26),
  `functional.transform_feedback.random_full_array_capture` (2) and
  `functional.uniform_api.random` (1) fail to compile exactly as before.
- Cases that started failing to compile: none.
- New GL errors: none. The Fail and NotSupported gl-error rows of every group
  in the table above (`functional.shaders.builtin_functions` 824,
  `functional.shaders.derivate` 88, `functional.fbo.*`,
  `functional.fragment_out.*`, `functional.texture.units` 33 and the rest)
  are the same rows with the same messages.
- New refusals: none, and none lifted (`functional.texture.wrap` 1,008,
  `functional.texture.compressed` 266, `functional.draw_buffers_indexed` 76,
  `functional.multisample` 48 and the rest unchanged).
- The tip totals are the 5454be1b ones: Pass 8,062, Fail 32,434,
  NotSupported 1,877, Crash 0, other 437.

Whatever the 65 commits did to the GC7000UL back end's output on the board,
this HOST-ONLY column cannot show it: render verdicts are meaningless here
(see the warning at the top).

## For the i.MX8MP board-vs-host compare

The i.MX8MP campaign's board-vs-host compare
(`docs/es3/board-recipe-imx8mp.md` step 14,
`tools/cts/imx8mp-gles3-compare.py`) should take this column's
`host-gc-gles3-notpass.tsv` as its host list, in place of the vivante rows of
the 26d1c4a2 `docs/es3/host-gles3-notpass.tsv` that the 9bc82915 compare used:
it is at the campaign commit 05769bf2 and covers the whole mustpass. The
recipe is not edited here.

## Files

- `hgc<N>/<group>.tsv`: the part files (`--part` output, unchanged). Their
  per-case render Fails are not listed, as in every part.
- `host-gc-gles3-tip.tsv`: the per-group tip of all parts, in the format of
  `docs/es3/host-gles3-tip.tsv` (vivante rows only; the 37 `functional.shaders`
  part rows summed into one).
- `host-gc-gles3-notpass.tsv`: the not-pass rows of all parts sorted by case,
  in the format of `docs/es3/host-gles3-notpass.tsv` (vivante rows only).
  `tools/cts/host-gles3-tally.py --apply-owners` fills its rc and owner columns.

Both merged files have a second `#` line naming their source. `--merge` of
`host-gles3-tally.py` needs one part per GPU, so they are rebuilt from the
parts with the commands below (the three header lines are copied from the
files as they are, not rebuilt):

```sh
cd docs/es3/host-gc-gles3-05769bf2
T=$(printf '\t')
{ sed -n 1,3p host-gc-gles3-tip.tsv
  awk -F'\t' '/^# section tip$/ { s = 1; next } /^#/ { s = 0; next }
    s { g[$2] = 1; for (i = 3; i <= 7; i++) t[$2, i] += $i }
    END { for (k in g) { printf "vivante\t%s", k
      for (i = 3; i <= 7; i++) printf "\t%d", t[k, i]; print "" } }' hgc*/*.tsv |
  LC_ALL=C sort -t "$T" -k2,2; } > tip.new
{ sed -n 1,3p host-gc-gles3-notpass.tsv
  awk '/^# section notpass$/ { s = 1; next } /^#/ { s = 0; next } s' hgc*/*.tsv |
  LC_ALL=C sort -t "$T" -k2,2; } > notpass.new
cmp tip.new host-gc-gles3-tip.tsv && cmp notpass.new host-gc-gles3-notpass.tsv
```
