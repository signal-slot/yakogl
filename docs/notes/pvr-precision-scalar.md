<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 20 non-matrix `builtin_functions.precision` not-pass lines (SH-PREC)

Plan #400, item `cause-prec`. This is an investigation only. No file under
`src/`, `tests/` or `tools/` changed, and nothing ran on a board. It was
read at integration tip `c28470db`, 2026-11-28, against VK-GL-CTS `93bca01`
(the revision tools/cts/README.md names).
`tools/cts/am62p-notpass-causes.py` files these lines under `SH-PREC`
("precision cases outside the bound (sign, tanh, ...)", status `open`). The
matrix cases (`SH-PREC-MAT`) are out of scope here.

## 1. The cases

All 20 lines of `docs/es3/am62p-gles3-notpass.tsv` that `SH-PREC` takes are
**QualityWarning**, not Fail. Each carries the message "Shader output 0 has
low-quality shader precision". There are two functions:

| function | cases | tree row | build, run |
|---|---|---|---|
| `sign` | `precision.sign.{lowp,mediump}_{vertex,fragment}.{vec2,vec3,vec4}` (12) | 8 | b25997e4, 2026-11-22 |
| `tanh` | `precision.tanh.highp_{vertex,fragment}.{scalar,vec2,vec3,vec4}` (8) | 9 | b25997e4, 2026-11-22 |

(Rows are counted from 0 over the non-comment lines of
`tools/cts/am62p-gles3-tree.txt`, as the TSV header counts them.)

**Why QualityWarning and not Fail.** In dEQP 93bca01
(`modules/glshared/glsBuiltinPrecisionTests.cpp`, the compare loop near
line 5036) a value outside the reference interval is a QualityWarning when
it is inside the interval's *warning* range, and a Fail otherwise. The
reference intervals here pass through `FloatFormat::convert`/`roundOut`
(`framework/common/tcuFloatFormat.cpp`). Those build the result by `|=`
onto a default `Interval`, whose warning range is `(-inf, +inf)`
(`tcuInterval.hpp` constructors and `operator|`). So any non-NaN value
outside the bound is reported as a QualityWarning. These 20 lines are wrong
values, not near misses.

Both causes were found and fixed after b25997e4: `4a858e7b`
(sign, item `pvr-fsign`) and `b804b77d` (tanh, item `shaderc-tanh`). Neither
commit is an ancestor of b25997e4, and both are ancestors of `472630df`, the
build that board-a reruns rows 0-10 at. The board has not confirmed them yet.

## 2. `sign`, lowp and mediump (12 cases)

**dEQP's bound.** `Sign` is a `PreciseFunc1` (line 3451), with precision 0.
The reference is exactly `deSign(x)` (`framework/delibs/debase/deMath.h`):
`(x > 0) - (x < 0)`, which is 0 for both +0.0 and -0.0.

**What the PowerVR computed.** The FSIGN template
(`src/shaderc/online/powervr/shapes.rs`, `FSIGN`, about line 356) computes
`ft0 = sat(x * +inf)` and `ft1 = -1.0`. A `tst gez` selects between them
with `movc`. pco types that test `s32`, so it reads only the sign bit, and
`-0.0` (0x80000000) took the -1.0 arm. That is a sign-of-zero error, not an
imprecise formula: the result is -1 where dEQP wants exactly 0.

**Why only lowp and mediump.** `DefaultSampling<float>::genRandom`
(line 4492) returns `-(0 + 0)` = -0.0 when it picks an exponent below the
format's `minExp` (the subnormal branch) and then a zero significand
(1 in 17) with the negative sign (1 in 4). The exponent is drawn cubically
between `cbrt(minExp - 1.5)` and `cbrt(maxExp + 0.5)`. With mediump's and
lowp's small `minExp` that branch is common. With highp's -126 it is rare.
Also, `round(fmt, x)` of an input below the lowp/mediump range gives a
signed zero. So the lowp/mediump input sets contain -0.0, and the highp
set almost never does. This is derived from the source; the board log of
these cases is not in the tree. `tools/cts/deqp-precision-values.py` on a
row-8 log would list the inputs.

