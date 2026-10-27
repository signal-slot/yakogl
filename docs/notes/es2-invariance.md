<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (old PR #44, ccfarm `01a11d80-bb8a-7598-8efd-71e4b54b42be`, commit `e0473221c`). Written on 2026-10-09 against the integration tip `cc27948`, not on main; the text is as written, only file paths are changed to main's layout. **Status (2026-10-12): fixed on main, differently.** Main took the alternative this record lists in section 5, Mesa's rule: `f9688bc5` (merged by `e17fb6a7`) marks the sums on the way to an `invariant` output exact and does not fuse them; the AM62P board passed image 1 on `e17fb6a7` (1,379 / 0). The "proposed fix" below was not made. `tests/es2_invariance.rs` now runs all twelve cases on every back end with no `#[ignore]`.
# dEQP-GLES2 `shaders.invariance.*.common_subexpression_{1,2,3}`: the 9 AM62P regressions

Item `es2-invariance-repro` of plan #4. Host only; no file under `src/`, `tools/` or `Cargo.toml` changed.
Every claim is marked **[measured]** (run on the host in this session, integration tip `cc27948`),
**[read]** (read from source or a committed record, not run) or **[inferred]** (reasoning, not run).

## Result in one paragraph

The PowerVR back end decides whether to fuse `a * b + c` into one `fmad` from **how many instructions read the
product**: `fusion_pairs` (`src/shaderc/online/powervr/prepare.rs:1014`) takes a product as a candidate only
when it is read exactly once (`once(v, read)`, `prepare.rs:1104`; the same rule for the `fsat` clamp at
`:1107`). In each of the three cases, the first program reads some products a second time to compute the
unrelated varying `v_unrelated`, and the second program does not. So the same `gl_Position` expression is
an `fmul` then an `fadd` in one program and one `fmad` in the other **[measured]**. The hardware rounds `fmad`
once (pco declares a hardware ffma, `pco_nir.c:42-43` **[read]**; the board fails as single rounding predicts
**[inferred]**). So the two draws place some vertices differently, and dEQP sees red pixels. The rule came in
with `6f96104` (2026-10-05, opt-in) and became the default in `18972dc` (2026-10-05). Both are after the last
full AM62P tree `d60b9d76` (2026-10-02), where the cases passed **[read]**. GC7000 and V3D compute
`gl_Position` identically in both programs of every case **[measured]**: neither has a fusion pass. Proposed
fix: make the fusion decision a function of the sum's own expression, not of the product's other readers
(section 5).

## 1. What the cases check [read]

VK-GL-CTS `93bca0186`, `modules/gles2/functional/es2fShaderInvarianceTests.cpp` (checkout under
`$CCFARM_DEPS_DIR/VK-GL-CTS`):

- **Harness** (`InvarianceTest`, :112-409). `init` (:165-240) builds two programs from a pair of vertex
  shaders and one fragment shader. It uploads 72 narrow triangles (vertex 3 = vertex 2 + 0.01 × random) and
  72 normal ones, from `de::Random(123)`; x, y, z lie in [-1, 1] and w in [0.2, 1] (:50-60, :205-239). `iterate`
  (:257-366) clears to black, draws the narrow pattern with program 1 in red, then with program 2 in green.
  It fails on **any pixel with red ≠ 0** (`checkImage`, :368-409). With a depth buffer, it repeats this on the
  normal pattern: `GL_ALWAYS` for program 1, then `GL_EQUAL` for program 2 (:305-357), so the depths must be
  equal too. A single rounding difference in one vertex's `gl_Position` can move a triangle edge across a
  pixel centre, or change its depth.
- **Fragment shader** (:425-432): `u_color` plus `dot(v_unrelated, vec4(1))` in blue, so `v_unrelated` is live.
- **Precisions** (:485-491): highp (`HIGH` 1e20, `MEDIUM` 1e14, `LOW` 1e9), mediump (1e4, 1e2, 1e1), lowp
  (0.9, 1.1, 1.15). The GLSL is substituted by `formatGLSL` (:500-517).
- **common_subexpression_1** (:558-591). Program 1 computes `a`, `b`, `c`, `d`, `e`. `v_unrelated = a + b + c + d + e`,
  and `gl_Position = a_input + fract(c) + e`. Program 2 computes only `b`, `c`, `e` and sets `v_unrelated = 0`.
  `c = b - HIGH * a_input.zzxx + a_input.xzxy`, where `b = HIGH * a_input.zzxx`.
- **common_subexpression_2** (:593-624). `a = MEDIUM * (x + y)`, `c = a * a`, `d = c / MEDIUM / MEDIUM`, and
  `gl_Position = a_input + d`. Program 1 also reads `a`, `c`, `d` for `v_unrelated`.
- **common_subexpression_3** (:626-660). `x = a_input.x * 0.2`, `a, b, c = swizzles × 0.7 / 0.7 / 0.5`, and
  `gl_Position = a_input + x * (a + b + c)`. Program 1 also computes `v_unrelated = x*a + x*b + x*c`.

GLSL ES 1.00 §4.6.1 requires that an `invariant` output computed by the same expression with the same inputs
gives the same value in every program. Unrelated code may not change it. A compiler fails these cases when
a choice about how to evaluate `gl_Position` depends on code that does not feed it. Examples: fusing or
not fusing a multiply-add, reassociating, or folding differently.

`common_subexpression_0` (:521-553) is the control. Its products are each read once in both programs (its
`v_unrelated` repeats the subexpression in the source instead of sharing a variable). It passes on the board
**[read]** and on the host **[measured]**.

## 2. Reproduction [measured]

`tests/es2_invariance.rs` (new) builds both programs of each case at each precision, with the same front
end and chain as the product:

- **PowerVR** (`--features host-test,gpu-powervr,shaderc-online[,report-es2]`): `lower::compile_to_ir`, then
  `powervr::chain::build_vertex`, which is the code `compile_vertex_with` runs for a dEQP image's
  `glCompileShader`. The test traces the allocated program back from the epilogue's writes of output dwords
  0..3 (`Built::writes`) to the vertex inputs. The result is a canonical expression: moves are transparent,
  and the operands of `fadd`, `fmul` and the factors of `fmad` are sorted, which IEEE 754 makes exact.
- **GC7000** (`gpu-vivante,shaderc-online`): `select::select`, traced lane by lane through write masks and
  swizzles from `vs_pos_out_reg`, with constant-file reads resolved to their literal.
- **V3D** (`gpu-v3d,shaderc-online,no-shader-table`): `v3d::prepare::prepare_vertex`, with every varying
  component asked for (as the link asks), traced from `Prepared::position`.

Results at `cc27948`:

| back end | cs_0 (×3 precisions) | cs_1 | cs_2 | cs_3 |
|---|---|---|---|---|
| PowerVR | same | **differ** ×3 | **differ** ×3 | **differ** ×3 |
| GC7000 | same | same | same | same |
| V3D | same | same | same | same |

This matches the boards: AM62P images 1-7 at `b83216d4` failed exactly these 9 cases (record `bf5c575`,
unmerged; summarised in `docs/plan-status.md`, stage 1) **[read]**. No regression is recorded for them on
the i.MX8MP or the Pi 4.

The PowerVR expressions of `highp.common_subexpression_3`, `gl_Position.x` (`in0` = `a_input.x`, `in1` = `.y`,
`in2` = `.z`; `0x3f333333` = 0.7, `0x3f000000` = 0.5, `0x3e4ccccd` = 0.2):

```
program 1: fmad(fadd(fadd(fmul(0.7, in0), fmul(0.7, in1)), fmul(0.5, in2)), fmul(0.2, in0), in0)
program 2: fmad(fmad(0.5, in2, fmad(0.7, in0, fmul(0.7, in1))), fmul(0.2, in0), in0)
```

The other two cases have the same pattern. In cs_2, program 1 computes `fadd(fmul(1/M², c), in_k)` and program 2
computes `fmad(1/M², c, in_k)`, because `d` is read again by `v_unrelated`. Case cs_1 differs in two places:

1. `+ e`. Program 2 fuses the last product of `e` into the final sum; program 1 does not, because `e` also
   feeds `v_unrelated`.
2. `c`. Program 1 computes `fmad(-HIGH, z, b)` and program 2 computes `fmad(HIGH, z, -(HIGH*z))`. `b` is read
   twice in program 1, so the other product is fused. The two results are the rounding error of `HIGH*z`
   with **opposite signs**. At mediump this changes `fract(c)` for every vertex.

Bits, with `fmad` modelled both ways (`powervr::differing`, 4,096 vertices drawn from dEQP's ranges):

| case | lowp | mediump | highp | with `fmad` rounded as product then sum |
|---|---|---|---|---|
| cs_1 | 1,505 | 4,096 | 1,506 | 0 / 0 / 0 |
| cs_2 | 1,962 | 1,953 | 1,972 | 0 / 0 / 0 |
| cs_3 | 174 | 174 | 174 | 0 / 0 / 0 |

(Vertices whose `gl_Position` differs in some component, with `fmad` rounded once.) The last column explains
why nothing on the host caught this. The USC interpreter of `tests/usc_exec` evaluates `fmad` as
`f(0) * f(1) + f(2)` (`tests/usc_exec/mod.rs:191`), which is two roundings, because Rust does not contract.
In that model the fused and unfused forms are the same bits **[measured]**. An interpreter run of both
programs therefore agrees bit for bit, and would wrongly pass. The new test compares the expressions
instead.

## 3. Cause

**Pass and function [read, measured].** `src/shaderc/online/powervr/prepare.rs`, `fusion_pairs`
(:1014), through `prepare_fused` (:491). The chain calls `prepare_fused` for vertex shaders at
`chain.rs:80` and for fragment shaders at `chain.rs:438`. The `candidate` closure (:1100-1110) accepts an
`fmul` only when its value is defined once **and read once** (`once(v, read)`). The read count includes
every group, every output, the discard and alpha-to-one. A product shared with an unrelated output
therefore stays an `fmul` + `fadd`, while the same expression in a program without that output becomes
one `fmad`. The fusion loop (:1172) tries the `fadd`'s first operand and then its second, so when one
product is excluded, the other one can be fused instead (cs_1 point 2 above).

The doc comment (:960-965) says that "4.6.1's `invariant` asks the same computation of every program,
which a pass that always fuses the same shape gives". The pass does not always fuse the same shape: the
shape is the same, but the decision depends on the product's other readers.

No other pass is involved **[measured]**. The front end gives the same IR shape in both programs: GC7000
and V3D, which select straight from it, trace identically. `share_common` (`prepare.rs:867`, value
numbering) runs after the fusion (`chain.rs:81`) and only merges groups that compute equal values.

**Commit [read].** The history starts at the 2026-10-04 import `afb1633`. It does not contain `d60b9d76`
(2026-10-02, the last full AM62P tree), so `git log d60b9d76..` is impossible. The import has no fusion pass: its `prepare.rs` only evaluates
`Ffma` in a test model. The pass and its read-once rule were added in `6f96104` (2026-10-05 05:11, "fuse a * b + c
into one fmad, opt-in"; the commit message says "an fadd of a product that it alone reads", default off). They
were made unconditional in `18972dc` (2026-10-05 06:25, "fusion is how the compiler works; the switch goes").
`f4c41d0` added the `fsat` clamp under the same rule, and `39bd698` widened candidates to the fragment
shader's varyings. **`18972dc` is the commit that turned the regression on.**

**Why the hardware sees it [inferred].** If the USC `fmad` rounded twice, the two forms would be equal bits
and the cases would pass. pco tells NIR that the hardware has `ffma`
(`nir_float_muladd_support_has_ffma | _fuse`, `src/imagination/pco/pco_nir.c:42-43` of the Mesa 26.2.1
PowerVR tree under `$CCFARM_DEPS_DIR/mesa-26.2.1-pvr` **[read]**), and
the board fails exactly the 9 cases this predicts. A board-side `fmad` rounding probe would turn this into a
measurement. It is not needed for the fix, because removing the context dependence makes the result equal
whichever way `fmad` rounds.

**GC7000 and V3D [read, measured].** Neither back end has a context-dependent fusion. GC7000 `select` emits
`mad` only for an IR `Ffma` (`vivante/builtins.rs:139`, plus fixed driver arithmetic for `gl_FragCoord` and
`gl_PointCoord`). The front end creates `Ffma` only from the shape of a construct (matrix products
`lower.rs:6039`, `mix` `lower.rs:6487`), never from use counts. V3D has no FMA: `v3d/prepare.rs:888-901`
splits every `Ffma` into `fmul` + `fadd`. On V3D, the coordinate (binning) shader and the render shader are
both `prepare_vertex_with` of the same module, with and without varyings (`v3d/chain.rs:445-446`), and with
no fusion they compute the same position. Both back ends are deterministic for these cases, and their 24
tests run active.

## 4. Tests added

`tests/es2_invariance.rs`, 12 tests per back end: cs_0 to cs_3 × 3 precisions. No `[[test]]` entry is
needed, because cargo discovers the file, like the other `tests/*.rs`.

- PowerVR: the 9 cs_1/2/3 tests are `#[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad
  only when one sum reads it"]`. The 3 cs_0 controls are active. Proven failing at the tip **[measured]**:

  ```
  cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 \
      --test es2_invariance -- --ignored
  test result: FAILED. 0 passed; 9 failed; 0 ignored; 0 measured; 3 filtered out
  ```

  Each failure prints the differing components of both programs and the fused/unfused vertex counts
  above. `--nocapture` prints every expression for every test.
- GC7000 and V3D: all 12 active, all pass **[measured]**. The gate runs the GC7000 module in its
  `gpu-vivante,shaderc-online` rows and the PowerVR module in its `gpu-powervr,shaderc-online` rows. The
  V3D rows of `tools/check.sh` run named `--test` binaries only, so the V3D module runs only by hand until
  someone adds a line there (`tools/` is outside this item).

Runs at `cc27948` **[measured]**: `host-test,gpu-powervr,shaderc-online` and `…,report-es2`: 3 passed,
9 ignored. `host-test,gpu-vivante,shaderc-online` and `…,report-es2`: 12 passed.
`host-test,gpu-v3d,shaderc-online,no-shader-table` (also with `report-es2`): 12 passed. `cargo fmt --check`
is clean, and `cargo clippy --test es2_invariance` reports no warning in the new file in the three configs.

## 5. Proposed fix (item es2-invariance)

**Recommended: fusion as a function of the sum's own expression.** In `fusion_pairs`, take an `fmul` as a
candidate whatever reads it (drop `once(v, read)` from the `Fmul` arm of `candidate`, keeping
`once(v, def)` and `steady`). When the product has other readers, the pair must **not** delete the product's
group in `prepare_with` / `fuse_mul_add`. Today the pair always removes the inner group; it would need a
flag for "keep the inner". The `fadd` becomes `fmad(a, b, c)` and the `fmul` stays for its other readers.
Every `fadd(fmul(a, b), c)` of the same shape is then fused in every program, which is the guarantee the
doc comment at :960-965 already claims. Instruction count is unchanged (the `fadd` becomes the `fmad`); the
factors `a` and `b` may live a little longer **[inferred]**. Apply the same change to the `fsat` arm (:1107)
for consistency: a clamp is bit-exact either way except possibly for the sign of a zero or a NaN, so it is
not what fails here **[inferred]**.

Residual context dependence the fix should also remove or document **[read]**:

- `barrier` (:1084) treats any `SelOp::Machine(_)` between product and sum as a barrier, besides loop and
  `if` edges. A `sin` or `cos` in **unrelated** code that selection places between them would block the fusion
  in one program and not the other. The barrier should be control flow only, or the check should be whether
  the product's sources are rewritten in between (the `steady` rule already covers that).
- The choice between the two operands (:1172, first operand first) is deterministic once read counts no longer
  matter.
- `share_common` must stay **after** the fusion (`chain.rs:80-81`). If value numbering ran first, cs_0's
  shared subexpression would get two readers, which is harmless with the fix but was the failure mode before.

**Alternative: Mesa's rule.** Leave non-invariant code alone, and never fuse inside the computation of an
`invariant` output. GLSL's `propagate_invariance` marks those computations `precise`, and NIR's `~` (inexact)
rules such as the `ffma` fusion skip exact instructions **[inferred: from Mesa's design; no NIR or GLSL source
tree is available locally to cite]**. This keeps today's code for non-invariant shaders, and so keeps the
byte agreement with pco. But the front end drops `invariant` today (`sema.rs:615-648` and `:808-819` accept it;
no IR field carries it **[read]**). It would need an IR flag, a backward slice from the invariant outputs,
and `#pragma STDGL invariant(all)`. That is more code than the recommended fix, and every `shaderc` user pays
for the bigger IR.

**Risk to other passes and tests [inferred].** The recommended fix changes the PowerVR code of every shader
with a product that is read by more than one instruction and also feeds a sum: different bits, the same
instruction count. Tests that compare our bytes with pco's for the same source (`tests/usc_limits.rs`, the
"pco" agreement in `Gate`) may agree less often, because pco fuses only single-use products (Mesa's
`is_used_once` on the fusion rule, inferred). Run the gate's PowerVR rows and look at those counts before
merging. Tests that compare our program with the IR evaluator (`tests/usc_exec`, `online_oracle`) are
unaffected, because their `fmad` rounds twice. gl_test frame checksums on the AM62P may change, and blend
variants (`blend.rs` `fused`) are a separate fusion that this does not touch.

**Predicted board result [inferred].** With the fix, the host expressions of both programs are identical in all
9 cases (the ignored tests then pass and their `#[ignore]` is removed). The AM62P then draws both programs
identically, and `shaders.invariance.*.common_subexpression_{1,2,3}` pass, bringing images 1-7 back to the
10,101 cases of the `b83216d4` run with 0 failures. Nothing changes on the i.MX8MP or the Pi 4.
Confirmation needs one AM62P image containing `shaders.invariance` (image 1-7's range; `am62p-es2-final-a`).
