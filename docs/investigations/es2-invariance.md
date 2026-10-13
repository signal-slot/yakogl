<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES2 `shaders.invariance.*.common_subexpression_{1,2,3}`: the 9 AM62P regressions

Item `es2-invariance-repro` of plan #20, host only. No file under `src/`, `tools/` or `Cargo.toml` is
changed by this item. Every claim is marked:

- **[measured]**: run on the host in this session, at main `9390731` (2026-10-12).
- **[read]**: read from source or a committed record, not run.
- **[inferred]**: reasoning, not run.

The first record of this investigation is `tools/cts/log/investigations/es2-invariance.md`. It was
written on 2026-10-09 against the integration tip `cc27948`, before the fix, and the owner landed it on
main with a status note. This file is the record at the current tree: the cause, the fix as landed, the
regression test and its proof, and what is left for the plan.

## Status

**Fixed on main and confirmed on the board. Nothing is left for the fix item `es2-invariance`.**

- The cause is in the PowerVR back end's multiply-add fusion (section 3).
- The fix is `f9688bc5`, merged as `e17fb6a7` (2026-10-10): a sum on the way to an `invariant` output is
  not fused into an `fmad` (section 4) **[read]**.
- AM62P image 1 built from `e17fb6a7` passes 1,379 / 0; the 9 cases pass (`tools/cts/RESULTS.md`, "AM62P:
  image 1 on `e17fb6a7`", record `records/am62p/deqp-e17fb6a7/img1`) **[read]**.
- `tests/es2_invariance.rs` runs 12 cases on each of the three back ends. All 36 pass at `9390731`, and
  the 9 PowerVR cases of `common_subexpression_{1,2,3}` fail when the fix is disabled (section 5)
  **[measured]**.

## 1. What the cases check [read]

VK-GL-CTS `93bca0186`, `modules/gles2/functional/es2fShaderInvarianceTests.cpp` (the checkout under
`$CCFARM_DEPS_DIR/VK_GL_CTS`). The line numbers below were checked against that file in this session.

- **Harness** (`InvarianceTest`, :112-409). `init` (:165-240) builds two programs from a pair of vertex
  shaders and one fragment shader, and uploads 72 narrow triangles and 72 normal ones from
  `de::Random(123)`. `iterate` (:257-366) clears to black, draws the pattern with program 1 in red, then
  with program 2 in green. `checkImage` (:368-409) fails on any pixel whose red channel is not 0: "Detected
  variance between two invariant values" (:361). With a depth buffer, program 1 draws with `GL_ALWAYS` and
  program 2 with `GL_EQUAL`, so the depths must be equal too. One rounding difference in one vertex's
  `gl_Position` can move a triangle edge across a pixel centre.
- **Fragment shader** (:425-432) reads `v_unrelated` into the blue channel, so it is live in program 1.
- **Precisions** (:485-491):

  | precision | `HIGH` | `MEDIUM` | `LOW` |
  |---|---|---|---|
  | highp | 1e20 | 1e14 | 1e9 |
  | mediump | 1e4 | 1e2 | 1e1 |
  | lowp | 0.9 | 1.1 | 1.15 |

  `formatGLSL` (:500-517) substitutes them; the GLES2 module has no version line and uses `attribute` and
  `varying`.
- **common_subexpression_1** (:558-591). `gl_Position = a_input + fract(c) + e`. Program 1 also builds
  `v_unrelated = a + b + c + d + e`, so `b` and `e` get extra readers.
- **common_subexpression_2** (:593-624). `gl_Position = a_input + d`, where `d = (a * a) / M / M`. Program 1
  also reads `a`, `c` and `d` for `v_unrelated`.
- **common_subexpression_3** (:626-660). `gl_Position = a_input + x * (a + b + c)`. Program 1 also computes
  `v_unrelated = x*a + x*b + x*c`.
- **common_subexpression_0** (:521-553) is the control. Its `v_unrelated` repeats the expression in the
  source instead of sharing a variable, so each product has one reader in both programs. It passed on the
  board throughout.

Both vertex shaders of every pair declare `invariant gl_Position` and compute it with the same expression.
GLSL ES 1.00 section 4.6.1 requires an `invariant` output computed by the same expression from the same
inputs to have the same value in every program, whatever unrelated code the program contains. A compiler
fails these cases when a choice about how to evaluate `gl_Position` depends on code that does not feed it:
fusing a multiply-add in one program and not in the other, reassociating, or folding constants differently.

## 2. Reproduction on the host [measured]

`tests/es2_invariance.rs` builds both programs of each case at each precision with the product's front end
(`lower::compile_to_ir`) and each back end, and compares the computation of `gl_Position` as the back end
hands it to its encoder:

- **PowerVR**: `powervr::chain::build_vertex`, the code a dEQP image's `glCompileShader` runs. The test
  traces the allocated program back from the epilogue's writes of output dwords 0..3 to the inputs. A move is
  transparent; the operands of `fadd` and `fmul` and the factors of `fmad` are sorted, which IEEE 754 makes
  exact. On a difference, `powervr::differing` evaluates both trees for 4,096 vertices in dEQP's ranges,
  with `fmad` rounded once and then with `fmad` rounded twice.
- **GC7000**: `select::select`'s instructions, traced lane by lane through write masks and swizzles from the
  position register.
- **V3D**: `v3d::prepare::prepare_vertex`'s `position`, with every varying component asked for.

Results at `9390731`, and with the fix disabled (section 5):

| tree | back end | cs_0 (x3) | cs_1 (x3) | cs_2 (x3) | cs_3 (x3) |
|---|---|---|---|---|---|
| `9390731` | PowerVR | same | same | same | same |
| `9390731` | GC7000 | same | same | same | same |
| `9390731` | V3D | same | same | same | same |
| `9390731` with `mark_exact` not called | PowerVR | same | **differ** | **differ** | **differ** |

The 9 differing pairs are exactly the board's 9 cases. `highp.common_subexpression_3`, `gl_Position.x`
(`in0..in3` are `a_input.xyzw`; 0.7 = `0x3f333333`, 0.5 = `0x3f000000`, 0.2 = `0x3e4ccccd`), with the fix
disabled:

```
program 1: fmad(fadd(fadd(fmul(0x3f333333, in0), fmul(0x3f333333, in1)), fmul(0x3f000000, in2)), fmul(0x3e4ccccd, in0), in0)
program 2: fmad(fmad(0x3f000000, in2, fmad(0x3f333333, in0, fmul(0x3f333333, in1))), fmul(0x3e4ccccd, in0), in0)
```

Program 2 fuses `0.7 * x + 0.7 * y` and `+ 0.5 * z` into `fmad`s because `a`, `b` and `c` have no other
reader there; in program 1 they also feed `v_unrelated`, so they stay `fmul` + `fadd`. In
`common_subexpression_2`, program 1 computes `fadd(fmul(1/M, ...), in_k)` and program 2 `fmad(1/M, ..., in_k)`.
In `common_subexpression_1`, `c` becomes `fmad(-H, z, H*z)` in program 1 and `fmad(H, z, -(H*z))` in program 2
(the rounding error of `H*z` with opposite signs; at mediump this changes `fract(c)` for every vertex), and
the final `+ e` is fused only in program 2.

Vertices whose `gl_Position` differs in some component, out of 4,096, with `fmad` rounded once:

| case | lowp | mediump | highp | `fmad` rounded twice (the `tests/usc_exec` model) |
|---|---|---|---|---|
| cs_1 | 1,505 | 4,096 | 1,506 | 0 / 0 / 0 |
| cs_2 | 1,962 | 1,953 | 1,972 | 0 / 0 / 0 |
| cs_3 | 174 | 174 | 174 | 0 / 0 / 0 |

The last column is why no host test caught the regression before: the USC interpreter of `tests/usc_exec`
and the byte model of `tests/usc_limits.rs` round `fmad` as a product and then a sum, and in that model the
fused and the unfused form give the same bits. The test therefore compares expressions, not interpreter
results.

## 3. Cause [read, measured]

**Pass and function.** `src/shaderc/online/powervr/prepare.rs`, `fusion_pairs` (:1101), called from
`prepare_fused` (:492), which `chain.rs` calls for vertex shaders (:84-85) and fragment shaders (:468-469).

- The `candidate` closure accepts an `fmul` only when its value is defined once **and read once**
  (`once(v, read)`, :1196). The `fsat` arm has the same rule (:1199).
- The read count includes every group and every output. A product shared with the unrelated `v_unrelated`
  stays an `fmul` followed by an `fadd`, while the same expression in a program without that output becomes
  one `fmad`.
- When one operand of a sum is excluded, the loop fuses the other operand instead, which is how cs_1's `c`
  came out with opposite signs of the rounding error.

No other pass decides differently: GC7000 and V3D select from the same IR and trace identically, and
`share_common` (value numbering) runs after the fusion (`chain.rs:86`).

**Commit.** The history on main starts at the 2026-10-04 import `afb1633` and does not contain `d60b9d76`
(the last full AM62P tree, 2026-10-02, where the cases passed), so `git log d60b9d76..` is not possible
(`docs/es2-cts-failures.md`, section D). The pass was added opt-in in `6f96104` (2026-10-05) and became the
default in `18972dc` (2026-10-05). `18972dc` turned the regression on.

**Why the hardware sees it [inferred].** pco declares a hardware `ffma`, and the board failed exactly the 9
pairs whose expressions differ. Single rounding is the only model in which the two forms give different bits
(the table above).

**GC7000 and V3D.** GC7000 emits `mad` only for an IR `Ffma` (`vivante/builtins.rs:139`), which the front end
creates from the shape of a construct (matrix products, `mix`), never from use counts. V3D has no FMA:
`v3d/prepare.rs:996` splits `Ffma` into `fmul` + `fadd`; its binning and render shaders are the same
`prepare_vertex_with` of one module. Both back ends are deterministic for these cases, and no regression was
recorded on the i.MX8MP or the Pi 4.

## 4. The fix as landed (`f9688bc5`, merged as `e17fb6a7`) [read]

The fix follows Mesa's rule: `propagate_invariance` marks every assignment that feeds an invariant output,
`glsl_to_nir` gives those instructions `nir_fp_exact`, and the `ffma` rule matches only an inexact `fadd`.
The first record proposed the opposite rule (fuse every product whatever reads it) and listed Mesa's as the
alternative; main took the alternative.

- `ir::IoDecl::invariant` (`ir.rs:587`) records `invariant gl_Position;`, `invariant varying`, and
  `varying v; invariant v;` (`lower.rs:1301` `declared_invariant`), and `#pragma STDGL invariant(all)`
  (`pp.rs:79` `all_invariant`, set at :393).
- `chain.rs:120` `invariant_slots` passes the invariant output slots to `prepare_fused`.
- `prepare.rs:1039` `mark_exact` marks every value that feeds an invariant output as `EXACT`, to a fixpoint,
  conservatively for fixed temporaries; when any output is invariant, it also marks the sources of control
  flow and the discard.
- `fusion_pairs` skips an exact sum (:1264-1267).

Non-invariant code keeps the read-once rule, so it keeps its byte agreement with pco. The commit measured:
tools/shaders 681 compiled, 0 changed; dEQP's 1,897 shaders, 1 changed (`pragma_fragment`, whose pragma now
makes `gl_FragColor` invariant).

