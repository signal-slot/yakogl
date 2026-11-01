<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex-shader ops behind the `arithmetic` refusals

Triage of the vertex sources that still end in `Valhall: arithmetic other
than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and
abs) is not lowered yet` (`MaliError::Arithmetic`, tools/mali/corpus-verdicts.txt)
at the tip 7ad18d27: gl_test cases 86 anyall, 181 fusemad, 182 fsat, 188
predif and the corpus vertex sources below. The vertex refusal site is the
`_ => return Err(MaliError::Arithmetic)` arm of the vertex `IrOp::Alu` match
in `src/shaderc/online/mali/nir.rs` (`AluOp::Fadd`, `Fneg`, `Frcp`, `Fmul`,
`Ffma` of a matrix term, `Fdot*`, `Imod`, `Ieq`, `Ior`, `Bcsel` are taken; the
integer ops are `IntValue`; everything else falls to `Arithmetic`). Mesa's side
is 26.2.1 on drm-shim; the sequences are in `tools/mali/testdata`.

## How the ops were named

A temporary `eprintln!("{a:?}")` in that arm (with the arm made non-fatal so
every op of a source is listed) and a temporary test that ran
`corpus_verdict_lines()`; reverted, nothing under `src/` changes. The
fragment-stage rules (`nir_fs.rs` `alu32`: `Fabs`, `Fmax`/`Fmin` with a zero
as `Fclamp(clamp::ZERO_INF)` / `ZERO_ONE`, `Frsq`) already exist; the vertex
stage lacks them.

| Source | Case | Ops the vertex arm meets (`AluOp`) |
|---|---|---|
| `es100/anyall.vert` | 86 | `Feq` (4: `equal(a_uv, 0)` x2 lanes and its widened lanes), `Ball` (the `all`/`any` reduction as the IR holds it) |
| `gltest/fma_vs_ctrl.vert` | 181 (second half) | `Fabs` (`abs(u_a*u_b)`); `floor(..)+0.25` was not reached once `Fabs` returned a value, so `Ffloor` may follow |
| `gltest/fsat_vs_ctrl.vert` | 182 | `Fmax`, `Fmin` (`max(u_q,0)`, `min(t,1)`, `clamp`) |
| `gltest/fsat_vs_folded.vert` | 182 | `Fmax`, `Fmin` (clamp, `min(max())`, `max(min())`) |
| `gltest/predif_vs_uni.vert` | 188 | `Flt` (`u_x > 0.0`, then `Bcsel` of the if-converted arms) |
| `es100/vrcp.vert` | corpus | `Frsq` (`inversesqrt`; `Frcp` is taken) |
| `es100/psiz_if.vert` | corpus | `Flt` (`a_position.x > 0.0`, `gl_PointSize` merged by `Bcsel`) |
| `es100/partial_varying.vert` | corpus | `Flt` (`a_coords.x > 0.0`, a one-lane varying under an if) |

All eight cases/sources end at the **first** op listed; once it is lowered the
next gap can differ (below).

## Mesa's vertex sequences

