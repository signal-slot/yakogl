<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali precision: what the front end keeps of Mesa's `lower_precision` rules

Issue #11254 (part of #11251, M3 of `docs/notes/mali-compiler-road-to-es2.md`
section 3, its hardest problem 2; related #4536). A read-only study: can
Mesa 26.2.1's precision rules (GLSL IR `lower_precision`, then the NIR
passes panfrost runs) be applied on what our front end hands on? No code
under `src/` changed.

Citations: Mesa paths are relative to the Mesa 26.2.1 tree (`src/...`).
`LP` is `src/compiler/glsl/lower_precision.cpp`. Our paths are under
`src/shaderc/online/` unless written in full, at origin/main `92b2601d5`.

Marks: **[probe qN]** was drawn on the drm-shim reference (Mesa 26.2.1,
GPU id 0xa867) for this note, the vertex NIR read from the run's `.stderr`
(the shaders are in section 5). **[probe pN]** is a probe of
`docs/notes/mali-precision.md` A.9, not rerun. **[read]** is read from the
source only.

## 0. Answer in brief

- **Yes, the rules can be applied.** Steps D.1 and D.2 of
  `mali-precision.md` are on main: `precision.rs` runs Mesa's class rule
  (`find_lowerable_rvalues`) on the syntax tree, before inlining, and
  `lower.rs` records the result per instruction (`Module::prec`) and per
  slot (`Module::local_prec`). Every rule of `LP` has a place in that
  scan (section 1). Mesa's conversions (`f2fmp`, `f2f32`) are not in the IR
  by design; they follow from the classes on each edge (a Medium user of a
  High value narrows it, a High user of a Medium value widens it).
- **What is lost, and where:**
  1. **All of it, on the Mali path, as soon as `mali/glsl.rs` rewrites
     anything.** `glsl::passes` (Mesa's `do_rebalance_tree` and
     `do_algebraic`, and the index ladders) returns a module with empty
     tables (`mali/glsl.rs:135-138`, `:154-161`). Every `prec_of` is then
     `Unknown`, and the vertex walk's `== Prec::Medium` guards fall through
     to the fp32 arms without a refusal (`mali/nir.rs:4905-4920`) [read].
  2. **One fact Mesa's GLSL passes read is not used.** `do_rebalance_tree`
     runs after `lower_precision`, so a tree with an `f2fmp` inside it is
     no reduction. `glsl.rs`'s `reduction` (`mali/glsl.rs:1156-1205`) does
     not look at precision [Mesa side: probe q8, q8h, q9; our side: read].
  3. **Five sites record `Unknown` on purpose** where the front end folded
     or shared a value Mesa keeps apart (section 2.2). `Unknown` is meant
     to make the reader refuse (`ir.rs:850-853`), but today's readers
     treat it as "not Medium" and build fp32 (section 2.1).
- **Smallest change:** carry and read the two tables in `mali/glsl.rs`
  and refuse `Unknown` in its readers (no front-end file; estimate 170-290 lines with tests). Optional, in the
  front end: replace two of the five `Unknown` sites with a class (estimate
  20-60 lines). Section 4.
- **Risk to the other back ends:** none from the `glsl.rs` change (Mali
  only). The front-end options change only the side tables, which no file
  outside `mali/` reads (section 4.3).

## 1. The rules of `lower_precision`, one per row

Mesa runs `lower_precision` right after `ast_to_hir`, before
`lower_builtins` and before any GLSL optimisation
(`src/compiler/glsl/glsl_parser_extras.cpp:2487-2497`). It has two halves:
`find_lowerable_rvalues` classes each node High (CANT_LOWER), Medium
(SHOULD_LOWER) or None (UNKNOWN) and finds the roots (`LP:184-620`);
`lower_variables` retypes the variables (`LP:961-1352`).

"Ours" says where our scan classes the form (`precision.rs`) and where the
lowering records it (`lower.rs`, `ir.rs`). "Kept" says whether `ir::Module`
(or the reflection beside it) still holds what the rule reads.

### 1.1 Leaves

