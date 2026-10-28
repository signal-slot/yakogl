<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mesa and chains of three or more additions / multiplications (do_rebalance_tree)

Valhall online compiler (`src/shaderc/online/mali`) refused
`MaliError::Rebalance` ("three or more additions or multiplications in a
chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered
yet", `nir.rs` `refuse_rebalanced`) for `tools/shaders/es100/matattr3.vert`,
`es100/matattr4.vert` and `gltest/mulone_fs_mods.frag`; since plan #154's
chains item all three are byte-equal (the last section). This note records what
Mesa 26.2.1 (panfrost, drm-shim, GPU_ID 0xa867) emits for them. **Finding: in
all three, `do_rebalance_tree` does not rebalance anything.** The refusal is
over-eager; the chains are lowered in the source's order (matattr3/4) or by
NIR's constant-propagation rule (mulone_fs_mods).

## References

`tools/mali/testdata/chain_<name>.{vert,frag,expect.json}`. Per case (shim =
`$CONNECTED_DEPS_DIR/mesa-shim`, a `drm_shim_ref.sh build` of Mesa 26.2.1;
64x64, one draw):

    tools/mali/drm_shim_ref.sh run <shim> V F target/chain/<name>
    python3 -I tools/mali/pan_ref_diff.py extract target/chain/<name>.trace --draw \
        > tools/mali/testdata/chain_<name>.expect.json

| name | V | F |
|---|---|---|
| matattr3 | `es100/matattr3.vert` | `gl_test_es100/interp_color.frag` |
| matattr4 | `es100/matattr4.vert` | `gl_test_es100/interp_color.frag` |
| mulone_fs_mods | `gl_test_es100/passthrough.vert` | `gltest/mulone_fs_mods.frag` |

The NIR quoted below is the final NIR in `BIFROST_MESA_DEBUG=shaders` output
(`target/chain/<name>.stderr`, not kept).

## Order of the reassociating passes

1. `do_common_optimization` (`src/compiler/glsl/glsl_parser_extras.cpp`,
   ~l.2596-2606) runs, in this order: `propagate_invariance`,
   `do_if_simplification`, `opt_flatten_nested_if_blocks`,
   `do_dead_code_unlinked`, `do_tree_grafting`, `do_minmax_prune`,
   **`do_rebalance_tree`**, **`do_algebraic`**, `do_lower_jumps`.
   (`glsl_parser_extras.cpp:2604-2605`.) So rebalance precedes GLSL's
   algebraic pass, and the tree it sees still holds `x * 1.0` and every
   constant.
2. `do_rebalance_tree` (`src/compiler/glsl/opt_rebalance_tree.cpp`):
   `ir_rebalance_visitor::handle_rvalue` -> `handle_expression` ->
   `visit_tree(is_reduction)`; if a reduction with `num_expr > 2`, the tree is
   turned into a vine (`tree_to_vine`) and back (`vine_to_tree`, DSW). It
   skips: an assignment to an `invariant`/`precise` variable
   (`visit_enter(ir_assignment)`); a tree with **two or more constants**
   (`is_reduction`, `opt_rebalance_tree.cpp:209-216`); a
   tree containing an `ir_dereference_array`/`_record` (`foo[a+b]+c`); a
   matrix-typed operand; mixed expression types or operations; and operations
   other than add, mul, and/or/xor, logic and/or/xor, min, max
   (`is_reduction_operation`). Swizzles are not expressions (`as_expression()`
   is null), so they count towards nothing; `num_expr` counts only
   `ir_expression` nodes.
3. `do_algebraic` (`opt_algebraic.cpp`) has **no `x * 1.0` rule**: its
   `ir_binop_mul` case (l.276-284) only calls `reassociate_constant`, and
   `is_one()` appears only in the min/max saturate rules (l.322-352). Its one
   job here is `reassociate_constant` (l.175-216, with `reassociate_operands`,
   l.148-166): for `op(const, tree)` of the same operation (add or mul, no
   matrices) it walks down the tree to the first sub-expression that has a
   constant operand and **swaps** the outer constant with that sub-expression's
   other operand (l.196-201: the outer constant takes the place of the sub-expression's
   non-constant operand, which goes to the outer expression), so the two
   constants meet and constant folding joins them. It does not remove the `* 1.0`; it moves the other operand out.
4. `x * 1.0` is removed in NIR, not in GLSL. After `glsl_to_nir`,
   `nir_opt_algebraic` (`src/compiler/nir/nir_opt_algebraic.py`) has
   `fmul(a, 1.0) -> fcanonicalize(a)` (l.292; `fmul(a, -1.0) -> fneg` is l.296),
   and `fcanonicalize(a@32)` is `a` unless the shader flushes fp32 denormals
   (l.162; also l.167, l.171, the rules in `optimize_fcanonicalize`,
   l.154-172, added at l.600). So the `* 1.0` goes away in two steps, and with
   a denormal-flushing mode it would stay as a real `fmul a, 1.0` (late
   lowering of `fcanonicalize`, l.4094). Then: "Propagate constants up
   multiplication chains" (l.2393-2397: `fmul(fmul(a, b) [used once, a and b
   not const], #c) -> fmul(fmul(a, c), b)`), "Reassociate constants in add/mul
   chains" (l.2406: `fmul(#a, fmul(b, #c)) -> fmul(fmul(a, c), b)`) and, in
   `before_ffma_optimizations`, the opposite move "Propagate constants down
   multiplication chains" (l.3608-3609), then the late `ffma` fusion
   (`fadd(fmul(a, b), c) -> ffma`); `reassoc_fma_optimizations` (l.4353, run by
   `nir_opt_reassociate_for_fma`, l.4446) only touches `fadd` trees fed by
   `fmul`s. The panfrost backend adds none of its own reassociation.

