<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 vertex-shader loops that unrolling unmasks

Plan #402 item `ref-vsmix3` (issue #20, ROADMAP stage 5). This item adds
references only and changes no compiler code. Plan #402 `unmask-forecast`
said that the loop lowering (`lower-vsloop`) would raise two refusals. One
is the vertex-shader "a uniform other than a float, vec2, vec3, vec4 or
mat4" (vsunif), by 51 cases, all `vs_int_uniform`. The other is "a
constant operation on or to a value that is not a finite normal float"
(vsconst), by 30 cases. This note finds which `shaders.loops.*` cases those
are, measured at the tip. It keeps only those whose programs read an int or
bool uniform or hold a non-finite constant, and only shapes no earlier
reference covers.

The references are `tools/mali/testdata/deqp_vsmix_*`. The test is
`tests/mali_deqp_vsmix.rs`:

```text
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsmix
```

**HOST-ONLY.** Every count here comes from the board slices or from a
host-only compile. Nothing here ran on the NanoPC-T6.

## 1. The measure

1. **The case list.** The cases of the four slices
   `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/refusals.tsv` whose
   refusal text holds "a loop in a vertex shader": 196 cases. Of these,
   153 are in `shaders.loops.*`.

   ```sh
   cat tools/cts/log/2026-11-24-t6-gles2-S[1234]-8ff01055/refusals.tsv |
       grep -F 'a loop in a vertex shader' | cut -f1 | sort -u > cases.txt
   tools/mali/deqp-host-measure.sh --cts <VK-GL-CTS> \
       --slice tools/cts/log/2026-11-24-t6-gles2-S1-8ff01055 ... --slice ...-S4-8ff01055 \
       --cases cases.txt --out <out>
   ```

   This ran at the tip `22c5fc5f`. The VK-GL-CTS tree was the shallow
   clone of its tip `05ca62dc`, not the `93bca01` the earlier notes name:
   fetching that commit was refused on that host. A later blob-less
   fetch of both commits showed that the loop programs' generator,
   `modules/gles2/functional/es2fShaderLoopTests.cpp`, is the same at
   both. Between them, `modules/glshared` changes only
   `glsAttributeLocationTests.cpp` and `glsBuiltinPrecisionTests.cpp`, so
   dEQP writes the same loop programs at `93bca01`. The tree had
   `external/fetch_sources.py` and the three files of
   `tools/cts/cts-yakogl-target.patch`, which are all new files and were
   written verbatim. Of the 196 cases, 188 stop at a refusal on the host
   and 8 compile (the constant `for`/`while` `nested*` cases, as in
   `mali-deqp-vsloop2.md` row 31). 187 of the 188 stop at the loop. The
   other one, `struct.uniform.loop_nested_struct_array_vertex`, already
   stops at vsconst (`ref-vsconst`). That leaves **145 `shaders.loops.*`
   cases at the loop**.
2. **The programs' sources.** In the measure's `programs/`, each case has
   its own program. Of the 145:
   - **72** read a `uniform mediump int` (`ui_one` .. `ui_five`) beyond its
     declaration. These are the uniform and dynamic iteration groups,
     except `basic_*_float` and `infinite_with_unconditional_break_*`.
   - **73** read none. These are the 43 constant-iteration cases left and
     30 uniform and dynamic ones.
   - **0** hold a float literal that is not finite and normal, or a
     division by a constant zero.

   The loops of the 73 count with a constant or an attribute only.
   Unrolling them makes the counter's compares constant (`i == 2`). That
   is the vsconst site `nir.rs` `Icmp(Eq)` and `Inot` of
   `mali-deqp-vsconst.md` (`staticloop`, covered by
   `deqp_cfold2_zero_operand_icmp`). No new reference is needed for them:
   all 32 constant-iteration programs that no reference holds verbatim were drawn,
   and each has the bytes of an existing reference (§3).
