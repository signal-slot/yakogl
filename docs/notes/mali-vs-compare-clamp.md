<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex compare/reduce and clamp lowerings (cases 86 and 182)

**Status: design, implementation in vs-cmp-clamp** (plan #336, issue #33). This
note changes no code. It reads the Mesa 26.2.1 / drm-shim references of the two
vertex shaders that still stop at `MaliError::Arithmetic` for a compare, a
reduction or a clamp, says which instruction each IR operation becomes, where in
`src/shaderc/online/mali` it is made, and which host tests pin it. It is the
two-case slice of `notes/mali-vs-alu-ops.md` (ranked items 1 and 2: vs-cmp-reduce
and vs-clamp), whose reference table and Mesa sequences it does not repeat.

Locations are at `17e91019` (`nir.rs` etc. are in `src/shaderc/online/mali/`).

## 1. The references (all present)

| Case | Vertex shader | Fragment partner | Reference | Older twin (same words) |
|---|---|---|---|---|
| 86 anyall | `tools/shaders/es100/anyall.vert` | `gl_test_es100/interp_color.frag` | `tools/mali/testdata/vsarith_86_anyall.{vert,frag,expect.json}` | `alu_anyall.*` |
| 182 fsat | `tools/shaders/gltest/fsat_vs_ctrl.vert` | `gltest/fma_fs_varying.frag` | `vsarith_182_fsat.*` | `alu_fsat.*` |
| 182 fsat, folded form | `tools/shaders/gltest/fsat_vs_folded.vert` | `gltest/fma_fs_varying.frag` | `vsarith_182_fsatfolded.*` | `alu_fsat_folded.*` |

`vsarith_86_anyall.expect.json` and `alu_anyall.expect.json` differ only in the
`trace` file name, as `notes/mali-vs-alu-ops.md` says for all four pairs. Their
`vs.position` program is the plain position shader (`LD_ATTR_IMM.v4`, `FRCP`,
`FMAX`/`FMIN` of the 1/w clamp, three `FMA`, four stores); every difference is in
`vs.varying`. No reference is missing for these two cases. What no vertex
reference draws is a *plain* `FMAX.f32`/`FMIN.f32` of two computed operands
(no clamp modifier, no constant bound); the fragment stage has one
(`Alu::Fmax`/`Alu::Fmin` rows in `src/shaderc/online/mali/emit.rs:44-45`), the vertex arms should refuse
that shape (`MaliError::Arithmetic`) rather than guess.

## 2. Case 86: what Mesa emits (`vsarith_86_anyall`, `vs.varying`)

```
LD_ATTR_IMM.v2.f32.slot0.wait0 @r0:r1, r60, r61, index:0x1, table:0x1    ; a_uv (x r0, y r1)
FADD.f32 r0, r1^.abs, r0^.abs                                            ; |y| + |x|
FCMP_OR.f32.eq.f1 r0, r0^, 0x0, 0x0                                      ; e = (|y|+|x| == 0) ? 1.0 : 0.0
LD_ATTR_IMM.v3.f32.slot1.wait1 @r2:r3:r4, r60^, r61^, index:0x2, table:0x1   ; a_n (x r2, y r3, z r4)
FMAX.f32 r1, r4^, r2^                                                    ; max(z, x)
FMAX.f32 r1, r1^, r3^                                                    ; max(max(z, x), y)
FCMP_OR.f32.lt.f1 r1, 0x38000000.h1, r1^, 0x0                            ; f = (0.5 < max) ? 1.0 : 0.0
LEA_BUF_IMM ... STORE.i32.estream.slot0 @r0 ... STORE.i32.estream.slot1.end @r1
```

Two varying words are stored (`e`, `f`); `z = 0.0` and `w = 1.0` of `v_color` are
the link's constants (`link.rs` `VaryingConsts`, the layout of
`notes/mali-varying-constants.md`); the fragment loads `.v2`. Work registers 32,
preload `0x3800000000000000`, as every vertex varying program.

Where the instructions come from (nothing here is a "bool vector" or a
"reduction" instruction):

* `all(equal(uv, 0))` is, in NIR, `iand(feq(y, 0), feq(x, 0))` (the
  reduction runs from the last lane). `nir_opt_algebraic.py line 1213` folds
  `iand(feq(a, 0.0), feq(b, 0.0))` into `feq(fadd(fabs(a), fabs(b)), 0.0)`:
  the `FADD` with two `.abs` sources and one compare.
