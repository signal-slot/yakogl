<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the shader NIR, step 1 (straight-line float shaders in Mesa's pass loop, shadow mode)

**Status: design, no code** (issue #11253, part of #11251). It fixes the
data structure, the generated tables, the translation from the front end's
IR, the pass list, the bridge to the existing back half, shadow mode and
the cut into steps for the first item of recommendation (C) in
`mali-compiler-road-to-es2.md` (its section 4.4, item 2).

- **Tip:** origin/main `92b2601d5` (2026-11-23). Our `file:line` sites are at
  that commit, under `src/shaderc/online/mali/` unless the path says
  otherwise.
- **Mesa:** 26.2.1, paths relative to the Mesa tree (`src/...`), the tree
  `tools/mali/build_pan_shc.sh` makes (`MESA_PAN_TREE`). Sizes are `wc -l`
  of the file, comments included.
- **Measured here:** only counts made with `grep` and with Mesa's own Python
  (`nir_opcodes.py`, `nir_algebraic.py`, read through
  `tools/mali/gen_blend_nir_rules.py`'s loader). Nothing was built or run.
  Every size in lines or days for our code is an **estimate**, marked so.
- **The goal of the whole front half** (M1 to M5) is in the road note. This
  note is about step 1 only, but section 2 fixes the structure in its final
  form so that M2 (int and bool), M3 (precision) and M4 (control flow) add
  passes and translation rules, not a second structure.

## 0. Decisions in brief

1. **A new module, `src/shaderc/online/mali/snir/`** ("shader NIR"). It is
   compiled only with `cfg(any(test, feature = "host-test"))` until a stage
   class moves to it (section 7.4), so the product binary does not change
   before then.
2. **The structure is Mesa's**: SSA defs with bit size and component count,
   ALU (with `fp_math_ctrl`), `load_const`, `undef`, intrinsic, tex, phi and
   jump instructions, blocks in a control-flow tree, and **use lists as
   intrusive doubly linked lists in Mesa's order** (section 2). Indices into
   arena vectors, never pointers.
3. **One generator, `tools/mali/gen_nir_tables.py`**, replaces
   `gen_blend_nir_rules.py`. It writes the opcode table, the three algebraic
   passes' tables and the constant folding of every reachable opcode, all
   from Mesa's own Python files, and the gate compares the committed output
   with a fresh run (section 3).
4. **`blend_nir.rs` is the seed and is then replaced**: its passes and its
   nir_search port move to `snir/`; blend shaders become the first user of
   `snir/` (4,103 keys byte-equal before the old files are deleted).
5. **The translation** takes `ir::Module` after only the two GLSL-IR passes
   (`do_rebalance_tree`, `reassociate_constant` without its fold), with the
   precision tables kept through them (#11266). It undoes the front end's
   early lowerings where Mesa does them later, from a small per-value
   **origin table** the front end adds (section 4.4), and it refuses any
   value whose class is `Prec::Unknown`. For M3 it makes Mesa's `f2fmp` /
   `f2f32` conversions from the per-value classes on each edge (section 4.6,
   from #11254's answer in `docs/notes/mali-precision-frontend.md`).
6. **Pass order is Mesa's**, from `preprocess_shader` in the GLSL linker to
   the start of `bi_compile_variant_nir`. A pass step 1 does not port is
   replaced by a **guard**: the shader is refused (not compiled differently)
   when the pass's trigger is present (section 5).
7. **The bridge** is a new selector `snir/select.rs` that transcribes
   `bi_emit_instr` and calls the existing `emit.rs` rows. Four early BIR
   passes Mesa runs and the back half lacks (`bi_lower_swizzle`,
   `bi_opt_copy_prop`, `bi_opt_constant_fold`, the first `bi_opt_cse`) are
   ported and run only for the new path (section 6).
8. **Shadow mode** compiles every committed reference through both paths in
   a host test. The rule is **"equal or refused, never differ"** for the new
   path, at two checkpoints: the NIR at selection (against Mesa's printed
   NIR) and the words (against `*.expect.json`). A stage class moves when the
   criteria of section 7.3 hold (section 7).
9. **19 steps** of 2 to 6 days (estimate), 5 to 7 lanes in parallel after
   the first week (section 8). The first engine number is the 4,103 blend
   keys (step S8); the first shader number is "vertex shaders whose NIR at
   selection is Mesa's" (step S12 once S7, S9a and S10a are in).

## 1. Step 1: what it covers

**The class.** A stage is in step 1's class when all of these hold. The
classifier (`snir/scope.rs`, step S10a) reads `ir::Module`
(`src/shaderc/online/ir.rs:860-935`) and the program's link facts:

| Condition | Why |
|---|---|
| `m.body` holds only `Node::Instr` (no `If`, `Loop`, `Break`, `Continue`, `Return`, `ir.rs:556-570`) | one block; control flow is M4 |
| every value is float (`ValueKind`, `ir.rs:361`); no `ConstI32`, `ConstBool`, compare, `B2f`, `Bcsel`, int op | int and bool are M2 |
| no `LoadUniformIdx`, no `LoadLocal`/`StoreLocal` with an index (`ir.rs:476-505`) | indirect access is M2/M4 |
| `m.has_prec()` and every float value and local is `Prec::High` (`ir.rs:839-856`, `:965-1016`); no uniform declared `mediump` or `lowp`; no value `Prec::Unknown` | precision is M3; `Unknown` is always a refusal (section 4.6) |
| no `Tex`, `TexSize`, `Discard` | look-ups are step S14 (after step 1); discard is M4 |
| no `gl_FragCoord`, `gl_FrontFacing`, `gl_PointCoord`, no invariant output | sysval lowering and `exact` (section 4.3) come later |
| the link facts move nothing (`moved` empty, no vertex constants given to the fragment shader) | `nir_opt_varyings` is not ported in step 1 (section 5, row L5) |

**The structure is still the final one.** Step 1 builds one block (plus the
empty end block Mesa always has), no phis, no derefs with an index. But the
types and lists of section 2 hold blocks, phis, jumps, 1-bit and 16-bit
values and every opcode from the first commit, so M2 to M4 only add code.

**How many references are in the class (estimate).** A `grep` over the
committed pairs at the tip (each `tools/mali/testdata/<name>.expect.json`
with a `.vert` and a `.frag`; comments stripped; the patterns are a rough
classifier, not the real one):

| Pairs | Count |
|---|--:|
| committed reference pairs | 1,575 |
| no control flow, `?:`, int, bool, struct or dynamic index in either stage | 691 |
| of those, vertex shader without `mediump`/`lowp` | 405 |
| of those, both stages without `mediump`/`lowp` | 100 |
| of those, and no `texture` call | 79 |

So step 1's number is mostly vertex shaders (about 400). A fragment shader
of dEQP-GLES2 is usually `mediump`, and Mesa computes it at 16 bits; that
waits for M3. These counts are an upper bound: the regular expressions do
not see gl_FragCoord, invariance or link facts.

## 2. The data structure

### 2.1 What Mesa has, and what of it decides the words

| Mesa | `src/compiler/nir/nir.h` | What the order of the words depends on |
|---|---|---|
| `nir_instr`: list node, block, type, `pass_flags`, `index` | `:1004-1022`; the types `:991-1002` | instruction order in the block list |
| `nir_def`: `uses` list, `index`, `num_components`, `bit_size`, `divergent` | `:1056-1080` | the use list order; `index` is set when the instruction is inserted (`impl->ssa_alloc++`, `nir.c:1241-1256`) |
| `nir_src`: tagged parent (instruction or `nir_if`), `use_link`, `ssa` | `:1082-1092` | an `if` condition is a use in the same list |
| `nir_alu_src` (src + swizzle), `nir_alu_instr` (def, op, `no_signed_wrap`, `no_unsigned_wrap`, `fp_math_ctrl`, srcs) | `:1250-1262`, `:1609-1643` | `fp_math_ctrl` excludes and feeds rules (`nir_search.c:332-335`, `:423`) |
| `nir_intrinsic_instr`, `nir_tex_instr`, `nir_load_const_instr`, `nir_jump_instr`, `nir_undef_instr` | `:2059-2082`, `:2624-2773`, `:2904-2910`, `:2975-2981`, `:2985-2988` | const indices (base, range, align, wrmask, io semantics, ...) are printed and select words |
| `nir_phi_instr`: def + list of `nir_phi_src` (pred block, src) | `:2990-3011` | phi sources in list order |
| `nir_cf_node`, `nir_block` (instr list, successors, **predecessors as an ordered `util_dynarray`**, idom, dominance), `nir_if`, `nir_loop`, `nir_function_impl` | `:3393-3472`, `:3652-3662`, `:3754-3776`, `:3907-3949` | block order; predecessor order |

The use-list rules, which every pass that walks uses inherits:

- **Insert** (`nir_instr_insert`, `nir.c:1265`): `add_defs_uses`
  (`nir.c:1258-1262`) appends each source, in source order, to the **tail**
  of its def's list (`add_use_cb`, `nir.c:1231-1238`), then numbers the defs.
- **Rewrite one source** (`nir_src_rewrite`, `nir.h:4873-4880`): unlink, then
  append at the tail of the new def's list.
- **Rewrite all uses** (`nir_def_rewrite_uses`, `nir.c:1739-1745`): walk the
  old list in order (if-uses included) and rewrite each, so they arrive at
  the new list's tail in the old order.
- **Remove** (`nir_instr_remove_v`, `nir.c:1369`; `remove_defs_uses`,
  `:1363`): unlink each source.

`nir_search` visits uses in list order (`add_uses_to_worklist`,
`nir_search.c:603-615`), and so do copy_prop, the vectoriser, `opt_move` and
the use-count conditions (`is_used_once`). This is why the order must be
Mesa's.

### 2.2 Ours: `snir/mod.rs`

Rules from the crate (`src/shaderc/online/arena.rs:3-17`, `ir.rs:30-34`): no
heap, everything in the scratch arena (4 MiB, `store.rs:87`), indices not
pointers, running out of scratch is a refusal (`MaliError::OutOfScratch`).

```text
DefId(u32), InstrId(u32), SrcId(u32), BlockId(u32), CfId(u32), VarId(u32)

Def    { instr: InstrId, index: u32, comps: u8, bits: u8,   // bits: 1, 8, 16, 32, 64
         first_use: SrcId, last_use: SrcId }                // doubly linked, Mesa's order
Src    { def: DefId, parent: Parent, prev_use: SrcId, next_use: SrcId,
         swizzle: [u8; 16] }                                // ALU sources only read the swizzle
Parent = Instr(InstrId) | If(CfId)
Instr  { block: BlockId, prev: InstrId, next: InstrId, pass_flags: u8,
         kind: Kind, srcs: (first SrcId, count), def: Option<DefId> }
Kind   = Alu { op: NirOp, fp_math_ctrl: u8, nsw: bool, nuw: bool }
       | LoadConst { values: (first, count) }               // u64 per component, as nir_const_value
       | Undef
       | Intrinsic { op: NirIntrinsic, const_index: [i32; 9] } // Mesa's 9 slots (nir.h:2031), by nir_intrinsic_info order
       | Tex { op, dim, dest_type, texture_index, sampler_index, src_types: (first, count), ... }
       | Phi                                                // srcs in Mesa's list order; each with its pred
       | Jump { kind: Break | Continue | Return | Halt | Goto | GotoIf }
       | Deref { .. }                                       // var and array derefs; step 1: var only
Block  { cf: CfId, first: InstrId, last: InstrId, succ: [BlockId; 2],
         preds: (first, count) /* ordered, as util_dynarray */, idom: BlockId, index: u32 }
Cf     = Block(BlockId) | If { cond: SrcId, then_: CfList, else_: CfList } | Loop { body: CfList, cont: CfList }
Var    { mode: FunctionTemp | ShaderIn | ShaderOut | Uniform, comps, bits, precision, location, name }
Impl   { body: CfList, end_block: BlockId, ssa_alloc: u32, valid_metadata: u32 }
```

What each choice answers:

- **Use lists are linked lists, not sequence numbers.** `blend_nir.rs`
  stamps each source with a sequence number and rebuilds a list by scanning
  the whole block and sorting (`blend_nir.rs:193-198`, `:291-357`). That is
  exact but O(n) per query, and it caps a value at 48 uses
  (`MAX_USES`, `:67`). A shader has thousands of values; Mesa's list is O(1)
  per insert and remove. The linked list also holds `if` uses, which a
  per-block scan cannot see.
- **`Def::index` is Mesa's**: given at insertion from `Impl::ssa_alloc`
  (`nir.c:1241-1256`), renumbered only where Mesa calls
  `nir_index_ssa_defs` (`bifrost_compile.c:4420`). The printer renumbers for
  comparison anyway (section 7.2), but #11255 may find a pass that reads it.
- **`fp_math_ctrl` per ALU instruction.** `glsl_to_nir` sets it per
  expression (`glsl_to_nir.cpp:2145-2614`: `fneg`, `fabs` and the compares
  add `preserve_inf`, `fsat`/`fmin`/`fmax`/`feq`/... add `preserve_nan |
  preserve_inf`) and to `exact | preserve_nan | preserve_inf` for an
  assignment to an invariant or precise variable (`:1899-1902`). nir_search
  skips a pattern whose `fp_math_ctrl_exclude` meets it and carries it into
  the replacement (`nir_search.c:332-335`, `:423`). The blend engine ignores
  it, which is right only because a blend shader has none
  (`gen_blend_nir_rules.py:31-35`).
- **No precision field per value.** Mesa has none either: `lower_precision`
  works on GLSL IR before NIR, and NIR sees 16-bit values, `f2fmp`/`i2imp`
  conversions, a variable's `precision` and `io_semantics.medium_precision`.
  M3 therefore adds translation rules (section 4.6) and the pass
  `nir_lower_mediump_io`, not a field (`Var::precision` is there from the
  start; `nir_lower_mediump_vars` does not run on panfrost, 4.6).
- **Booleans are 1-bit defs** (M2): `bits == 1`, as `nir_def::bit_size`.
- **Opcodes are a generated enum `NirOp`** (all 446 of `nir_opcodes.py`,
  counted with Mesa's Python), with a generated info table (section 3). A
  hand enum like `blend_nir.rs:73-94` (17 opcodes) does not scale.
- **Intrinsics are a generated enum `NirIntrinsic`** of the subset the
  translation and the ported passes make (from `nir_intrinsics.py`), with
  the const-index slots in `nir_intrinsic_info` order, so the printer prints
  them as Mesa does.
- **Derefs exist but stay small.** Step 1 needs a variable deref only, for
  function-temp locals (constructors, section 4.5). Array derefs come with
  M2/M4.

The API is Mesa's, by name, so that a ported pass reads like its C:
`instr_insert(cursor, i)`, `instr_remove(i)`, `src_rewrite(s, d)`,
`def_rewrite_uses(old, new)`, `def_rewrite_uses_after(old, new, after)`,
`foreach_use(d)`, `foreach_use_including_if(d)`, `def_is_unused`,
`Cursor::{Before, After}{Instr, Block, CfNode}`, a `Builder` with
`fp_math_ctrl` and the cursor. Capacities grow (arena vectors), so the fixed
`MAX = 256` of `blend_nir.rs:63` goes away.

**The printer (`snir/print.rs`)** writes `nir_print_shader`'s instruction
lines (`32x4   %1 = @load_input (...) (base=..., ...)`, as in
`tools/mali/testdata/fbload_fbload.stderr:21-50`), normalised as in section
7.2. It is part of step S1 because every later test compares through it.

### 2.3 What step 1 builds of it

One `Impl`, body = one block, the end block, no `If`/`Loop`, no phis, no
jumps. Function-temp variables and var derefs exist only until the
one-block `nir_lower_vars_to_ssa` (step S6b) removes them. `NirOp` values
outside the reachable set (section 3.3) are a refusal when a builder makes
one.

## 3. The blend engine and the generator: kept, widened, replaced

### 3.1 What exists

- `blend_nir.rs` (2,902 lines): one block, 17 opcodes (`:73-94`), use order
  by sequence numbers (`:193-357`), and transcriptions of copy_prop
  (`:745`), dce (`:784`), cse (`:842`), vectorize (`:989`), lower_alu_width
  (`:1187`), constant_folding (`:1239`, with `eval` for its opcodes at
  `:1064`), opt_move (`:1256`), shrink_vectors (`:1518`),
  load_const_to_scalar (`:1535`), the fp class analysis
  (`:1566-2048`, a subset of `nir_range_analysis.c`), nir_search
  (`:2102-2655`: items, conditions, matching, replacement, worklist) and the
  pass order (`:2666-2738`).
- `blend_nir_rules.rs` (2,246 lines, generated): 179 + 24 + 4 = 207
  transforms.
- `tools/mali/gen_blend_nir_rules.py` (319 lines): loads Mesa's
  `nir_opt_algebraic.py` and `bifrost_nir_algebraic.py` with Mesa's parser
  (`:127-147`), keeps the automaton items built only of `SHADER_OPS`
  (`:43-47`), evaluates rule conditions from a string table (`CONDITIONS`,
  `:61-...`), fails on what Rust does not hold (`:26-35`).
- The gate re-runs it and compares (`tools/check.sh:3186-3190`).

### 3.2 What happens to each

| Piece | Fate | Where it goes |
|---|---|---|
| `blend_nir.rs` structure (`Instr`, `Src.seq`, `Nir`) | **replaced** by section 2 | `snir/mod.rs` |
| its passes (copy_prop ... load_const_to_scalar) | **widened**: all opcodes, structure-aware, Mesa's use lists | `snir/opt_*.rs` (steps S6a, S6b) |
| its nir_search port and fp class | **widened**: all generated items and conditions, bit-size rules of `construct_value`, the comm-expression limit | `snir/search.rs`, `snir/fp_class.rs` (step S5) |
| its pass order `optimize` | **replaced** by the full pipeline; the blend shader runs the same functions | `snir/pipeline.rs` (S7) |
| `blend_nir::build` (the blend NIR) | **kept**, rewritten onto `snir::Builder` | `blend_nir.rs` shrinks to the builder (S8) |
| `blend.rs` (selection, `finish`) | **kept**, reads `snir` | S8 |
| `blend_nir_rules.rs`, `gen_blend_nir_rules.py`, the check.sh row | **deleted** in the commit that switches blend to `snir` (S8b); the new generator's row replaces the row | `tools/check.sh` |

### 3.3 The new generator: `tools/mali/gen_nir_tables.py`

One script, one loader (the existing `load`, `gen_blend_nir_rules.py:127-147`),
four outputs, all committed, all `#![cfg_attr(rustfmt, rustfmt_skip)]` with
the generated-file header the old one has (`:283-299`):

| Output | From (Mesa) | What |
|---|---|---|
| `snir/opcodes.rs` | `src/compiler/nir/nir_opcodes.py` (2,158 lines) | `NirOp` (446), per op: name, `num_inputs`, `output_size`, `input_sizes`, output and input `nir_alu_type`, algebraic properties (commutative, associative, selection, ...), and the search-op mapping of `nir_search.c:76-147` (unsized conversions) |
| `snir/rules_opt.rs`, `rules_late.rs`, `rules_bifrost.rs` | `nir_opt_algebraic.py` (4,458 lines) `optimizations`, `late_optimizations`; `src/panfrost/compiler/bifrost/bifrost_nir_algebraic.py` (233 lines) `algebraic_late`; parsed by `nir_algebraic.py` (1,669 lines) | per pass: items, item-to-rules, rules in Mesa's order, search/replace node trees with `fp_math_ctrl` exclude/add, expression and variable conditions |
| `snir/fold.rs` | the `const_expr` of each op in `nir_opcodes.py`, rendered as `nir_constant_expressions.py` (686 lines) renders C | `fn eval(op, bit_size, srcs) -> Option<[u64; 16]>` for every reachable op |
| `snir/intrinsics.rs` | `src/compiler/nir/nir_intrinsics.py` | `NirIntrinsic` subset and its index slots (section 2.2) |

**Sizes measured with Mesa's parser (all transforms, before any filter):**

| Pass | Transforms | Automaton items | Search opcodes | Distinct condition strings | Conditions the old table does not know (transforms) | Max comm. expressions |
|---|--:|--:|--:|--:|--:|--:|
| `nir_opt_algebraic` | 2,935 | 1,973 | 224 | 176 | 158 (640) | 61 |
| `nir_opt_algebraic_late` | 487 | 477 | 92 | 53 | 31 (165) | 5 |
| `bifrost_nir_lower_algebraic_late` | 98 | 75 | 41 | 3 | 0 | 2 |

Search and replace trees together are 29,875 + 5,269 + 822 nodes. Mesa's
generated `nir_opt_algebraic.c` in the build tree is 51.9 MB, nearly all of it
transition tables. That decides the next points.

**Widening rules:**

1. **Conditions are evaluated, not looked up.** The widened script reads
   the option values of `bifrost_compile.h:80-160` for arch 10 (the
   `bifrost_nir_options_v9` table that `pan_compiler.c:94-97` returns for
   arch 9 and 10) into a dict. A tiny evaluator of C boolean expressions
   (`!`, `&&`, `||`, `&`, `==`, `!=`, `>=`, parentheses, `options->x`,
   `gpu_arch`, `is_kraid`, `info->stage`, and the float-controls helpers with
   GL's execution mode 0) then gives each rule's condition. An expression it
   cannot parse stops the generation (exit 1), as today. This replaces the
   per-string table (`gen_blend_nir_rules.py:61-108`), which knows 18 of 176
   strings. Stage-dependent conditions (`info->stage`) become a per-rule
   stage mask, not a constant.
2. **Pruning by a reachable opcode set.** A committed list
   `tools/mali/snir_ops.txt` names the opcodes the translation and the ported
   passes may create. The generator closes it under every enabled rule's
   replacement and keeps only items built of reachable opcodes (the blend
   script's rule, `:165-178`, made general). The closure is emitted as
   `REACHABLE`, and `snir::Builder` refuses an opcode outside it, so a pruned
   item can never be missing a match. M2 to M4 add names to the list.
3. **States are item bitsets, not Mesa's state numbers.** An instruction's
   state is the set of items it matches; nir_search revisits uses only when
   the state changes (`nir_algebraic_update_automaton`,
   `nir_search.c:617-639`). Mesa's numbered states are distinct item sets, so
   "the number changed" is "the set changed". The blend engine already relies
   on this (`blend_nir.rs:2164-2166`, 256 bits). With up to 1,973 items the
   set is 31 `u64` words a def (about 250 bytes; 2,000 defs make 0.5 MB of
   scratch, an estimate). No transition table is emitted.
4. **Mesa's limits, not ours.** `NIR_SEARCH_MAX_COMM_OPS` is 8
   (`nir_search.c:32`): an expression with `comm_expr_idx >= 8` is never
   swapped (`:364`) and a search tries `1 << min(comm_exprs, 8)` orders
   (`:658`). The old script refused more than 8 (`:26-31`); the new one
   emits the index and the engine applies Mesa's rule.
   `NIR_SEARCH_MAX_VARIABLES` is 24 (`nir_search.h:32`), so `Match::vars`
   becomes 24, not 32.
5. **Conditions map to enums.** The expression conditions (6 new names in
   `nir_opt_algebraic`, 1 in `_late`) and variable conditions (29 and 6 new
   names) become `Cond`/`VarCond` variants. A condition the engine does not
   evaluate yet stays `Unknown`: a match that reaches it refuses the shader,
   as in `blend_nir.rs:2111-2135`.
6. **Constant folding of every reachable opcode.** For each op and bit size
   the generator renders the `const_expr` into Rust the way
   `nir_constant_expressions.py` renders C: typed `src0..src3`, `dst`,
   `bit_size`, with GL's execution mode 0 folded in
   (`nir_is_rounding_mode_rtz(...)` and `nir_is_denorm_flush_to_zero(...)`
   are false). 151 of the 446 const expressions are statement blocks
   (`fadd`, `fmul`, `ffma`, `f2f16`, `f2i32` among them) and they call about
   100 distinct helper names. A small translator for the C subset (`if`,
   `else`, assignment, `? :`, casts, calls) is part of the script; helpers
   are hand-written once in `snir/fold_helpers.rs`, reusing
   `src/shaderc/online/mesa_fold.rs` (Mesa's libm bits,
   `mali-libm-fold.md`) and `select::f16_rtne`. `f2i32` sets `poison` for
   out-of-range inputs; the fold keeps that flag as Mesa's
   `nir_opt_constant_folding` reads it (not checked how, see section 9).
   An op whose expression the translator cannot render gets `None`, and
   folding it refuses the shader.

**How the gate proves the committed files are the generator's.** The new
row replaces `tools/check.sh:3186-3190` and runs on `MESA_PAN_TREE`:
`python3 -I tools/mali/gen_nir_tables.py "$MESA_PAN_TREE" --out <dir>`, then
`cmp` of each output with the committed file. A second check proves the
folds: a C file `tools/mali/nir_fold_vectors.c`, built inside the Mesa tree
like `pan_shc.c`, calls Mesa's `nir_eval_const_opcode` for every reachable
op on a fixed grid of inputs (zeros, signed zeros, ones, denormals, NaN,
infinities, a seeded random set) and prints `op bits inputs -> output`. The
committed `tools/mali/testdata/nir_fold_vectors.txt` must equal its output,
and a Rust test (`snir/fold.rs` tests) must give the same outputs. The
existing pattern for both is the blend reference row (`check.sh:3170-3185`).

## 4. The translation from `ir::Module`

### 4.1 Mesa's chain before `bifrost_preprocess_nir`

For an ES 2 GLSL program on panfrost, in order (the SPIR-V twin of this chain
is transcribed in `tools/mali/pan_shc.c`, `:124-338`):

1. **GLSL IR** (`src/compiler/glsl/glsl_parser_extras.cpp`):
   `do_common_optimization` (`:2577-2614`: `do_tree_grafting`,
   `do_rebalance_tree`, `do_algebraic`, ...), `lower_precision` (`:2491`),
   then `glsl_to_nir` (`:2539`; `glsl_to_nir.cpp:175`).
2. **The linker** (`gl_nir_link_glsl`, `gl_nir_linker.c:3738`):
   `gl_nir_inline_functions` (`:3961`, body `:142-151`), `prelink_lowering`
   (`:4111`, body `:1394-1466`) with `preprocess_shader` (`:1310-1391`:
   `nir_lower_global_vars_to_local` and `nir_lower_var_copies` `:1359-1360`,
   `nir_lower_mediump_vars` `:1364`, `nir_opt_constant_folding` `:1390`),
   `gl_nir_link_varyings` (`:4115`), which runs `gl_nir_opts` per shader
   (`gl_nir_link_varyings.c:4175`; the loop is `gl_nir_linker.c:46-130`),
   `nir_link_varying_precision`, and `gl_nir_lower_optimize_varyings`
   (`gl_nir_link_varyings.c:4331`; body `gl_nir_linker.c:1480-1527`):
   `nir_lower_io_passes` (`nir_lower_io.c:1418-1476`: outputs to temporaries
   `:1453`, `nir_lower_io` `:1468`, constant folding `:1474`) and
   `nir_opt_varyings_bulk` with `gl_nir_opts` as its clean-up callback.
3. **st/mesa** (`src/mesa/state_tracker/st_glsl_to_nir.cpp`):
   `st_link_glsl_to_nir` (`:438`): dead variables (`:529`),
   `st_nir_lower_wpos_ytransform` (`:554`), `nir_lower_system_values`
   (`:559`), `st_glsl_to_nir_post_opts` (`:569`, body `:236-366`:
   `st_nir_lower_builtin` `:285`, `nir_opt_intrinsics` `:366`),
   `st_finalize_nir` (`:374`, body `:723-760`: `nir_lower_tex` `:739`,
   `st_nir_lower_uniforms` `:701-720` = `nir_lower_io` of uniforms with the
   packed layout and `nir_lower_uniforms_to_ubo`).
4. **The driver at create time** (`panfrost_create_shader_state`,
   `src/gallium/drivers/panfrost/pan_shader.c:500`): `nir_lower_fragcolor`
   (`:555-562`), `pan_preprocess_nir` (`:566`; `src/panfrost/compiler/pan_compiler.c:111-130`:
   `bifrost_preprocess_nir` then `nir_lower_tex`),
   `nir_lower_indirect_derefs_to_if_else_trees` (`:568`), `nir_lower_io`
   (`:570`), constant folding (`:577`), the vertex varying layout
   (`:583-591`).

Then section 5 (`bifrost_preprocess_nir` onward).

### 4.2 What our front end has already done that Mesa does later

Each must not be done twice; where the front end did it early, the
translation puts back what Mesa has at that point, or step 1 refuses.

| Done early by our front end | Where | Mesa does it at | Rule for the translation |
|---|---|---|---|
| `a - b` is `fadd(a, fneg(b))` | `lower.rs:6210-6216` | `fsub` from `glsl_to_nir.cpp:2382-2384`, lowered by `(('fsub', a, b), ('fadd', a, ('fneg', b)))` (`nir_opt_algebraic.py:2379`) in the first `nir_opt_algebraic` | emit `fsub` where the origin table says "subtraction" (4.4) |
| `a / b` is `fmul(a, frcp(b))`; `a / literal` is `fmul(a, 1/literal)` folded | `lower.rs:5934-5961`, `:6225-6232` | `fdiv` from `glsl_to_nir.cpp:2423-2425`, lowered by `nir_opt_algebraic.py:1905` (`lower_fdiv`), then folded | emit `fdiv(a, b)` (with the literal, unfolded) where the origin table says "division" |
| `m * v` is `fmul` + `ffma` chain in Mesa's column order (`MatOrder::MesaGlsl`) | `ir.rs:195-209` | `lower_mat_op_to_vec.cpp:165-178` gives `mul` + `add`; the fusion is `nir_opt_algebraic_late` | emit `fmul` and `fadd` in that order (no `ffma`): GLSL ES 1.00 has no `fma()`, so every `Ffma` of the IR is a front-end fusion |
| built-ins expanded its own way (`mix` as `ffma(t, b - a, a)`, `step` as `b2f(!(x < e))`, `smoothstep`, `normalize`, ...) | `builtin_call.rs:860-1000` (`mix` `:954-961`, `step` `:977-986`) | `builtin_functions.cpp` (9,685 lines) bodies, inlined in GLSL IR; `mix` is `lrp` (`glsl_to_nir.cpp:2576-2577`), lowered by `nir_lower_flrp` | expand from the origin table with Mesa's bodies (step S10c); the walks recognise shapes instead (`nir.rs:4985-5100`: `is_mix`, `is_cross`, `is_reflect`, `vs_smoothstep_tail`), which is the fragile route this design leaves |
| constant expressions folded (`eval.rs`, `fold.rs`, `mesa_fold.rs`) | front end | GLSL `ast_to_hir` folds constant expressions; NIR folds the rest in `nir_opt_constant_folding` | keep the front end's folds of GLSL constant expressions; anything else it folded is an open question (Q1, section 9) |
| functions inlined | `ir.rs:26-27` | `gl_nir_inline_functions` at link | nothing: one `main` either way (parameter copies vanish in `nir_lower_vars_to_ssa`) |
| `do_rebalance_tree`, `reassociate_constant` | `glsl.rs:187-188` | GLSL IR (`glsl_parser_extras.cpp:2604-2605`) | keep: run before the translation |
| `reassociate_constant`'s fold "in place, fp32 as NIR computes it" | `glsl.rs:24-30` | the swap in GLSL IR, the fold in NIR | the new path's entry does the swap only (a flag on `glsl::passes`) |
| three NIR rules on negated constants (`neg_products`, `neg_compares`, `neg_zero_compares`) and the `ilt` ladders (`ilt_trees`) | `glsl.rs:150`, `:190-192` | `nir_opt_algebraic`; `nir_lower_indirect_derefs_to_if_else_trees` | not run for the new path: the generated rules do them at Mesa's time |

### 4.3 What the IR has lost

| Lost | Effect | Step 1 answer |
|---|---|---|
| the built-in call (name and arguments) behind an expansion | Mesa's body and its fp_math_ctrl cannot be rebuilt from the expansion in general | origin table (4.4) |
| `-` and `/` as operators | see 4.2 | origin table |
| the GLSL expression tree, as `lower_precision` reads it | M3 | not lost for precision: #11254 found that the front end applies the class rule on the syntax tree and records it per instruction and per slot (`docs/notes/mali-precision-frontend.md` section 0); the conversions are not in the IR and the translation makes them (4.6) |
| precision tables after `mali/glsl.rs` | `glsl::passes` empties `Module::prec`/`local_prec` when it rewrites, and rebalances trees Mesa leaves alone (Mesa rebalances after `lower_precision`) | dependency: #11266 (being fixed) keeps the tables through the passes; the translation reads the module after `glsl::passes` and assumes it landed |
| invariance spread to temporaries (`propagate_invariance`, `glsl_parser_extras.cpp:2597`, `:2612`) and so `exact` (`glsl_to_nir.cpp:1899-1902`) | wrong `fp_math_ctrl` on an invariant output's tree | guard: no invariant output in step 1 |
| which GLSL variable a local was (a user variable or a constructor's temporary) | none in one block: both become SSA | none |
| when a constant appeared (folded early vs folded by NIR) | instruction positions during the loop | Q1 |

### 4.4 The origin table (front end change, step S10b)

A new optional per-value table in `ir::Module`, beside `prec` and `lanes`
(`ir.rs:884-930`), with the same rule for passes that rebuild the module
(empty table = unknown = the new path refuses):

```text
Origin = Plain
       | Sub                         // this Fadd is `a - b`; src[1] is the Fneg
       | Div { literal: Option<u32> } // this Fmul is `a / b`; the literal before its reciprocal
       | MatVec                      // this Ffma/Fmul is a matrix product's term
       | Builtin { id: BuiltinId, args: (first, count) }   // the call's result; args are ValueIds
```

- Other back ends do not read it, so their words cannot move. Their gate rows
  prove it (the rule of #11251).
- #11254 found that precision needs no such table (the classes are already
  recorded, `docs/notes/mali-precision-frontend.md` section 0), so the
  origin table serves only the operator and built-in identity. Its rows
  must not disturb `Module::prec`: an origin is a second property map by
  the same `ValueId`, under the same rebuild rule.
- Fallback if the front end change is refused: recognition of the front
  end's expansions in the translation (as the walks do). It is listed as
  risk R2 because it is lossy.

### 4.5 Translation rules (step S10a)

The translation walks `m.body` in order. That order is the GLSL source order
Mesa's front end emits too (`nir.rs:12-14`).

| IR | Shader NIR |
|---|---|
| `ConstF32` | `load_const`, 32-bit, the IR's lane count, at the place of the value's first use (Q1) |
| `Alu(op, srcs)` | the `nir_op` of `glsl_to_nir.cpp`'s case for the GLSL operation (`fadd`, `fmul`, `fdot2..4` for `Fdot*` `:2557-2558`, `frcp`, `frsq`, `fsqrt`, `fexp2`, `flog2`, `fsin`, `fcos`, `ffloor`, `fceil`, `ftrunc`, `ffract`, `fsign`, `fmin`, `fmax`, `fneg`, `fabs`), each source an ALU source with the IR swizzle, `fp_math_ctrl` as `glsl_to_nir.cpp:2145-2614` sets it for that case; `Sub`/`Div`/`MatVec`/`Builtin` origins as 4.2 says |
| `LoadInput { Attrib }` (VS) | `load_input` with Mesa's io semantics and `dest_type` (the form after `nir_lower_io`; Q3 is whether the deref form must exist first) |
| `LoadInput { Varying }` (FS) | `load_interpolated_input` + `load_barycentric_pixel`, as `nir_lower_io` with `nir_lower_io_use_interpolated_input_intrinsics` makes it (`pan_shader.c:570-571`) |
| `LoadUniform { dword_off, comps }` | `load_ubo(0, offset)` as `st_nir_lower_uniforms` + `nir_lower_uniforms_to_ubo` make it (packed storage, `PackedDriverUniformStorage`), with the offsets of `select::UboLayout` (`select.rs:290-470`) |
| `StoreOutput` | `store_deref` to the output variable; the outputs are copied at the end of `main` by the `nir_lower_io_vars_to_temporaries` model (`nir_lower_io.c:1453`), then `store_output` |
| `LoadLocal`/`StoreLocal` (no index) | `load_deref`/`store_deref` of a function-temp variable; a masked store first swizzles its value as `glsl_to_nir.cpp:1934-1947` does; a whole store of a deref or constant is a copy (`:1903-1913`) lowered as `nir_lower_var_copies` does (`gl_nir_linker.c:1360`) |

The constructor is the case that needs locals in step 1: the front end
builds `vec4(v.xyz, 1.0)` as a local with one masked store per argument
and a load (`lower.rs:5667-5700`), as Mesa's GLSL IR does. Mesa's
`nir_lower_vars_to_ssa` then makes a `vec4` of the new and old lanes at each
masked store (`nir_lower_vars_to_ssa.c:736-750`), an `undef` standing for a
lane never written. The one-block port of that pass (S6b) gives the same
`vec`s and `undef`s.

`glsl::passes` gets an entry for the new path: `ilt_trees` and the three
negated-constant rules off, `reassociate` without its fold (4.2). The old
entry does not change. Both entries keep the precision tables once #11266
has landed.

### 4.6 Precision: what the translation does now (step 1) and for M3

From #11254 (`docs/notes/mali-precision-frontend.md`, not yet on main at the
tip; branch `issue/11254`):

- **`Prec::Unknown` is a refusal**, in step 1 and after, as `ir.rs:850-853`
  asks. Five front-end sites record it on purpose (that note's section 2.2):
  the folded reciprocal (`lower.rs:5937-5958`), a shared pinned constant
  (`lower.rs:5990-6000`), a constant passed into `pow`/`asin`/`acos`, a
  run-time index into a folded mediump constant array, and an unevaluated
  `?:` condition. A module without tables (`has_prec()` false) is refused
  whole.
- **Step 1** takes only stages whose float values and locals are all
  `Prec::High` (section 1), so it makes no conversion.
- **M3**: Mesa's conversions are not in the IR; they follow from the classes
  on each edge. For each source edge whose producer and user classes differ,
  the translation emits what `lower_precision` leaves in GLSL IR and
  `glsl_to_nir` turns into NIR: `f2fmp` into a Medium tree and `f2f32` out of
  it, with the 16-bit arithmetic in between. The edge rules (constants,
  built-in results, temporaries, calls, storage) are that note's section 1
  table, one row each, and become translation rules one by one.
- **NIR-level precision passes for panfrost**: `nir_lower_mediump_vars` does
  not run (it needs `int16`, which panfrost has off;
  `gl_nir_linker.c:1362-1365`, `pan_screen.c:591`), and panfrost calls no
  `nir_fold_16bit_*`. `nir_lower_mediump_io` (`bifrost_nir.c:914-916`) does
  run; its output-store rule was only read in #11254, not probed. So M3
  ports `nir_lower_mediump_io` and the 16-bit paths of the passes already in
  section 5 (vectorize width 2, `nir_lower_bit_size`, `pan_nir_fuse_io_cvt`),
  not `nir_lower_mediump_vars`.

## 5. The pass list, in Mesa's order

Classes: **S1** step 1 ports it (it changes a straight-line float shader);
**N** it does nothing on step 1's class, so step 1 skips it, with a **guard**
that refuses the shader when the pass's trigger is present; **M2/M3/M4/L**
needed later (L = the link, `nir_opt_varyings`). A pass that appears again
later in the order is listed once, at its first place, with the other places.

### 5.1 Link and state tracker (section 4.1, items 2 and 3)

| # | Pass | Mesa file (lines) | On a straight-line float shader | Class |
|---|---|---|---|---|
| L1 | `nir_lower_global_vars_to_local`, `nir_lower_var_copies` | `nir_lower_global_vars_to_local.c` (106), `nir_lower_var_copies.c` (143) | copies of whole locals become load + store | S1 (in the translation, 4.5) |
| L2 | `nir_lower_mediump_vars` | `nir_lower_mediump.c` (895), `gl_nir_linker.c:1362-1365` | not called on panfrost (it needs `int16`, off: `pan_screen.c:591`; `mali-precision-frontend.md` section 3) | none, never ported |
| L3 | `gl_nir_opts` loop: `nir_lower_vars_to_ssa`, `nir_remove_dead_variables`, `nir_opt_find_array_copies`, `nir_opt_copy_prop_vars`, `nir_opt_dead_write_vars`, `nir_lower_alu`, `nir_lower_pack`, `nir_opt_copy_prop`, `nir_opt_remove_phis`, `nir_opt_dce`, `nir_opt_loop`, `nir_opt_if`, `nir_opt_dead_cf`, `nir_opt_cse`, `nir_opt_peephole_select` (limit 8), `nir_opt_phi_precision`, `nir_opt_algebraic`, `nir_opt_constant_folding`, `nir_opt_undef`, `nir_opt_peephole_select` (limit 0, discard), `nir_opt_loop_unroll` (32 iterations, `bifrost_compile.h:140`); after the loop `nir_lower_flrp`, `nir_lower_var_copies` | `gl_nir_linker.c:46-130` | vars_to_ssa makes the constructors' `vec`s; copy_prop, dce, cse, algebraic and folding change the code; the rest find nothing in one block (no phis, ifs, loops, arrays) | S1: vars_to_ssa (one block), copy_prop, dce, cse, algebraic, constant folding, undef, flrp. N with guards: the if/loop/phi/array passes (trigger: any `If`, `Loop`, phi or array deref). `nir_lower_alu` and `nir_lower_pack`: N, guard on `imul_high`/`umul_high` and pack ops |
| L4 | `nir_link_varying_precision` | `nir_linking_helpers.c` (1,599) | nothing at highp | N, guard: mediump varying; M3 |
| L5 | `nir_lower_io_passes` + `nir_opt_varyings_bulk` (+ `gl_nir_opts` again on progress) | `nir_lower_io.c` (1,518), `nir_opt_varyings.c` (6,283) | outputs copied at the end; dead outputs removed; constants and uniform expressions moved across stages | S1 for the IO form and dead-output removal (taken from the program's link facts, applied at this place, then the L3 loop again); L for the rest: guard on non-empty `moved` or vertex constants |
| L6 | `st_nir_lower_wpos_ytransform`, `nir_lower_system_values` | `nir_lower_wpos_ytransform.c` (332), `nir_lower_system_values.c` (836) | nothing without gl_FragCoord or sysvals | N, guard: gl_FragCoord, gl_FrontFacing, gl_PointCoord |
| L7 | `st_nir_lower_builtin`, `nir_opt_intrinsics` | `st_nir_lower_builtin.c` (262), `nir_opt_intrinsics.c` (768) | nothing without built-in uniforms (`gl_DepthRange`) or subgroup ops | N, guard: a built-in uniform |
| L8 | `nir_lower_tex` (st), `st_nir_lower_uniforms` (`nir_lower_io` uniform, `nir_lower_uniforms_to_ubo`) | `nir_lower_tex.c` (1,904), `nir_lower_uniforms_to_ubo.c` (168) | uniforms become `load_ubo` | S1 for uniforms (the translation's form, 4.5); tex N (no look-ups) |

### 5.2 Driver, create time (section 4.1, item 4), and `bifrost_preprocess_nir`

All in `src/panfrost/compiler/bifrost/bifrost_nir.c` (1,312 lines) unless
named.

| # | Pass | Mesa file (lines), call site | On a straight-line float shader | Class |
|---|---|---|---|---|
| D1 | `nir_lower_fragcolor` | `nir_lower_fragcolor.c` (101), `pan_shader.c:559` | gl_FragColor becomes `FRAG_RESULT_DATA0` store | S1 (FS) |
| P1 | `nir_split_var_copies` | `nir_split_var_copies.c` (109), `:404` | nothing (copies already lowered) | N |
| P2 | `nir_lower_terminate_to_demote` (FS) | `nir_lower_terminate_to_demote.c` (116), `:416` | nothing without discard | N, guard: discard (M4) |
| P3 | `nir_lower_halt_to_return`, `nir_lower_returns` | (28), `nir_lower_returns.c` (312), `:423-424` | nothing without return | N, guard: `Return` (M4) |
| P4 | `nir_lower_global_vars_to_local` | (106), `:427` | nothing | N |
| P5 | `bi_optimize_loop(allow_copies)` | `:202-276`, called `:429` | see 5.4 | S1 |
| P6 | `nir_lower_var_copies` | (143), `:431` | nothing | N |
| P7 | `nir_lower_tex` (pan) | `nir_lower_tex.c` (1,904), `pan_compiler.c:118-129` | nothing without look-ups | N, guard: tex (S14) |
| D2 | `nir_lower_indirect_derefs_to_if_else_trees` (io) | (258), `pan_shader.c:568` | nothing | N, guard: indexed IO (M2/M4) |
| D3 | `nir_lower_io`, `nir_opt_constant_folding` | (1,518), `pan_shader.c:570-577` | the IO form (already made by the translation) | S1 (translation) |
| D4 | varying layout (`pan_varying_collect_formats`, `pan_build_varying_layout_compact`) | `pan_shader.c:583-591`; `pan_nir_collect_varyings.c` (448) | the VS varying layout the FS takes | S1, reusing `select::generic_layout` (`select.rs:151`) |

### 5.3 Driver, compile time, and `bifrost_postprocess_nir` (`:864-1093`)

| # | Pass | Mesa file (lines), call site | On a straight-line float shader | Class |
|---|---|---|---|---|
| C1 | `panfrost_nir_remove_fragcolor_stores` | `pan_nir_remove_fragcolor_stores.c` (32), `pan_shader.c:141` | removes stores past `nr_cbufs` (1 for gl_FragColor) | S1 (FS) |
| C2 | `panfrost_nir_lower_res_indices` | `pan_nir_lower_res_indices.c` (269), `pan_shader.c:180` | UBO/texture indices to the driver's tables | S1 for UBO indices |
| C3 | image passes (`pan_nir_lower_image_64bit`, `nir_lower_image_atomics_to_global`, `pan_nir_lower_buf_image_access`) | `:873-882` | nothing | N |
| C4 | `nir_opt_sink`, `nir_opt_move` (const, ubo, comparisons, copies, ssbo) | `nir_opt_sink.c` (436), `nir_opt_move.c` (160), `:885-890` | moves `load_const` and `load_ubo` next to their first user | S1 |
| C5 | `pan_nir_resize_varying_io` | (115), `:901-902` | resizes IO to the layout's bit sizes | S1 if it changes 32-bit IO (Q4); else N with a guard |
| C6 | FS: `nir_lower_is_helper_invocation`, `pan_nir_lower_helper_invocation`, `pan_nir_lower_sample_pos`, `pan_nir_lower_noperspective_fs`, `nir_lower_frag_coord_to_pixel_coord`, `pan_nir_lower_var_special_pan` | `:905-911` (116, 31, 78, 233, 37, 48) | nothing in the class | N, guards: the sysvals of 5.1 L6; `noperspective` is not GLSL ES 1.00 |
| C7 | FS: `nir_lower_mediump_io` | `nir_lower_mediump.c` (895), `:914-916` | nothing at highp (inputs and outputs fp32) | N, guard: mediump IO; M3 |
| C8 | FS: `bifrost_nir_lower_load_output` | `:651-659`, `:918` | nothing (no framebuffer fetch) | N |
| C9 | FS: `pan_nir_lower_fs_inputs` | `pan_nir_lower_fs_inputs.c` (120), `:927-928` | varying loads become `load_var_pan`/`load_var_buf_pan` | S1 |
| C10 | FS: `pan_nir_lower_fs_outputs` | `pan_nir_lower_fs_outputs.c` (203), `:940` | colour store becomes `load_cumulative_coverage_pan`, `atest_pan`, `load_blend_descriptor_pan`, `blend_pan` | S1 |
| C11 | VS: `nir_lower_viewport_transform`, `nir_lower_point_size` | (121), (116), `:942-943` | the position's `ffma` by the viewport sysvals; point size clamp | S1 |
| C12 | VS: IDVS choice, `bifrost_nir_lower_vs_atomics`, `pan_nir_lower_noperspective_vs` | `:950-957` | IDVS on; nothing else | S1 (the flag only) |
| C13 | VS: `pan_nir_lower_vs_outputs` | `pan_nir_lower_vs_outputs.c` (345), `:958-960` | per output `lea_buf_pan` and `store_global` | S1 |
| C14 | `pan_nir_lower_tex`, `pan_nir_lower_image` | (1,122), (287), `:963-964` | nothing | N, guard: tex (S14) |
| C15 | scratch and local-variable lowering (`nir_lower_scratch_to_var`, indirect derefs, explicit IO) | `:973-1007` | nothing: no locals left | N, guard: a local left after vars_to_ssa |
| C16 | `nir_lower_mem_access_bit_sizes` | (586), `:1009-1017` | splits each vector `store_global` into 32-bit stores; splits UBO loads | S1 (the store and UBO cases) |
| C17 | `nir_lower_ssbo`, subgroups, `bi_lower_subgroups` | `:1022-1069` | nothing | N |
| C18 | `nir_opt_idiv_const`, `nir_lower_64bit_phis`, `nir_lower_int64`, `nir_lower_idiv` | `:1072-1081` | nothing (the 64-bit `iadd` of the store address is native on v10: `lower_int64_options = ~nir_lower_iadd64`, `bifrost_compile.h:119`) | N, guard: int division (M2) |
| C19 | `nir_lower_flrp`, `nir_lower_alu` | (618), (246), `:1083-1084` | `flrp` left by a built-in becomes `ffma`/`fadd` forms | S1 (flrp); `nir_lower_alu` N (mul_high) |
| C20 | `nir_lower_bit_size` (`bi_lower_bit_size`, `:34-99`) | (434), `:1087` | nothing at 32 bits | N, guard: an 8- or 16-bit ALU op (M3) |
| C21 | `nir_lower_alu_width` (`bi_vectorize_filter`, `:106-179`), `nir_lower_load_const_to_scalar`, `nir_lower_phis_to_scalar` | (511), (90), (298), `:1088-1090` | vector ALU ops become scalar; vector constants become scalars | S1 (phis N) |
| C22 | `bi_optimize_loop` | `:1092` | 5.4 | S1 |
| C23 | `panfrost_nir_lower_sysvals`, `nir_inline_sysval` | `pan_nir_lower_sysvals.c` (268), `pan_shader.c:185-194` | viewport scale/offset become `load_ubo` of UBO 1 | S1 (VS) |

### 5.4 `bi_optimize_loop` (`bifrost_nir.c:202-276`), run to a fixpoint

| Pass (order) | Mesa file (lines) | On a straight-line float shader | Class |
|---|---|---|---|
| `nir_split_array_vars`, `nir_shrink_vec_array_vars`, `nir_opt_deref` | `nir_split_vars.c` (1,811), `nir_deref.c` (1,640) | nothing | N, guard: array deref (M2/M4) |
| `nir_lower_vars_to_ssa` | (905) | nothing after the first run | S1 (one block) |
| `nir_lower_wrmasks` | (111) | nothing (no masked memory stores) | N |
| `nir_opt_find_array_copies` (first call only) | (684) | nothing | N |
| `nir_opt_copy_prop_vars`, `nir_opt_dead_write_vars`, `nir_opt_combine_stores` | (1,634), (322), (429) | nothing (no derefs left) | N, guard: a deref left |
| `nir_lower_alu_width`, `nir_opt_vectorize` (`bi_vectorize_filter`) | (511), (723) | splits; `vectorize` pairs nothing at 32 bits (filter width 1) | S1 (both: vectorize also runs on the blend shader's 16-bit code) |
| `nir_opt_copy_prop`, `nir_opt_dce`, `nir_opt_cse` | `nir_opt_copy_propagate.c` (194), (271), (115) + `nir_instr_set.c` (888) | yes | S1 |
| `nir_opt_peephole_select` (limit 64) | (628) | nothing (no if) | N, guard: `If` (M4) |
| `nir_opt_idiv_const` | (229) | nothing | N, guard: int division (M2) |
| `nir_opt_algebraic` | `nir_search.c` (916) + rules | yes | S1 |
| `nir_opt_constant_folding` | (633) | yes | S1 |
| `nir_opt_dead_cf`, `nir_opt_loop`, `nir_opt_if` (arch >= 9) | (459), (780), (1,594) | nothing | N, guard: `If`/`Loop` (M4) |
| `nir_opt_phi_to_bool`, `nir_opt_loop_unroll`, `nir_opt_remove_phis` | (235), (1,207) + `nir_loop_analyze.c` (1,598), (211) | nothing | N, guard: phi or loop (M4) |
| `nir_opt_undef` | (345) | `undef` lanes left by constructors | S1 |
| after the loop: `nir_lower_undef_to_zero`, `nir_remove_dead_variables` | (68), (206) | an `undef` left becomes 0 | S1 |

### 5.5 `bi_optimize_late` (`:279-397`) and the start of compilation

| Pass (order) | Mesa file (lines), call site | On a straight-line float shader | Class |
|---|---|---|---|
| `nir_opt_shrink_stores` | (132), `:283` | nothing (stores already scalar) | N |
| `bi_optimize_loop` | `:284` | 5.4 | S1 |
| `nir_opt_shrink_vectors` | (634), `:286` | narrows loads to the lanes read | S1 |
| `nir_opt_load_store_vectorize` (ubo, global, shared, ssbo) | (2,076), `:298-310` | joins the scalar UBO loads C16 split (the join `select.rs:455-530` models for the walks) | S1 (UBO loads with constant offsets) |
| `pan_nir_fuse_io_cvt` | (176), `:312` | nothing at 32 bits | N, guard: `f2f16` next to IO (M3) |
| `nir_opt_algebraic` until no progress | `:316-320` | yes | S1 |
| `nir_lower_int64` | (1,466), `:327` | nothing | N |
| `nir_lower_bit_size` | (434), `:330` | nothing | N, guard (C20) |
| late loop: `nir_opt_algebraic_late`, `nir_lower_alu_width`, folding, copy_prop, dce, cse | `:335-344` | yes: the `ffma` fusion happens here | S1 |
| `bifrost_nir_opt_boolean_bitwise` (arch < 9) | `:347-348` | not on v10 | none |
| `pan_nir_lower_bool_to_bitsize` | (318), `:350` | nothing without bools | N, guard: a 1-bit def (M2) |
| `nir_lower_alu_width`, `nir_opt_vectorize` | `:351-352` | as 5.4 | S1 |
| `bifrost_nir_lower_algebraic_late` until no progress | `:355-361` | `fsat`/clamp forms | S1 |
| late loop again if anything changed | `:363-371` | yes | S1 |
| `nir_opt_sink`, `nir_opt_move` (all) | `:375-381` | moves loads and constants to their users | S1 |
| `bi_lower_divergent_indirects` | `:385-396` | only with indirect IO | N |
| `nir_lower_load_const_to_scalar`, folding, `nir_lower_phis_to_scalar`, copy_prop, dce | `bifrost_compile_shader_nir`, `:1282-1287` | yes | S1 |
| IDVS variant: clone, `nir_inline_sysval(load_shader_output_pan)`, loop of folding, dce, dead_cf, cse | `bifrost_compile.c:4374-4400` | each VS variant keeps its own stores | S1 |
| `nir_index_blocks`, `nir_index_ssa_defs`, print | `bifrost_compile.c:4409-4420` | the NIR at selection | S1 (the checkpoint of 7.2) |

Step 1 ports, by this list, about 30 Mesa passes (most small). The guards
are the list of what M2, M3, M4, L and S14 must remove, one by one.

## 6. The bridge to `emit.rs` and the back half

### 6.1 What the back half takes today from the walks

- **Vertex:** `nir::build` (`nir.rs:4154`) makes a `nir::Nir` (`:719`):
  nodes `NirNode` (`:605-615`) with `Kind` (`:394-520`: `Input`, `Uniform`,
  `Sysval`, `Alu`, `Lea`, `Addr`, `Store`, ...) and `Val` sources
  (`:97-102`), plus varyings, masks, blocks and regions.
  `select::UboLayout::vs` (`select.rs:417`) reads which dwords it loads;
  `select::vs_variant` (`select.rs:831`) makes the BIR of each IDVS variant
  (`mod.rs:1439`, `:1450`); `emit::emit` makes each ALU row
  (`select.rs:1022`). Then `finish` (`mod.rs:886-899`).
- **Fragment:** `nir_fs::walk` (`nir_fs.rs:4389`), motion, `nir_fs::layout`,
  one of six selectors (`mod.rs:1543-1690`), `UboLayout::fs32/fs16`
  (`select.rs:541`, `:579`), then `finish`.
- **`emit.rs`** is keyed on `nir::Alu` (`emit.rs:12`) with 99 rows
  (`emit.rs:39`), each one `bi_emit_alu` case at one bit size and width.
- **`finish_passes`** (`mod.rs:901-945`) starts at `bi_opt_push_ubo` and then
  follows `bi_compile_variant_nir` from `mod_prop`. It does **not** run
  `bi_lower_swizzle`, `bi_opt_copy_prop`, `bi_opt_constant_fold` or the
  first `bi_opt_cse` + `bi_opt_dce` (`bifrost_compile.c:4442-4476`): the
  walks' selectors emit BIR that is already in that form (for example
  "copy propagation takes the immediates into the COLLECT",
  `blend.rs:281-284`).
- **The record** (`StageCode`, `mod.rs:960-990`): attributes, varyings,
  samplers, colour outputs, `compacted`, `helper_threads`, `can_discard`,
  `rows`, `marks`, `prods`.

### 6.2 What changes

1. **`snir/select.rs`** transcribes `bi_emit_instr` (`bifrost_compile.c:3626-3659`)
   over the NIR at selection, block by block (`emit_block` `:3672-3695`,
   later `emit_if`/`emit_loop` `:3697-3770` and deferred phis
   `:3577-3624` for M4):
   - `load_const`: `bi_emit_load_const` (`:2227`);
   - ALU: a generated-op to `emit::Row` map (`snir/rows.rs`), then
     `emit::emit` unchanged. A (op, bits, width) with no row is a refusal
     (`MaliError::Arithmetic`), as today. `bi_emit_alu`'s source handling
     (swizzles to halves, `bi_alu_src_index`) is transcribed once here, not
     per shape;
   - intrinsics of step 1, each a case of `bi_emit_intrinsic` (`:1579`):
     VS `load_input`/`load_attribute_pan` (`:1594-1611`, via
     `select::ld_attr`, `select.rs:234`), `load_ubo` (`:1723`), `lea_buf_pan`
     (`:1966`, via `select::lea`, `select.rs:220`), `store_global` (`:1736`,
     via `select::store`/`address`, `select.rs:790-830`),
     `load_idvs_output_buf_index_pan`; FS `load_var_pan`/`load_var_buf_pan`
     (`:1612-1632`), `load_cumulative_coverage_pan`, `atest_pan`
     (`:1633`), `load_blend_descriptor_pan`, `blend_pan` (`:1658`). The
     `select.rs` helpers become `pub(super)`; nothing else in them changes.
   - the BIR value of a def is `bi_def_index`: Mesa numbers BIR values by the
     NIR index (`bifrost_compile.c:4419-4423`). `ra.rs` renumbers by first
     appearance before LCRA (`ra.rs:16`), so the numbering does not reach the
     registers; whether any other pass reads it is open (Q6).
2. **`select::UboLayout::from_loads(m, loads)`**: the uniform block layout
   from the list of `load_ubo` the NIR keeps, instead of from `nir::Nir`
   (`UboLayout::vs` and `fs32` stay for the old path). It feeds
   `fau::push_ubo` as today.
3. **`emit::Alu` moves out of `nir.rs`** into `emit.rs` (a pure move with a
   re-export), so `emit.rs` no longer depends on the walk it will outlive.
4. **Four early BIR passes in `opt.rs`** (step S11a): `bi_lower_swizzle`
   (`bi_lower_swizzle.c`, 271 lines), `bi_opt_copy_prop` (492),
   `bi_opt_constant_fold` (96), and the first `bi_opt_cse` (178) + dce, in
   `bifrost_compile.c:4442-4476` order. They run in a new
   `finish_passes_nir`, which then calls the shared tail of
   `finish_passes`. The old path's `finish_passes` keeps its order, so its
   words cannot move.
5. **The record** is filled from the NIR and the layout: attributes read
   (`load_input` locations), varyings written (VS `store_global` sections) or
   read (FS loads), colour outputs (`blend_pan`), and the `rows` mask
   (`emit::bit`).
6. **IDVS**: the two VS variants come from the NIR clone of 5.5, each
   selected and finished as today (`mod.rs:1436-1460`). `no_psiz_copy`
   (`mod.rs:947`) stays as it is.

## 7. Shadow mode

### 7.1 Where both paths run

- **In the product: nowhere.** `snir/` is not compiled without
  `host-test` (decision 1). `compile_stage` (`mod.rs:1345`) is unchanged
  until a class moves (7.4).
- **In host tests:** a new test file `tests/mali_snir_shadow.rs` walks every
  committed pair (`tools/mali/testdata/<name>.{vert,frag,expect.json}`, the
  set `tests/mali_ref/mod.rs` reads). For each stage it runs the classifier;
  for a stage in class it compiles through the new path (an entry
  `mali::compile_module_nir`, `cfg(feature = "host-test")`, the twin of
  `compile_module_on`, `mod.rs:1272`) with the same link facts the old path
  gets (`compile_pair` in `tests/mali_ref/mod.rs:189`, the GL program's
  relink and link). The old path runs as today.
- **In `snir/` unit tests:** the printer, the passes and the folds against
  hand cases and `tools/mali/testdata/nir_fold_vectors.txt`.

### 7.2 How results are compared

Two checkpoints, each against Mesa, never only against the old path:

1. **The NIR at selection.** Mesa prints it for every variant under
   `BIFROST_MESA_DEBUG=shaders` (`bifrost_compile.c:4409-4415`), and
   `tools/mali/drm_shim_ref.sh` keeps that output in `<prefix>.stderr`
   (`drm_shim_ref.sh:18`, `:100-101`; example
   `tools/mali/testdata/fbload_fbload.stderr:2-50`). A new script
   `tools/mali/nir_refs.sh` re-runs the shim for each committed pair (a
   host with `MESA_PAN_TREE`), cuts each `impl main { ... }` block, and
   normalises it: header lines and `decl_var` lines dropped, SSA names
   renumbered in order of definition, `// name` comments dropped, everything
   else (sizes, swizzles, intrinsic indices, constants) kept. It commits one
   small file per pair, `tools/mali/testdata/<name>.nir.sha`: one line per
   variant (`vs.position`, `vs.varying`, `fs`) with the SHA-256 of the
   normalised text. One file per pair keeps parallel pull requests free of
   conflicts (no shared list, no count line). The full texts are not
   committed (about 3 KB a variant, about 10 MB for all pairs, an estimate);
   the script writes them to a local directory for debugging. With a debug
   Mesa build, `NIR_DEBUG=print_vs,print_fs` prints the NIR after every pass
   (`tools/mali/README-pan_shc.md:378-381`), which locates the first pass
   that differs.
2. **The words.** The new path's regions, `work_reg_count`, preload and FAU
   against the reference's (`tests/mali_ref/mod.rs:583` `differences`), and
   against the old path's output.

**An early checkpoint for the translation alone.** The two checkpoints above
need every pass of section 5, so they give their first number late, and the
questions that can change this design (Q1, Q3, R2, R3) are all about the
translation. The shim reference is a debug build (`drm_shim_ref.sh:60`), so
`NIR_DEBUG=print_vs` and `print_fs` work on it today. `nir_refs.sh`
therefore also keeps one early point per stage, the line `ssa` in
`<name>.nir.sha`.

- **Why not `glsl_to_nir`'s own print.** It still holds Mesa's GLSL IR: an
  empty function `gl_mesa_tmp_<hash of the source>`, user functions not
  inlined, constructor and constant temporaries as variables (`vec_ctor`,
  `const_temp`, `mat_ctor`, `flattening_tmp`) read through `deref_var`.
  `ir::Module` records none of these, and Mesa's first variable passes
  remove them all. (Measured for #11273 on the 1,575 pairs.)
- **Where the point is.** `NIR_PASS` prints a pass's name before the pass
  runs, and the NIR after it only when the pass changed the shader
  (`nir.h:5264-5296`); both only for a stage `NIR_DEBUG` names. So a dump
  alone does not say where in the chain it is, and the point is fixed by a
  name instead: **the first `nir_opt_find_array_copies` in the stage's own
  stream** (one draw with `print_vs`, one with `print_fs`). That name is
  printed only by `gl_nir_opts` (`gl_nir_linker.c:67`), right after its
  `nir_lower_vars_to_ssa` and `nir_remove_dead_variables` (`:55-66`), in the
  first round the linker runs for the stage. The reference is the last dump
  of the stage before that name: locals are SSA values, no optimisation of
  `gl_nir_opts` has run.
- **What the line holds.** `<variant> ssa : <sha256> : <pass>,<pass>,...`:
  the hash of the normalised dump, and the names of the passes that printed
  a dump before the point (the passes that changed this shader), in order.
- **What the shadow test does with it.** The translation's output stands
  for the passes that only undo GLSL IR form (`nir_lower_variable_initializers`,
  `nir_lower_returns`, `nir_inline_functions`, `nir_opt_deref`,
  `nir_split_var_copies`, `nir_lower_global_vars_to_local`,
  `nir_lower_var_copies`); the test then runs `lower_vars_to_ssa` (S6b) and
  the removal of dead variables. Any other name in the line's list is a pass
  Mesa needed here: the test runs our port of it, or gives `refused` with
  that name when there is none yet (`nir_lower_mediump_vars`,
  `nir_opt_constant_folding`, ...). So the early number never reports a
  `differ` that is only a pass not written yet.

This number needs S1, S10a, S6b's `lower_vars_to_ssa` and the S12 script,
and it is the first one the work produces.

Per stage and pair the test gives one verdict: `equal` (both checkpoints
equal), `nir-equal` (NIR equal, words not yet comparable before step S11),
`refused` (with the guard or gap text), or `differ`. **The test fails on any
`differ`**: the new path must be Mesa's or refuse, which is the rule the walks
follow too. It prints the counts per verdict; it pins none (a pinned count
is a line every branch edits). `#11252`'s differential gives the same counts
per commit over the whole corpus, with the new path as a column.

### 7.3 The rule for moving a class to the new path

A class is a classifier predicate (section 1 is the first one: "VS,
straight-line, float, highp, no tex, trivial link"). It moves when, at one
main commit:

1. every committed reference pair in the class is `equal` through the new
   path (0 refused, 0 differ in the shadow test, for that class);
2. on #11252's corpus run, no shader of the class that is `equal` on the old
   path is anything else on the new path, and the new path's `equal` count
   for the class is at least the old one's;
3. the board result of the class does not depend on this move (words are
   equal), so no board run is needed; a class whose words were not all
   compared (a corpus shader without a reference) is drawn first.

### 7.4 How the product path stays unchanged until then

- Before a move: `snir/` is not in the product build; the old path is the
  only path.
- The move commit: `compile_stage` asks `snir::scope::owns(m, link)` first.
  If it owns the stage, the new path compiles it; **if the new path refuses
  a stage it owns, the old path compiles it** (a fallback, so a move can
  only lose compile time, never a shader). The `cfg` widens to the product
  in the same commit. The gate's Mali rows, the blend rows and the
  `tests/mali_*` files are the proof that words did not move.
- The old walk code for a class is removed when no class uses it any more
  (the road note: the new half replaces `nir.rs`, `nir_fs.rs` and
  `vec16.rs`), each removal its own commit.

## 8. The cut into steps

All sizes and days are **estimates** (lines of Rust or Python we write,
generated files excluded; days of one person). "Accept" is the test that
proves the step; each step lands as its own pull request, rebased on the
current main and checked with `tools/check.sh --changed origin/main`.

| Step | What, files | Size | Days | Accept | Depends on |
|---|---|--:|--:|---|---|
| **S1** snir core | `snir/mod.rs` (section 2.2: defs, srcs, instrs, blocks, cf tree, vars, linked use lists, cursors, builder), `snir/print.rs` | 1,800 | 5 | unit tests: use-list order after insert/rewrite/rewrite-all/remove equals the order `nir.c:1231-1262`, `:1739-1745` and `nir.h:4873-4880` give on hand cases; the printer reproduces `fbload_fbload.stderr:21-50` from a hand-built NIR | none |
| **S2** generator, ops | `tools/mali/gen_nir_tables.py` (loader, condition evaluator, `NirOp` + info, `NirIntrinsic` subset), `snir/opcodes.rs`, `snir/intrinsics.rs`, check.sh row (regenerate + cmp), `tools/mali/snir_ops.txt` | 450 | 3 | the gate row; a test that the evaluator gives the old table's 18 known strings the old values | none |
| **S3** generator, rules | `gen_nir_tables.py` rules part; `snir/rules_{opt,late,bifrost}.rs` | 350 | 3 | the gate row; the blend subset of the new tables lists the same 207 transforms in the same order as `blend_nir_rules.rs` | S2 |
| **S4** generator, folds | `gen_nir_tables.py` fold part (C subset to Rust); `snir/fold.rs`, `snir/fold_helpers.rs`; `tools/mali/nir_fold_vectors.c`, `tools/mali/testdata/nir_fold_vectors.txt` | 1,250 | 5 | Rust folds equal Mesa's vectors bit for bit; gate row rebuilds the vectors | S2 |
| **S5** nir_search | `snir/search.rs` (from `blend_nir.rs:2102-2655`: automaton with item bitsets, matching with comm orders and Mesa's limits, `fp_math_ctrl`, conditions, replacement with bit sizes), `snir/fp_class.rs` (from `:1566-2048`) | 1,200 | 5 | unit cases from `nir_search.c`; then S8 | S1, S3, S4 |
| **S6a** passes, basic | `snir/opt_copy_prop.rs`, `opt_dce.rs`, `opt_cse.rs` (`nir_instr_set.c` semantics), `opt_constant_folding.rs`, `lower_load_const_to_scalar.rs`, `opt_undef.rs`, `lower_undef_to_zero` | 900 | 4 | unit cases; S8 | S1, S4 |
| **S6b** passes, shape | `lower_alu_width.rs`, `opt_vectorize.rs`, `opt_shrink_vectors.rs`, `opt_move.rs`, `opt_sink.rs`, `lower_flrp.rs`, `lower_vars_to_ssa.rs` (one block, masked stores as `nir_lower_vars_to_ssa.c:710-750`) | 1,600 | 5 | unit cases; S8 for the blend-shared ones | S1 |
| **S7** pipeline | `snir/pipeline.rs`: section 5 in order, every N row a guard with its refusal text, the IDVS clone | 500 | 3 | a test that runs the pipeline on a hand NIR with each guard's trigger and gets that refusal; S8 | S5, S6a, S6b (a skeleton can start after S1) |
| **S8** blend on snir (shadow) | `blend_nir::build` on `snir::Builder`; blend runs both engines in tests | 600 | 4 | all 4,103 keys (`blend_deqp.words`, `blend_random.sha`; `tests.rs:5199`, `:5213`) give the same words through `snir` | S5, S6a, S6b, S7 |
| **S8b** blend switch | `blend.rs` reads `snir`; delete `blend_nir.rs`'s passes, `blend_nir_rules.rs`, `gen_blend_nir_rules.py` and their check.sh row | -4,000 | 2 | the blend gate rows and tests unchanged | S8 |
| **S9a** IO, vertex | `snir/io_vs.rs`: outputs at the end (`nir_lower_io_vars_to_temporaries` model), `load_ubo` form, `nir_lower_viewport_transform`, `nir_lower_point_size`, `panfrost_nir_lower_sysvals` (viewport), `pan_nir_lower_vs_outputs`, `nir_lower_mem_access_bit_sizes` (stores, UBO loads), `nir_opt_load_store_vectorize` (UBO) | 1,400 | 5 | per pass, NIR before and after on hand cases taken from shim dumps | S1 |
| **S9b** IO, fragment | `snir/io_fs.rs`: `nir_lower_fragcolor`, `panfrost_nir_remove_fragcolor_stores`, `pan_nir_lower_fs_inputs`, `pan_nir_lower_fs_outputs` | 800 | 4 | as S9a | S1 |
| **S10a** translation core | `snir/from_ir.rs`, `snir/scope.rs` (section 1 classifier, `Unknown` refused), `glsl::passes` entry for the new path | 1,200 | 5 | the classifier's in-class count over the committed pairs is printed; translation of 10 hand shaders printed and read against Mesa's NIR after `glsl_to_nir`; then the early checkpoint of section 7.2 over the in-class pairs | S1, S2, #11266 |
| **S10b** origin table | `ir.rs` (`Origin`), `lower.rs`, `builtin_call.rs` record it | 400 | 4 | the other back ends' gate rows unchanged; front-end unit tests per origin; `Module::prec` unchanged on all pairs | S10a's classifier (to count origins in class) |
| **S10c** built-ins as Mesa | `snir/builtins.rs`: `builtin_functions.cpp` bodies of the ES 2 float built-ins in class | 800 | 5 | NIR-equal on the references that call each built-in | S10a, S10b |
| **S11a** early BIR passes | `opt.rs`: `lower_swizzle`, `copy_prop`, `constant_fold`, `cse` (first), `finish_passes_nir` in `mod.rs` | 900 | 5 | BIR unit cases; the old path's words unchanged (all Mali rows) | none |
| **S11b** selection, vertex | `snir/select.rs` (VS part), `snir/rows.rs`, `UboLayout::from_loads`, the `emit::Alu` move, VS record | 900 | 5 | VS words equal on the in-class pairs that are NIR-equal | S1, S9a, S11a |
| **S11c** selection, fragment | `snir/select.rs` (FS part), FS record | 800 | 5 | FS words equal on the in-class pairs that are NIR-equal | S1, S9b, S11a |
| **S12** references and shadow test | `tools/mali/nir_refs.sh` (+ normaliser), `tools/mali/testdata/*.nir.sha`, `tests/mali_snir_shadow.rs`, a self-test row in check.sh | 700 | 4 | the script's normalised text for `fbload_fbload` equals a hand-normalised copy (both the `ssa` point and the one at selection); the test runs on all pairs and reports counts | S1 (printer) |
| **S13** product hook and first move | `mod.rs` dispatch (`owns`, fallback), `cfg` widened | 300 | 3 | section 7.3 for the VS class | S11b, S12, #11252 |
| **S14** (after step 1) look-ups | `pan_nir_lower_tex`, `nir_lower_tex` subset, `bi_emit_tex_valhall` (`bifrost_compile.c:3489`) | 1,500 | 6 | in-class pairs with `texture2D` equal | S11b/S11c |

Total for step 1 (S1 to S13): about 17,000 lines written and 85 person-days
(estimate), about 4,000 lines removed by S8b.

**Order and parallel lanes.**

```text
week 1   S1  S2  S11a  S12(script part)  S10b      (S10a needs #11266 landed)
week 2   S3  S4  S6a  S6b  S9a  S9b  S10a  S12
week 3   S5  S7 (needs S5, S6)  S10c (after S10b)
week 4   S8  S11b  S11c
week 5   S8b  S13
```

- Independent from the start: S1, S2, S11a, S12's script.
- After S1 (and S2 for the enum): S6a, S6b, S9a, S9b, S10a, the S12 test,
  in parallel by different people; S3 and S4 after S2, in parallel.
- S5 joins S1, S3 and S4; S7 joins S5 and S6; S8 joins S5 to S7.
- S11b and S11c are parallel, each after its IO step and S11a.
- Generated files have one owner each (S2: opcodes, S3: rules, S4: folds),
  so two pull requests never regenerate the same file.

**Which lane goes first.** The engine (S2 to S8) repeats what the blend
engine already proved, so its risk is size, not design. The translation
(S10a to S10c) is where this design can be wrong. When people are short, the
lane S1, S12 script, S10a and S6b's `lower_vars_to_ssa` goes first, to get
the early checkpoint of section 7.2.

**The order after step 1.** Step 1 moves shaders the old path compiles
already, so it adds no dEQP pass by itself. M2 (int, bool), M3 (precision),
M4 (control flow) and look-ups (S14) are ordered by #11252's count of
refused must-pass cases per class, largest first, not by this note.

**The first measurable numbers.**

0. **S1 + S10a + S6b's `lower_vars_to_ssa` + the S12 script**: stages whose
   NIR at the `ssa` point is Mesa's (the early checkpoint of section 7.2),
   the first number for the translation.
1. **S8**: 4,103 blend keys through the new engine (search, passes, generated
   tables). This is the first proof of the engine, before any translation.
2. **S12 once S7, S9a and S10a are in**: "vertex shaders whose NIR at
   selection is Mesa's", per class, out of about 400 in-class vertex shaders
   (section 1's estimate). This is the first number for the translation.
3. **S11b**: the same set by words; S11c adds fragment shaders.

## 9. Risks and open questions

Each has the cheapest way to answer it.

| # | Risk or question | How to answer it cheaply |
|---|---|---|
| R1 | **Orders that hang on pointers or hash slots** (cse's instruction set, the vectoriser's entry sort, range-analysis caches) make "NIR equal" unreachable for some shaders. | #11255 answers it. Until then, the S12 per-pass dumps (`NIR_DEBUG=print_*`) show at which pass a pair first differs; a pointer-order pass shows as a differ that changes between two Mesa runs of one shader. |
| R2 | **The origin table is refused or late**, and recognising the front end's expansions instead is lossy. | Count, over the in-class pairs, how many values have a non-`Plain` origin by a one-off scan of `ir::Module` after S10a: if few, step 1 can start with a guard on them. |
| R3 | **Translation emits the lowered IO form directly**, while Mesa runs `gl_nir_opts` on the deref form first; CSE or move order could differ. | Compare the S12 NIR for 20 vertex pairs; if they differ at the first `nir_opt_cse`, add the deref form for inputs and uniforms (vars exist in the structure already). |
| R4 | **Rule tables too large for the product image**, or the item bitsets too large for the scratch arena. | The S3 generator prints node counts with and without pruning; S5 records the scratch peak (`store.rs` `SCRATCH_PEAK`) on the largest committed shader (femtovg `filter_image`, 1,153 walk nodes, `nir.rs:59-61`). |
| R5 | **`nir_opt_varyings` is large** (6,283 lines) and step 1 takes its results from the old path's link facts. | Step 1 guards on motion and constants; the port is its own item after step 1. The shadow counts show how many in-class pairs the guard refuses. |
| R6 | **The early BIR passes (S11a) see forms the walks never made** and find a defect in `fau.rs` or `sched.rs`. | S11a's own BIR tests, then S11b's words; the old path is untouched, so a defect shows only on the new path. |
| R7 | **Shadow test run time** over 1,575 pairs, two paths. | Time it in S12; if too slow for `--quick`, run the in-class subset in the gate row and the full set in the nightly. |
| R8 | **Range analysis subset** (`nir_range_analysis.c`, 2,728 lines) too small for the full rule set's `is_*` conditions. | S5 counts refusals by condition name on the corpus; port the classes the counts name. |
| R9 | **Folds of transcendental ops** differ from Mesa's host libm in the last bit. | `nir_fold_vectors.txt` (S4) on the reachable ops; `mesa_fold.rs`'s levels (`mali-libm-fold.md`) already cover `fsin`, `fcos`, `fexp2`, `flog2`, `fpow`. |
| Q1 | Does Mesa's GLSL front end fold constant expressions before NIR (`ast_to_hir`), and so does our front end fold more than Mesa does there? | One shader with `1.0 / 3.0 + a.x` through a debug Mesa with `NIR_DEBUG=print_vs`: read the NIR after `glsl_to_nir`. |
| Q2 | Is `nir_opt_constant_folding`'s handling of `poison` (`f2i32` of NaN) reachable in class? | Grep the reachable set for ops whose `const_expr` sets `poison`; only `f2i*`/`f2u*` are expected, out of class until M2. |
| Q3 | Must `load_input`/`load_ubo` pass through `load_deref` for exactness? | R3's comparison. |
| Q4 | Does `pan_nir_resize_varying_io` (C5) change 32-bit highp IO? | Read `pan_nir_resize_varying_io.c` (115 lines) against one shim dump. |
| Q5 | Which `st_program.c` variant passes (`st_finalize_nir` callers at `st_program.c:403`, `:854`, `:1220`) run for an ES 2 program on panfrost? | Read `tools/mali/pan_shc.c`, which transcribes them, then one debug-Mesa run. |
| Q6 | Does any BIR pass after selection read the BIR value numbers (Mesa takes them from the NIR index)? | Read `opt.rs` `cse` and `fau.rs` `lower_constants` for index-ordered loops; #11255 covers Mesa's side. |

## 10. Not checked

- No code was built or run for this note. The pass effects in section 5 are
  read from Mesa's sources and from the one dump `fbload_fbload.stderr`;
  they are not measured per pass.
- Section 1's class counts come from regular expressions over the sources,
  not from the classifier; gl_FragCoord, invariance and link facts are not
  in them.
- Whether `nir_lower_io` and `nir_lower_uniforms_to_ubo` insert offset
  instructions whose positions survive to selection (R3, Q3).
- The exact behaviour of `nir_opt_varyings_bulk` on straight-line shaders
  with trivial link facts (only dead-output removal is assumed).
- `st_program.c` variant passes (Q5); `pan_nir_resize_varying_io` (Q4).
- How `nir_opt_constant_folding` uses `poison` (Q2).
- The size of the pruned rule tables and the scratch use of the item bitsets
  (R4).
- The `nir_intrinsics.py` subset's exact list (S2 makes it).
- Whether the first `bi_opt_cse` (`bifrost_compile.c:4473`) changes any BIR
  of the in-class shaders (S11a's tests answer it).
- The size estimates in section 8.
- #11254's results are taken as stated by that study (branch `issue/11254`,
  commit `193e8cae7`, not on main at the tip); #11266 is assumed landed.
