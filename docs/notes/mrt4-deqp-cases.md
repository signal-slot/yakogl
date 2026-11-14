<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# RC14: the dEQP-GLES3 cases a draw-target count of 4 should change

The expected-flip list for the board runs that set `DRAW_TARGETS` to 4 on the
Pi 4 (V3D 4.2) and the AM62P (PowerVR BXS-4-64). It is a list, not a session
record. Design: `docs/notes/mrt4-v3d-pvr.md` (section 4.4 is the probe).
Causes per row: `docs/notes/v3d-draw-buffers-indexed.md`,
`docs/notes/v3d-fragment-out.md`.

Case names are all in `tools/cts/gles3-mustpass.txt` and in the board trees
(`tools/cts/pi4-gles3-tree.txt` image A1, D1, FR2; `tools/cts/am62p-gles3-tree.txt`
lines 10 (draw_buffers_indexed) and the fbo/fragment_out lines).

## Where "current" comes from

| Board | Record | Commit |
|---|---|---|
| Pi 4 | `docs/es3/pi4-gles3-fails.tsv` rows from `tools/cts/log/2026-11-pi4-gles3-b1b5b845-rows00-15/` (A1, D1, FR1, FR2 consoles and tallies) | run at `b1b5b845`, recorded in `d498d5ad` |
| AM62P | `docs/es3/am62p-gles3-notpass.tsv` | build `ad2b5ed`, run 2026-10-13/14 (file header) |

The AM62P record is the older one and predates many fixes; re-take the
three groups below on the baseline image before the flip, so the flip is
measured against the same build. The Pi 4 tsv keeps status only; reason
strings for the Pi 4 are in the console logs above.

## 1. `functional.implementation_limits` (Pi 4 image A1)

| Case | Pi 4 now | AM62P now | After 4 |
|---|---|---|---|
| `…implementation_limits.max_draw_buffers` | Fail | Fail, "Requirement not satisfied" | Pass |
| `…implementation_limits.max_color_attachments` | Fail | Fail, "Requirement not satisfied" | Pass |

Pi 4: `A1.console.txt` rows of the tsv. The other nine `implementation_limits`
Fail rows (uniform components, varyings, block size, compressed formats) are
not count rows and stay.

Also read the count and may turn: `state_query.integers.max_draw_buffers_getfloat`,
`…_getinteger64`, `max_color_attachments_getfloat`, `…_getinteger64` (Fail on
both boards, SQ image; AM62P message "Got invalid float/integer value"). The
tree does not say whether the count is their cause; treat a flip as a bonus,
not an expectation.

## 2. `functional.draw_buffers_indexed.*`

76 cases, all `NotSupported` on both boards (Pi 4 D1: "369 passed, 0 failed,
76 not supported", `D1.tally.txt`; AM62P tsv: 76 rows, 10 + 26 + 40
split as below). **The count alone does not flip them.** The AM62P reason
strings are `Extension GL_EXT_draw_buffers_indexed not supported`
(`es3fDrawBuffersIndexedTests.cpp:1248`, `:1426`, `:1470`) and, for 10 rows,
`Extension GL_KHR_blend_equation_advanced not supported` (`:1245`). The tree
holds no Pi 4 reason strings for D1 in the tsv; the Pi 4 D1 console shows the
same advanced-blend string for the first case, and the cause table of
`v3d-draw-buffers-indexed.md` applies by name.

| Group | Rows | Now (both boards) | After the count 4 alone | After the extension too |
|---|---|---|---|---|
| `overwrite_common.*advanced_blend_eq*` and `overwrite_indexed.*advanced_blend_eq*` | 10 | NotSupported | NotSupported | NotSupported (no advanced blend on either GPU) |
| other `overwrite_common.*` / `overwrite_indexed.*` | 26 | NotSupported | NotSupported | Pass or Fail, not predicted |
| `random.max_required_draw_buffers.0-19` | 20 | NotSupported | NotSupported | needs 4 buffers: Pass or Fail |
| `random.max_implementation_draw_buffers.0-19` | 20 | NotSupported | NotSupported | uses `GL_MAX_DRAW_BUFFERS` (4): Pass or Fail |

Expected from the count flip: 0 of the 76 change. A change in this group on
the probe image means the extension string changed too. Missing from the
tree: a Pi 4 per-row reason split; the 10/26/40 split is by case name and the
AM62P run.

