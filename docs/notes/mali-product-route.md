<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the product route, and the varyings the link removes

Issue #6509. The rule of the Mali compiler: a program's words equal Mesa
26.2.1's, or the program is refused by name. This note says which route that
rule is measured on, and how the program's vertex shader is built so that it
stores what Mesa's link stores.

## One route

The GL core calls the back end's entry points in one order (src/gl/shader.rs
glCompileShader, src/gl/program.rs glLinkProgram):

1. `compile` of the vertex shader, `compile` of the fragment shader;
2. `compile_vs_linked` with no attribute map (`link_vs_for_fs`): the
   program's vertex shader, or 1 to keep the shader's own record;
3. `link`, which on the Mali runs `link_by_sources` (the vertex shader's
   constant lanes folded into the fragment shader) and `link_shaders_mali`
   (the two layouts paired by name; a fragment shader recompiled for the
   vertex shader's varying order, or refused).

Until this change the unit tests and every landing measurement called the
compiler's inner functions instead (`vs_constants`, `compile_fs_for_vs`,
`compile_vs_for_reads`). That route never compiled a stage alone, skipped
the varying-order step of `link_shaders_mali`, and always built the vertex
shader for the fragment shader's reads where the product kept the shader's
own code. On main `8c36fd847` 113 programs were equal to Mesa on that route
and not on the product's: 64 compiled with other words, 49 refused.

Now there is one route. `link_records` (src/shaderc/online/mali/tests.rs)
calls the entry points through `SHC_ONLINE` in the GL core's order. The
tests' `compile_linked` (and so `check_linked`, `check_program`,
`check_program_for` and every directory-driven reference test), the back
end's own link tests (`mali::tests_ref::link_records`) all go through it;
nothing rebuilds the link from the compiler's inner functions. The measuring
tools `tools/mali/route.sh` and `tools/mali/corpus_check.py` call the same
entry points in the same order from the integration tests' module
(`tests/mali_ref/mod.rs` `route`), so that they compare by the integration
tests' rule (issue #8783). A test of one stage calls `compile_alone`,
so every such call says so; where a program's reference is compared with
one stage alone, the test also pins the product's refusal of that program
by name (route_if_condition).

## What Mesa's link removes

gl_nir_link_glsl calls `gl_nir_link_varyings`, which runs
`gl_nir_lower_optimize_varyings` (gl_nir_linker.c:1480-1526): the IO lowered
to intrinsics, then `nir_opt_varyings_bulk` over the vertex and fragment
shader, with `gl_nir_opts` between its rounds. In nir_opt_varyings.c,
`remove_dead_varyings` (:2113) removes every output slot whose input the
consumer does not load, scalar by scalar, and the rest are compacted. So the
program's vertex shader stores only the components the fragment shader loads
at that point. A fragment shader that loads no varying leaves a vertex shader
with no varying program.

The loads at that point are those after the pre-link optimisations
(`gl_nir_opts`), not those of the finished fragment program. Two of them
decide what is loaded:

- dead code is gone (`nir_opt_dce`);
- a product with a constant zero is zero (nir_opt_algebraic.py:277-278,
  `a * 0.0` and `a * -0.0`). The rule matches a constant operand that is
  +0.0 in every lane read or -0.0 in every lane read: a vector with zeros of
  both signs matches neither line, and that product stays until the back
  end, which folds it pair by pair (route_zero_mixed_pairs,
  route_zero_mixed_swizzle: the varying stays stored).

The rule needs `nsz` and `nnan`, so it does not match an exact operation.
An assignment to an `invariant` (or `precise`) variable is built exact
(glsl_to_nir.cpp:1899-1902), and every variable read in such an assignment
becomes invariant itself, to a fixed point (propagate_invariance.cpp). So a
product an invariant output is computed from is kept, and its varying is
stored (route_zero_invariant). An `if` condition is not part of an
assignment: a product only a condition reads is folded also in a shader
with an invariant output.

`#pragma STDGL invariant(all)` does not make a built-in output invariant in
Mesa. The pragma is given to a variable where its declaration's qualifiers
are applied (ast_to_hir.cpp:4415-4418, shader outputs only), and
`gl_Position`, `gl_PointSize`, `gl_FragColor` and `gl_FragData` are made
without a declaration (builtin_variables.cpp reads no `all_invariant`). So
under the pragma alone a fragment shader is compiled as without it: the
zero product is folded and its varying is not stored (route_zero_pragma),
and the GLSL passes that skip an invariant assignment run
(route_pragma_reassoc, route_pragma_rebalance). A declared output (a
varying of a vertex shader) is invariant under the pragma.

The front end records which way an output is invariant
(`ir::Invariance`: `Declared` by an `invariant` statement or qualifier,
`Pragma` by the pragma alone). `IoDecl::is_invariant` is the
specification's meaning (GLSL ES 1.00 section 4.6.1: the pragma makes all
outputs invariant); `mali::mesa_invariant` is Mesa's, which the fold above
and the GLSL passes (glsl.rs) read. The vertex compiler refuses an
invariant output by the specification's meaning (nir.rs `build`), unless
every invariant output is a copy, which has no arithmetic to keep exact
(`invariant_copies`, deqp_misc_invariant_position). So a vertex shader
under the pragma that computes an output is refused whole, also one with
no declared output, which Mesa compiles as without the pragma.