## matattr3 (6-term float chain, mat3 attribute)

`off = a_xf[0][1] + a_xf[1][1] + a_xf[2][1] + a_xf[0][2] + a_xf[1][2] +
a_xf[2][2]`: five adds, but every leaf is an `ir_dereference_array`, so
`is_reduction` rejects the tree (step 2). Mesa keeps the source's
left-to-right order. NIR: `fadd c1.y c0.y`; `+ c2.y`; `+ c0.z`; `+ c1.z`;
`+ c2.z` (c0..c2 = attribute locations 1..3 = `index:0x1..0x3`); then
`off + a_pos.x`, `off + a_pos.y`, `ffma(.., scale, bias)` for the viewport.
Position shader (`vs.position`, 29 words, 32 work regs, preload
`0x3800000000000000`, FAU `42000000 x4 3f000000 4 8 0xc`):

    LD_ATTR_IMM.v3.f32 @r0:r1:r2 (index 2)      ; column 1
    LD_ATTR_IMM.v3.f32 @r4:r5:r6 (index 1)      ; column 0
    FADD.f32 r0, r5, r1                         ; c0.y + c1.y
    LD_ATTR_IMM.v3.f32 @r8:r9:r10 (index 3)     ; column 2
    FADD.f32 r0, r0, r9                         ; + c2.y
    FADD.f32 r0, r0, r6                         ; + c0.z
    FADD.f32 r0, r0, r2                         ; + c1.z
    FADD.f32 r0, r0, r10                        ; + c2.z
    LD_ATTR_IMM.v2.f32 @r2:r3 (a_pos, index 0)
    FADD.f32 r1, r2, r0 ; FADD.f32 r0, r3, r0
    FMA.f32 r1, r1, u0.w0, u0.w1 ; FMA.f32 r0, r0, u1.w0, u1.w1
    LEA_BUF_IMM / STORE.istream x3 (r1, r0, then z = r.. , w = 1.0f)

Words: see `chain_matattr3.expect.json` (`vs.position`, `vs.varying`,
`fs.fragment`). The varying shader is three `LD_ATTR_IMM.v3` plus estream
stores; the fragment shader is the interpolated colour with `ATEST`, `BLEND`.

## matattr4 (vec4 chain, mat4 attribute)

`rest = a_xf[0] + a_xf[1] + a_xf[2] + a_xf[3]` (derefs: no rebalance) and
`rest.y + rest.z + rest.w` (two `ir_expression`s over swizzles, `num_expr`
= 2, not > 2: no rebalance). NIR keeps `((c1 + c0) + c2) + c3` per lane
(`%3..%13`) then `fadd %11, %12` and `fadd %14, %13` for the second chain.
`vs.position` (36 words): `LD_ATTR_IMM.v4` (index 2), `.v4` (index 1);
three `FADD.f32 rN, r(4+N)^, r(0+N)^` (y, z, w lanes: columns 0 + 1); `.v4`
index 3 and three `FADD`; `.v4` index 4 and three `FADD`; then
`FADD r0, r0, r1` and `FADD r0, r0, r2` (the second chain, source order);
`LD_ATTR_IMM.v2` a_pos; `FADD`, `FADD`, two `FMA` viewport; stores. Exact
words in `chain_matattr4.expect.json`. In the second chain the loads' register
reuse (`r4:r5:r6:r7` three times, with `wait2`/`wait0` flags) is the
allocator's, not an effect of the order.

## mulone_fs_mods (multiplications and a constant 1.0)

    gl_FragColor = vec4(abs(u_x - 2.0 * u_a) * 1.0, 1.0 * -(u_q - u_a),
                        (u_b * 1.0) * u_q * 8.0, 1.0);

