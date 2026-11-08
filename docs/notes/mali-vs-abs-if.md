<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex `abs`/`floor` and the if-converted vertex `if` (cases 181 and 188)

**Status: design, implementation in vs-abs-if** (plan #336, issue #33). This
note changes no code. It maps gl_test case 181 `fusemad` (the control half,
`fma_vs_ctrl.vert`) and case 188 `predif` (`predif_vs_uni.vert`) to the Mesa
26.2.1 / drm-shim references, names where in `src/shaderc/online/mali` each
instruction is made, and the tests to add. Both are the "ranked items 3 and 5"
of `notes/mali-vs-alu-ops.md` (vs-abs-floor, vs-if-convert); item 4, the
uniform-only flat varying, is already in the tree (section 4).

Locations are at `17e91019` (`nir.rs` etc. are in `src/shaderc/online/mali/`).

## 1. The references (all present)

| Case | Vertex shader | Fragment partner | Reference | Older twin |
|---|---|---|---|---|
| 181 fusemad (control) | `tools/shaders/gltest/fma_vs_ctrl.vert` | `gltest/fma_fs_varying.frag` | `tools/mali/testdata/vsarith_181_fmactrl.{vert,frag,expect.json}` | none |
| 188 predif | `tools/shaders/gltest/predif_vs_uni.vert` | `gltest/fma_fs_varying.frag` | `vsarith_188_predif.*` | `alu_predif.*` (same words) |

The fused half of case 181 (`gltest/fma_vs_fused.vert`, `varyconst_181_fusemad.*`)
already compiles byte-equal (`VARYCONST_POPULATION` "181", `tests.rs`); gl_test's
case 181 draws both halves, so the control half is what keeps the case refused.
`vs.position` is the plain position shader in both references. Missing: nothing
for these two. Not covered (and out of this item): the `if` over `gl_PointSize`
(`vsarith_psiz_if`, reference present, no sweep case) and the conditionally
written varying lane (`vsarith_partial_varying`); they share the `if` machinery
of section 3 but are the later item `vs-frsq / psiz`, and must stay refused here.

## 2. Case 181 (control): `abs`, `floor`, the shared product

`fma_vs_ctrl.vert`: `p = u_a*u_b; v = (u_x - p, p*2.0, u_x - abs(u_a*u_b),
floor(u_a*u_b*4.0) + 0.25)`. All four lanes read uniforms alone: flat. The
varying program of `vsarith_181_fmactrl`:

```
MOV.i32 r0, u6.w0                               ; the other uniform, moved (FAU slot rule)
FMA.f32 r0, u5.w1, r0^, 0x0.neg                 ; p = u_a*u_b: ONE node, a product is an FMA with -0.0
FADD.f32 r1, u6.w1, r0.neg                      ; u_x - p        (not fused: p has other users)
FMA.f32.wait0 r1, r1^, 0x0, r1^                 ; the exact ffma(x, 0.0, x) of a flat lane      ; STORE
FADD.f32 r1, r0, r0                             ; p*2.0 = p + p  (late_rules)                   ; FMA ..0x0.. ; STORE
FADD.f32 r1, u6.w1, r0.neg.abs                  ; u_x - abs(p): `.neg.abs` on the source        ; FMA ..0x0.. ; STORE
FMA.f32 r0, r0^, 0x44000000.h1, 0x0.neg         ; p*4.0 (the half 4.0)
FROUND.f32.rtn r0, r0^                          ; floor
FADD_IMM.f32 r0, r0^, #0x3E800000               ; + 0.25                                        ; FMA ..0x0.. ; STORE.end
```

* the product `u_a*u_b` is computed once although the source writes it three
  times: NIR's CSE (here `Nir::alu`'s eager CSE, `nir.rs:1802`) makes it one
  node. It has four users (`fneg`, `fmul(.., 2.0)`, `fabs`, `fmul(.., 4.0)`),
  so `fuse` (`nir.rs:2916`) does **not** fuse it: both of its rules require
  `only_used_by_fadd` (`nir.rs:3000`) of the product *and* of the `fneg` in
  between. The fused half (`fma_vs_fused`) has single-use products and fuses.
  This rule needs no new code; it is the reason the test must run through the
  whole `build`.