## 3. `functional.fragment_out.*`

Mustpass 940 cases; `fragment_out.random.*` 100. Only `random.*` draw four
targets (basic: one attachment; array: three).

Cases that fail with `GL_INVALID_ENUM` at `es3fFragmentOutputTests.cpp:286`
("After framebuffer setup", `glFramebufferRenderbuffer` at
`GL_COLOR_ATTACHMENT3` past the cap):

- Pi 4, FR2, 26 cases (`FR2.console.txt` of the b1b5b845 run; names from its
  `Test case` lines): `random.` 0 8 17 21 24 25 29 33 41 45 55 56 66 67 70 71
  72 75 78 79 82 86 88 90 94 96. They match group G5 of `v3d-fragment-out.md`.
- AM62P, 13 cases (tsv): `random.` 0 8 17 21 25 29 41 55 67 71 72 86 96.
  The AM62P tsv also holds `fragment_out.random` Compile failed 26, Image
  comparison failed 40 and NotSupported 19; the tree does not say which of
  those draw four targets.

Expected after the flip: the INVALID_ENUM error goes away. Each of those
cases then either `Pass`es, or ends in its other cause: `NotSupported
"Unsupported attachment format"` (a float format G1-G4 of the note; the Pi 4's 19
`random` NotSupported rows are `random.{5,7,26,28,32,37,39,40,46,48,51,59,63,73,74,76,80,81,95}`) or `Image comparison failed`. Do not count a remaining row as a
fourth-target defect before checking its format. The other Pi 4 `random` Fail
rows (`11 27 31 43 64`, 5 cases, image comparison) are not count-related and
should not move. The 12 `array.float.rg16f_*` Fail rows and the 115 NotSupported
rows (G1-G3) are format rows and should not move.

Pi 4 totals today (`pi4-gles3-fails.tsv`): `fragment_out` 43 Fail + 115
NotSupported = 158 rows. Expected change: up to 26 of the 43 leave `Fail`,
none of the 115 move.

## Check lines for a board session's tally

Console tally per image (`tools/cts/deqp-uart-tally.py`, `deqp: Pass|Fail|NotSupported <case>` lines):

```sh
# 1. the two count cases (Pi 4 image A1; AM62P its implementation_limits line)
grep -E 'deqp: (Pass|Fail|NotSupported) dEQP-GLES3.functional.implementation_limits.max_(draw_buffers|color_attachments)$' A1.console.txt
# expect: two "deqp: Pass" lines

# 2. draw_buffers_indexed (Pi 4 image D1): still 76, none changed by the count
grep -c 'deqp: NotSupported dEQP-GLES3.functional.draw_buffers_indexed' D1.console.txt   # 76
grep -c 'deqp: Pass dEQP-GLES3.functional.draw_buffers_indexed' D1.console.txt           # 0

# 3. fragment_out.random INVALID_ENUM at :286 (Pi 4 image FR2): was 26, expect 0
grep -c 'GL_INVALID_ENUM at es3fFragmentOutputTests.cpp:286' FR2.console.txt             # 0
# which of the 26 now pass
for n in 0 8 17 21 24 25 29 33 41 45 55 56 66 67 70 71 72 75 78 79 82 86 88 90 94 96; do
  grep -h "deqp: [A-Za-z]* dEQP-GLES3.functional.fragment_out.random.$n\$" FR2.console.txt; done

# 4. no row that passed before turns Fail: list the image's non-Pass lines
#    and compare with its rows in docs/es3/pi4-gles3-fails.tsv; only the rows
#    of sections 1 and 3 may differ
grep -h 'deqp: Fail\|deqp: NotSupported' FR2.console.txt | sort

# 5. the totals of the image tally (mustpass restricted)
python3 tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt A1.console.txt
```

On the AM62P replace the greps with the tsv shape
(`grep -P '^(Fail|NotSupported)\t…'`) when comparing against
`docs/es3/am62p-gles3-notpass.tsv`.

## Not in the tree

- Pi 4 reason strings for D1 and FR1 in the tsv (status only).
- A post-2026-10 AM62P run of these groups: the tsv is `ad2b5ed`.
- Which `fragment_out.random` seeds draw four targets, other than the 26
  (Pi 4) and 13 (AM62P) INVALID_ENUM rows.
- Any result at a count of 4: nothing has run on a board at 4 yet.
