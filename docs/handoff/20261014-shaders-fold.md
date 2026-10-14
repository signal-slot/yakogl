<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# shaders-fold: exact round / roundEven, 2026-10-14, plan #144 item shaders-fold

Item shaders-fold: the largest dEQP-GLES3 `shaders` bucket whose cause is in
`src/shaderc/online/{fold,fmath,builtin_call,loop_count}.rs`. Changed:
`src/shaderc/online/builtin_call.rs` (`round`), new
`tests/es3_shaderc_fold.rs`. No other file under `src/` changed.

## How the bucket was chosen

`docs/es3-shader-failure-map.md` (commit `9168b2a`, PR #535) maps the
`396d667a` run. None of its buckets has its cause in these four files: they
are the integer render targets (GL, back ends), texture look-ups (back ends,
upload), the `trunc` refusal (compiles now) and unattributed rows. So the
newer per-case list of the AM62P was used:
`tools/cts/log/board-sessions/am62p-gles3-percase-2026-10-13.notpass.tsv`
(commit `cd238b6`, build `ad2b5ed`). That file is not on main: read it with
`git show cd238b6:tools/cts/log/board-sessions/am62p-gles3-percase-2026-10-13.notpass.tsv`
(branch `ccfarm/01a13578-d134-74b9-a507-0227e984ec7b`). The `round` /
`roundEven` lowering in `builtin_call.rs` is the same on that build and on
this item's base.

Its 1,397 `shaders` lines that are not Pass:

| cases | what | layer |
|---:|---|---|
| 987 | builtin_functions.precision, NoResult (image 7 tail, ResourceError) | PowerVR arena (HANDOFF Known defects) |
| 297 | texture_functions (compile refusals 226, texture size 38, "Fail" 33) | PowerVR compiler, GL |
| 24 | precision.int / uint, `m_program->isOk()` | PowerVR compiler |
| 24 | **builtin_functions.common.modf**, all precisions, "Result comparison failed" | not these files (below) |
| **16** | **builtin_functions.common.round / roundeven, `{float,vec2,vec3,vec4}_highp_{vertex,fragment}`, "Result comparison failed"** | **`builtin_call.rs`** |
| 15 | fragdepth (gl_FragDepth) | PowerVR compiler |
| 23 | derivate (FBO status 15, "Fail" 8) | GL, picture |
| 7 + 2 + 2 | qualification_order image, metamorphic, uniform_block link | back end, reflect |

The only value failures of the compiler's front end are the 40 of
`builtin_functions.common`. Of those, **round + roundeven, 16 cases, is the
largest bucket in the four files** and the only one there. modf (24) is
bigger but is not in them: the IR evaluator gives the exact modf for the same
programs (checked during this item), and modf is the only `common` function
with two outputs, so dEQP's executor binds two integer colour attachments
(`glsShaderExecUtil.cpp:663-667`), which the PowerVR back end does not draw
(`src/pvr/backend.rs`, "an integer colour attachment beside another").
How that reaches the case as "Result comparison failed" and not "Error in
draw" was not traced. Owner: the PowerVR back end (int-rt-pvr follow-up).

## Cause and fix

`round` was floor(x + 0.5) and `roundEven` built on it. The sum rounds
before the floor: 0.49999997 + 0.5 is 1.0, and above 2^23 every odd integer
+ 0.5 rounds to the even one above (round(8388609) gave 8388610). dEQP's
highp range is +-1e7, so about one input in six is above 2^23, and highp
wants the exact bits. mediump (inputs exact in f16, at most 1e3) and lowp
passed and still pass.

Now t = trunc(x) and d = x - t, both exact; |d| >= 0.5 picks t + sign(x)
for `round`; `roundEven` keeps t at |d| == 0.5 when t + sign(x) is odd. The
result gets `+ 0.0`, so a zero is +0.0, which is dEQP's reference
(`float(int(..))`; roundEven(-0.5) is +0.0 and highp compares the zero's
sign). ES 3.00 only (`COMMON_ES3`): no ES 2.0 answer changes.

## Tests (HOST-ONLY)

`tests/es3_shaderc_fold.rs`: dEQP's executor programs (vertex and fragment,
float to vec4, highp; mediump and lowp kept), run through the IR evaluator
on dEQP's special inputs, edge inputs and 4,000 seeded inputs over +-1e7,
compared with dEQP's reference and rule ported from
`es3fShaderCommonFunctionTests.cpp` (`roundEven`, `RoundCase::compare`,
`RoundEvenCase::compare`). Expected values come from the case sources; no
board log holds them. With the old code: 586 (round) and 584 (roundEven)
wrong inputs. Every program also builds on the back end wherever the same
program with `out0 = in0;` builds: the V3D refuses every vec2 / vec3
executor program, whatever the body (`Program(Prepare(Undefined(..)))` at
link, the known V3D row of `docs/es3-host-run.md`).

Measured on base `4e62d171` (V3D with `no-shader-table`, as the gate builds
it):

| check | powervr | vivante | v3d |
|---|---|---|---|
| `es3_shaderc_fold` | 4 / 4 | 4 / 4 | 4 / 4 |
| `es3_shaderc_corpus` pass ok, before -> after | 3457 -> 3457 | 3455 -> 3455 | 3457 -> 3457 |

`tools/check.sh --changed 4e62d171` (every row, since `src/shaderc` is
shared): 142 ok, every test row included (the report-es2 rows, so
`es2_report_guard`, on all three GPUs). 6 rows failed on this host only
because its `/tmp` was over quota ("Disk quota exceeded" from cc and
Python): v3d tex_filter_ref_check, the am62p netboot TFTP self-test and the
four Zephyr -Werror images. They were not verified here; CI runs them with
its own TMPDIR.

No pinned fixture under `tests/fixtures/` moved.

The first pull request of this item (#576) failed the gate on
`test_rules::a_test_binary_that_resets_the_compiler_holds_one_test`: the test
file calls `forget_compiled()` and holds 4 tests, and its mutex was not one
of the names that rule accepts as a serialiser. The mutex is now `COMPILING`,
as in `tests/online_oracle.rs`. It already covered the reset, both compiles,
the link and every read of their results.

## Open work

- Expected on the AM62P: the 16 round / roundeven highp cases turn green
  (image 7, line 14 of `tools/cts/am62p-gles3-tree.txt`). Not run on a
  board. The new code uses trunc, sign, abs, compares and selects, whose own
  highp dEQP cases pass on the AM62P. The same lowering serves
  `builtin_functions.precision.round*` / `roundeven*`, which are among image
  7's 987 cases without a result.
- Next-largest bucket in the four files: none on the 2026-10-13 list. The
  next value bucket of `shaders` is modf (24, PowerVR MRT with integer
  targets, above).
- Not on the board list, seen while reading: `packSnorm2x16` /
  `packUnorm2x16` still round with floor(x + 0.5) (`builtin_call.rs`
  `pack2x16`). Only an input just below .5 of a step goes the wrong way
  (0.49999997 packs to 1). dEQP's pack_unpack cases pass on the AM62P.