* `abs` is a source modifier of its consumer (`.abs`, and `.neg.abs` for
  `-abs(p)`): `Alu::Fabs` becomes a `FABSNEG.f32 a.abs` row (`emit.rs:57`,
  `:230`) that `mod_prop_forward` (`opt.rs:176`, `compose` `:156`) folds into the
  `FADD`'s source. `Alu::Fabs` constant folding, `fabs(fneg a)` and `fabs(fabs a)`
  are in `Nir::alu_sized` already (`nir.rs:1864-1869`, `:2010-2016`).
* `floor` is `FROUND.f32.rtn` (`Alu::Ffloor`, row `emit.rs:60`, `:247`); the
  `+ 0.25` is `FADD_IMM` by `fuse_add_imm` (`opt.rs:404`).
* `p*2.0` is `fadd(p, p)` by `late_rules` (`nir.rs:2792`), after `fuse`.

**What is missing, then, is only the walk's arms** (`build`'s vertex
`IrOp::Alu` match, arm list `nir.rs:2317-2471`, refusal at `:2470`):

| IR op | Arm to add | Node |
|---|---|---|
| `Fabs` | as `nir_fs.rs:2016-2017` (the fragment also drops a `fabs` of a value known not negative, `not_negative`, `nir_fs.rs:1875`; the vertex has no reference for it: keep the plain arm) | `vs.alu(Alu::Fabs, x(0), Undef, Undef)` |
| `Ffloor` | as `nir_fs.rs:2048` | `vs.alu(Alu::Ffloor, ..)` |

The order of nodes `Fmul(p, 4.0)` then `Ffloor` then `Fadd(.., 0.25)` is the walk's
instruction order; the schedule (`sched.rs`, `opt_move`) places the stores. The
byte-equal test settles it.

## 3. Case 188: the if-converted vertex `if`

`predif_vs_uni.vert`: `gl_Position = a_pos; if (u_x > 0.0) v = (u_a*2.0, u_q+0.5,
0.25, 1.0); else v = (u_q, u_a*0.5, 0.75, 1.0);`. Mesa's program
(`vsarith_188_predif`, `vs.varying`), no branch:

```
FADD.f32 r0, u6.w1, u6.w1                      ; then.x = a*2.0 (= a + a)
FADD_IMM.f32 r1, u6.w0, #0x3F000000            ; then.y = q + 0.5
MOV.i32 r2, 0x0                                ; (the else arm's a*0.5, as an FMA with -0.0 ...)
FMA.f32 r2, u6.w1, 0x38000000.h1, r2^.neg      ; ... else.y = a*0.5
FCMP_OR.f32.lt.m1 r3, 0x0, u5.w1, 0x0          ; c = 0.0 < u_x  (a 32-bit mask)
CSEL.u32.eq r0, r3, 0x0, u6.w0, r0^            ; x = c ? a*2.0 : q
CSEL.u32.eq r1, r3, 0x0, r2^, r1^              ; y = c ? q+0.5 : a*0.5
FMA.f32 r0, r0^, 0x0, r0^  ;  FMA.f32 r1, r1^, 0x0, r1^     ; the exact ffma(x,0,x) of the flat lanes
MOV.i32 r2, u7.w1 ; CSEL.u32.eq r2, r3^, 0x0, u7.w0, r2^   ; z = c ? 0.25 : 0.75 (two FAU words)
...four STOREs...; IADD_IMM.i32 r2, 0x0, #0x3F800000 ; STORE   ; w = 1.0 in both arms: stored once
```