dEQP `random.scalar_conversion.fragment_69` reads `e` in `vec4(e, a, c, 0)
* vec4(d, c, b, a)` with constant `a`..`d`: neither operand is a constant
at the link, so Mesa's vertex shader stores `e`; the back end folds the
product lane by lane later, and its fragment program loads nothing.

## Our build

`vs_for_fragment` (backend.rs) now always builds the program's vertex shader
for what the fragment shader reads (`FsReads::program`; a record key of its
own, the count 0 for none):

- with the vertex shader's constant lanes folded in (`link_by_sources`): the
  reads of the linked fragment record, as before;
- otherwise `program_reads` (mali/mod.rs): the fragment record's reads
  (location and component mask), and every other varying a live value of
  the front end's IR loads, whole (`live_varying_loads`). A value is live
  when an output store, an `if` condition or another live value reads it,
  or a local a live value loads stores it; a product with a constant that
  is one zero in every lane reads neither operand, unless an invariant
  output is computed from it (`zero_product`, `exact_values`: the rule of
  the section above). A varying no live value loads (not declared, declared
  and not read, read into a local nothing reads, or multiplied by 0.0) is
  not stored.

The vertex record lists only what is stored: a varying the link removed has
no slot (its mask was 0 before; the frame builder counts the listed slots as
the varyings written and refuses varyings without a varying program: a
program whose only varying is a constant drew GL_OUT_OF_MEMORY, issue
#6521, route_const_only). At
link, `link_shaders_mali` pairs the slots each record keeps: a slot of no
component holds nothing (a fragment shader compiled with the vertex
shader's equal lanes lists the merged one with mask 0), and a varying that
neither record keeps a slot for is not a missing slot.

A vertex shader alone that stores two lanes of equal value is built
without the later one (`or_without_consts`, `compile_vs_without_consts`, as
for its constant lanes): its record only answers glCompileShader, the
program's vertex shader is built at link, and Mesa's link keeps the first
of equal lanes (`deduplicate_outputs`). A shader refused again when built
without those lanes reports that second refusal, the form that is in the
way once the lanes are not.

## The attribute numbers

Removing a varying can leave an attribute unread. Mesa numbers the
attributes it reads 0..n in location order, and so does the vertex compiler
(issue #6470, docs/notes/mali-builtin-inputs.md "Attribute records"): the
program's vertex shader, built for the fragment shader's reads, numbers
only the attributes it still reads (route_attr_gap: `a_w` fed only the
removed `w`, so `a_v` at location 2 is record 1, as in Mesa's draw).

## What is not modelled

The liveness is flow-insensitive, so it keeps more than Mesa's pre-link DCE
in two shapes: a load in a branch of a constant condition (`if (d.y)` with
`d` a constant, dEQP `random.conditionals.*`), and a load whose value is
stored to the colour and overwritten before the end (dEQP
`random.scalar_conversion.fragment_15`). For these our vertex shader keeps
the store and the program is not Mesa's, as on main. Modelling them needs
constant conditions folded and output stores ordered in the liveness.

The same holds for what Mesa's GLSL passes and the first NIR passes remove
before the link and the liveness does not see:

- a zero that is not written as a constant operand of the product: a
  `const` variable, a local assigned 0.0, a struct member, a constant
  expression (`1.0 - 1.0`, `0.0 * 2.0`, `float(false)`), `c *= 0.0`;
- a constant condition: `if (false)`, a `const bool`, `true ? v : w`;
- a comparison Mesa decides from one lane of a vector: a lane of a zero
  product against a constant (`(w * vec4(0.0, -0.0, 0.0, -0.0)).x > 0.5`),
  a constant lane of a sum (`(vec4(w.x, 2.0, 1.0, 1.0) + vec4(2.0, 1.0,
  1.0, 1.0)).y > 3.5`);
- two equal results: `b ? v : v`, the same store in both branches;
- a local or an output stored and stored again before it is read.

In each our vertex shader stores the varying and Mesa's does not; the
program is not Mesa's, on main and here.

The vertex shader is built for our fragment record's reads, so it is
Mesa's only where that record is. Where our fragment shader loads less than
Mesa's, the vertex shader stores less too: a constant lane of the vertex
shader folded into another varying (issue #4510: the fragment shader
declares a subset, k1_loc), a lane only the back end makes constant taken
as a constant (`d.x * -0.0` in one lane). The fragment words are not
Mesa's on main and here; fixing the fragment record fixes both stages.

## Measuring

`tools/mali/route.sh measure <dir>` and `compare main.tsv tip.tsv`
(tools/mali/README-pan_shc.md, "The product route"). The rules of a Mali
branch are read from `compare` of the two trees over the same directories.