References (`vsarith_*`; the first three are byte-identical copies of the
`alu_*` ones that `docs/notes/mali-alu-ops.md` made with `drm_shim_ref.sh run`
and `pan_ref_diff.py extract --draw`, each a vertex shader + `fma_fs_varying`
or `interp_color` fragment pair, the draw's first program):

| Reference | Source pair | What the vertex program does |
|---|---|---|
| `vsarith_86_anyall` | `es100/anyall.vert` + `gl_test_es100/interp_color.frag` | NIR folds `all(equal(uv,0))` to `FADD.f32 a.abs, b.abs` then `FCMP_OR.f32.eq.f1 ..,0`; `any(greaterThan(n,0.5))` to an `FMAX.f32` chain and `FCMP_OR.f32.lt.f1 0.5h, max`; `.f1` writes 1.0, so no select |
| `vsarith_182_fsat` | `gltest/fsat_vs_ctrl.vert` + `fma_fs_varying.frag` | `max(q,0)` is `FADD.f32.clamp_0_inf x, 0x0.neg`, `min(t,1)` is `FMIN.f32.clamp_0_1`, `clamp(a,0,0.8)` is `FMAX.clamp_0_1`/`FMIN` pair; the varying `*0.5+0.25` is `FMA.f32` and a `FMA.f32 x,0,x` (the vertex stage's `+0`) |
| `vsarith_182_fsat_folded` | `gltest/fsat_vs_folded.vert` | only `FADD.f32.clamp_0_1 x, 0x0.neg` per lane (NIR folds the `clamp` chain to `fsat`) |
| `vsarith_188_predif` | `gltest/predif_vs_uni.vert` | if-converted: `FCMP_OR.f32.lt.m1 0x0, u5.w1` (constant on the left), one `CSEL.u32.eq` per lane; no branch |

Not made: `fma_vs_ctrl` (181), `vrcp`, `psiz_if`, `partial_varying`. The
shim could not be built in this session (see Gaps), so these have no
reference; they need `drm_shim_ref.sh run` before their lowerings.
`varyconst_181_fusemad` covers 181's first half (`fma_vs_fused`, already
`ok`); the case is held back by the control half, `fma_vs_ctrl`, whose
expected shape is: `u_a*u_b` once (`FMUL`), `FADD.f32 x, p.neg` for
`u_x - p`, `abs(p)` as the `.abs` source modifier of the `FADD`
(Valhall has no `FABS` instruction; the fragment stage does the same,
nir_fs.rs `alu32` `Fabs`), `floor` as `FROUND.f32.rtn`
(`s4_ffloor`), `* 2.0` / `+ 0.25` as `FMA`. That is a prediction from the
fragment references; the reference must confirm it.

### Per op

- **fsat / min / max as a modifier** (182): Mesa never emits `FMAX`/`FMIN`
  for a clamp against 0 or 1; `bi_opt_fsat`-style folding turns it into the
  producer's `.clamp_0_1` / `.clamp_0_inf`. The fragment stage's rule
  (`Fmax` against 0 is `Fclamp(ZERO_INF)`; `Fmin` after it `ZERO_ONE`,
  nir_fs.rs near 1694) is the same one; the vertex stage can call it.
  Unblocks 182 and `fsat_vs_ctrl`, `fsat_vs_folded`.
- **FMA fusion** (181, `varyfma`): `a*b+c` with a single use of the product is
  one `FMA.f32`; `u_x - p` with `p` used twice stays `FMUL` + `FADD`
  (`algfold_constfma*`, `s4_ffma`). 181's fused half is `ok` already
  (`ffma` of non-matrix terms is `FusedMultiplyAdd` in the vertex arm, see
  `sel_v_*_fma`, which the selector covers; the refusal there is a different
  error and is not an `Arithmetic` one).
- **select** (188, `psiz_if`, `partial_varying`): `Flt` feeds a `Bcsel`; the
  vertex arm already takes `Bcsel` as `CSEL.u32.eq` (188's references show
  `FCMP_OR.f32.lt.m1` + `CSEL`). The compare is a `Fcmp(Lt)` node the
  vertex path has no case for (`Alu::Icmp` exists, `Alu::Fcmp` does not). The
  stop after it: `predif` and `partial_varying` come back as
  `VaryingNotFromAttribute` (the lane is computed from uniforms only, see
  `mali-varying-constants.md` rule 3; `partial_varying`'s `tt.x` is a one-lane
  varying under an if, which Mesa leaves with zero in the other lanes).
- **any / all reductions** (86): do not need an op of their own: NIR has
  already folded them to `FADD |a|,|b|` / `FMAX` plus one compare when the
  vertex shader reaches the back end; the front end's `Ball` is the IR's
  4-lane reduce and must be expanded to that form (the spare lanes repeat a
  real lane: the shader's comment records the `(0,1)` spare bug).
  Needs `Feq`/`Flt` as an `FCMP_OR.f32.<cmp>.f1` with a `.abs` modifier and
  an `FMAX` chain.
- **frcp / frsq** (`vrcp`): `Frcp` is taken (`a/b` is `a * rcp(b)`);
  `inversesqrt` is `FRSQ.f32` (alu_vtransc has the `.f16` form). The vertex
  `Frsq` arm is a one-liner like nir_fs.rs 1719; the shape of its
  neighbours (`FRCP` of a `FRSQ`, `1/sqrt`) and the `+0` quirk need the
  reference.

## Proposed lowering items (each one agent session)

Ordered by the sweep cases they unblock; all touch `nir.rs` (the vertex
arm) and `tests.rs`, so they run one after another, with `bir.rs`/`select.rs`
only if a node is missing.

1. **vs-minmax-sat** (182 fsat): `Fmax`/`Fmin`/`Fsat` in the vertex arm with
   the fragment stage's modifier rules; byte-equal to `vsarith_182_fsat` and
   `vsarith_182_fsat_folded`. Probe first: the previous attempt (plan #153)
   stopped at `VaryingNotFromAttribute` because the varying lanes are
   uniform-only (flat); so this item either comes after the uniform-only flat
   varying work (`mali-varying-constants.md`, 182 `fsat` and 188) or carries
   its own pair of variants with an attribute in the expression.
2. **vs-fcmp-select** (188 predif; `psiz_if`, `partial_varying`): `Flt`
   (and `Fle`, `Feq`, `Fge`, `Fne`) as `FCMP_OR.f32.<cmp>.m1`, with
   the `Bcsel` -> `CSEL.u32.eq` already there; byte-equal to
   `vsarith_188_predif`. Same flat-varying caveat as item 1, so items 1 and 2
   are best done together as **vs-uniform-only-varyings** first.
3. **vs-abs-floor** (181 fusemad control half): `Fabs` as the `.abs`
   modifier, `Ffloor` as `FROUND.f32.rtn`; needs the `fma_vs_ctrl` reference
   first (shim). Unblocks the second half of 181, then case 181 passes if the
   fused half already does.
4. **vs-anyall** (86 anyall): `Feq`/`Flt` with the `.f1` form, the `FADD`
   `.abs` fold and the `FMAX` chain; byte-equal to `vsarith_86_anyall`. Needs
   2 and 3 for the compare and the modifier.
5. **vs-frsq** (`vrcp`): `Frsq` and the `1/x` combination; needs a
   reference.

Case 86 is the only one whose item (4) is not purely a vertex-side change of
one op: its `Ball` expansion is front-end shaped.

## Gaps of this triage

- The drm-shim could not be built here: `drm_shim_ref.sh build` needs
  `mesa-26.2.1.tar.xz` from archive.mesa3d.org, and the download was
  refused in this session; `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1` is the
  pan_shc subset, not the full tree. So the four missing references
  (`fma_vs_ctrl`, `vrcp`, `psiz_if`, `partial_varying`) were not made, and
  `vsarith_*` for 86, 182, 188 are the copies of the earlier `alu_*`
  references named above (same pairs, same Mesa 26.2.1, same
  `pan_ref_diff.py extract --draw`).
- Case 181 has no `vsarith_181_*` of its own for that reason.
