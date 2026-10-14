<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES3 per case on main e51de0af, 2026-10-14, item oq-gc

Record: `tools/cts/log/board-sessions/am62p-gles3-percase-2026-10-13.md`;
every case that is not Pass: `am62p-gles3-percase-2026-10-13.notpass.tsv`
beside it.

## State of main

In "ES 3.0 (goal 4)", replace the AM62P tree paragraph's numbers: the
AM62P dEQP-GLES3 tree on `ad2b5ed` (main `e51de0af` plus docs; same `src/`,
`driver/` and `capi/` as main `a63fcc5b`), 2026-10-14: of 42,810, **36,552
pass / 2,880 fail / 2,195 NotSupported / 185 QualityWarning / 998 without a
result** (`f2a13877`: 32,669 / 5,549 / 2,318 / 185 / 2,089). It is the first
board run of `c4dce9125`: `fragment_ops` 1,109 -> 0 fail,
`transform_feedback` 640 -> 0, `fbo` 872 -> 367. The 998: image 7's
`builtin_functions.precision` tail 987 (the `ResourceError`) and 11 slow
`texture.filtering.3d` cases not started.

## Open work

- The "AM62P dEQP-GLES3 rows that `c4dce9125` changes" line in "State of
  main" is done (this record).

## Known defects

- **GLES3 hang**: for image 20 it is the 600 s image lifetime, not a GPU
  hang. The two non-mustpass `bgra*_linear_mipmap_linear` cases take about
  4.5 min each, so `r11f_g11f_b10f_linear_mipmap_linear` starts at 549 s;
  alone it completes in 158 s (Fail, no reset). Next: narrow
  `tools/cts/am62p-gles3-tree.txt`'s `3d.formats` and `3d.sizes` rows to
  the mustpass names, one image per `*_linear_mipmap_linear` case (not done
  in this item). Image 21's `3x7x5_linear_mipmap_linear`
  has not run alone yet. Image 11's 4 resets are
  `shaders.metamorphic.bubblesort_flag.variant_1` (3) and `variant_2` (1).
- **Depth-PBO uploads** (`tools/cts/RESULTS.md` row "AM62P GLES3 (depth
  uploads)", 10 cases): pass on the board at `ad2b5ed`, with a call trace
  that raises no GL error. The row can close.

## Next board session

- The 11 slow `texture.filtering.3d.*_linear_mipmap_linear` mustpass cases,
  one image each (up to about 4.5 min each).