| # | Rule | Mesa | What it reads | Ours | Kept |
|---|---|---|---|---|---|
| L1 | A float or bool constant is None, so it joins its parent; inside a lowered tree it becomes its fp16 value | `LP:335-346`, `:736-752` | the type | `precision.rs:817`; the value is `Op::ConstF32` (32-bit bits), its width the user's class | yes; shared constants: 2.2 S2 |
| L2 | An int or uint constant is High (no `int16` cap) | `LP:153-182`, `:339-340`; `src/gallium/drivers/panfrost/pan_screen.c:591` | the type | `precision.rs:816`; an int value records `None` (`lower.rs:9171`) | yes |
| L3 | A variable read is its declared precision; the declared precision is the qualifier, else the default in scope | `LP:348-358`; `src/compiler/glsl/ir.h:2089-2092`; `src/compiler/glsl/ast_to_hir.cpp:2764-2800` | the variable's precision | `precision.rs:819-827` (scope table), `:270-285` (`handle`) | yes: the read records its tree's class; storage is `local_prec`, `IoDecl::prec` (`ir.rs:773`), and for uniforms `reflect::UniformDecl::precision` (`reflect.rs:843`), not `ir::UniformDecl` (`ir.rs:798`) |
| L4 | Built-in variables have fixed precisions (`gl_FragCoord` mediump in 1.00, `gl_PointSize` mediump in 1.00, ...) | `src/compiler/glsl/builtin_variables.cpp` (`mali-precision.md` A.2) | the name and version | `precision.rs:651-680` | yes |
| L5 | A struct member read is the member's precision (the default where the struct was declared); the struct read under it is a separate tree | `LP:361-370`, `:315-333`; `ir.h:2183-2188`; `ast_to_hir.cpp:8096` | the field's precision | `precision.rs:683-701`, `:909-919`; a struct slot is stored at full width (`lower.rs:7949-7951`) | yes [probe q3] |
| L6 | An array element read is the array's precision; the index is a separate tree | `LP:372-381`, `:315-333`; `ir.h:2138-2146` | the array's precision | `precision.rs:896-907`, mark `:1146-1150` | yes [probe q5]; run-time index into a folded `const`: 2.2 S4 |
| L7 | A texture look-up is its sampler's precision (lowp by default); the coordinate, lod and bias are separate trees; with `glsl_16bit_load_dst` the result is 16-bit | `LP:383-394`, `:315-333`, `:471-487`; `pan_screen.c:589` | the sampler variable's precision | `precision.rs:803-810`, `:1037-1043`; the `Tex` records it; a projective divide is the coordinate's tree (`lower.rs:4393-4396`) | yes [probe p4] |

### 1.2 Operations

| # | Rule | Mesa | What it reads | Ours | Kept |
|---|---|---|---|---|---|
| O1 | An operation joins its operands: a High one makes it High, else a Medium one Medium | `LP:224-283`, `:396-415` | the operands' states | `precision.rs:840-852` (`join`, `:241-248`) | yes: each `Alu` records its tree's class |
| O2 | An operation of int or uint type is High; a Medium operand under it is a root of its own (`int(m * 2.0)`) | `LP:400-401`, `:214-221` | the result type | `precision.rs` `typed`; `lower.rs:9162-9190` | yes |
| O3 | A swizzle joins its operand (no visitor case, combined edge) | `LP:315-333` | nothing | `precision.rs:917-919`; in the IR a swizzle is part of `Src`, no instruction | yes |
| O4 | A derivative joins like any operation when `fp16_derivatives` is on (arch 6 and up) | `LP:404-415`; `pan_screen.c:587-588` | the cap | `precision.rs:1025` | yes |
| O5 | `b2f` inside a lowered tree is `b2f16`, `f2b` is `f162b` | `LP:792-815` | the operation | `B2f`/`F2b` record the tree's class | yes (by edge) |
| O6 | The root: the topmost Medium node of a joined chain; it gets `f2f32` unless its type is bool; a bare dereference root gets nothing; each read below it gets `f2fmp` | `LP:252-282`, `:819-856`, `:723-735` | the tree | `precision.rs:1068-1110` (`mark`) | yes, as edges: no conversion is an instruction (`mali-precision.md` C.4) |
| O7 | The target of an assignment is High; the index of an indexed target is not a target | `LP:198-212`; `src/compiler/glsl/ir_hv_accept.cpp:274-280`, `:309-311` | the position | `precision.rs:853-870`, `:1120-1126` | yes |

### 1.3 Temporaries, calls and assignments