* `any(greaterThan(n, 0.5))` is `ior(ior(flt(.5, z), flt(.5, y)), flt(.5, x))`.
  `nir_opt_algebraic.py line 1178` (`ior(ior(is_used_once flt(a,b), d), flt(a,c))`
  -> `ior(flt(a, fmax(b,c)), d)`) takes `z` and `x` first, then `:1150`
  (`ior(flt(a,b), flt(a,c))` -> `flt(a, fmax(b,c))`) takes `y`: this
  reproduces `FMAX(z, x)` then `FMAX(.., y)` of the listing. The two rules
  were read in Mesa's tree, not run: the vs-cmp-clamp agent confirms the
  order against the shim's NIR print (as the `.stderr` of `texmore_46` holds
  one) before it encodes a different one.
* the `? 1.0 : 0.0` is `b2f32` of the compare, which `bi_optimizer`'s
  result-type fold makes the `.f1` form (`opt.rs`, `mod_prop_backward`, the
  `ResultType::F1` assignment, `src/shaderc/online/mali/opt.rs:285`): no `MUX` is left.

## 3. Case 182: what Mesa emits (`vsarith_182_fsat`, `vsarith_182_fsatfolded`)

`fsat_vs_ctrl.vert`: `t = max(u_q, 0.0)`, `v = (min(t, 1.0)*0.5 + 0.25, t*0.5,
clamp(u_a, 0.0, 0.8)*0.5 + 0.25, min(max(u_x, 0.2), 1.0)*0.5 + 0.25)`. The
varying program (all lanes are uniforms alone, so flat: the fragment is
`LD_VAR_BUF_IMM.f32.v4.src_flat32`):

```
FADD.f32.clamp_0_1  r0, u6.w0, 0x0.neg           ; min(max(q,0),1) = fsat(q)
FMA.f32 r0, r0^, 0x38000000.h1, 0x34000000.h1    ; * 0.5 + 0.25  (half immediates)
FMA.f32.wait0 r0, r0^, 0x0, r0^                  ; the exact ffma(x, 0.0, x) of a flat lane
STORE.i32.estream.slot1 @r0 ...
FADD.f32.clamp_0_inf r4, u6.w0, 0x0.neg          ; t = max(q, 0): its other use
FMA.f32 r4, r4^, 0x38000000.h1, 0x0.neg          ; t * 0.5
FMA.f32.wait1 r4, r4^, 0x0, r4^   ; STORE
MOV.i32 r2, u7.w0                                ; the bound 0.8 (FAU word 3f4ccccd)
FMIN.f32.clamp_0_1 r2, u6.w1, r2^                ; clamp(a, 0, 0.8)
FMA.f32 r2, r2^, 0x38000000.h1, 0x34000000.h1 ; FMA ..0x0.. ; STORE
MOV.i32 r2, u7.w1                                ; the bound 0.2 (3e4ccccd)
FMAX.f32.clamp_0_1 r2, u5.w1, r2                 ; min(max(x, 0.2), 1.0)
FMA.f32 r2, r2, 0x38000000.h1, 0x34000000.h1 ; FMA ..0x0.. ; STORE.end
```

`fsat_vs_folded.vert` (a helper `h(c) = c*0.5 + 0.25` over `clamp(u_x,0,1)`,
`min(max(u_q,0),1)`, `max(min(u_a,1),0)`, `min(1, max(0, u_a - 1.1))`): four
`FADD.f32.clamp_0_1`, the last `FADD.f32.clamp_0_1 r2, u6.w1, r2` with
`r2` a `MOV` of the word `bf8ccccd` (-1.1), i.e. `fsat(a + -1.1)`: the clamp is a
modifier on the `fadd` that produces the value.

The rules the sequence shows:

1. `fmin(fmax(a,0),1)` and `fmax(fmin(a,1),0)` are `fsat(a)` (`nir_opt_algebraic.py line 1088`
   and `:1091`); `fmax(a,0)` alone is `fclamp_pos` (`bifrost_nir_algebraic.py line 34-36`
   family, `nir_op_fclamp_pos` at `bifrost_compile.c line 2893`).
2. A clamp whose producer takes a clamp modifier is folded into it
   (`bi_optimizer_clamp`, already `src/shaderc/online/mali/opt.rs:256-270`, `mod_prop_backward`): `FADD`,
   `FMIN`, `FMAX` take it. A clamp of a *uniform* (no producer instruction) stays
   an `FCLAMP`, which `lower_opt_instructions` makes `FADD x, -0.0` carrying the
   clamp (`src/shaderc/online/mali/opt.rs:317-337`): the `FADD.f32.clamp_0_1 r0, u6.w0, 0x0.neg` lines.
