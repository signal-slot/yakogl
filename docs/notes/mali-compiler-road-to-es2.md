<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): does family-by-family lowering reach zero refusals, or does the compiler need general mechanisms?

**Status: study, no code.** Sections 0 to 5 read and measure; they change
nothing under `src/`, `tests/` or `tools/`. Section 6 is the baseline of the
corpus check that section 4.4 names as the first work item, built after the
study (`tools/mali/corpus_check.py`). It answers the owner's question for the
`gpu-mali` online compiler (`src/shaderc/online/mali/`, NanoPC-T6 Mali-G610;
the Verdin iMX95 Mali-G310 gets the same words, `mali-g310-vs-g610.md`):
will removing refusal families one by one, in count order (plan #387,
issue #8942), converge to zero refusals on dEQP-GLES2 and then on real
applications? Or must the compiler first get general mechanisms, and if so
which ones, how large, and in what order?

- **Tip:** origin/main `48f6017cb` (2026-11-21). All `file:line` sites are at
  that commit, under `src/shaderc/online/mali/` unless the path says
  otherwise. Mesa sites are Mesa 26.2.1 (`src/...` of the tarball, the
  drm-shim tree that `tools/mali/drm_shim_ref.sh build` makes).
- **Measure:** `tools/mali/deqp-host-measure.sh` at the tip, run on
  2026-11-21 over the 10,597 cases the board refused in the slices
  `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/`. This is the method
  of `mali-deqp-families.md` sections 1, 2 and 18, with VK-GL-CTS
  `93bca018`. All 10,597 cases began; 106 processes exited 1, and 2 exited
  139 (the known SIGSEGV of families 18.4). **HOST-ONLY**: the counts are
  compile and link outcomes, never render verdicts. Board counts are quoted
  from `tools/cts/RESULTS.md` and section M of `docs/es2-cts-failures.md`.
- **Analysis script:** a scratch Python reader over the measure's
  `measure.tsv` and `src/<case>.vert|.frag`. It is not committed. Section 2
  gives its rules so that the numbers can be made again.
- **Caveat:** the Zephyr board run at `de2f2c103` (9,545 Fail; 10,489
  refused compiles of 40 texts) has records that are not in this tree (they
  are gitignored on the board host). Its 40-text ranking cannot be rebuilt
  here, so the per-case population is the `37305608` slices, as in families
  section 18.

## 0. Answer in brief

1. **The back half of the compiler is already general and is not the
   problem.** Everything after instruction selection transcribes Mesa's
   passes: the BIR optimisations, FAU, the pressure scheduler, LCRA register
   allocation, flow and packing. That half raises about 10 of the roughly
   527 gap sites. The refusals come from the front half: two hand-written
   walks, `nir.rs` (vertex, 6,273 lines) and `nir_fs.rs` (fragment, 13,372
   lines), plus six alternative fragment selectors. These walks build "the
   NIR as Mesa would have it after its passes" directly, shape by shape.
   Among them they hold about 360 of the gap sites.
2. **Family by family does not converge to zero on dEQP-GLES2.** It
   converges on the template-generated groups. It flattens on the
   generated groups. The evidence:
   - The 6,126 cases still refused at the tip contain 4,901 distinct
     refusing shaders.
   - The template groups (`shaders.operator`, `uniform_api.value`,
     `shaders.matrix`, `shaders.swizzles`, `texture.specification`, ...)
     collapse well: their 3,184 cases reduce to 789 normalised shapes.
   - The generated groups (`shaders.random`, `shaders.indexing`,
     `shaders.loops`, `shaders.struct`, `uniform_api.random`,
     `fragment_ops.interaction`, `uniform_api.info_query`, ...) do not:
     2,942 cases are 2,749 distinct normalised shaders.
   - In a seeded sample of 200 refused shaders, 162 (81 %) also contain at
     least one further mechanism class beyond their first refusal, and 117
     (59 %) contain two or more. The mean is 1.8 classes.
   - Since the board run of `37305608`, 7,167 cases left their board
     family. Of those, 2,696 (38 %) met another refusal text rather than
     compiling.
3. **Where it compiles, family by family does not hold its own bar either.**
   The audit `mali-deqp-compiled-audit.md` sampled 300 distinct pairs of
   once-refused shaders that now compile. 108 of them (about 33 % of the
   cases) are not Mesa's words. Ten open issues track shapes that compile to
   other words than Mesa's (#4732, #6433, #6229, #6326, #6446, #6448,
   #6563, #6586, #6729, #6730). One case is a miscompile with a wrong value.
4. **Real applications are bounded corpora, so there family by family does
   converge.**
   - femtovg/Slint: 42 of 42 programs compile (`tools/mali/corpus-verdicts.txt`).
   - glmark2: 14 of 33 scenes are compile-clean
     (`tests/fixtures/mali_glmark2_verdicts.txt`). The rest are blocked by 11
     texts.
   - An arbitrary application is a dEQP-random-like population, not a
     corpus.
5. **Recommendation: (C), a mix that leans to (B).**
   - Stop adding new shape arms to the walks.
   - Land only the per-family items that fix wrong values or defects, or
     that are already written.
   - Build the front half as Mesa builds it: a typed SSA NIR (int, bool and
     bit size per value, blocks and phis). Run Mesa's own algebraic rules on
     it, machine-generated as they already are for blend shaders, in Mesa's
     pass loop. Feed the result into the existing general back half.
   - Verify by byte-compare against Mesa, as today, but over the whole
     corpus at once, with the old and new paths side by side (the S1b
     method of #31).
   - First three work items: section 4.4.
   - Three hardest problems: section 5.

## 1. The compiler as it is: stages, general or matched

The input is the shared front end's `ir::Module` (`src/shaderc/online/ir.rs:860`):
- structured nodes (`If`, `Loop`, `Break`, `Continue`, `Return`, `ir.rs:556`)
  over instruction slots;
- locals as slots, and no phis;
- no stored type per value: the kind float, int or bool is derived from the
  op (`ValueKind`, `ir.rs:361`);
- a per-value precision table `Prec` (`ir.rs:839`) that any rebuilding pass
  resets to `Unknown`.

The back end enters at `mod.rs:1749 lower_source`, then runs
`compile_module_on` (`mod.rs:1206`), `compile_stage` (`:1275`) and
`finish_passes` (`:897`).

| # | Stage | Where | General or matched |
|---|---|---|---|
| 1 | GLSL-level passes: `ilt_trees`, `do_rebalance_tree` (DSW), reassociate_constant, folds | `glsl.rs` (2,639 lines) | **Mixed.** The algorithms are transcribed, but trees the pass cannot place are refused (`Rebalance` 14 sites, `ArithConstant`/`NegZeroShared` about 20) |
| 2a | VS walk: IR to a lane graph ("Mesa's NIR when selection starts"), with folds, algebraic rules, cse, viewport, late ffma, opt_move | `nir.rs:3930 build` | **Matched.** The module doc says "for the shapes this back end takes" (`nir.rs:8-11`). Each algebraic rule is an arm. A loop is refused outright (`nir.rs:3778`) |
| 2b | FS precision: one width for the **whole shader**, taken from the default precision at `main` | `mod.rs:1069 FsPrecision::of` (it re-parses the source; the `Defect(IrIndex)` cases come from here) | **Matched, per shader.** A value of the other width gives `MixedPrecision` |
| 2c | FS: picks one of six selectors (`vec16::program`, mask/array, bool colour, int colour, guarded/kept/frag32, the fp16 pair `select`) | `mod.rs:1471-1576`, `vec16.rs:413` | **Matched** at the top level |
| 2d | FS walk: 16/32-bit lane nodes, rules per pair, loops unrolled with a copy of Mesa's cost (`unroll_cost`, `nir_fs.rs:4899`), kept loops detected | `nir_fs.rs:4158 walk`, `:4860 walk_loop` | **Matched.** `Fs::op` (`nir_fs.rs:1392-1499`) takes Mesa's fmul/fadd rules case by case. Any other case is a refusal |
| 2e | FS varyings: the lanes the link keeps, compaction, load width; nir_opt_varyings backward motion | `nir_fs.rs:7851 layout`, `link.rs`, `motion.rs:516` | **Mixed.** Link and motion are general rules with table caps. Load and colour-pair forms are lists |
| 2f | FS colour pairing into two-lane fp16 nodes | `nir_fs.rs:9830 select`, `:11678 select_highp` | **Matched.** The colour is built only from enumerated `Leaf` kinds; anything else is `ColourForm` |
| 2g | VS selection: IDVS split, stores, UBO layout | `select.rs` | General over its input |
| 3 | ALU emission: one `bi_emit_alu` case per (op, bits, width) row | `emit.rs:39 ROWS` (99 rows; Mesa's `bi_emit_alu` has 140 `nir_op` cases) | General per row; the row set is closed |
| 4 | BIR passes: mod_prop forward/backward, dce, cse, lower_opt, mkvec_swz, isel, fuse_add_imm, fuse_cmp | `opt.rs`, in `bi_compile_variant_nir` order (`mod.rs:906-915`) | **General** |
| 5 | FAU: push_ubo, lower_constants (Mesa's histogram and hash-slot order, `tools/mali/qsort_pointer_order.c`) | `fau.rs:113`, `:702` | **General**, with table caps |
| 6 | Scheduling: `bi_pressure_schedule` | `sched.rs:341` | **General** |
| 7 | RA: out-of-SSA, lower_vector, LCRA on the split file (32 registers) | `ra.rs:566` | **General, but no spilling and no full-file retry** (`ra.rs:697` returns `Spill`) |
| 8 | Flow: slots, flow nops, merge_flow, mark_last | `flow.rs` | **General** |
| 9 | Packing over the generated ISA tables, every word decode-checked | `pack.rs:996`, `isa/v10.rs` (generated, 14,987 lines) | **General** |
| - | Blend shaders: a real single-block NIR running Mesa's algebraic passes **generated from Mesa's own `nir_algebraic.py`** with nir_search worklist order | `blend_nir.rs:2508 algebraic`, `blend_nir_rules.rs` (`tools/mali/gen_blend_nir_rules.py`: 179 + 24 + 4 = 207 transforms over 17 opcodes) | **General.** It is the one place where Mesa's rules are transcribed by machine |

The rule "a precision per IR value" (#4536) is a design only
(`mali-precision.md` Part C). A NIR-level `bits` and `width` exist per node
(`nir.rs:605`), but each walk sets them per shader path.

**Size.** The non-test compiler grew from 36,866 lines (main, 2026-11-01) to
56,756 lines (2026-11-21). The measure: `git ls-tree` at the last main
commit of each date, `*.rs` under `src/shaderc/online/mali/` without the
test files. `MaliError` stayed at 63 to 69 variants over the same period.
The growth went into arms inside the existing variants: a new shape is a
new arm, not a new error.

### 1.1 `MaliError`

There are 69 variants (`mod.rs:117`): 68 gap variants and `Defect` with 20
sub-kinds. A grep of `MaliError::<Variant>` outside the tests gives about
527 gap raise sites, 241 `Defect` sites and 36 `OutOfScratch` sites. The
gap sites by file: `nir_fs.rs` about 270, `nir.rs` about 88, `glsl.rs` about
20, `vec16.rs` 8, `select.rs` 5. `motion`, `pack`, `fau`, `sched` and `ra`
have 5, 4, 4, 1 and 1. `opt`, `flow`, `emit`, `bir`, `preload` and `link`
have 0. The table below groups the gap sites by reason class. The counts
are approximate, because some variants are raised through helpers.

| Reason class | Variants (largest first) | Sites (about) |
|---|---|--:|
| missing operation | `Arithmetic` 32, `TexKind` 13, `UniformIndexed` 12, `FragOutput` 8, `NoColour` 6, `IndexedLocal`, `IfForm`, `ConvertWiden` 5 each, `TexVertex`, `SpecialInput` 4 each, ... | 110 |
| type domain (int/bool) | `IntValue` 9, `UniformType` 3, `UniformArray` 2, `IntColour` 2, plus `FsUniform` (shared with precision) | 40 |
| precision (fp16/fp32) | `FsUniform` 24, `Fp16Inexact` 23, `FragCoordMediump` 10, `FsVaryingLoad` 8, `MixedPrecision` 7, `UniformPrecision`, `TexHighp` 2 each | 75 |
| control flow | `LoopForm` 23, `ControlFlow` 18, `LoopVertex` 16, `DiscardForm` 10, `GuardMediump` 3, `LoopMediump` 2 | 72 |
| order not Mesa's (algebraic, fold, link) | `ArithConstant` 21, `ConstantFold` 15, `Rebalance` 14, `VaryingNotFromAttribute` 6, `AlgebraicForm` 6, `NegZeroShared` 5, `VaryingDuplicate` 4, `PositionLane`, `FusedMultiplyAdd`, `AttributeNumbering` 3 each | 80 |
| reference missing (a form no reference draws) | `ColourForm` 25, `FsOperandForm` 20, `TexCoord` 14, `VaryingForm`, `TexLanes` 5 each, `ColourPair` 4, `TexCoordShared`, `ColourZeroHalf` 3 each, `CompareForm` 1 | 80 |
| deliberate guard (output would differ) | `BlendVariantNotLowered` 24, `Invariant` 2 (many `Arithmetic` sites are guards too: the `vs_clamp` refusal at `nir.rs:3566` sits in front of an existing `FMAX.f32` row) | 26+ |
| size, tables, RA | `VaryingLocation` 15, `TooManyValues` 12, `BlockTableFull` 11, `TexCount`, `FragmentTooLarge` 5 each, `UboRowsFull`, `MovedTooLarge` 4 each, ..., `Spill` 2 | 60 |

The structural fact behind the table is that **a refusal in the walks is
usually the absence of an arm, not a guard in front of a general emitter**.
Four examples:
- `IntValue` at `nir_fs.rs:2414`: only iadd, imul, ineg, i2f, u2f and idiv
  have node builders.
- `ColourForm`: the fp16 colour is assembled from a closed `Leaf` list.
- `LoopVertex` at `nir.rs:3778`: the vertex walk has no loop at all.
- The algebraic arms of `Fs::op`: an unmatched combination is refused even
  where a plain node could be emitted, because Mesa's rule order is not
  modelled.

So removing a family means writing the arms that produce that family's
output, one reference at a time.

## 2. The tail, measured

### 2.1 What still refuses at the tip, by text and by reason class

Of the 10,597 board-refused cases, **4,471 compile and link on the host**
(9ac31f30: 4,401; 7b540fee: 4,465), and **6,126 still refuse**. The
refusals by reason class, using the first refusal per case:

| Reason class | Cases | vs / fs | Largest texts |
|---|--:|---|---|
| missing operation | 1,580 | 648 / 932 | `Arithmetic` 1,464 (vs 574, fs 890), `TexVertex` 60, `TexKind` 42 |
| reference missing (colour, operand, coordinate forms) | 1,233 | 0 / 1,233 | `ColourForm` 821, `TexCoord` 182, `FsOperandForm` 152, `ColourPair` 61 |
| type domain | 1,141 | 655 / 486 | `UniformType` 462, `FsUniform` 280 (also precision), `IntValue` 206, `UniformArray` 193 |
| order not Mesa's (algebraic and fold) | 903 | 707 / 196 | `ArithConstant` 386, `ConstantFold` 204, `FusedMultiplyAdd` 139, `NegZeroShared` 137 |
| control flow | 626 | 280 / 346 | `LoopMediump` 281, `LoopVertex` 241, `ControlFlow` 73, `DiscardForm` 16 |
| precision | 351 | 99 / 252 | `MixedPrecision` 166, `UniformPrecision` 99, `FsVaryingLoad` 43, `FragCoordMediump` 42 |
| type domain, through a defect | 187 | 0 / 187 | `Defect(IrIndex)` 185 + 2 through a link (the bvec re-parse of `FsPrecision::of`, #8234) |
| order not Mesa's (link) | 91 | 51 / 40 | `VaryingDuplicate` 49, `VaryingNotFromAttribute` 32 |
| registers and size | 8 | 4 / 4 | `FragmentTooLarge` 4, `Spill` 2, `TooManyValues` 2 |
| defect | 6 | 6 / 0 | `Defect(Encode)` 6 (bind_aliasing) |

By stage, every one of the 6,126 cases is refused in the IR-to-NIR walks
or the GLSL passes (stages 1 to 2f), except the 2 `Spill` cases (RA) and the
4 + 2 size-table cases. Classified this way, **about 2,225 cases (36 %) are
refused because the form or the order is not Mesa's**: the reference-missing
and order classes. These are not missing operations. They exist only
because the walks reproduce Mesa's passes by hand.

### 2.2 Distinct shapes behind the counts

For each case, the analysis takes the refusing shader and compares the
sources at four levels:
- **exact**: the text as written.
- **norm1**: comments and blank space removed, numbers replaced by `#`.
- **norm2**: norm1, with precision qualifiers removed, `vec2..4`, `ivecN`,
  `bvecN` and `mat2..4` folded to one name each, and swizzles folded to one
  token.
- **sig**: the set of language constructs used (the keywords, built-ins and
  operators of the analysis script), with identifiers and values ignored.

| | Cases | exact | norm1 | norm2 | sig |
|---|--:|--:|--:|--:|--:|
| All refused, globally distinct | 6,126 | 4,901 | 4,861 | 3,373 | 1,400 |
| Summed per refusal text | 6,126 | 4,903 | 4,865 | 3,438 | 2,019 |

By dEQP group (cases / distinct norm2):

| Low diversity (templates) | | High diversity (generated) | |
|---|---|---|---|
| shaders.operator | 1,194 / 235 | shaders.random | 1,502 / 1,403 |
| uniform_api.value | 818 / 190 | shaders.indexing | 462 / 412 |
| shaders.matrix | 387 / 140 | shaders.loops | 456 / 405 |
| shaders.swizzles | 234 / 6 | uniform_api.info_query | 125 / 116 |
| shaders.conversions | 169 / 70 | shaders.struct | 86 / 86 |
| shaders.constant_expressions | 151 / 93 | uniform_api.random | 86 / 86 |
| texture.specification | 106 / 2 | fragment_ops.interaction | 81 / 81 |
| shaders.invariance | 36 / 11 | shaders.functions, discard, return, ... | 144 / 160 total distinct over the rest |
| **all groups with distinct/cases at or below 0.8** | **3,184 / 789** | **all groups above 0.8** | **2,942 / 2,749** |

The family method works where one arm serves many cases. This holds for
the left column, where there are 4 cases per shape (`texture.specification`
has 53 per shape, `shaders.swizzles` 39). It does not hold for the right
column, which is 48 % of what still refuses: there, nearly every case is a
different shader. A shader compiles only when **every** construct in it has
an arm, and its words are Mesa's only when every combination of rules has
been reproduced.

### 2.3 What stands behind the first refusal

There are two measures, one empirical and one structural.

**Empirical: what happened to the cases when their family was worked.**
The table compares each case's board family at `37305608` with its host
outcome at the tip. The families are those of the measure script's
`FAMILIES` table.

| Board family | Cases | Compile now | Same text | Moved to another text | Compile share of those that left |
|---|--:|--:|--:|--:|--:|
| intval | 3,223 | 2,073 | 191 | 959 | 68 % |
| ARITH (#367) | 1,585 | 408 | 1,019 | 158 | 72 % |
| varylink | 1,067 | 878 | 63 | 126 | 87 % |
| unitype | 820 | 203 | 391 | 226 | 47 % |
| FRAGCOLOR_LANE (#367) | 793 | 203 | 406 | 184 | 52 % |
| loops | 774 | 68 | 507 | 199 | 25 % |
| constop | 738 | 231 | 136 | 371 | 38 % |
| CONST_FOLD (#367) | 463 | 20 | 340 | 103 | 16 % |
| uniarray | 272 | 33 | 140 | 99 | 25 % |
| mixprec | 251 | 131 | 7 | 113 | 54 % |
| colourpair | 171 | 164 | 6 | 1 | 99 % |
| LOCALS (#367) | 160 | 15 | 0 | 145 | 9 % |
| others | 280 | 44 | 224 | 12 | 79 % |
| **Total** | **10,597** | **4,471** | **3,430** | **2,696** | **62 %** |

The largest flows are:
- intval to Arithmetic 253, to UniformType 182, to IrIndex 152;
- unitype to ColourForm 121;
- constop to ColourForm 119;
- loops to UniformType 98;
- ColourForm to Arithmetic 97;
- intval to MixedPrecision 96.

So fixing family N **does** expose family N+1 in the same shaders: 38 % of
the cases that left a family met another text. Plan #387's forecast
(`mali-lowering-schedule.md` 4c) assumes "no inflow", so its forecast is an
upper bound.

The share depends on the kind of fix. The two families removed by a rule
that is general over its input both compile at high rates:
- `varylink`: the nir_opt_varyings motion in `motion.rs`, 87 %;
- `colourpair`'s U16_TO_U32 rule, 99 %.

So does `intval` (68 %), which was removed by giving the walks an int
domain. The families removed by shape-specific folds compile at low rates:
`CONST_FOLD` 16 %, `constop` 38 %, `loops` 25 %, and the local-array
lifting of `LOCALS` 9 %.

**Structural: a sample of 200.** The sample is seeded with 387 and drawn
uniformly from the 6,126 refused cases. For each refusing shader, the
analysis lists which of ten mechanism classes its source uses:
- int/bool;
- a loop;
- `if`, `?:` or `discard`;
- a transcendental or `/`;
- a common built-in such as `min`, `clamp` or `floor`;
- mixed fs precision;
- a non-float or array uniform;
- a dynamic index;
- a texture look-up other than `texture2D`;
- a struct.

The detectors are regular expressions in the scratch script. They
over-count a construct the compiler already lowers, so they are an upper
bound per shader. Two shaders were read by hand to check them, for example
`shaders.random.conditionals.combined.0`, whose const ivec compares,
nested `if` and `bool` colour lanes are all real obstacles. The class
the first refusal names is then removed, and the rest is counted:

| Classes beyond the first refusal | 0 | 1 | 2 | 3 | 4 | 5+ |
|---|--:|--:|--:|--:|--:|--:|
| Cases in the sample (of 200) | 38 | 45 | 63 | 42 | 8 | 4 |
| All 6,126 refused | 996 | 1,471 | 1,674 | 1,562 | 329 | 94 |

In the sample, the first refusal is a *form* text (lane, order, fold,
reference) in 87 of 200 cases. The further classes are int/bool 120,
non-float uniforms 65, branches 43, common built-ins 43, dynamic index 19,
mixed precision 15, loops 13, transcendentals 13, structs 11 and texture
kinds 9. The mechanism the most shaders need next is the **integer and
boolean domain**, followed by **uniform types** and **branches**, whatever
their first refusal says.

### 2.4 Applications

- **femtovg/Slint**: 42 of 42 programs compile (`tools/mali/corpus-verdicts.txt`,
  checked by `tools/mali/fvg_refusals.py`). The corpus is bounded and the
  family method finished it.
- **glmark2** (`tests/fixtures/mali_glmark2_verdicts.txt`, regenerated on
  2026-11-19): 14 of 33 scenes compile clean. 33 shader lines refuse, under
  11 texts: TEX_MULTI 8, the texture-coordinate link 6, `ConstantFold` 5,
  `Arithmetic` 4, `ArithConstant` 3, `IntValue` 2, and one each of
  `FragCoordMediump`, `ControlFlow`, `TexCoord`, `LoopMediump` and
  `FsOperandForm`. These are the same classes as dEQP's. glmark2 is a
  corpus, so the method can finish it.
- **The tools corpus** (`tools/mali/corpus-verdicts.txt`, 328 lines): 256 ok,
  18 desktop-GLSL front-end rejections, 54 Valhall refusals of 19 texts.
- An arbitrary application is not a corpus. Its shaders are the
  dEQP-random kind: each one combines constructs. The right-hand column of
  2.2 is the better model for it.

## 3. How Mesa does it, and what we mirror

Mesa's path for an ES 2 shader on v10 runs in this order:

1. **GLSL IR**: `lower_precision` (`compiler/glsl/glsl_parser_extras.cpp:2491`,
   1,366 lines; panfrost enables fp16, 16-bit constants and 16-bit load
   destinations at `gallium/drivers/panfrost/pan_screen.c:585-589`), then
   `do_rebalance_tree`.
2. **The linker**: `nir_lower_mediump_vars`, `nir_opt_varyings_bulk`
   (`compiler/glsl/gl_nir_linker.c:1364`, `:1524`).
3. **`bifrost_preprocess_nir` and `bifrost_postprocess_nir`**
   (`panfrost/compiler/bifrost/bifrost_nir.c:400`, `:864`): IO lowering,
   `nir_lower_mediump_io`, `nir_lower_idiv`, `nir_lower_flrp`,
   `nir_lower_bit_size`, `nir_lower_alu_width`.
4. **The fixpoint `bi_optimize_loop`** (`bifrost_nir.c:202-276`):
   copy_prop, dce, cse, peephole_select, **nir_opt_algebraic**,
   constant_folding, dead_cf, opt_loop, **nir_opt_loop_unroll** (at most 32
   iterations, `bifrost_compile.h:140`), and the rest.
5. **`bi_optimize_late`** (`:279-395`): `nir_opt_algebraic_late`,
   `pan_nir_lower_bool_to_bitsize`, `bifrost_nir_lower_algebraic_late`,
   vectorize, opt_sink and opt_move.
6. **The back end**: `bi_emit_alu` with 140 cases, the BIR passes, the
   pressure scheduler, `bi_ra.c` with LCRA (1,326 lines) and the SSA spiller
   `bi_spill_ssa.c` (1,595 lines), the `va_*` flow passes and `va_pack.c`.

| Mesa component | Ours | Verdict |
|---|---|---|
| `do_rebalance_tree` | `glsl.rs` DSW | mirrored; unmeasured trees guarded |
| `lower_precision`, `nir_lower_mediump_*` (a width per value) | `FsPrecision` (`mod.rs:1069`): one width per shader | **lacking**, replaced by a per-shader choice; design in `mali-precision.md` Part C |
| NIR + `bi_optimize_loop` + nir_opt_algebraic | `nir.rs` / `nir_fs.rs` build the post-pass NIR by hand; hand-written folds (`mali-algebraic-folds.md`) | **replaced by per-shape code** (except blend shaders) |
| nir_search + generated rules | `blend_nir.rs` + `blend_nir_rules.rs`, 207 transforms, 17 opcodes, one block, no phis | **mirrored for blend only**; the generator and matcher are reusable |
| peephole_select, loop_analyze, loop_unroll, lower_returns | `IfRegion`/`LoopRegion` (`nir.rs:518-575`) and `walk_loop` for measured forms | replaced per shape; vertex loops absent |
| `pan_nir_lower_bool_to_bitsize`, the int ALU | a few int ops in `nir.rs` `Alu`; `mask_op` arms | **mostly lacking** |
| nir_opt_varyings | `motion.rs`, `link.rs` | partly mirrored (general within caps) |
| `bi_emit_alu` | `emit.rs` `ROWS`, 99 of Mesa's 140 cases | mirrored as a table, incomplete |
| bi_opt_*, va_lower_isel, va_optimize, va_lower_constants, pressure schedule, LCRA, flow, pack | `opt.rs`, `fau.rs`, `sched.rs`, `ra.rs`, `flow.rs`, `pack.rs` | **mirrored, general** (no spilling) |

These are the general mechanisms whose absence produces the refusal
classes of 2.1, with a size estimate for a faithful port (Rust, generated
tables excluded):

| Mechanism | Classes it removes (cases at the tip, first refusal) | Mesa size | Estimate |
|---|---|---|---|
| **M1. A shader NIR in Mesa's pass loop**: an SSA graph with blocks and phis, nir_search with the *full* generated rule set, constant folding for every opcode, copy_prop/cse/dce/vectorize/alu_width/move/sink in `bi_optimize_loop` order | order 903 + link 91 + reference-missing 1,233 (the colour/operand/coordinate forms are what the passes and the vectoriser produce) + the order-guarded part of `Arithmetic` | about 8k lines of passes + nir_search 916 + tables | 8-12k (the blend engine, 2.9k lines, is the seed) |
| **M2. A typed value domain**: int, uint and 1-bit bool per value, `bool_to_bitsize`, `boolean_bitwise`, `nir_lower_idiv`, the int rows of `bi_emit_alu` | type domain 1,141 + IrIndex 187, and the int counters loops need; int/bool is also the most frequent next class (2.3) | about 500 + rows | 2-4k |
| **M3. Precision per value** (#4536): `lower_precision` on the front end's IR, `nir_lower_mediump`, 16-bit folds, the width-2 vectorise filter | precision 351 + `FsUniform` 280 + `LoopMediump`'s fp16 phis; the mediump vertex walk (the largest compiled-but-not-Mesa cause in the audit, about 710 cases) | 1,366 + 895 | 3-5k, including the front-end change |
| **M4. General control flow**: loop_analyze, loop_unroll, opt_loop, peephole_select, lower_returns, `emit_if`/`emit_loop` with blocks | control flow 626 (most of them NotSupported) | about 5k | 4-6k, after M1 and M2 |
| **M5. RA spilling** and the full register file | `Spill` 2 (it grows once loops unroll) | 1,749 | about 2k, last |

The `Arithmetic` text (1,464) is mostly **rows and lowerings**, not
mechanism: transcendental kernels, `fdiv` as `frcp`, `fmod`, `fsign` and
`floor`. In Mesa these are NIR lower passes plus `bi_emit_alu` cases.
Under M1 they become emit rows with a reference each; the walks' guards in
front of them go away.

## 4. Options

### 4.1 (A) Continue family by family

The cost and the forecast:
- **Sessions to the plan's goal.** `mali-lowering-schedule.md` 3 orders
  about 66 sessions to bring every text under 100 cases (its criterion
  c2). That is not zero. The schedule assumes no inflow. With the measured
  38 % inflow, the same work leaves roughly 1.4 times the residue that
  schedule shows, and adds sessions for the texts the inflow pushes over
  100: `ControlFlow`, `ColourPair`, `TexKind` and `FsVaryingLoad` are the
  ones the schedule names.
- **Where it flattens.** The template groups (3,184 cases, 789 shapes) are
  reachable at the observed 5-15 shapes per session. The generated groups
  (2,942 cases, 2,749 shaders, 1.8 further classes each) are reachable only
  as fast as each family's arms happen to cover random combinations. The
  measured compile share for shape-specific fold families is 16-38 %.
- **The estimate.** The refusals flatten at about **1,500 to 2,500
  must-pass cases**, roughly 9-15 % of 17,165, mostly in shaders.random,
  indexing, loops and struct. After that, each session gains tens of
  cases.
- **Growth.** The walks keep growing at the observed pace: +20k lines in 3
  weeks, almost all of it in `nir.rs`/`nir_fs.rs`/`vec16.rs`.
- **Words.** "Compiles, not Mesa's" stays at about a third of compiled
  cases unless every audit cause gets its own arm.
- **Risk.** Low per step, but there is no end state. Every random case
  needs a new arm, and the arms interact (#4732, #6433).

### 4.2 (B) General mechanisms first

Build M1 to M3, then M4, in a new front half. Lower the front end's IR to
the new shader NIR, in the way glsl_to_nir lowers it. That NIR carries type
and bit size per value from the start. Then run Mesa's pass loop with the
generated rules, and select through the existing `emit.rs`/BIR back half,
which stays as it is. The new half replaces `nir.rs`, `nir_fs.rs` and
`vec16.rs` (20.8k lines). It does not sit beside them for good.

- **Cost.** About 20-30k lines (M1 8-12k, M2 2-4k, M3 3-5k, M4 4-6k). This
  is the same order as the 20k lines the walks gained in the last three
  weeks.
- **Verification.** The rule stays "Mesa's words, byte for byte".
  - **The regression set.** The 1,506 `*.expect.json` references in
    `tools/mali/testdata/` (57 `tests/*mali*` files) must stay byte-equal
    through the new path.
  - **A shadow mode, as #31's S1b did** ("647 shaders through both paths,
    0 differ"). Both paths compile every corpus and dEQP shader, and a
    shader moves to the new path once the new path matches Mesa where the
    old one matched.
  - **A whole-corpus differential.** It counts equal / differ / refused
    against the drm-shim per commit. The shim is built on this host
    (`~/.cache/yakogl-gate/mali-shim`), so a reference for any shader is
    one `drm_shim_ref.sh run`. The audit `mali-deqp-compiled-audit.md`
    did this by hand for 300 pairs.
- **A relaxed bar ("semantically correct") has no tool in the tree.**
  There is no Valhall interpreter, simulator or differential executor. The
  only tools are the encoder/decoder (`isa/`), Mesa's disassembler
  (`tools/mali/va_disasm_ref.sh`) and pandecode/replay
  (`tools/mali/panthor/`). A relaxed bar would need a BIR-level or
  word-level interpreter, about 3-5k lines over the decode tables
  `isa/v10.rs` already has. Until one exists, byte equality is the only
  checkable bar on the host, and the board is the only semantic check.
- **Risk.** The first few weeks raise no pass count, because the new half
  must reach parity before it pays. The order of Mesa's passes must be
  reproduced exactly (problem 1 below).

### 4.3 (C) A mix (recommended)

Under (C), plan #387's per-family items fall into three groups:
- **Keep and finish:**
  - the items that fix wrong values or defects: the IrIndex fix (187
    cases, a defect in `FsPrecision::of`), the odd-lane half-register
    miscompile (`nir_fs.rs` about 12205, 13 cases computing wrong values),
    the library SIGSEGV;
  - the board rerun, and the `check.sh` row for `tests/mali_deqp_*.rs`;
  - the G310-versus-G610 check (#9141), and `gl_ref.py` attribute bindings
    (#9163);
  - the missing references of #9154;
  - every reference and shape item already done: their Mesa references
    become M1's regression set, and their designs are the record of what
    Mesa does.
- **Land if written, else stop.** These items add arms to the walks:
  intval step 3a (vs16-1/2), constprod step 1, arith step 1, unif2 step 1,
  mixcol step 1, negzero, cfold step A and the fmaloc top step. Each one
  is replaced by M1/M2/M3, so a new one should not be started.
- **Made unnecessary by (B):**
  - intval step 3b and intval-rest step 1 (M2);
  - gl_FragColor lane sources steps 1 and 2 (M1: the lane forms are what
    Mesa's vectorise and algebraic passes make; `mali-fclane-lowering.md`
    is the specification of that output);
  - the fragment and vertex loop items (M4);
  - the remaining constprod/negzero/cfold steps (M1: the generated rules
    and constant folding do them in Mesa's order).

  That is 14 of the plan's 23 open items. The 66-session schedule would be
  replaced by M1 to M4.

### 4.4 The first three work items of (C)

1. **The corpus differential as a tool.** Turn the audit's method into a
   committed host tool:
   - input: every dEQP shader of the measure (the `.qpa` sources that
     `deqp-host-measure.sh` already keeps) plus `tools/shaders` and
     glmark2;
   - per shader: our verdict (refused, equal, differ) against the
     drm-shim reference made on demand;
   - output: one table per commit.

   This becomes the convergence meter for both A and B. Today it is the
   missing measure: the plan counts refusals, not words.
   Size: about 500 lines of script. **Built:** `tools/mali/corpus_check.py`,
   baseline in section 6.
2. **The shader NIR, step 1.** Generalise `blend_nir.rs` into a shader
   NIR:
   - blocks, phis, and int/uint/bool1 types with a bit size per value;
   - `gen_blend_nir_rules.py` widened to the full opcode set of
     nir_opt_algebraic, `_late` and `bifrost_nir_algebraic`;
   - a straight translation from the front end's `ir::Module` for
     **straight-line float shaders, both stages**;
   - `bi_optimize_loop`'s passes in order;
   - a bridge to `emit.rs`.

   Acceptance: in shadow mode, every committed reference the old path
   matches is matched by the new path, and the differential's equal count
   on the corpus does not fall.
3. **Types and precision in the shader NIR.**
   - M2: int and bool through `bool_to_bitsize` and `boolean_bitwise`.
   - M3: the front end's `Prec` kept through the passes that now reset it,
     and Mesa's `lower_precision` rules applied when the NIR is built
     (`mali-precision.md` Part C, steps D.1-D.3).
   - Switch over per stage when the differential's equal count on the new
     path passes the old one.
   - Then M4 (control flow and loops), then M5 (spilling).

## 5. The three hardest problems

1. **Mesa's order, exactly.**
   - *The problem.* Byte equality on arbitrary shaders needs Mesa's pass
     loop reproduced to the instruction: the fixpoint order of
     `bi_optimize_loop`, nir_search's worklist and use-list order, cse and
     vectorize choices, opt_move/sink, and orders that hang on hash slots.
     One example is the constant-promotion threshold, which reads a hash
     table in slot order behind a `qsort_r` that compares pointers
     (`tools/mali/qsort_pointer_order.c`, `fau.rs` `lower_constants`).
   - *Evidence it can be done.* `blend_nir.rs` reproduces nir_search order
     for 4,103 blend keys. Within its scope, the back half is byte-equal
     to Mesa.
   - *Unknown.* How many orders in the remaining passes depend on pointer
     or hash order, which a port cannot see. Whether a 100 % equal rate is
     reachable on shaders.random at all, or whether a residue will need the
     relaxed bar and so the interpreter that does not exist.
2. **Precision per value, through our front end.**
   - *The problem.* Mesa decides fp16 or fp32 per value on GLSL IR
     (`lower_precision`'s visitor works on the GLSL expression tree, before
     NIR), then again in `nir_lower_mediump_io` and the 16-bit folds. Our
     front end lowers to `ir::Module` (`src/shaderc/online/lower.rs`,
     16,746 lines) and keeps a `Prec` table that any rebuilding pass
     resets.
   - *Unknown.* Whether `ir::Module` keeps enough of the GLSL tree for
     `lower_precision`'s rules: built-in result precision, constant
     precision, `++`/`--`, struct members and function parameters (#4536
     lists D.1 to D.3). Whether it changes the other back ends' output,
     which must stay byte-identical. This is the largest compiled-but-not-
     Mesa cause (the mediump vertex arithmetic, about 710 cases of the
     audit).
3. **Control flow and loops at Mesa's words, with registers.**
   - *The problem.* `nir_opt_loop_unroll` and loop_analyze heuristics,
     peephole_select limits, lower_returns, phis of fp16 vectors across
     back edges (`LoopMediump`), and Valhall's block layout and flow
     control all interact. Unrolled loops raise register pressure, and our
     RA has no spiller and no full-file retry (`ra.rs:697`).
   - *Unknown.* How many dEQP loop and indexing cases Mesa itself compiles
     with spills or the full register file. If many do, M5 moves before M4
     finishes. Whether the existing single-block unroller in `nir_fs.rs`
     can be kept as a reference oracle while M4 is built.

## 6. The corpus check: the baseline

`tools/mali/corpus_check.py` is work item 1 of section 4.4. The command and
its modes:

```sh
# every program of the selection (default all three): rows, then the summary
python3 -I tools/mali/corpus_check.py run [--select deqp|testdata|glmark2]... \
    [--sample N --seed K] [--deqp-cases slices|FILE] [--jobs N] [--cache DIR] [--out ROWS]
python3 -I tools/mali/corpus_check.py summary ROWS [--board-refused slices|FILE]
python3 -I tools/mali/corpus_check.py diff OLD_ROWS NEW_ROWS   # exit 1 on a regression
```

- **The corpus.** `deqp`: every dEQP-GLES2 must-pass case, run on the host
  through `deqp-host-measure.sh --cases` (the library of the tree), each
  program read from the `.qpa`. `testdata`: every pair of
  `tools/mali/testdata` against its committed reference (the rule of
  `route.sh`). `glmark2`: the 55 programs of the 33 default scenes
  (`tests/mali_glmark2_corpus.rs`).
- **Our side.** The integration tests' own measure (`tests/mali_ref/mod.rs`
  `measure`): the GL core's entry points in its order, then `differences`
  (words, FAU words, `work_reg_count`, preload, the varying program and its
  offset). It is one rule with `route.sh`, which fixes issue #8783.
- **Mesa's side.** A drm-shim draw (`drm_shim_ref.sh run --uniform-values`,
  then `pan_ref_diff.py extract --draw`), made on demand.
- **The rows.** One row per (program, stage), with these verdicts:
  - EQUAL, DIFFER (with the class of the first difference), REFUSED (with
    the `MaliError` text and the reason class of section 1.1);
  - REJECTED: refused by the front end or the link, and Mesa refuses too
    (dEQP's negative cases);
  - BLOCKED: this stage compiled, the other stage refused;
  - NOREF: compiled, and Mesa drew nothing;
  - ERROR: the comparison could not read the reference;
  - CRASH: the measure's process died.

  A dEQP case takes its worst program.
- **Caches.** Under `~/.cache/yakogl-gate/mali-corpus`, outside the tree:
  - the sources and Mesa's references, by content hash (and by the hash of
    the reference tools);
  - our rows, by the tree key of the compiler (`src/shaderc`, `src/mali`,
    `src/util`, `tests/mali_ref`) and the program;
  - the dEQP run, by the tree key of the library.

  So a rerun is incremental, and a tree on another main measures its own
  side again.

**Where the bulk lives.** The rows of this baseline are
`~/.cache/yakogl-gate/mali-corpus/rows-v0.1.0-2631-g8560b50a3.tsv` on the
gate host that made them (32,526 rows; never committed). `summary` remakes
every table below from that file. The numbers were measured at origin/main
`786cfacd2` plus this branch, which changes no compiler code.

**Wall time on this host** (12 threads, other builds running beside it):

| Run | Wall time |
|---|--:|
| Everything cold: dEQP build and run (161 s), 10,693 Mesa references (1,144 s), our 12,075 programs (63 s) | 1,388 s |
| The board-refused cases alone (`--deqp-cases slices`), cold: 8,879 references (580 s), our side (16 s); the dEQP run (about 150 s) comes on top | 606 s |
| A change of the compiler: the library and the dEQP run again (152 s), our side again (48 s), the references cached | 255 s |
| A change of the tests' module only: our side again | 46 s |
| Nothing changed | 8 s |
| `--sample 200 --seed 387` of the dEQP cases, cached | 5 s |

Per change, the cost is the dEQP run plus our side: about 4 minutes, of
which 50 s is the compile and compare of every program. The references are
made once per program (0.3 s each on 11 threads).

### 6.1 dEQP-GLES2 at the baseline

All 17,165 must-pass cases, by each case's worst program:

| Outcome | Cases | Board-refused cases (the study's 10,597) |
|---|--:|--:|
| EQUAL (every program is Mesa's to the byte) | 3,475 | 2,967 |
| DIFFER (compiled; a program is not Mesa's) | 1,935 | 1,444 |
| NOREF (compiled; Mesa refuses the draw) | 32 | 32 |
| REFUSED | 6,130 | 6,128 |
| REJECTED (a negative case: we and Mesa both refuse) | 873 | 4 |
| NOSHADER (dEQP logs no shader: blend, depth_stencil, clipping, fbo, state queries, ...) | 4,720 | 22 |
| **Compiled: share not Mesa's** | **35.6 % (1,935 of 5,442)** | **32.5 % (1,444 of 4,443)** |

The first refusal by reason class (the classes of section 1.1):

| Reason class | All cases | Board-refused |
|---|--:|--:|
| missing operation | 1,572 | 1,572 |
| reference missing | 1,237 | 1,237 |
| type domain | 1,141 | 1,141 |
| order (algebraic, fold) | 895 | 894 |
| control flow | 626 | 626 |
| precision | 351 | 351 |
| type domain, through a defect (`Defect(IrIndex)`) | 187 | 187 |
| order (link) | 87 | 87 |
| not the code generator (the link's varying offsets) | 11 | 10 |
| deliberate guard (`Invariant`) | 9 | 9 |
| registers and size | 8 | 8 |
| defect (`Defect(Encode)`) | 6 | 6 |

Behind the 6,128 refused cases are 4,902 distinct refusing shaders.

The DIFFER cases:
- by stage, of all 1,935: the vertex program in 1,336, the fragment program
  in 377, and both in 222;
- by the first difference: words in 1,885, FAU words alone in 30, and the
  varying program in 20.

The vertex program is the larger part. In the audit, too, the largest cause
was on the vertex side: the mediump vertex arithmetic, M3. This baseline
does not attribute causes.

By dEQP group (the largest; `summary` lists all of them):

| Group | Cases | EQUAL | DIFFER | REFUSED | Distinct refusing shaders |
|---|--:|--:|--:|--:|--:|
| shaders.operator | 2,949 | 908 | 847 | 1,194 | 1,194 |
| shaders.random | 1,898 | 272 | 111 | 1,513 | 1,413 |
| uniform_api.value | 1,008 | 43 | 124 | 818 | 212 |
| shaders.matrix | 738 | 125 | 226 | 387 | 375 |
| shaders.swizzles | 648 | 240 | 174 | 234 | 113 |
| shaders.loops | 625 | 162 | 7 | 456 | 450 |
| shaders.indexing | 568 | 45 | 61 | 462 | 426 |
| shaders.conversions | 524 | 131 | 170 | 169 | 104 |
| shaders.constant_expressions | 462 | 268 | 43 | 151 | 151 |

### 6.2 The other corpora

- **testdata** (1,391 pairs against their committed references; 1,355
  distinct programs):
  - both stages EQUAL: 625 programs;
  - a stage DIFFER: 35;
  - a stage REFUSED: 693;
  - CRASH: 2, the `lanecycle` stack overflow of
    `tests/mali_deqp_defects.rs`. `route.sh measure` now reports it as a
    REFUSED row and goes on. Before, a share that died ended the whole
    measure.

  By `route.sh`'s program verdicts: EQUAL 646, DIFFER 34, NOREF 7,
  REFUSED 704.
- **glmark2** (55 programs, 48 distinct):
  - 14 of 33 scenes compile clean, as `tests/fixtures/mali_glmark2_verdicts.txt`
    says.
  - 8 scenes are Mesa's to the byte.
  - 4 programs DIFFER: the buffer scenes' vertex program, conditionals
    `fragment-steps=5`, and loop `vertex-steps=5`.
  - The 8 programs of `ideas` are ERROR. They declare uniforms through a
    macro precision (`uniform MEDIUMP_OR_DEFAULT float ...`), which
    `tests/mali_ref`'s `uniform_names` does not read, so their uniform
    words cannot be computed. This is a limit of the comparison, not a
    verdict.

### 6.3 Against the study's numbers

**"4,471 of 10,597 board-refused cases compile".** Here 4,443 compile
(EQUAL + DIFFER + NOREF), 6,128 are REFUSED, 22 are NOSHADER and 4 are
REJECTED. That is 4,469 not refused, against the study's 4,471. The
difference, case by case against the study's `measure.tsv`:
- **10 cases the study counts as compiling are REFUSED here.** The link
  refuses them with "a Mali link whose stages place a varying at different
  offsets". The study counted only the code generator's text ("online
  shaderc: the code generator refused"), so a link refusal looked like a
  compile to it.
- **8 cases the study counts as refused compile here, and DIFFER.** They are
  `attribute_location.bind_aliasing.cond_*`. dEQP binds their attribute
  locations, and the board refuses them with `Defect(Encode)`. The measure,
  like Mesa's reference draw, binds none.
- **22 cases are NOSHADER** (`fragment_ops.scissor.clear_*` and the like).
  dEQP logs no shader source for them, so there is no program to measure.
  The study counted them as compiling.
- **4 cases are REJECTED** (`shaders.constants.const_float_assign_*`). Our
  front end refuses them, and so does Mesa. The study did not count front
  end refusals as refusals.

The refusal texts agree with section 2.1 to the case. For example, fs
`Arithmetic` is 890, `ColourForm` 821, vs `UniformType` 462 and
`Defect(IrIndex)` 187, and the distinct refusing shaders are 4,902 against
4,901.

**"About a third of compiled cases differ from Mesa"** (the audit's 300
sampled pairs, 108 not Mesa's). Over every compiled board-refused case the
share is 32.5 % (1,444 of 4,443), and over the whole must-pass list 35.6 %.
The audit's estimate holds, now measured rather than sampled.

- The vertex program is the larger part: 1,336 of the 1,935 DIFFER cases
  differ only there.
- 30 cases differ only in FAU words. These are real differences, for
  example:
  - Mesa packs a uniform-array compare's constants as fp16 pairs where we
    keep an fp32 word;
  - the last bit of a folded constant (`length`, `degrees`);
  - `gl_MaxVertexUniformVectors`, whose value differs.

The next measure of the plan is `diff` of two row files: what moved
refused -> equal, refused -> differ, and equal -> differ, the last being a
regression.