The fix brought its own unit tests: `chain.rs` `an_invariant_position_is_computed_alike_whatever_else_the_
shader_computes` (:1419, the 4 pairs x 3 precisions on the prepared program), `prepare.rs`
`a_sum_an_invariant_output_reads_is_not_fused` (:1564) and `a_test_of_a_program_with_an_invariant_output_is_
not_fused` (:1604), `lower.rs` `an_output_declared_invariant_is_marked_in_the_declarations` (:8611).

**What `tests/es2_invariance.rs` adds**: the PowerVR program after allocation (a later pass that
reintroduces a difference is caught here, not by the chain test), the vertex-level bit counts of
`differing`, and the GC7000 and V3D back ends with the same 12 pairs.

**Residual risk [inferred].**

- Outside invariant computations, `fusion_pairs` still decides by the read count. Section 4.6.1 allows that,
  but a varying not declared `invariant` can still be computed differently in two programs. dEQP-GLES2
  checks only declared invariants.
- The `fsat` fusion is not gated on `EXACT`. A clamp is bit-exact either way, except possibly for a zero's
  sign or a NaN.
- The `barrier` rule (:1176) treats any `Machine` op between a product and its sum as a barrier, so unrelated
  code can still block a fusion in non-invariant code. It no longer matters for exact values.