3. A bound other than 0 and 1 stays an instruction, reached through a `MOV` of its
   FAU word (the instruction already has `u6.w1`), and the 0/1 side is the
   modifier: `FMIN.f32.clamp_0_1 (a, 0.8)` and `FMAX.f32.clamp_0_1 (a, 0.2)`.
   The NIR the shim built for those two lanes is `fmin(fmax(a,0),0.8)` and
   `fmin(fmax(a,0.2),1)` as the source writes them; which rules make the
   `.clamp_0_1` land on the `fmin`/`fmax` (and not on a separate `FADD`) was not
   traced here. This is the one gap of the design: the implementer runs the
   shim with the NIR print and writes down the two rules (the nearest are
   `nir_opt_algebraic.py line 1114`, `:1118` and `:1138-1139`) before encoding them.
4. The two uses of `t` stay two instructions of different clamps
   (`.clamp_0_1` for `min(t,1)`, `.clamp_0_inf` for `t*0.5`): `fsat` is made first
   (the fragment walk's comment, `src/shaderc/online/mali/nir_fs.rs:2025-2037`) and `fclamp_pos` is what
   is left of `fmax(a,0)` for the other user.

## 4. Where it is produced

**`nir.rs`, the vertex walk's `IrOp::Alu` match** (`build`, the arm list at
`:2317-2471`; the refusal is `_ => return Err(MaliError::Arithmetic)` at
`:2470`). The fragment walk has every one of these operations already
(`src/shaderc/online/mali/nir_fs.rs:2016-2074`); the vertex arms copy its decisions with the vertex's
`vs.alu(..)` and 32-bit width:

| IR op (`AluOp`) | Vertex arm to add | Result node |
|---|---|---|
| `Fmax(x, 0.0)` / `Fmax(0.0, y)` | as `src/shaderc/online/mali/nir_fs.rs:2022-2023` | `Alu::Fclamp(clamp::ZERO_INF)` |
| `Fmax(x, y)` otherwise | refuse unless a reference draws it (section 1) | `Alu::Fmax` |
| `Fmin(x, 1.0)` over a `Fclamp(ZERO_INF)` node | as `src/shaderc/online/mali/nir_fs.rs:2027-2037` | `Alu::Fclamp(clamp::ZERO_ONE)` of the inner source |
| `Fmin(x, y)` otherwise | refuse unless a reference draws it (section 1) | `Alu::Fmin` |
| `Feq`, `Flt`, `Fle` (per lane) | as `src/shaderc/online/mali/nir_fs.rs:2065-2068`; `Fle` is `Fcmp(Ge)` with swapped sources | `Alu::Fcmp(Cmpf::Eq/Lt/Ge)` |
| `Fabs` (the `fadd(fabs, fabs)` sources) | the 181 arm (`notes/mali-vs-abs-if.md`) | `Alu::Fabs` |
| `Ball`, `Bany` | below | built from `Fadd`/`Fabs`/`Fmax` and one `Fcmp` |
| `Bcsel(c, 1.0, 0.0)` | already there (`src/shaderc/online/mali/nir.rs:2374-2376`) | `Alu::B2f` |

`Alu::Fcmp`, `Alu::Fclamp`, `Alu::Fabs`, `Alu::B2f` and their `emit.rs` rows
(`src/shaderc/online/mali/emit.rs:57-66`, `:231-234`, `:252`) exist and are shared with the fragment
walk; so do the `.f1` fold (`src/shaderc/online/mali/opt.rs:285`) and the clamp composition. The new code
is the walk arms and one pattern:

* **`Ball`/`Bany` of the front end's compare vectors.** The IR gives `Feq(w4)`
  (resp. `Flt(w4)`) then `Ball` (resp. `Bany`) with the *last lane read repeated
  into the spare ones* (`src/shaderc/online/mali/nir_fs.rs:4148-4161` documents it; `n` is the count of
  distinct lanes of the swizzle). The vertex arm does what Mesa's passes make of
  the reduction, not the reduction itself:
  * `Ball` over `Feq(v, 0.0)` with `n = 2`: `Fadd(Fabs(y), Fabs(x))` (the
    lane order of the listing: the *later* lane first) and one
    `Fcmp(Eq)` of it against `0.0`. For `n = 3` the same rule composes
    (`:1213` on the nested `iand`); no reference draws `n = 3` or `n = 4`, so
    the arm takes `n = 2` and refuses the others with `MaliError::Arithmetic`.
  * `Bany` over `Flt(c, v)` with a constant `c` and `n = 3`:
    `Fcmp(Lt)` of `c` against `Fmax(Fmax(z, x), y)`. `n = 2` is
    `Fcmp(Lt, c, Fmax(y, x))` by the same rules (not drawn: refuse); `n = 4`
    likewise.
  * the compares of the `Feq`/`Flt` vector that only these reductions read
    must not be walked into nodes of their own (they would be dead, but the
    walk's `IrOp::Alu` arm computes every lane: the arm for `Feq`/`Flt` of a
    width above 1 whose only users are `Ball`/`Bany` returns `Val::Undef`
    lanes and the reduction arm reads the *sources*, via `m.def(s[k].val)`,
    as `is_mat_term` (`src/shaderc/online/mali/nir.rs:2170`) reads the matrix product's).
* **The lane subset.** `a_uv` is a vec2 and `a_n` a vec3 attribute: the
  `Kind::Input { comps }` already carries the declared width
  (`src/shaderc/online/mali/nir.rs:2225`), and `LD_ATTR_IMM.v2` / `.v3` come from `select.rs`
  `ld_attr` (`:197`) by the lanes read (`vs.reads`), as the 2D coordinate
  look-ups already read a `.v2`. If a `.v4` were loaded here, the bytes differ
  from the reference: that is what the byte-equal test catches.

**`select.rs`, `emit.rs`: no change expected** (the nodes above have rows).
**`link.rs`: no change** (the two stored words and the link's `z = 0.0`,
`w = 1.0` constants are the layout of `vs_constants`; the lanes are attribute
derived, so smooth, `src_f32`).

**Case 182's lanes are flat varyings.** All four read uniforms alone
(`Vs::uniform_only`, `src/shaderc/online/mali/nir.rs:1691`, accepts `Kind::Uniform` and `Kind::Alu`, so a
`Fclamp`/`Fmin`/`Fmax` node over a uniform is promoted to flat), and the
`ffma(x, 0.0, x)` conversion at `src/shaderc/online/mali/nir.rs:2712-2730` produces the `FMA.f32 r, r,
0x0, r` of the listing. `notes/mali-vs-alu-ops.md` lines 138-144 and its item 4
(the flat varying "still to come") are out of date: the flat varying landed
(`VARYCONST_POPULATION` "181" and "117", `tests.rs`); 182 finishes with the ops
alone.

## 5. Tests to add (`src/shaderc/online/mali/tests.rs`)

All byte-for-byte against the `expect.json` named, through the existing helpers.

1. `a_vertex_any_and_all_over_vectors_is_byte_equal_to_mesa_26_2_1`: `check_program(
   vsarith_86_anyall.vert, .frag, .expect.json)` (the vertex pair; the
   fragment's `LD_VAR_BUF_IMM.v2` comes with it). Add the pair to
   `vs_reference_sources()` (`src/shaderc/online/mali/tests.rs:2094`) so the corpus and the
   "every vertex reference compiles" tests see it.
2. `a_vertex_saturate_is_byte_equal_to_mesa_26_2_1` and
   `a_vertex_folded_saturate_is_byte_equal_to_mesa_26_2_1`: `vsarith_182_fsat`
   and `vsarith_182_fsatfolded` through `check_linked` (both programs are
   flat: the check on `fs.fragment`'s `src_flat32` rides along), and the FAU
   tail of `vs.varying` (`FauWord::Constant(0x3f4c_cccd)`, `0x3e4c_cccd`,
   `0xbf8c_cccd` by word).
3. The gap table of `src/shaderc/online/mali/tests.rs:1130-1148`: the entries that name
   `MaliError::Arithmetic` for a vertex `Flt`/`Fmax` are moved or replaced by
   the shapes still refused (a plain two-operand `max`, `any` of a four-lane
   compare, `all` over three lanes); `tools/mali/corpus-verdicts.txt` lines
   for `es100/anyall.vert`, `gltest/fsat_vs_ctrl.vert` and `fsat_vs_folded.vert`
   become `ok` (regenerated by the corpus test, in the same commit: the gate
   compares the bytes).
4. A rule test for the two reductions: `any(greaterThan(n, vec3(0.5)))` with
   the lanes read in another order is not asserted (no reference); it must
   *refuse*, not produce a different chain.

The regression set only moves forward: cases 86 and 182 pass the sweep once the
draw is correct; `tools/mali/sweep_passsets.py --delta tools/cts/log` is the
board item's check, not this one's. Case 182 is flat-only, no frame-side change.
