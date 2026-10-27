<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mesa and chains of three or more additions / multiplications (do_rebalance_tree)

Valhall online compiler (`src/shaderc/online/mali`) refuses
`MaliError::Rebalance` ("three or more additions or multiplications in a
chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered
yet", `nir.rs` `refuse_rebalanced`) for `tools/shaders/es100/matattr3.vert`,
`es100/matattr4.vert` and `gltest/mulone_fs_mods.frag`. This note records what
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
   So rebalance precedes GLSL's algebraic simplification, and the tree it sees
   still holds `x * 1.0` and every constant.
2. `do_rebalance_tree` (`src/compiler/glsl/opt_rebalance_tree.cpp`):
   `ir_rebalance_visitor::handle_rvalue` -> `handle_expression` ->
   `visit_tree(is_reduction)`; if a reduction with `num_expr > 2`, the tree is
   turned into a vine (`tree_to_vine`) and back (`vine_to_tree`, DSW). It
   skips: an assignment to an `invariant`/`precise` variable
   (`visit_enter(ir_assignment)`); a tree with **two or more constants**; a
   tree containing an `ir_dereference_array`/`_record` (`foo[a+b]+c`); a
   matrix-typed operand; mixed expression types or operations; and operations
   other than add, mul, and/or/xor, logic and/or/xor, min, max
   (`is_reduction_operation`). Swizzles are not expressions (`as_expression()`
   is null), so they count towards nothing; `num_expr` counts only
   `ir_expression` nodes.
3. `do_algebraic` (`opt_algebraic.cpp`): `x * 1.0 -> x`, and
   `reassociate_constant` (l.175: moves a constant down a same-operation tree
   only when a sub-expression has a constant operand itself).
4. After `glsl_to_nir`, `nir_opt_algebraic` (`src/compiler/nir/
   nir_opt_algebraic.py`): "Propagate constants up multiplication chains"
   (l.2394: `fmul(fmul(a, b) [used once, a and b not const], #c) ->
   fmul(fmul(a, c), b)`, also l.2406 and l.3609) and the late `ffma` fusion
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
have fewer than three expressions. `do_algebraic` removes the `* 1.0`s; the
remaining `(u_b * u_q) * 8.0` is left (`reassociate_constant` finds no constant
operand inside). NIR's l.2394 rule then moves the constant inward. Final NIR
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
addend: the order is `(8 * u_q) * u_b`, not the source's. Which FAU word is
which uniform is the JSON's `fau` list (u2/u3 are uniform pairs of this
program); it was not decoded further here.

## What nir.rs would have to do

`refuse_rebalanced` flags any chain of three `Fadd`/`Fmul` nodes whose links
are single-use and equal-width. The three programs show it is both too
broad and, as applied, unnecessary here:

1. **Ignore trees GLSL would not rebalance.** The IR has lost which values
   came from `ir_dereference_array`, from two constants, or from a
   single-component swizzle pair. For matattr3/4 the chain is built from
   attribute-column loads and is in the order the program was written;
   emitting it in source order (`fadd(fadd(fadd(a, b), c), d)` with
   Mesa's operand order `late-loaded, earlier-loaded`) is byte-equal. A rule
   "refuse only when the source tree is a pure reduction of >2 expression nodes
   with no array deref and at most one constant" needs the front end (lower.rs)
   to record, per Fadd/Fmul, that its operands were written as a tree
   (`num_expr`), whether an operand came from an array deref
   (matrix column of an attribute), and the count of constants in the tree
   before `x * 1.0` folding.
2. **Constant motion.** For a `fmul(fmul(a, b), #c)` with one use,
   NIR gives `fmul(fmul(a', c), b')`, where `(a', b')` are the pair in
   NIR's canonical operand order (here swizzle order: u_q before u_b). The
   builder must apply this before building FMAs, and fold `x * 1.0` first.
3. **Do the real rebalance where it applies.** The DSW pair
   (`tree_to_vine` / `vine_to_tree`) is small; sel_u_rebalance (the existing
   reference, `tests.rs` `a_chain_of_additions_is_mesa_s_rebalanced_tree`)
   is the case where it does apply. Implement it on the (add or mul) node
   tree before NIR lowering, with the rejection list of step 2 of the
   previous section; then the refusal can go.
4. Everything else (viewport `ffma`, f16 packing, FAU packing of constants)
   is already what the existing alu_* references cover.

Caveat: the NIR text above is the final NIR of the compile, so which pass made
each step is inferred from the passes' source and the NIR, not traced rule by
rule (no NIR dump between passes was taken).