## 5. The test and its proof [measured]

`tests/es2_invariance.rs` has 12 tests per back end (cs_0 to cs_3 x 3 precisions), all active, no
`#[ignore]`. No `[[test]]` entry is needed: cargo discovers the file. Runs at `9390731`:

| features | result |
|---|---|
| `host-test,gpu-powervr,shaderc-online,report-es2` | 12 passed |
| `host-test,gpu-vivante,shaderc-online,report-es2` | 12 passed |
| `host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2` | 12 passed |

`cargo fmt --check` is clean.

**Proof that the test sees the defect.** The git operations that would give a pre-fix tree (`git worktree
add`, `git archive`, `git clone`, `git checkout <rev> -- .`) were refused in this session, so the defect was
re-created in place: the call `mark_exact(sel, invariant, seen);` in `fusion_pairs` (`prepare.rs:1147`) was
replaced by a no-op, the PowerVR configuration was run, and the line was restored (`git status` clean
afterwards, and the 12 PowerVR tests pass again). This is the first "deliberate break" the fix's commit
message names, and it gives the state before `f9688bc5` for these shaders (the fix changed nothing else in
the fusion):

```
$ cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_invariance
test powervr::highp_common_subexpression_0 ... ok
test powervr::highp_common_subexpression_1 ... FAILED
test powervr::highp_common_subexpression_2 ... FAILED
test powervr::highp_common_subexpression_3 ... FAILED
test powervr::lowp_common_subexpression_0 ... ok
test powervr::lowp_common_subexpression_1 ... FAILED
test powervr::lowp_common_subexpression_2 ... FAILED
test powervr::lowp_common_subexpression_3 ... FAILED
test powervr::mediump_common_subexpression_0 ... ok
test powervr::mediump_common_subexpression_1 ... FAILED
test powervr::mediump_common_subexpression_2 ... FAILED
test powervr::mediump_common_subexpression_3 ... FAILED
  of 4096 vertices, 174 get a different gl_Position with fmad rounded once, 0 with fmad rounded as a product then a sum (the usc_exec model)
  ...
test result: FAILED. 3 passed; 9 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.27s
```