(`FMA.f32 r2, u6.w1, 0x38000000.h1, r2^.neg` is the product `a*0.5` as an `FMA`
whose addend is `-(+0.0)` read from a register: the instruction already names
the FAU word `u6.w1`, so the zero cannot be a second one. The existing
constant lowering (`fau.rs`) does this for the fragment stage and for
`varyconst_181_fusemad`'s products; no new rule.)

How NIR gets here: the GL front end's `nir_opt_peephole_select` (limit 8, uniform
loads allowed: `nir::PEEPHOLE_LIMIT_GL`, `nir.rs:82`) flattens an `if` whose two
arms hold at most 8 ALU operations, no look-up, no jump; each lane the arms leave
different becomes `bcsel(c, then, else)`; bifrost's second call (limit 64,
`PEEPHOLE_LIMIT`) finds nothing more. `bcsel_pan` is a `MUX.i32.int_zero` that
`lower_isel` rewrites as `CSEL.u32.eq` (`opt.rs:379`; row `emit.rs:267`:
`Alu::Bcsel` -> `MuxI32 [else, then, c]`). Both arms run unconditionally, so
their operations are in the program in source order (then-arm, else-arm) and the
compare is a node of its own before the selects.

The vertex walk refuses every `IrNode::If`: `build`'s node loop returns
`MaliError::ControlFlow` for `IrNode::If` (`nir.rs:2214`, `_ => return Err(..)`),
and the compare `Flt` for `Arithmetic` before that. The fragment walk has the whole
machinery (`walk_if`, `nir_fs.rs:2799`: each side walked in its own region, the
locals and the colour restored between them, a `bcsel` per lane left different
at the join; the `IfRegion`/`peephole_select` pair of `nir.rs:471`, `:1363` is the
block-keeping variant). For the vertex stage the design is the flattened one:

1. **Restructure `build`** so its per-instruction match is callable on a node list
   (a `VsWalk { vs, w, position, point_size, varyings }` and
   `fn walk_nodes(&mut VsWalk, nodes: &[IrNode])`; the top-level loop becomes one
   call). This is the largest edit of the item: the match spans
   `nir.rs:2205-2560` and closes over those five variables.
2. **`IrNode::If { cond, then_, else_ }`** (fields as `nir.rs:915`): take the
   condition's lane 0 (the `Flt` mask node); require both arms to be
   straight-line instruction lists (no `If`, `Loop`, `Break`, `Return`) holding at
   most `PEEPHOLE_LIMIT_GL` ALU instructions each and no `IrOp::Tex`
   (the peephole takes no `tex`, `nir_fs.rs:2815`); else `ControlFlow` as now.
   Snapshot `w.locals`, `position`, `point_size`, `varyings`; walk `then_`; keep
   the result and restore the snapshot; walk `else_`; at the join, for every local
   and every output lane the arms left different, `vs.alu(Alu::Bcsel, cond, t,
   e)`. A lane equal in both (`t == e`) stays `t`, a lane written by one arm only
   takes the other's pre-`if` value (an unwritten varying lane is `Val::Undef`:
   a `Bcsel` with `Undef` is `VaryingForm`, as an unwritten lane is today).
   `AluOp::Bcsel` itself already exists (`nir.rs:2374-2377`; `bcsel(c, 1.0,
   0.0)` is `B2f`, `bcsel(a, b, b)` is `b`).
3. **`AluOp::Flt` (and `Fle`, `Feq`)** in the vertex match: `Alu::Fcmp(Lt)` with
   the sources as the IR has them (`0.0 < u_x`, `nir_fs.rs:2065`, row `emit.rs:252`
   `FCMP_OR.f32.lt.m1 r, 0x0, u5.w1, 0x0`).
4. **Which node order Mesa has for the compare** (before or after the arms'
   arithmetic) decides the register numbers `r0..r3` in the listing: the
   reference puts `FCMP` after both arms' ops. Walk the condition first (it is
   an earlier IR instruction) and let `opt_move` (`nir.rs:3129`) and the
   scheduler place it; if the bytes then differ, the order is
   `sched.rs`/`opt_move`'s to reproduce, not the walk's.