**Host evidence, at c28470db.**
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_fsign_negzero`:
2 passed. The test runs the USC bytes of `o = vec4(sign(x))` through the
byte model for ±0, ±1e-30, ±1 and ±inf. I also ran a revert check: with
`("type", "s32")` put back at shapes.rs line 374,
`negative_zero_gives_zero_not_minus_one` fails with
"sign(0x80000000) = -1, want 0".

**Fix (landed, `4a858e7b`).** The `tst` in `FSIGN` is typed `f32`, so an
IEEE compare takes -0.0 >= 0, and -0.0 takes the `sat(x * inf)` arm and
gives 0. Paths: `src/shaderc/online/powervr/shapes.rs` (FSIGN), and the
models that follow it in `tests/usc_exec/ir.rs` and `tests/usc_limits.rs`
(`FSIGN_F32_DEVIATION`). Test: `tests/pvr_fsign_negzero.rs`. The same class
in other templates was audited by item `fsign-audit`.

## 3. `tanh`, highp (8 cases)

**dEQP's bound.** `Tanh` is derived (line 2726): `sinh(x) / cosh(x)`, with
`sinh` and `cosh` defined as `(exp(x) ∓ exp(-x)) / 2` (lines 2724-2725). The
interval is built from those operations: exp at highp has
`3 + 2|x|` ulp (line 2266), each add and sub rounds outwards, and `Div`
(line 2160) gives a finite bound while the denominator stays a normal
number. For |x| up to about 88.03, cosh(x) <= 2^126, so the highp bound
around tanh(±88) = ±1 is tight and does not contain 0.

**What the PowerVR computed.** The shared front-end lowering
(`src/shaderc/online/builtin_call.rs`, `lower_es3`, the
`"sinh" | "cosh" | "tanh"` arm, about line 248) used to compute
`(e^x - e^-x) * rcp(e^x + e^-x)`. For 87.34 < |x| the sum passes 2^126, so
`rcp(sum)` is below 2^-126. The USC flushes subnormals to zero, so
`rcp(sum)` read 0, and tanh(±88) came out 0 instead of ±1. The formula is
fine in exact arithmetic. The defect is how denormals are handled: one
intermediate leaves the normal range before dEQP's bound gives up. The
hardware ops can meet the bound. No `frcp` or `fexp` imprecision is
involved.

**Why only highp.** At mediump and lowp, `exp(x)` overflows the format
well below 88 (mediump's largest value is 65504, so e^x overflows for
x > ~11). The cosh interval then contains +inf, `Div` widens to an
unbounded interval, and dEQP accepts 0. Only highp keeps the bound finite
right up to the flush point. This is derived from the source.

**Host evidence, at c28470db.**
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test shaderc_tanh_ftz`:
5 passed. The test runs the front end's IR of dEQP's vertex and fragment
executor shaders through a flush-to-zero evaluator, and checks
x in {1e-3, 0.5, 9, 10, 20, 44, 88} and their negatives against dEQP's
highp interval rebuilt in f64. I also ran a revert check: with the arm put
back to `diff * rcp(sum)`, 4 of the 5 tests fail
("Fragment tanh(88) is 0 under a flush to zero", the same for tanh(-88),
the vertex case, and `rcp_reads_the_halved_cosh`).

**Fix (landed, `b804b77d`).** The lowering keeps the halves and divides
once: `(0.5 * diff) * rcp(0.5 * sum)`, which is `sinh * rcp(cosh)`.
`rcp(cosh)` stays normal for |x| <= 88.03, the whole range where dEQP's
bound is finite. Path: `src/shaderc/online/builtin_call.rs`. Test:
`tests/shaderc_tanh_ftz.rs`. The same lowering serves Vivante and V3D.
Other transcendental lowerings that could lose an intermediate to the flush
were audited by item `ftz-audit`.

## 4. What is left

- Board confirmation. Rows 8 and 9 at a build that contains `4a858e7b`
  and `b804b77d` (472630df, board-a) should take all 20 lines out of the
  TSV. If any remain, run `tools/cts/deqp-precision-values.py` on that
  row's log and reopen this note.
- No further PowerVR change is proposed for SH-PREC. No `src/gl` change
  is involved.