The earlier attempt of this item (ccfarm `01a12c70`, unmerged) ran the same test file on a copy of the
pre-fix commit `32778b57` and got the same 9 failures with the same vertex counts **[read]**.

**Where the gate runs it.** `tools/check.sh` runs the whole suite in its `host-test,gpu-powervr,shaderc-online`
and `host-test,gpu-vivante,shaderc-online` configurations (with and without `report-es2`), so the PowerVR
and GC7000 modules are in the gate. The V3D configurations run named `--test` binaries only, so the V3D
module runs only by hand until a `--test es2_invariance` line is added there (`tools/` is outside this
item).

## 6. For the plan [read, inferred]

- Item `es2-invariance` (the fix): covered by `f9688bc5` / `e17fb6a7` and confirmed on the AM62P. Mark it
  done; no change is proposed.
- The next AM62P tree from origin/main is expected to show the 9 cases passing, as image 1 on `e17fb6a7`
  did. The i.MX8MP and the Pi 4 are unaffected: neither back end ever computed the pairs differently.
- Open outside this item: a `--test es2_invariance` row for the V3D configurations of `tools/check.sh`.

## 7. Re-check at main `c3f4820` (2026-10-13) [measured]

The item was run again on main `c3f4820` (its earlier result, ccfarm `01a11d80`, conflicted with main). Nothing in
`powervr/prepare.rs`, `powervr/chain.rs` or `tests/es2_invariance.rs` changed after `9390731`, and the line
references in sections 3 and 4 still hold (`fusion_pairs` :1101, `once(v, read)` :1196/:1199, `mark_exact`
:1039 called at :1147, the `EXACT` skip at :1266, `chain.rs` :84-86 and :468-470).

- The three configurations of section 5 run 12 passed each (36 / 36).
- The deliberate break was repeated: the call at `prepare.rs:1147` was replaced by `let _ = (&invariant, &seen);`,
  the PowerVR configuration ran, and the call was restored (`git status` clean, 12 passed again). The result
  was the same as in section 5: `common_subexpression_0` passed at the three precisions, the 9 cases of
  `common_subexpression_{1,2,3}` FAILED, and `mediump_common_subexpression_1` reported "of 4096 vertices,
  4096 get a different gl_Position with fmad rounded once, 0 with fmad rounded as a product then a sum".
- `tools/check.sh` still has no `--test es2_invariance` line for the V3D configurations.