The third lane `((u_b * 1.0) * u_q) * 8.0` has two constants (1.0, 8.0), so
`is_reduction` rejects it (step 2, `contains_constant`); the first two lanes
have fewer than three expressions. Then `do_algebraic` does **not** drop the
`* 1.0`. With `ir1` = the outer `* 8.0` (constant operand 1),
`handle_expression` (l.276-284) calls `reassociate_constant(ir1, 1, 8.0,
(u_b * 1.0) * u_q)`. That sub-tree has no constant operand, so the function
recurses into its operand 0 (l.204), `u_b * 1.0`, whose operand 1 is the
constant: `reassociate_operands(ir1, 1, ir2, 0)` (l.200) swaps `8.0` and `u_b`.
The lane is now `((8.0 * 1.0) * u_q) * u_b`; constant folding makes it
`(8.0 * u_q) * u_b`. This, not NIR l.2394, is where the order `(8 * q) * b`
comes from: the outer operand `u_b` is never a constant and the inner product
already has 8.0 in it, so l.2394 and l.2406 have nothing to match. (Lane 1
`abs(..) * 1.0` and lane 2 `1.0 * -(..)` meet no `reassociate_constant` case,
as the other operand is not a multiplication; their `* 1.0` goes in NIR, l.292
then l.162.) Final NIR
(uniforms in one `load_ubo` vec4, x=u_x y=u_q z=u_a w=u_b):

    %3  = ffma -2.0, %2.z, %2.x           ; u_x - 2*u_a
    %4  = fabs %3
    %5  = fneg %2.z ; %6 = fadd %2.y, %5 ; %7 = fneg %6    ; -(u_q - u_a)
    %9  = fmul 8.0, %2.y                  ; 8 * u_q
    %10 = fmul %9, %2.w                   ; * u_b
    f2f16 each; vec4(%11, %12, %13, 1.0); atest; blend

Not `(u_b * u_q) * 8` and not the source's order. `fs.fragment` (16 words,
`work_reg_count` 32, preload `0x3000000000000000`; FAU lanes
`3c003c00` etc. in the JSON):

    MOV.i32 r0, u2.w1 ; FADD.f32 r0, u3.w0, r0.neg
    MOV.i32 r1, u2.w0 ; FMA.f32 r1, 0x40000000.neg, u2.w1, r1
    FADD.f32 r0.h0, r0.neg, 0x0.neg ; FADD.f32 r1.h0, r1.abs, 0x0.neg
    MKVEC.v2i16 r0, r1.h0, r0.h0
    MOV.i32 r1, 0x0
    FMA.f32 r1, 0x48000000.h1, u3.w0, r1.neg
    FMA.f32 r1, r1, u3.w1, 0x0.neg
    FADD.f32 r1.h0, r1, 0x0.neg ; MKVEC.v2i16 r1, r1.h0, u4.w0.h0
    ATEST ; BLEND.slot0.v4.f16.end ; IADD_IMM r48 ; BRANCHZI

The exact words are in `chain_mulone_fs_mods.expect.json` (`fs.fragment`).
The product `8.0 * u_q` is one `FMA` (8.0 as the shifted immediate, the
addend a zero register with `.neg`), then `* u_b` another `FMA` with a `-0`
addend: the order is `(8 * u_q) * u_b`, not the source's. The words show both
points of this section. There is no multiply by 1.0 anywhere in the 16 words:
lane 0 is `FMA -2.0 * u_a + u_x` then only the `.abs` source of the f16
conversion `FADD`, lane 1 is the negated `FADD`, lane 2 has exactly the two `FMA`s. And the first `FMA` has `8.0` first
and `u3.w0` (= u_q: the `FADD r0, u3.w0, ..` of lane 1 is `u_q - u_a`) second,
the second has `u3.w1` (= u_b) last, which is the result of the
`reassociate_constant` swap. `tools/mali/testdata/algfold_189_mods.*` is the
same shader from the algebraic-folds study. Which FAU word is
which uniform is the JSON's `fau` list (u2/u3 are uniform pairs of this
program); it was not decoded further here.

## The lowering (`glsl.rs`, plan #154 chains)

`MaliError::Rebalance` used to be raised by `nir.rs` `refuse_rebalanced` for
any chain of three single-use `Fadd`/`Fmul` links. It is gone; the two GLSL
passes are now done on the front end's IR before either builder runs
(`src/shaderc/online/mali/glsl.rs`, called from `mod.rs`
`compile_module_on` and `vs_constants`), in Mesa's order (one
`do_common_optimization` at compile time, glsl_parser_extras.cpp:2291:
rebalance, then algebraic):

