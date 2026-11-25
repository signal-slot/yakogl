<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Plan #402 baseline: HOST-ONLY refusal counts at 789ee9bb

Every figure below is HOST-ONLY: a compile and link outcome of the Mali
online compiler at the tip, run through `deqp-gles2` against the Linux-OSAL
library, which never executes a command stream. It is not a board result
and not a render verdict. The board counts it is compared with are the
whole-tree run at 8ff01055 (`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055`)
and the 37305608 board counts (plan #377 c7's caps).

## Commit and sources measured

- Tip measured: `789ee9bb` (`git describe`, clean tree).
- dEQP: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, external
  sources fetched with `external/fetch_sources.py`, the yakogl target applied
  from `tools/cts/cts-yakogl-target.patch`.
- Library features: osal-linux, gpu-mali, shaderc-online, no-shader-table,
  report-es2 (the script's set: the C OSAL in place of host-test).
- 5838 cases asked (the refused cases of the four slices), 5838 began,
  5509 with a host compile refusal, 870 not reproduced on the host
  (59 chunk processes exited 1).

## Commands

```
tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS checkout> --out <out> \
  --slice tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055 \
  --slice tools/cts/log/2026-11-24-t6-gles2-S2-8ff01055 \
  --slice tools/cts/log/2026-11-24-t6-gles2-S3-8ff01055 \
  --slice tools/cts/log/2026-11-24-t6-gles2-S4-8ff01055

bash tools/mali/refusal-compare.sh \
  --base tools/cts/log/2026-11-24-t6-gles2-S{1,2,3,4}-8ff01055 \
  --new <out>
```

`refusal-compare.sh` was run with `--status Fail` (its default), so all counts
are cases whose board status is Fail; NotSupported cases are counted apart.
The host cause text is cut at " is not lowered", the board text is not, and
the two labels differ; the table below matches the causes by their fixed
text. A host count is the case's FIRST refusal at the tip: a case that
reaches a later refusal once its first is lowered is counted only under the
first (so a count can rise when another refusal is lowered or when a
refusal's own case set shifts).

## The c4 refusals (HOST-ONLY, Fail)

"Cap / 8ff01055" is the 37305608 cap for the first three rows (c4's caps) and
the 8ff01055 board count for the rest (c4: fewer than it).

| Refusal | Cap / 8ff01055 board | Host at tip | Difference | Verdict vs cap |
|---|---:|---:|---:|---|
| VS: a uniform other than a float, vec2, vec3, vec4 or mat4 | cap 362 (board 371) | 0 | -362 | within cap |
| VS: a loop in a vertex shader | cap 105 (board 176) | 260 | +155 | over cap |
| VS: a constant operation on or to a value that is not a finite normal float | cap 114 (board 144) | 128 | +14 | over cap |
| FS: a mediump fragment shader uniform other than ... | 271 | 283 | +12 | not fewer |
| VS: a uniform array | 193 | 197 | +4 | not fewer |
| FS: an integer or boolean value | 188 | 171 | -17 | fewer |
| FS: a fragment shader that mixes highp and mediump or lowp float values | 168 | 159 | -9 | fewer |
| FS: a fragment varying load or conversion of a form no reference draws | 50 | 55 | +5 | not fewer |
| FS: back end defect, IrIndex | 206 | 204 (+10 as a link error naming the vertex shader's IrIndex) | -2 (+8) | fewer without the link-error row |
| FS: back end defect, LaneCycle | 9 | 10 | +1 | not fewer |
| VS: back end defect, Encode | 6 | 6 | 0 | not fewer |

Reading the table: the host numbers differ from the board's because the
host measures the tip (lowerings merged since 8ff01055, for example the
unitype lowering of 5e1feec9 empties the VS uniform-type row) and because
masking moves cases between rows. The loop and constant rows are over their
37305608 caps at the tip before any plan-402 lowering; the host count is not
a board count, so the board rerun (plan #377's board-t6) is what c4 finally
tests.

## Top 10 other refusals by count (HOST-ONLY, Fail)

| # | Host count | Refusal |
|---:|---:|---|
| 1 | 825 | FS: a gl_FragColor lane other than the varying's own lane or a constant |
| 2 | 767 | FS: arithmetic other than +, -, * and a dot or matrix product |
| 3 | 728 | VS: arithmetic other than +, -, * and a dot or matrix product |
| 4 | 306 | VS: a constant product in a shape Mesa's algebraic rules move or fold |
| 5 | 186 | FS: a texture coordinate other than a vec2 varying read whole or two constants |
| 6 | 172 | FS: an operand of a form or in a place no reference draws |
| 7 | 108 | VS: a fused multiply-add other than a matrix product's |
| 8 | 91 | FS: a constant product in a shape Mesa's algebraic rules move or fold |
| 9 | 60 | VS: a vertex shader texture look-up other than 2D or cube at a level |
| 10 | 43 | FS: a texture look-up other than texture2D(sampler2D, coordinate) |

Next (not top 10): 40 two varying lanes of equal value (FS), 39 control flow
(FS), 38 fp16 arithmetic on gl_FragCoord (FS).
