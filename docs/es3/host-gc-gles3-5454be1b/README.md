<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# HOST-ONLY GC7000UL dEQP-GLES3 at 5454be1b

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

- Commit (library and harness): `5454be1b6bf561d9ed3ee686c5e039bc323147c4`
- GPU vivante, module gles3 (the library reports OpenGL ES 3.0)
- VK-GL-CTS `93bca018` (named in the commit messages of hgc1, hgc2, hgc4,
  hgc5 and hgc6; that of hgc3 names none)
- One `tools/cts/host-deqp-run.sh` run per filter (`GPU=vivante
  MODULE=gles3`), each run directory written to a part file with
  `tools/cts/host-gles3-tally.py --part`; the commit messages named in the
  table below hold the runs' own notes

## Coverage

74 part files, all in this directory as `hgc<N>/<group>.tsv` and all in the
`--part` format (`tools/cts/host-gles3-tally.py` `load_part` reads every one).
Each part's `cases=` equals the number of `tools/cts/gles3-mustpass.txt`
cases under its filter, no case is under two filters, and together they are
the whole mustpass: 42,810 cases. Crash is 0 in every part: every case
began and ended with a result.

The hgc2, hgc4 and hgc5 parts were committed as `<group>.part` (1bacfc9a,
72658f77, b22499c4), an extension no `tools/license_audit.allow` row covers,
and those commits never reached main. They are in this directory as
`<group>.tsv` with the same bytes, as 47edb814 and 5afe26cd did for hgc3 and
hgc6; every part here is a `.tsv`.

| dir | parts | filter | commit |
|---|---|---|---|
| hgc1 | 1 (6,402 cases) | `dEQP-GLES3.functional.shaders.operator.` | f114aa05 |
| hgc2 | 3 (6,008 cases) | `dEQP-GLES3.functional.shaders.builtin_functions.`, `dEQP-GLES3.functional.shaders.matrix.`, `dEQP-GLES3.functional.shaders.random.` | 1bacfc9a |
| hgc3 | 33 (8,206 cases) | `dEQP-GLES3.functional.shaders.<group>.` per file | 47edb814 |
| hgc4 | 1 (5,105 cases) | `dEQP-GLES3.functional.texture.` | 72658f77 |
| hgc5 | 3 (6,138 cases) | `dEQP-GLES3.functional.fbo.`, `dEQP-GLES3.functional.fragment_ops.`, `dEQP-GLES3.functional.fragment_out.` | b22499c4 |
| hgc6 | 33 (10,951 cases) | `dEQP-GLES3.functional.<group>.` per file (`info.tsv`: `dEQP-GLES3.info.`) | 5afe26cd |

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

| group | 26d1c4a2 | 5454be1b |
|---|---|---|
| functional.implementation_limits | 41 / 8 / 0 / 0 / 0 | 43 / 6 / 0 / 0 / 0 |
| functional.state_query | 918 / 19 / 9 / 0 / 0 | 921 / 16 / 9 / 0 / 0 |
| functional.texture | 19 / 3508 / 1578 / 0 / 0 | 19 / 3748 / 1338 / 0 / 0 |
| functional.uniform_api | 1604 / 577 / 0 / 0 / 0 | 1605 / 576 / 0 / 0 / 0 |

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
files as they are):

```sh
cd docs/es3/host-gc-gles3-5454be1b
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