**The 1.0 lane is the one point to decide.** Both arms store `w = 1.0`; after
step 2 it is `t == e == Val::Const(1.0)`. The varying layout (`nir.rs:2615-2660`)
treats a `Val::Const` lane as one Mesa's link propagates into the fragment shader
(and `mod.rs`'s `vs_constants` / `compile_fs_for_vs` would drop it from the
varying and from the fragment's loads), or, if the vertex shader is compiled
alone, as `VaryingNotFromAttribute`. The reference *stores* it
(`IADD_IMM.i32 r2, 0x0, #0x3F800000`, then a `STORE`), and the fragment loads
all four lanes flat (`LD_VAR_BUF_IMM.f32.v4.src_flat32`). Mesa's link does not
see a constant: at link time `v.w` is a `phi` of two constants of a branch (the
`nir_opt_peephole_select` that merges them runs after `nir_opt_varyings`).
So the join's result for a lane equal in both arms must not be `Val::Const`
to the layout code: either keep the join as a node (`Bcsel` with equal arms is a
no-op and `alu` folds it, so the walk must push an opaque node: `Alu::Mov` exists
but is today the 16-bit one-lane copy, so it is a new node kind or a widened
`Mov`, its emit being the `IADD_IMM.i32 r, 0x0, #bits` of the reference), or record "constant by `if`" for the lane so
`vs_constants` and the layout leave it a stored flat lane. This is a decision the
implementer takes with the reference in hand; either way `vs_constants` and the
vertex build must agree (they are called with the same walk).

**Flat promotion** needs nothing new: each lane is `Kind::Alu` over `Kind::Uniform`
and constants only, so `uniform_only` (`nir.rs:1691`) is true, `flat` is set, the
`ffma(x, 0.0, x)` conversion (`nir.rs:2712-2730`) is added to the `Bcsel`
lanes, and `link.rs` (`VaryingConsts::push_flat`, `:174`; `flat()`, `:181`) gives the
fragment its `src_flat32` load. The flat and smooth rule (`flat && smooth` is
refused) holds: every lane is flat.

## 4. The uniform-only flat varying: already landed

`notes/mali-vs-alu-ops.md` lines 138-144 and its item 4 describe the flat
varying as still missing ("the next refusal is `VaryingNotFromAttribute`"). It is
in the tree: `Nir::uniform_only` (`nir.rs:1691`) and the `flat` flag of the
varying loop (`nir.rs:2640-2655`), the exact `ffma(x, 0.0, x)` store
(`nir.rs:2712-2730`), `VaryingConsts::push_flat` (`link.rs:174`), measured by
`varyconst_117`/`150`/`181`/`191`/`199` in `VARYCONST_POPULATION`. Cases 181
(control) and 188 therefore finish with the ops of sections 2 and 3: the
refusals named in `notes/mali-vs-alu-ops.md` (`Fabs`, `Ffloor`, `Flt`,
`ControlFlow`) are the whole list, in that order.

## 5. Tests to add (`src/shaderc/online/mali/tests.rs`)

1. `a_vertex_abs_and_floor_over_a_shared_product_are_byte_equal_to_mesa_26_2_1`:
   `vsarith_181_fmactrl` through `check_linked` (both programs, the flat
   fragment `LD_VAR_BUF_IMM.v4.src_flat32`), plus an assertion that no `FMA`
   with a fused addend appears for `u_x - p` (the shared-product rule), and the
   pair in `vs_reference_sources()` (`tests.rs:2094`).
2. `a_vertex_if_on_a_uniform_is_if_converted_byte_equal_to_mesa_26_2_1`:
   `vsarith_188_predif` the same way, with the flat `w = 1.0` lane asserted
   stored (`IADD_IMM.i32 .. #0x3F800000`) and the FAU tail words
   (`0x3e800000`... as the `fau` array of the reference holds them).
3. Gap table (`tests.rs:1130-1148` shape): a vertex `if` that holds a look-up, a
   loop or more than eight ALU operations in an arm stays `ControlFlow`; an `if`
   with only one arm writing a varying lane stays refused (`partial_varying`
   needs no instruction, a different rule); `psiz_if` stays refused until its
   own item. `tools/mali/corpus-verdicts.txt` moves `gltest/fma_vs_ctrl.vert` and
   `gltest/predif_vs_uni.vert` to `ok`, in the same commit (the gate compares
   the bytes). `es100/psiz_if.vert` and `es100/partial_varying.vert` keep their
   lines.
4. No frame-side change: the draw of both is a position shader, a varying
   shader and a flat fragment, all existing. The sweep's delta
   (`tools/mali/sweep_passsets.py --delta tools/cts/log`) is the board item's.