3. **What Mesa does with the 72.** Each program was drawn once on Mesa
   26.2.1's drm-shim (`tools/mali/drm_shim_ref.sh run
   $CONNECTED_DEPS_DIR/mesa-shim <vert> <frag> <out> --uniform-values`),
   and the shaderdb line's `N loops` was read:
   - **60 kept**: Mesa leaves 1 to 3 loops in the varying program. These
     are step 2 of the loop lowering (`mali-deqp-vsloop2.md` rows 9-26),
     whose shapes `deqp_vsloop2_*` and `deqp_loops2_*` already hold. Their
     int uniform is the kept loop's bound.
   - **8 deleted**: `only_continue` (uniform and dynamic, three loop
     types) and `do_while` `unconditional_break` (uniform and dynamic).
     The loop is removed and the int uniform with it: 5 uniform words,
     2 loads. Unrolling unmasks nothing here.
   - **4 unrolled, int uniform kept**: `for` and `while`
     `unconditional_break`, uniform and dynamic. The loop becomes one pass
     under `0 < ui_three` (or `0 < one*ui_three`), so the int uniform is
     still read: 6 uniform words, 3 loads. **These 4 are the cases where
     unrolling unmasks vsunif.**

## 2. The shapes (HOST-ONLY)

"F" is the board's status at `8ff01055`. All four cases are Fail.

| shape | cases | board | example case (`dEQP-GLES2.functional.shaders.loops.`) | Mesa | reference |
|---|---:|---|---|---|---|
| `uni_unconditional_break` | 2 | 2 F | `for_uniform_iterations.unconditional_break_vertex` | one pass under `0 < ui_three` | covered: `deqp_loops2_vs_for_uni_unconditional_break` (the `for` program verbatim; the `while` program, `while(i++ < ui_three)`, has the same bytes) |
| `dyn_unconditional_break` | 2 | 2 F | `for_dynamic_iterations.unconditional_break_vertex` | one pass under `0 < one*ui_three` | **new**: `deqp_vsmix_dyn_unconditional_break` (the `while` program has the same bytes) |
| **total** | **4** | 4 F | | | 1 new, 1 covered |

The 141 other `shaders.loops.*` cases at the loop are not in this table.
The 60 kept ones and the 8 deleted ones are covered by shape (§1.3). Of
the 73 without an int uniform, the 43 constant-iteration and 12
`infinite_with_unconditional_break_*` cases are covered by bytes (§3). The
18 `basic_*_float` cases are kept loops in `mali-deqp-vsloop2.md` rows 7
and 8 and are covered there by shape. They read no int uniform, so they
are outside this item.

## 3. Covered by bytes

`deqp_ref.sh --instrument shim` was run on each program that no
reference holds verbatim, and each result was compared with every
`tools/mali/testdata/*.expect.json`, ignoring the `trace` name:

- the 32 constant-iteration programs fall into five groups. Each group has
  the bytes of `deqp_vsloop_con_cond_break` (14 programs),
  `deqp_loops_vs_for_const` (4), `deqp_loops_vs_nested_const` (7),
  `deqp_vsloop_uni_only_continue` (6) or
  `deqp_loops2_vs_do_con_101_iterations` (1).
- the 8 deleted programs, and the dynamic `infinite_with_unconditional_break_*`
  ones, have the bytes of `deqp_vsloop_uni_only_continue` or
  `deqp_vsloop_uni_inf_break_last`.
- the uniform `while` `unconditional_break` has the bytes of
  `deqp_loops2_vs_for_uni_unconditional_break`. The two dynamic ones have
  each other's bytes and no reference's, so they get the new reference.

Programs outside `shaders.loops.*` that are outside this item, and that no
reference's bytes match (recorded for the planner):
`invariance.highp.loop_2` and `invariance.lowp.loop_2` (no int uniform;
each has its own bytes), and `loops.do_while_dynamic_iterations.basic_{lowp,mediump}_float`
(no int uniform; kept loops, vsloop2's row 8, whose bytes differ from
`deqp_loops2_vs_do_dyn_basic_highp_float`).

## 4. What Mesa 26.2.1 does, and what the tip does past the loop

The reference was drawn with `tools/mali/deqp_ref.sh <vert> <frag>
tools/mali/testdata/deqp_vsmix_dyn_unconditional_break --instrument shim`.
The shim was built in `$CONNECTED_DEPS_DIR/mesa-shim` with the meson
options of `drm_shim_ref.sh build`, from the Mesa 26.2.1 tree that
`$CONNECTED_DEPS_DIR/mesa-pan` had already unpacked (build dir
`build-shim`). Self-check first: `deqp_vsloop_con_cond_break`, drawn
again, matched its committed `.expect.json` byte for byte, apart from the
`trace` name.

`deqp_vsmix_dyn_unconditional_break`, `vs.varying` (25 words): it loads
`a_one` and narrows it to fp16 (`FADD.f32 r0.h0`). It adds 0.5 in fp16
(`FADD.v2f16`) and converts with `F16_TO_S32.rtz`. Then comes
`IMUL.i32 r0, r0^, u5.w1` (`ui_three`) and `ICMP_OR.s32.ge.m1 r0, 0x0,
r0^`. The three lanes are picked from the fp16 pairs by
`MUX.v2i16` and widened with `F16_TO_F32`. `vs.position` is the plain
24-word transform.

**Past the loop, at the tip (HOST-ONLY probe, not committed).** The loop
was unrolled by hand in the source (`if (0 < one*ui_three) { res =
res.yzwx; }`, and the `?:` form). The pair was compiled with
`mali_ref::compile_pair` and compared with the reference. It **compiles**:
the tip no longer refuses a `uniform mediump int` read in the vertex
shader (since `5e1feec9`, `mali-deqp-vsunif.md`). The first difference is
instruction 1 of `vs.varying`. Ours is `FADD_IMM.f32` and `F32_TO_S32`,
19 words. Mesa's is the fp16 `FADD.f32 r0.h0` / `FADD.v2f16` /
`F16_TO_S32` and the `MUX.v2i16` tree, 25 words. The uniform form
`if (0 < ui_three)` compiles too (15 words; ours `ICMP_OR.s32.lt` and
`CSEL.u32.eq` in fp32). So at this tip, unrolling these programs unmasks
**no vsunif refusal**. What is left is the mediump narrowing of the
attribute-derived int and of the select. The forecast's +51 vsunif was
made before `5e1feec9` took the VS int uniforms, or counted step-2 kept
loops. The `remeasure-402` item should recount it.

## 5. For the lowering

- `lower-vsloop` (step 1, one guarded pass): when it takes
  `deqp_vsmix_dyn_unconditional_break` out of LoopVertex, the test's
  `refusals_pinned` fails with "now compiles". Then remove the pin and
  the `#[ignore]`. If the bytes still differ (the fp16 narrowing above),
  move the reference to a `DIFFERS` list as `tests/mali_deqp_vsunif.rs`
  does, and name the first difference.
- vsconst: the `shaders.loops.*` loops hold no non-finite constant. The
  vsconst cases that unrolling unmasks are the constant-iteration guards,
  and `mali-deqp-vsconst.md`'s `staticloop` references already cover them.