| # | Rule | Mesa | What it reads | Ours | Kept |
|---|---|---|---|---|---|
| T1 | A compiler temporary is None until assigned; the first Medium assignment makes it Medium; any non-constant assignment that is not Medium makes it High | `LP:584-610` | each assignment's class | `precision.rs:949-963` (`temp_rule`) | yes |
| T2 | A constructor of more than one value is `vec_ctor`/`mat_ctor`: T1 over its non-constant arguments; one scalar is a swizzle (joined) | `src/compiler/glsl/ast_function.cpp:1511`, `:1705`, `:1528-1530` | the arguments | `precision.rs:973-997`; a Medium temporary that becomes a slot is stored at 16 bits (`lower.rs:9211-9222`) | yes [probe p3, p3b] |
| T3 | `c ? a : b` is `conditional_tmp`, T1 over both sides; the condition is a root of its own; a constant condition chooses a side at `ast_to_hir` time | `ast_to_hir.cpp:1904-1990` | both sides | `precision.rs:871-895`, `:1127-1145`; lowered to `Bcsel` or to an `If` into a slot (`lower.rs:3507-3541`) | yes [probe p6a, p6b] |
| T4 | `&&`, `||` with a right side that emits code are `and_tmp`/`or_tmp` | `ast_to_hir.cpp:1675`, `:1709` | the right side | `precision.rs:840-848` | yes |
| T5 | `++`/`--` is `x = x + 1` (a tree of its own); the value of `x++` is `_post_incdec_tmp` | `ast_to_hir.cpp:1358`, `:2012-2045`, `:1112` | `x`'s precision | `precision.rs:828-839`, `:1111-1114`; `lower.rs:5502` (`step_value`), `:5550` (`step_by_one`) | yes [probe q1] |
| T6 | A compound assignment `l op= r` is `l = l op r`: one tree, the class of the operation | `ast_to_hir.cpp:1756-1790` | `l` and `r` | `precision.rs:853-870` | yes [probe q2] |
| T7 | An assignment's right side is its own tree; the target's precision does not lower it (only its storage converts) | `LP:198-212`, `:1114-1193` | the right side | `precision.rs:853-870` | yes [probe q2, p3] |
| T8 | A user function call is its declared return precision; the body is classed once, by its own declarations | `LP:461-463`, `:544-581` | the signature | `precision.rs:1013-1023`, `:539-560` | yes: the inlined body records the body's classes [probe p2] |
| T9 | A parameter is read at its declared precision and stored at 32 bits; a 16-bit argument of an `out`/`inout` parameter converts both ways | `LP:1025-1036`, `:1283-1352` | the parameter | `precision.rs:539-560`; an unassigned `in` parameter is an alias of the argument's value (`lower.rs:4757-4767`), any other a full-width slot (`lower.rs:4775`) | yes: the alias is a value, so each use converts by edge [probe q4] |
| T10 | A returned value of a 16-bit variable goes through a 32-bit temporary | `LP:1195-1224` | the variable | the result slot or value of the inlined call | yes |
| T11 | A built-in call with no fixed precision is Medium when every non-constant argument is a Medium root, else High; a Medium call is inlined with a lowered copy of its body (unqualified parameters mediump) | `LP:489-541`, `:858-930` | the arguments | `precision.rs:1044-1063`; every step of the expansion records the call's class (`lower.rs` `Sink`), `pow`, `asin`, `acos` High (`lower.rs:4382-4386`) | the class yes [probe q10]; the steps are our expansion, not Mesa's body (M1's work, `mali-precision.md` E.3) |
| T12 | Built-ins of fixed precision: `floatBitsTo*`, `*BitsToFloat` High; the pack built-ins' arguments highp | `LP:495-500`; `src/compiler/glsl/builtin_functions.cpp` | the name | `precision.rs:1032-1036`; `lower.rs:4390-4392` | yes |
| T13 | A call whose arguments are all constant is folded at 32 bits; a constant argument is folded to a constant | `ast_function.cpp:659`, `:61` | constness | `precision.rs:1025-1031` (`FOLDS`, `FOLDED`); `lower.rs:4429-4480` (`full_width_args`) | partly: 2.2 S3 |

### 1.4 Storage and scope

| # | Rule | Mesa | What it reads | Ours | Kept |
|---|---|---|---|---|---|
| S1 | Stored at 16 bits: a mediump or lowp float local, global, `const` or Medium temporary; a mediump or lowp float uniform (or array of them) outside a block, with `fp16_const_buffers`; not a struct; not inputs, outputs, parameters, ints or bools | `LP:1022-1060`; `pan_screen.c:588` | the variable | `local_prec` (`lower.rs:7949-7961`, `:9211-9222`); uniforms by `reflect` (`mali/nir.rs` `fp16_uniform`) | yes; a folded `const` is kept at 32 bits on purpose (2.2 S4) [probe q5: a mediump uniform array is fp16] |
| S2 | A `const` initialiser is lowered to fp16 with `glsl_16bit_consts` | `LP:1038-1053`; `pan_screen.c:586` | the initialiser | as S1 | yes |
| S3 | Storage alone rounds nothing after NIR's folds (`f2f32(f2fmp(a@32))` is `a`) | `src/compiler/nir/nir_opt_algebraic.py:1991` | NIR | the edges | yes [probe q2: `m += p.y` is one 32-bit `fadd` of `p.x`] |
| S4 | A `precision` statement replaces a default that exists in any scope; only a type with no default gets one scoped to the block | `src/compiler/glsl/glsl_symbol_table.cpp:194-207` | statement order | `precision.rs:350-376`, `:514` | classed yes; but the lowering refuses a `precision` statement inside a body ("this statement is not lowered yet", `lower.rs:7880`), for every back end |
| S5 | `invariant` outputs: `propagate_invariance` marks the trees exact, and NIR's `~` rules (the late mediump rules among them) skip exact values | `glsl_parser_extras.cpp:2597`, `:2610`; `nir_opt_algebraic.py:4170-4186` | the output | `IoDecl::invariant` (`ir.rs:767`); not per value | derivable: walk back from the `StoreOutput`, as `mali/glsl.rs:1209-1226` already does |

### 1.5 The GLSL passes after the rule

Mesa's compile-time GLSL passes run after `lower_precision`, on the lowered
tree (`glsl_parser_extras.cpp:2491`, then `opt_shader` at `:2291`, the
order `:2597-2610`: grafting, then `do_rebalance_tree`, then
`do_algebraic`). `is_reduction`
(`src/compiler/glsl/opt_rebalance_tree.cpp:201-257`) refuses a tree with a
second operation or a second type, so:

- a tree of mediump **attribute** reads has an `f2fmp` on each read and is
  **not** rebalanced: `a + b + c + d` stays `((a+b)+c)+d` at 16 bits
  [probe q8]; the same with highp attributes is `(a+b)+(c+d)` at 32 bits
  [probe q8h];
- a tree of **16-bit locals** read more than once (so not grafted) has no
  `f2fmp` (`LP:1226-1260` removes it) and **is** rebalanced at 16 bits
  [probe q9]. A local read once is grafted first, which brings its `f2fmp`
  in [read].

Our `mali/glsl.rs` runs these passes on `ir::Module`. Its `reduction`
(`mali/glsl.rs:1156-1205`) compares the operation and the lane count, not
the precision; the class tables it would need are in its input module, but
it does not read them. This rule is lost today [read for our side; Mesa
side probe q8, q8h, q9].

## 2. What resets or drops `Prec`

### 2.1 Passes that rebuild a module

The rule is written at `ir.rs:907-922`: a pass that renumbers or rewrites
instructions or locals carries the tables entry for entry, or gives empty
ones (then `has_prec` is false and every answer is `Unknown`).

| Pass | File | Tables | On the Mali path |
|---|---|---|---|
| `relocate` | `lower.rs:655-830` | carried (`:820-821`) | yes |
| `zero_varyings` | `lower.rs:862-912` | carried (one load becomes `ConstF32`, same id) | yes |
| `lodkey::remap_mag_switch` | `lodkey.rs:129-195` | carried | yes |
| `glsl::passes` (`ilt_trees`, rebalance, reassociate, the negation rules) | `mali/glsl.rs:135-138`, `:154-161`; called at `mali/mod.rs:1282-1284`, `:1927-1928` | **dropped** when anything changed | **yes: the only one** |
| PowerVR `unroll` | `powervr/unroll.rs:286-295` | dropped | no |
| PowerVR `abi` | `powervr/abi.rs:227`, `:375` | instructions replaced, tables kept as they were (`has_prec` false when the length moved) | no |
| Vivante `wide128` | `vivante/wide128.rs:446-455` | dropped | no |
| Vivante `int_tex` | `vivante/int_tex.rs:44` | instructions replaced, tables kept | no |

What happens after the drop: the readers in `mali/nir.rs` test
`prec_of(..) == Prec::Medium` (`:3321`, `:3358-3359`, `:3467`, `:3626`,
`:4827`, `:4870-4871`, `:4907-4930`, `:5478`) and `nir_fs.rs:805` tests
`== Prec::High`. None of them refuses `Unknown`, so a vertex shader whose
trees `glsl.rs` changed is built on the fp32 arms [read; not run].

### 2.2 Values the lowering records as `Unknown`

| Site | What happens | Mesa | Why the front end cannot say |
|---|---|---|---|
| S1 folded reciprocal, `lower.rs:5937-5958` | `x / c` with a constant `c` is folded to `x * (1/c)` at fp32 | in a 16-bit tree, `fdiv` by the fp16 `c`, folded at 16 bits | keeping `Frcp` moved PowerVR words (`lower.rs:16772-16779`); `Unknown` when `1/c` is not exact in fp16 |
| S2 shared pinned constant, `lower.rs:5990-6000` | one `ConstF32` read by a 16-bit and a 32-bit tree in one run | each read is its own constant | High is right for a plain constant (each user converts); only a constant pinned to its fp16 value (S3) becomes `Unknown` |
| S3 constant into a full-width built-in, `lower.rs:4429-4480` | an argument of `pow`, `asin`, `acos` in a Medium call that is a folded constant, or not known to vary | the parameter's `f2fmp` of the constant is folded to the fp16 value, then read at 32 bits | the instruction stream has one 32-bit constant; the "rounded, then widened" use has no form |
| S4 run-time index into a folded `const` mediump, `precision.rs:896-907` | the folded reads are 32-bit values; the indexed read is of Mesa's 16-bit storage | a 16-bit array | the lowering keeps no 16-bit slot for it (`lower.rs:7951-7961`; a `const` array has no slot at all, `Run::Constants`, `lower.rs:7930-7947`) |
| S5 a `?:` of a constant condition not evaluated, `precision.rs:880-890` | `ast_to_hir` may fold the condition; the scan evaluates only the measured forms | the chosen side | `Scan::constant` (`precision.rs:720-790`) is partial by design |

Besides these, the scan's own `Unknown` (a value with no context,
`lower.rs:9178-9187`) is a front-end defect counter, tested to be zero on
the corpus (`precision.rs:1679`, test
`every_value_of_the_corpus_has_its_kind_s_class`).