1. **What the IR keeps.** A local, a uniform and an input are loads at each
   read, so GLSL's variable dereferences are visible: a leaf of a tree is
   a load or a constant, an expression is an ALU value whose one use is an
   ALU operand. A matrix input or uniform, an array element (`LoadLocal`
   with an index, `LoadUniformIdx`, a uniform array) or a struct member is
   an array/record dereference: no reduction (matattr3/4's columns).
2. **Which rvalue is a candidate.** The rebalance visitor does not handle a
   whole assignment's right-hand side (`visit_enter(ir_assignment)`,
   opt_rebalance_tree.cpp:151-163), but its operands, and every other
   rvalue. A constructor's lane is `vec_ctor.x = swizzle(arg)`
   (ast_function.cpp:1619-1632): the swizzle's operand is a candidate. In
   the IR a whole right-hand side is a value whose one use is a
   `StoreLocal` of every lane of the local (or a `StoreOutput` of every
   lane of the output) with an identity swizzle; a constructor's
   `StoreLocal` writes some lanes only. So matattr3's `float off = ...` (5
   adds) is not rebalanced (and its 4-add operand reads matrix columns),
   sel_u_rebalance's `vec4(a_pos.x + u0.x + u1.x + u2.x, ..)` is.
3. **The rebalance.** `is_reduction` (:201-257) over the tree: one
   operation (`fadd` or `fmul`; `a - b` is the front end's `fadd(a,
   fneg(b))`, GLSL's `ir_binop_sub`, no reduction), one width, at most one
   constant, no dereference that ends it, more than two expressions; then
   `tree_to_vine` / `vine_to_tree` (:70-123) transcribed, and the visitor
   goes on into the new tree's operands. The result is written over the
   tree's instructions in post-order (the order glsl_to_nir emits), each
   leaf load where it is first read; the tree's value keeps the root's id.
   dep4's `(x + y + z + w) * 0.25` becomes `(x + y) + (z + w)`
   (`intbig_case184_dep4.expect.json`: `FADD r1, r4^, r1^`, `FADD r0, r2^,
   r0^`, `FADD r0, r1^, r0^`), sel_u_rebalance's lane `(a_pos.x + u0.x) +
   (u1.x + u2.x)`.
4. **The constant swap.** `do_algebraic`'s `reassociate_constant`
   (opt_algebraic.cpp:175-216, `reassociate_operands` :148-166), bottom-up
   over every `fadd`/`fmul`, on the tree as written: the outer constant
   swaps with the operand beside the first constant found operand 0 first,
   and the two constants fold (NIR's `nir_opt_constant_folding`; GLSL
   26.2.1 has no constant folding of its own). `((u_b * 1.0) * u_q) * 8.0`
   becomes `((8.0 * 1.0) * u_q) * u_b`, then `(8.0 * u_q) * u_b`; the `1.0`
   is what the `8.0` meets, so the swap must come before `x * 1.0` is
   dropped, which stays NIR's (`Nir::alu`'s identity rule, l.292/l.162).
   `2.0 * (u_q * 3.0)` is `u_q * 6.0` the same way (algfold_constmul).
5. **What stays refused, by name.** A tree that would be rebalanced but
   reads a local `do_tree_grafting` may graft (assigned once, whole, and
   read once: opt_tree_grafting.cpp:389-401), a multi-use value or a
   texture result as a leaf, or a swizzled sub-expression, is
   `Rebalance` (the IR does not say what GLSL had there); only trees the
   old refusal also refused (three or more links), so no verdict moves
   back. A constant moved across a change of width (`vec2(3.0) * (q *
   0.75)`) is `ArithConstant`. An assignment to an `invariant` output is
   skipped by both passes, as in Mesa.

References pinned (`tests.rs`): `the_chain_population_is_byte_equal_to_mesa_26_2_1`
(chain_matattr3/4, varyconst_63_matattr, chain_mulone_fs_mods,
chain_min_mulone, chain_min_reassoc, algfold_189_mods, the algfold folds,
case 192's four, sel_u_rebalance), `case_184_programs_are_byte_equal_to_mesa_26_2_1`
(dep1/2/4), and the IR shapes in `glsl_rebalances_the_trees_it_takes` and
`glsl_reassociates_a_constant_into_the_product_beside_another`. The two
minimal references settle the caveat this note had: `chain_min_mulone`
(`u_x * 1.0`, drawn without `--uniform-values`) keeps no multiply, so
`x * 1.0` goes in NIR; `chain_min_reassoc` (`8.0 * u_q * u_b`) is the
source's order, so `(8 * q) * b` in mods is the swap, not NIR l.2394.

Not covered: the other reduction operations (`min`, `max`, the integer
and boolean ones) are neither rebalanced nor refused, as before; no
reference draws a chain of them.