## 3. The NIR-level passes panfrost runs, and what each needs

| Pass | Runs for panfrost? | Needs from the front end | Kept |
|---|---|---|---|
| `nir_lower_mediump_vars` | **no**: it runs only when `fp16` and `int16` are both on (`src/compiler/glsl/gl_nir_linker.c:1362-1365`); panfrost has `int16` off (`pan_screen.c:591`) | nothing | - |
| `nir_lower_mediump_io` | fragment stage only (`src/panfrost/compiler/bifrost/bifrost_nir.c:912-916`), inputs and outputs, except the varyings in `bi_fp32_varying_mask` (flat, or x and y of one load used as a texture coordinate, `bifrost_nir.c:450-500`) | each varying's precision (`IoDecl::prec`, `ir.rs:773`), its interpolation (`IoDecl::interp`); for an output store, whether the stored value is an `f2f32` (a 32-bit output fed by a Medium root is stored at 16 bits too, `src/compiler/nir/nir_lower_mediump.c:87-100`) | yes: the store's operand class gives the `f2f32` [read] |
| vertex outputs | stored at 32 bits: the varying layout is collected with `lower_mediump` false (`src/gallium/drivers/panfrost/pan_shader.c:588`) | nothing | - |
| `glsl_16bit_consts` | yes (`pan_screen.c:586`): `const` initialisers lowered (`LP:1038-1053`) | S1/S2 | yes |
| `glsl_16bit_load_dst` | yes (`pan_screen.c:589`): the look-up result width is the sampler's (`LP:450`, `:484`) | L7 | yes |
| `fp16_const_buffers` | yes from arch 6 (`pan_screen.c:588`) | the uniform's precision and type (plain float, not in a struct) | beside the module (`reflect.rs:843`) |
| `nir_opt_algebraic` mediump folds (`nir_opt_algebraic.py:1971`, `:1991`, `:1322`, `:1980`, `:2002`) and the contraction rule (`:2054-2060`) | yes | the conversions at the roots and at storage (O6, S1), and `contract`/exact (S5) | yes, as edges; exact by the walk of S5 |
| late mediump rules and `f2fmp` to `f2f16` (`nir_opt_algebraic.py:4164-4193`) | yes (`bifrost_nir.c:338`, `:365`), `preserve_mediump` off | the same; the rule fires only where NIR still has a scalar op (`mali-precision.md`, #4536 comment of 2026-11-12) | yes [probe q9: `ma * mb` of two 16-bit locals is a 32-bit `fmul`] |
| `bi_lower_bit_size` | yes (`bifrost_nir.c:35-80`, `:330`) | the operation (`fpow`, `fexp2`, `flog2`, `fsin`, `fcos` at 32 bits) | `pow` is expanded by the front end; recorded High instead (`lower.rs:4372-4386`) |
| `pan_nir_lower_bool_to_bitsize` | yes (`bifrost_nir.c:350`) | the compare widths | yes (O1, O5) |
| `nir_fold_16bit_*`, `nir_opt_16bit_tex_image` | **no** (no call in `src/panfrost`, `src/gallium/drivers/panfrost`) | nothing | - |

## 4. The smallest change

### 4.1 What is needed

The front end already keeps the classes (section 1). What is missing sits
after it, on the Mali side:

1. `mali/glsl.rs` carries `prec` and `local_prec` through its rewrites.
   Every rewrite stays inside one tree, so a new or moved instruction
   takes the tree's class: rebalance and reassociate move operands of one
   operation of one type (`opt_rebalance_tree.cpp:243-247`, Mesa's own
   rule); the negation rules replace one instruction by one of the same
   meaning; `ilt_trees` builds int compares (`None`) and `Bcsel`s of the
   element it replaces (that element's class). `glsl.rs` adds no slots, so
   `local_prec` is copied as it is. Estimate: 80-120 lines.
2. `mali/glsl.rs` reads the classes to stop a reduction where Mesa's tree
   holds a conversion: an operand whose value is not 16-bit storage (an
   attribute, a varying, a 32-bit local, a highp uniform, a High value)
   read by a Medium operation. Estimate: 30-50 lines.
3. The readers in `mali/nir.rs` and `mali/nir_fs.rs` (section 2.1)
   refuse a value whose class is `Unknown`, as `ir.rs:850-853` asks,
   instead of taking the fp32 arm. This can turn some shaders that compile
   today into refusals; which ones was not measured. Estimate: 20-40
   lines.
4. Tests: probes q8, q8h and q9 as committed references (their words),
   plus a unit test that `has_prec` holds after `glsl::passes` on each
   rewrite. Estimate: 60-80 lines.

Total about 170-290 lines, all in `src/shaderc/online/mali/`. This is the
change #11253's shader NIR needs too: the GLSL passes come before NIR in
Mesa, so the new builder reads the module after them.

### 4.2 Optional front-end changes (close two `Unknown` sites)

- **S4**: give a mediump `const` array's run-time-indexed reads the class
  of 16-bit storage (Medium on the ladder's values) instead of `Unknown`,
  in `precision.rs:896-907` and where the ladder is built. Mesa's
  `lower_constant` (`LP:993-1020`) stores the array at fp16, so a highp
  user reads the rounded element. Estimate: 20-30 lines and a probe. Not
  verified against Mesa.
- **S1**: a sparse side table that names each folded reciprocal and its
  divisor, so the Mali reader can fold at fp16 as Mesa does. A new
  `Module` field means each literal that lists every field gains it
  (`Module::empty`, `ir.rs:931`; `lower.rs:990`; the others found use
  `..`, but the tests in `tests/` were not searched). Estimate: 40-60
  lines.

S2 and S3 need a "round, then widen" mark on one use of a constant, which a
per-value table cannot hold; they stay `Unknown` until the shader NIR has
per-use constants (#11253). S5 shrinks as `Scan::constant` learns forms.

### 4.3 The other back ends stay byte-identical

- **The `glsl.rs` change** is in `mali/`, which no other back end builds
  or calls.
- **The front-end options** write only `prec`, `local_prec` or a new side
  table. The only readers of `prec_of`, `local_prec_of` and `has_prec`
  outside `ir.rs`, `precision.rs` and `lower.rs` are in `mali/`
  (`grep -rn "prec_of\|local_prec_of\|has_prec" src/shaderc/online`:
  `mali/nir.rs`, `mali/nir_fs.rs`, `mali/glsl_tests.rs`; `lodkey.rs` only
  copies). The instruction stream, `comps`, `lanes` and `nodes` do not
  change.
- **How the existing tests show it.** `lower.rs`/`ir.rs`/`precision.rs`
  reach every row of `tools/check.sh` (CLAUDE.md: `src/shaderc` reaches
  everything), and these rows compare words: `powervr_word_pins`
  (`lower.rs:16772-16800`: the PowerVR words of the shapes precision work
  once moved, pinned to main's), the V3D Mesa pins
  (`tests/fixtures/v3d_mesa_pins.txt`, `v3d_mesa_programs*.txt`), the
  online refusal lists per GPU (`tests/fixtures/online_refused_*.txt`,
  `tests/online_corpus.rs`) and the GC7000 differential
  (`tests/backend_differential.rs`, values to 1e-4, not words).
- **Gap:** I found no whole-corpus word snapshot for the PowerVR and
  Vivante online compilers (only the pins and the refusal lists). A
  front-end change that moved a word outside the pinned shapes could pass
  these rows. Not checked further.

## 5. Probes run for this note

Each vertex shader has `attribute highp vec4 p; varying highp vec4 v;`
and writes `gl_Position = p;`. The fragment shader is `precision mediump
float; varying highp vec4 v; void main() { gl_FragColor = v; }`. Run as
`tools/mali/drm_shim_ref.sh run $W qN.vert fs.frag qN` (`$W` a shim
install); the result is the second vertex NIR in `qN.stderr` (the variant
that stores the varyings). The files are not committed.

| Probe | Body of `main` (after `gl_Position = p;`) | Mesa's vertex NIR | Rule |
|---|---|---|---|
| q1 | `mediump float m = p.x; m++; v = vec4(m);` | `16 f2f16 p.x`, `16 fadd .., 0x3c00`, `32 f2f32` | T5 |
| q2 | `mediump float m = p.x; m *= 2.0; m += p.y; v = vec4(m);` | `16 f2f16 p.x`, `16 fadd %2, %2` (the `* 2.0`), `32 f2f32`, `32 fadd .., p.y` | T6, T7, S3 |
| q3 | `struct S { mediump float x; highp float y; }; uniform S s;` `v = vec4(s.x * 3.0, s.y * 3.0, 0.0, 1.0);` | one `32x2 load_ubo` (the struct is fp32), `16 f2f16 .x`, `16 fmul 0x4200`, `32 f2f32`; `32 fmul .y, 3.0` | L5, S1 |
| q4 | `float f(mediump float x) { return x * 3.0; }` `v = vec4(f(p.x));` | `16 f2f16 p.x`, `16 fmul 0x4200`, `32 f2f32` | T8, T9 |
| q5 | `uniform mediump vec4 mu[4]; attribute highp float i;` `v = mu[int(i)] * 3.0;` | `32 f2i32 i`, `16x4 load_ubo` (fp16 storage), `16x2 fmul` x2, `32 f2f32` x4 | L6, O2, S1 |
| q8 | `attribute mediump float a, b, c, d;` (four declarations) `v = vec4(a + b + c + d);` | `16 fadd a, b`, `16 fadd .., c`, `16 fadd .., d`, `32 f2f32`: not rebalanced | 1.5 |
| q8h | q8 with `highp` | `32 fadd a, b`, `32 fadd c, d`, `32 fadd`: rebalanced | 1.5 |
| q9 | highp `a, b, c, d`; `mediump float ma = a;` (and `mb`, `mc`, `md`) `v = vec4(ma + mb + mc + md, ma * mb, mc * md, 0.0);` | `16 fadd ma, mb`, `16 fadd mc, md`, `16 fadd`, `32 f2f32`; `32 fmul a, b`, `32 fmul c, d` | 1.5, late rule |
| q10 | `mediump float m = p.x;` `v = vec4(sqrt(m), sqrt(p.y), sqrt(m) * p.z, 0.0);` | `16 frsq`, `16 frcp`, `32 f2f32`; `32 frsq p.y`, `32 frcp`; `32 fmul .., p.z` (the Medium call result widened once) | T11 |

## 6. A test list

Each rule with the shader that pins it and the width Mesa gives. "Verified"
is a probe above or in `mali-precision.md` A.9; "read" is derived from the
source only. dEQP-GLES2 names are from `VK-GL-CTS`
(`data/gles2/shaders/*.test`, `modules/gles2/functional/es2fShader*Tests.cpp`).

| Rule | Shader | Mesa's width | Status | dEQP-GLES2 |
|---|---|---|---|---|
| L1 | p3b: `vec2(m, 2.0) * 1.5` | 2.0 and 1.5 as fp16, `fmul` 16 | verified (p3b) | `shaders.operator.binary_operator.mul.mediump_float_vertex` |
| L2 | `int(m * 2.0)` | `fmul` 16, `f2f32`, `f2i32` | read | `shaders.conversions.scalar_to_scalar.float_to_int_vertex` |
| L3 | p1c: `const mediump float c = 1.3; v = c * a;` | 32, `c` unrounded | verified (p1c) | `shaders.constants.const_float_assign_varying` (not checked to be this form) |
| L4 | `gl_FragCoord.xy / 64.0` in ES 1.00 | 16 | verified (`prec_84`, `mali-precision.md` Part 2) | `shaders.builtin_variable.fragcoord_xyz` (not checked) |
| L5 | q3 | member mediump 16, member highp 32, struct storage 32 | verified (q3) | `shaders.struct.uniform.basic_vertex`, `shaders.struct.local.basic_vertex` |
| L6 | q5 | element 16 (fp16 storage), index 32 | verified (q5) | `shaders.indexing.uniform_array.float_dynamic_read_vertex` (name not checked) |
| L7 | p4: `texture2D(s, p.xy).x * 1.5` | look-up 16 | verified (p4) | `shaders.texture_functions.vertex.texture2d` (name not checked) |
| O1 | `m * b` (b highp) | 32 | verified (p3) | `shaders.operator.binary_operator.mul.highp_float_vertex` (mediump tail, audit V1) |
| O3 | `m2.yx * m2` | 16 | read | `shaders.swizzles.vector_swizzles.mediump_vec2_yx_vertex` |
| O5 | `bool(m)`, `float(b)` in a Medium tree | `f162b` / `b2f16` | read | `shaders.conversions.scalar_to_scalar.bool_to_bool_vertex` (audit V1) |
| O6 | a bare mediump read stored highp | no conversion pair | verified (q2 tail) | - |
| O7 | `arr[int(m * 2.0)] = x` | index `fmul` 16 | read | - |
| T1, T2 | p3 / p3b | 32 / 16 | verified | `shaders.conversions.vector_combine.vec2_vec2_to_vec4_vertex` |
| T3 | p6a / p6b | `bcsel` 32 / 16; `flt` 32 | verified | `shaders.operator.selection.mediump_float_vertex` |
| T4 | `(m > 0.5) && f(m)` | `and_tmp` Medium | read | `shaders.operator.binary_operator.logical_and.*` (not checked) |
| T5 | q1 | `fadd` 16 | verified | `shaders.operator.unary_operator.pre_increment_effect.mediump_float_vertex`, `post_increment_result.lowp_int_vertex` (int: 32) |
| T6 | q2 | `*=` 16, `+=` highp 32 | verified | `shaders.operator.binary_operator.mul_assign_effect.mediump_float_vertex`, `add_assign_effect.*` |
| T8 | p2 | body 32, `f2f16`, caller 16 | verified (p2) | `shaders.functions.declarations.default_vs_explicit_precision_vertex` |
| T9 | q4; an `out`/`inout` lowp parameter | body 16, parameter storage 32 | q4 verified; `out`/`inout` read | `shaders.functions.qualifiers.in_lowp_float_vertex`, `out_lowp_float_vertex`, `inout_lowp_float_vertex` |
| T11 | q10; `normalize(m)` | 16; `pow` 32 | q10 verified; `pow` from #4536 comment | `shaders.operator.exponential.sqrt.mediump_vec4_vertex` (audit V1), `shaders.operator.exponential.pow.mediump_float_vertex` |
| T12 | `floatBitsToInt` | ES 3 only | read | - |
| T13 | `sin(0.5) * m` | `sin` folded at 32, then rounded as a constant of a 16-bit `fmul` | read | `shaders.constant_expressions.builtin_functions.*` (not checked) |
| S1 | q5, q9 | fp16 uniform array; 16-bit locals | verified | `shaders.uniform_api` cases with a mediump array (not checked) |
| S4 | `{ precision mediump float; }` in a vertex body, then `float u` | `u` 16 | verified (`mali-precision.md` probes v_inner2, v_inner4) | `shaders.scoping.*` (not checked); refused by the lowering today |
| S5 | `invariant varying` of a Medium tree | exact: no late rule | read | `shaders.invariance.*` is ES 3; ES 2: `tests/es2_invariance.rs` |
| 1.5 | q8, q8h, q9 | not rebalanced / rebalanced / rebalanced at 16 | verified | `shaders.random.*` (trees of three or more additions; not checked which) |

## 7. Not checked

- Our compiler was not run on q1-q10 (no `cargo`, no `tools/check.sh`,
  per the item's rules). That `glsl.rs` rebalances q8 and that the vertex
  walk then builds fp32 is read from the code (`mali/glsl.rs:1156-1205`,
  `mali/nir.rs:4905-4930`), not observed.
- Fragment-stage probes: none were run here. Section 3's output rule
  (`nir_lower_mediump_io` lowering a store of an `f2f32`) is read only.
- The dEQP names marked "not checked" were not matched to the exact
  source text of the case; the others were read in `VK-GL-CTS`.
- Whether `lower.rs` shares any ALU value between two trees besides
  constants and parameter aliases: none found by reading `const_value`
  (`lower.rs:5981-6010`) and the alias (`:4757-4767`); no test proves it.
- The mechanism behind p1 and p1c (a `const` read folded or not) was not
  re-read; its encoding in `lower.rs:7949-7961` is taken from the earlier
  probes.
- The S4 option (16-bit `const` array storage) has no Mesa probe.
- The shim install used was one built earlier on this host by another
  session (Mesa 26.2.1, unpatched compiler); it was not rebuilt.
