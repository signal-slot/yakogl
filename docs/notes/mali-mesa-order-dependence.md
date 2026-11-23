<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: orders in Mesa 26.2.1's ES 2 shader path that hang on pointers, hash slots or sort ties, and whether a port can reproduce them

Item #11255 (part of #11251; `mali-compiler-road-to-es2.md` section 5,
problem 1). Read-only study, 2026-11-24. Mesa paths are relative to the Mesa
26.2.1 tree (`src/...`). It tells the Rust shader NIR (#11253) which orders
it must model.

## 0. Answer in brief

- **32 places** on the path an ES 2 shader takes on Valhall v10 iterate a
  hash table or set, sort with `qsort`, or order by a value that is not in
  the source (section 3). About 25 more files in the same directories have
  such a walk or sort, but an ES 2 shader on panfrost does not reach them
  (section 3.4).
- **25 are reproducible as they are**: the result does not depend on the
  order (a set is filled, a union of kills, a monotone fixpoint, a lookup in
  a set that never holds two equal entries), or the sort is total.
  Many of them iterate **pointer-keyed** tables, whose slot order changes
  from run to run with address-space randomisation; the port does not need
  the order, only the same result.
- **6 need a named model** (3 models, section 6): the use-list order of
  each SSA value (insertion order, the nir_search worklist reads it), Mesa's hash table
  slot order for **content** keys (the constant histogram, already in
  `fau.rs` `U64Table`; the spiller's next-use tables, which also need
  deletion), glibc's `qsort_r` as a **stable** sort (measured), and the
  constant sort that compares element addresses (identity, measured).
- **2 residues are not reproducible from the input** (#8, and a corner
  of #28), and in theory Mesa's
  own words for one shader can differ between two runs there:
  `nir_opt_varyings` output deduplication (a lookup whose hash walks a
  pointer-hashed set) and the constant sort when its array crosses a
  4 GiB-aligned address. #8 is not reachable from an ES 2 program (it
  needs a geometry shader); the second has a chance of about 1e-7 per compile (estimate).
  Neither was seen (section 4.6, 4.7).
- **Measured** (section 5): 324 ES 2 programs, each compiled by Mesa's
  compiler (`pan_shc`) under 5 address layouts (randomisation on twice, off,
  off with a different allocation pattern, on with mmap'd allocations): the
  words, FAU, layouts and disassembly were identical in all 324. In the same
  programs, the dominance-frontier set walk of the phi builder visited
  blocks in a different order between runs in 62 of 284 loop programs.
- NIR SSA numbers follow creation order inside the pass loop, but
  `bifrost_compile.c:4421` renumbers them in program order before any BIR
  is built. Creation order changes no word (section 4.5).

## 1. Method

- **Read**: every file under `src/compiler/nir`, `src/compiler/glsl`,
  `src/mesa/state_tracker`, `src/mesa/program`, `src/panfrost/compiler`,
  `src/gallium/drivers/panfrost` and `src/util/dag.c` that contains
  `hash_table_foreach`, `set_foreach`, `hash_table_u64_foreach`,
  `hash_table_call_foreach`, `_mesa_hash_table_next_entry`,
  `_mesa_set_next_entry`, `qsort` or `util_qsort_r` (52 files in NIR alone;
  no `std::map`/`unordered_*` exists in these trees). For each, whether an ES
  2 shader on v10 reaches it, from the pass lists in
  `src/compiler/glsl/gl_nir_linker.c` (`gl_nir_opts` :46-130,
  `preprocess_shader` :1310-1391, `prelink_lowering`, `nir_opt_varyings_bulk`
  :1524), `src/mesa/state_tracker/st_glsl_to_nir.cpp`,
  `src/gallium/drivers/panfrost/pan_shader.c`, and
  `src/panfrost/compiler/bifrost/bifrost_nir.c` (`bi_optimize_loop`
  :202-276, `bi_optimize_late` :279-395, `bifrost_preprocess_nir` :400-432,
  `bifrost_postprocess_nir` :864-1093, `bifrost_compile_shader_nir`
  :1268-1290). Then the code that consumes each iteration.
- **Also read**: the containers (`src/util/hash_table.c`, `set.c`,
  `u_qsort.h`), use lists (`nir.c`, `nir.h`), `nir_worklist.h`,
  `nir_instr_set.c`, `nir_search.c`, `nir_opt_vectorize.c`, `nir_opt_cse.c`,
  and every NIR or BIR place that orders by SSA index.
- **Run** (section 5): `pan_shc` (`tools/mali/README-pan_shc.md`, the debug
  build of Mesa 26.2.1 on this host) through `tools/mali/glsl2pan.py`,
  under `setarch -R` and `GLIBC_TUNABLES`; gdb on the same binary to print
  two set walks; two small C programs for glibc's `qsort_r`. The drm-shim
  driver (`tools/mali/drm_shim_ref.sh`) is not built on this host, so the
  GLSL-only places (glsl_to_nir, `lower_precision`, the varying linker) were
  read, not run.

## 2. What decides each container's order

| # | Container | Order of a walk | Port |
|---|---|---|---|
| U1 | `struct hash_table` (`util/hash_table.c`) | Slot order (`_mesa_hash_table_next_entry` :628). Open addressing: start `hash % size`, step `1 + hash % rehash` (:485-519); sizes from `hash_sizes` :94-111 starting at 19 slots (inline storage); grow when `entries >= max_entries`, rehash at the same size when `entries + deleted >= max_entries` (:479-483); a rehash reinserts in old slot order (:461); remove leaves a tombstone (:579-589), and insert reuses the first tombstone on the probe path (:493-526); clear keeps the size (:319-339). | For content keys: deterministic, a model of these rules reproduces it (`fau.rs` `U64Table` does it for insert-only). |
| U2 | `struct set` (`util/set.c`) | The same algorithm (`hash_sizes` :57, insert :463-511). | As U1. |
| U3 | Pointer keys: `_mesa_hash_pointer` (`hash_table.c:735-739`, `(p>>2)^(p>>6)^(p>>10)^(p>>14)`), `_mesa_pointer_hash_table_*`, `_mesa_pointer_set_*` | Slot order follows the heap addresses. Randomisation moves the heap base by whole pages, and the hash mixes bits 14 and up, so the slots move between runs (measured, section 5). It also depends on every allocation before it in the process (a dEQP process compiles thousands of shaders). | **Not reproducible.** The port must not need it: each walk of such a table has to be shown order-independent (section 3). |
| U4 | `qsort`, `util_qsort_r` (`u_qsort.h:55-61` calls glibc `qsort_r`) | glibc 2.44 sorts by merge sort, stable while its temporary buffer can be allocated. | Stable sort (Rust `sort_by`). Measured: 0 of 3,550 arrays with ties left unstable (sizes 2 to 16,384). |
| U5 | Use lists (`nir.c:1236`, `:1687`, `nir.h:4877-4879`), `exec_list`, `list.h`, `u_dynarray`, `nir_instr_worklist` (FIFO), block predecessors (a `u_dynarray`, `nir.h:3423`) | Insertion order. `nir_src_rewrite` moves a use to the tail of the new value's list; `nir_def_rewrite_uses` (`nir.c:1739-1745`) moves them in old order. | Deterministic from the input; the port must perform the same list operations in the same order. |
| U6 | SSA index (`nir.c:1249`, `:1730`: `impl->ssa_alloc++` at creation) | Creation order, holes for deleted values, until `nir_index_ssa_defs` (`nir.c:2207`). `nir_print_shader` renumbers too (`nir_print.c:3176`), so a NIR dump changes it. | Only needed for exact intermediate numbers; renumbered before emission (section 4.5). |
| U7 | ralloc / gc addresses | Allocation order. | Matter only through U3 and the address-comparing sort (4.7). |

## 3. The places

"Reach" says whether an ES 2 shader on v10 reaches the place. "Order from"
names what the walk order depends on (U1-U6). "Verdict": **as is** (the
result does not depend on the order, or the sort is total), **model** (the
port must reproduce the order with the named model), **residue** (not
reproducible from the input).

### 3.1 GLSL linker and state tracker

| # | Mesa file:line | Pass | What is iterated / sorted | What it decides | Order from | Verdict |
|---|---|---|---|---|---|---|
| 1 | `compiler/glsl/gl_nir_linker.c:446` | `remove_dead_functions` | pointer set of functions never called | which functions are unlinked | U3 | as is: removal of a set |
| 2 | `compiler/nir/nir_functions.c:581` | `nir_shader_fully_linked` | pointer set of used functions | a boolean | U3 | as is |
| 3 | `compiler/glsl/gl_nir_lower_samplers_as_deref.c:380` | `gl_nir_lower_samplers_as_deref` | string-keyed remap table | whether a sampler variable is in it (binding of unused samplers) | U1 (strings) | as is: membership |
| 4 | `compiler/glsl/lower_precision.cpp:220-275`, `:899-928` | `lower_precision` | pointer sets/tables (`lowerable_rvalues`, `lowered_builtins`, `clone_ht`) | lookups only; never walked | - | as is |
| 5 | `compiler/glsl/gl_nir_link_varyings.c:1453` | attribute location assignment | `qsort` with `compare_attr` (:60-70) | attribute order | total (slots, then original index) | as is |
| 6 | `compiler/glsl/gl_nir_link_varyings.c:2852-2860` | varying packing | `qsort` with the match comparators (:2494-2546) | varying order before packing | total (class, order, original index); the xfb comparator has ties only for two xfb-only varyings (ES 3) | as is |
| 7 | `compiler/glsl/gl_nir_link_varyings.c:3382` | `canonicalize_shader_io` | `qsort` with `io_variable_cmp` (:3339-3354) | IO variable list order | total (location, else name; names are unique) | as is |
| 8 | `compiler/nir/nir_opt_varyings.c:3229-3250`, `:3362` | `nir_opt_varyings` `deduplicate_outputs` | a table keyed by a **set** of stores; its hash (`dedup_ht_key_hash`) chains XXH32 over the set in slot order, and each entry holds block and value **pointers** | whether an output is found to duplicate an earlier one | U2 over pointer content | **residue**; not reachable from ES 2, needs a geometry shader (4.6) |
| 9 | `compiler/nir/nir_opt_varyings.c:2654`, `:2793`, `:2801` | `nir_opt_varyings` code motion | pointer table `clones_ht` | lookups only | - | as is |
| 10 | `compiler/nir/nir_opt_vectorize_io.c:478` | `nir_opt_vectorize_io` (from `nir_opt_varyings.c:6182`, `:6253`) | `qsort` with `compare_intr` (:116-125) | groups of IO loads/stores to merge | total: SSA index of offset/vertex sources, IO semantics, then instruction index | as is for the words; U6 for exact intermediate numbers (4.5) |

### 3.2 NIR passes (linker `gl_nir_opts` and panfrost's loops)

| # | Mesa file:line | Pass | What is iterated / sorted | What it decides | Order from | Verdict |
|---|---|---|---|---|---|---|
| 11 | `compiler/nir/nir_phi_builder.c:144` | `nir_phi_builder_add_value` (used by `nir_lower_vars_to_ssa`) | dominance frontier, a pointer set (`nir.c:739`) | worklist order of the iterated frontier | U3 | as is (4.1); measured varying |
| 12 | `compiler/nir/nir_lower_vars_to_ssa.c:870` | `nir_lower_vars_to_ssa` | pointer set of stores | a bitset of store blocks | U3 | as is |
| 13 | `compiler/nir/nir_lower_vars_to_ssa.c:561` | `lower_copies_to_load_store` | pointer set of `copy_deref`s | the order copies are lowered; each is lowered in place | U3 | as is (4.2) |
| 14 | `compiler/nir/nir_opt_copy_prop_vars.c:304` | `gather_vars_written` | pointer table of written derefs | merges write masks (OR) into the parent | U3 | as is |
| 15 | `compiler/nir/nir_opt_copy_prop_vars.c:556` | `lookup_entry_and_kill_aliases` | pointer table, one bucket per variable | kills for cast, SSBO or shared derefs | U3 | as is; not reached in ES 2 (no casts, SSBOs, shared) |
| 16 | `compiler/nir/nir_opt_copy_prop_vars.c:604` | same | same | source-alias kills per bucket | U3 | as is (4.3) |
| 17 | `compiler/nir/nir_opt_copy_prop_vars.c:668` | `apply_barrier_for_modes` | same | kills per bucket | U3 | as is |
| 18 | `compiler/nir/nir_opt_copy_prop_vars.c:975` | `invalidate_copies_for_cf_node` | same | kills by mode per bucket | U3 | as is |
| 19 | `compiler/nir/nir_opt_copy_prop_vars.c:995` | same | pointer table of derefs written in a loop/if | the order of `kill_aliases` calls | U3 | as is (4.3) |
| 20 | `compiler/nir/nir_opt_find_array_copies.c:271`, `:276`, `:282` | `nir_opt_find_array_copies` | pointer tables of cast and variable nodes | alias callbacks | U3 | as is; GLSL ES 2 has no cast derefs, so the walks are empty or not taken |
| 21 | `compiler/nir/nir_split_vars.c:1424`, `:1440` | `nir_shrink_vec_array_vars` | pointer sets of copy partners | a fixpoint of OR (components) and MAX (array length) | U3 | as is: monotone, order-free; the sets fill only from `copy_deref`, none left on this path |
| 22 | `compiler/nir/nir_opt_vectorize.c:651-683` | `nir_opt_vectorize` | a set with `hash_instr` (hashes SSA and block **pointers**, :44-151) | lookup only: the earlier instruction to combine with | U3, lookup | as is (4.4) |
| 23 | `compiler/nir/nir_opt_vectorize.c:470` | `instr_try_combine_phi` | `util_qsort_r` with `compare_scalar` (:417-427) | component order of a merged phi | total (component, then index) | as is |
| 24 | `compiler/nir/nir_instr_set.c:829-841`, `nir_opt_cse.c:90-92` | `nir_opt_cse` | a set, lookup only (`_mesa_set_search_or_add`) | the representative: the first equal instruction in program order that dominates | lookup | as is |
| 25 | `compiler/nir/nir_search.c:603-637`, `:869-910` | `nir_opt_algebraic`, `_late`, `bifrost_nir_lower_algebraic_late` | instruction worklists: initial program order, then the **uses** of each changed value | which instruction is matched next, so which rule fires first | U5 | **model**: use-list order (6.1) |
| 26 | `compiler/nir/nir_opt_load_store_vectorize.c:1750`, `:1755` | `nir_opt_load_store_vectorize` (`bi_optimize_late`; ES 2 uniforms are UBO 0, sysvals UBO 1) | a table keyed by resource and offset terms, hashed by their **SSA indices** (:241-264; Mesa's comment: "not to include pointers ... so that the order of the hash table walk is deterministic"); each bucket `qsort`ed by `sort_entries` (:301-318) | bucket order; loads merged inside a bucket | U1 over U6; `sort_entries` total (offset, then instruction index) | as is for the words (4.5) |

### 3.3 Bifrost back end (BIR)

| # | Mesa file:line | Pass | What is iterated / sorted | What it decides | Order from | Verdict |
|---|---|---|---|---|---|---|
| 27 | `panfrost/compiler/bifrost/bifrost_compile.c:4498` | constant promotion | `hash_table_u64` of constant value -> count (on 64 bit: key = value, hashed by `_mesa_hash_pointer`, `hash_table.c:870-872`), insert-only | the array the threshold is read from | U1 over content | **model**: exists (`fau.rs` `U64Table`) |
| 28 | `panfrost/compiler/bifrost/bifrost_compile.c:4503` | same | `util_qsort_r` with `compare_u32` (:4268-4273), which subtracts the element **addresses** truncated to 32 bits | the order of counts; the threshold is `sorted[max_amount - 1]` | U4 + U7 | **model**: identity (measured, `tools/mali/qsort_pointer_order.c`); **residue** if the array crosses a 4 GiB-aligned address (4.7) |
| 29 | `panfrost/compiler/bifrost/bi_spill_ssa.c:72`, `:116`, `:714`, `:1070`, `:1074`, `:1266` | SSA spiller | next-use `hash_table_u64` keyed by value index, with insert, remove (:79-81) and clear | copies and minima of next-use sets; each walk fills another table, so the next table's slot order follows this walk | U1 over content, with deletions | **model** (6.2) |
| 30 | `panfrost/compiler/bifrost/bi_spill_ssa.c:877` | `compute_w_entry_usual` | the next-use table of the block entry | the order candidates are listed | as 29 | **model** (6.2) |
| 31 | `panfrost/compiler/bifrost/bi_spill_ssa.c:802`, `:819`, `:921`, `:1186` | spiller | `util_qsort_r` with `cmp_dist` (:493-515): ties when two values are in the same rematerialisation class at the same distance | which values stay in registers and which spill | U4 (stable) over 30's order | **model** (6.2) |
| 32 | `util/dag.c` (`bi_pressure_schedule.c`), `bi_opt_cse.c:142`, `bi_opt_dual_tex.c:115`, `bi_opt_control_flow.c:153`, `bi_repair_ssa.c:250`, `bi_opt_copy_prop.c:459`, `bifrost_compile.c:4417`, `gallium/drivers/panfrost/pan_nir_lower_sysvals.c:249` | scheduler and BIR passes, sysval ids | lists (dag heads and edges) and tables used for lookup only | - | U5 / lookup | as is (the back half is already byte-equal in its scope) |

### 3.4 Present in the same trees, not reached by an ES 2 shader on v10

`nir_lower_goto_ifs.c` (unstructured control flow), `nir_validate.c`
(checks only), `nir_opt_shared_vars_to_subgroup.c`,
`nir_lower_io_array_vars_to_elements.c`, `nir_lower_io_vars_to_scalar.c`,
`nir_gather_xfb_info.c`, `nir_schedule.c`, `nir_opt_reassociate.c`,
`nir_opt_preamble.c`, `nir_opt_large_constants.c`,
`nir_lower_non_uniform_access.c`, `nir_lower_shader_calls.c`,
`nir_linking_helpers.c` (`nir_compact_varyings`, `nir_link_opt_varyings`),
`nir.c:602` and `:2170`, `nir_control_flow.c:932`; in GLSL:
`gl_nir_link_uniform_blocks.c:985` (UBOs), `gl_nir_link_atomics.c:325`,
`gl_nir_link_interface_blocks.c:256`, `ast_to_hir.cpp:7137` (switch),
`gl_nir_detect_function_recursion.c` (errors only), `opt_dead_code.cpp:53`
(`do_dead_code` has no caller in 26.2.1); `mesa/program/program.c:422`
(fixed function only), `prog_parameter_layout.c:253` (ARB programs),
`st_program.c:788` (transform feedback); the Midgard compiler. A search
that finds nothing (no match in a pass's file) is not listed.

## 4. Proofs for the important places

### 4.1 Phi placement (#11)

`nir_phi_builder_add_value` (:118-171) walks each worklist block's
dominance frontier in slot order and only marks blocks `NEEDS_PHI` in
`val->ht`; the marked set is the iterated dominance frontier of the store
blocks, whatever the walk order. Phis are created later, on demand, in the
rename walk (`nir_phi_builder_value_get_block_def` :187-264), which follows
the program. `nir_phi_builder_finish` (:272-300) fills each phi's sources
in predecessor order (`nir_foreach_pred`, a `u_dynarray`, U5) and inserts
it at the block start. Nothing reads the walk order. Section 5 shows the
walk order changing between runs while the words stay.

### 4.2 Copy lowering in `nir_lower_vars_to_ssa` (#13)

Each `copy_deref` is replaced by loads and stores built at its own position
(`nir_lower_deref_copy_instr`) and removed. The resulting instruction list
is the same in any order; only the SSA numbers of the new values follow the
walk (U6), which 4.5 shows changes no word.

### 4.3 `nir_opt_copy_prop_vars` (#14-19)

The pointer table maps a variable to its array of copy entries. Every walk
either kills entries per bucket, or calls `kill_aliases` once per written
deref. The surviving entries are the same in any order: a kill removes the
entries that alias one deref, so several kills remove the union.

The arrays are not stable: `copy_entry_remove` (:429-446) moves the last
entry into the hole, so the order inside one bucket depends on the kill
order. It is still harmless. `lookup_entry_for_deref` (:458-483) takes an
equal entry, or else the last one that contains the deref. When an entry
is created, every aliasing entry in the bucket is killed
(`lookup_entry_and_kill_aliases_copy_array` :502-532). So two entries of a
bucket never alias, and at most one contains a given deref. The lookup has
at most one candidate, whatever the order.

### 4.4 Instruction sets (#22, #24)

Both sets are lookup only. `nir_opt_vectorize`'s equality
(`instrs_equal` :226-271: same op, bit size, same source values or both
constant, same high swizzle bits) is an equivalence relation. When an equal
entry is found, it is removed or replaced (:651-683), so the set never holds
two equal entries, and a lookup finds the one entry of its class. Its hash
includes pointers, but only the probe path depends on it, not the result.
`nir_opt_cse` keeps the first equal instruction in program order that
dominates (`nir_instr_set.c:829-841`). A port needs the equivalence classes,
not the hash.

### 4.5 SSA creation order (#10, #13, #26)

NIR numbers a value when it is created (U6). In the passes before
emission, the places that read the number to order something are
`nir_opt_vectorize_io` (#10, the comparator) and
`nir_opt_load_store_vectorize` (#26, the bucket hash; `:516` also orders a
key's offset terms by index). In both, the order only arranges groups that
are merged independently. #10 merges runs of loads or stores that compare
equal; the order between unequal groups changes no merge. #26 merges loads
inside one bucket, and a merged load takes the position of its first
load. Different buckets touch different loads (UBO loads are never killed
by stores). Then `bifrost_compile.c:4420-4421` renumbers blocks and values
in program order before `bi_emit_*` turns `def->index` into BIR values
(`compiler.h:1421-1424`). So creation order reaches no word. A port that
wants Mesa's intermediate numbers (to diff NIR) must count values at
creation like `nir.c:1249`. A NIR dump renumbers (`nir_print.c:3176`), so a
dump compares with a program-order numbering anyway.

### 4.6 `nir_opt_varyings` output deduplication (#8): residue

For each output slot, `deduplicate_outputs` (:3253) puts one
`dedup_entry` per store into a fresh set (hashed by XXH32 over the entry,
which holds the block and value **pointers**, :3166-3184,
:3196-3221). It then looks the set up in a table whose key hash walks the set in slot order
(:3229-3243) and whose equality is set equality (:3246-3250). Two equal
sets get the same hash only if their entries sit in the same slots. That
holds when they were filled in the same order, or when no two entries
collide. The stores of each slot are added in program order (`list` of
stores, U5). So two output slots with equal sets get different slot
layouts only if each holds two or more stores in one block, in a different
relative order, and two entries collide; whether entries collide depends
on the pointers (U3), so the duplicate would be missed in some runs only.
For a vertex shader this cannot happen: `nir_lower_io_passes`
(`nir_lower_io.c:1440-1461`), run by the linker right before
`nir_opt_varyings_bulk` (`gl_nir_linker.c:1519-1525`), moves every output
into a temporary with a single copy at each exit block
(`nir_lower_io_vars_to_temporaries.c:26`, `:128-142`). So each output slot
gets at most one store per block, and equal sets are always filled in the
same order. Only geometry shaders (one store per emit) could reach it.
If it happened, one more varying would stay: a different varying layout and different varying load/store words in both stages.

### 4.7 The constant sort that compares addresses (#28): model plus residue

`compare_u32` returns `(uint32_t)a - (uint32_t)b` on the element
**addresses**. glibc's merge sort only compares an element of the left run
with one of the right run, and the left run has the lower addresses. So it
always takes the left element, and the array stays in its input order
(slot order of #27). The project measured this with glibc 2.44 (0 of 3,200
arrays changed, `tools/mali/qsort_pointer_order.c`). `fau.rs`
`lower_constants` models it as the identity. The truncation to 32 bits
makes a residue: if the array (4 bytes per distinct constant, `rzalloc`ed)
crosses an address that is a multiple of 4 GiB, a left element can compare
greater than a right one and move. The threshold can then change, so a
constant is pushed to the FAU or kept inline, which changes instruction
words and the FAU table. The chance per compile is about the array size
over 4 GiB (an estimate: under 1e-7 for 100 constants). With a libc whose
`qsort` is not a merge sort (musl), the result is undefined, because the
comparator does not describe the values. So the reference must stay a
glibc build.

## 5. Experiments (run on this host, 2026-11-24)

Programs: the 327 GLSL ES 1.00 pairs in `tools/mali/testdata/` whose vertex
or fragment stage has a loop, an `if` or an indexed access. 3 do not
translate to SPIR-V (`glsl2pan.py`); 324 remain. Binary: `pan_shc` of the
Mesa 26.2.1 debug build (`-O0`, NIR validation on). Each pair was translated
once (`glsl2pan.py --keep`), then `pan_shc vs.spv fs.spv` was run under 5
layouts:

| Layout | How |
|---|---|
| randomised, run 1 and run 2 | plain (kernel `randomize_va_space` 2) |
| not randomised | `setarch -R` |
| not randomised, other allocation reuse | `setarch -R` with `GLIBC_TUNABLES=glibc.malloc.tcache_count=0` |
| randomised, large blocks mmap'd | `GLIBC_TUNABLES=glibc.malloc.mmap_threshold=4096:glibc.malloc.tcache_count=0` |

Result: stdout (the JSON: code words, FAU words, uniform and varying layout,
register counts) and stderr (disassembly with bytes) were byte-identical
across the 5 layouts for **all 324** programs.

That only proves something if the layouts change the pointer-keyed walks.
With gdb (`set disable-randomization off`; a breakpoint at
`nir_phi_builder.c:154` printing `cur->index` and `next->index`), the
dominance-frontier walk of #11 was recorded 4 times per program for the 284
programs with a loop. The order was the same in all 4 runs for 222. It
differed in 62 (2 distinct orders in 42, 3 in 15, 4 in 5). The visited
multiset was always the same. Example: `deqp_idxloc_fs_mat3_dl_sl`, 59
visits, 4 distinct orders, identical words. A second probe at
`nir_opt_copy_prop_vars.c:998` (#19) was reached in 22 programs. Each walk
there held the derefs of a single array variable, and no order change was
seen. So #19 rests on the reading in 4.3.

glibc 2.44 `qsort_r` with ties (`cmp_dist`-like, 4 distance values): 0 of
3,550 arrays left unstable, sizes 2 to 16,384 elements (U4, #31).

Not run: the drm-shim GLSL path (not built here), so #1-#7 were only read.
No release (`-O2`) build. No second compile in the same process.

## 6. Verdicts

- **Reproducible as is (25)**: #1-#7, #9-#24, #26, #32 (#10 and #26 as
  is for the words, see 4.5).
- **Reproducible with a named model (6 places, 3 models)**:
  1. *Use-list order* (#25): each value keeps its uses in attach order;
     a rewrite appends at the tail in the old order (U5). In the Rust NIR
     this is a design rule (an ordered use list, order-keeping removal),
     not extra code. Without it, nir_search fires rules in another order
     and can reach another fixpoint.
  2. *Mesa's hash table for content keys* (#27, #29, #30): U1 with growth,
     same-size rehash, tombstones and clear. `fau.rs` `U64Table` has the
     insert-only part for up to 151 slots. The spiller needs removal and clear
     too, and more sizes. Estimate: 150-250 lines of Rust, shared by both.
  3. *glibc `qsort_r` as a stable sort* (#28, #31): Rust's stable
     `sort_by`; for #28, the identity.
- **Not reproducible from the input (2 residues)**: #8 (dedup lookup,
  4.6) and the 4 GiB corner of #28 (4.7). Both depend on addresses, so
  Mesa's own words for one shader can in theory differ between two runs
  there. Neither was seen (324 programs times 5 layouts), and #8 is not
  reachable from GLSL ES 2 (4.6). Their effect if hit: #8 changes
  the varying layout and varying instructions of both stages, #28 the set
  of FAU-promoted constants, hence instruction words and FAU words.

So, as far as read and measured, nothing in Mesa's order prevents a 100 %
byte-equal rate. The orders a port must copy are insertion orders and the
container rules above, never addresses.

## 7. For the Rust shader NIR (#11253)

- Keep every list Mesa keeps in insertion order: uses, instructions,
  blocks' predecessors, variables, phi sources, worklists (FIFO).
- Where Mesa walks a pointer-keyed table, do not try to copy the order;
  copy the result (a set, a union, a fixpoint, a lookup by equivalence
  class).
- Model Mesa's hash table only for the content-keyed walks: the constant
  histogram (done) and the spiller's next-use tables (with M5).
- Sorts: stable, with Mesa's comparators.
- SSA numbers: free inside the pass loop. Program-order numbering at the
  hand-off to BIR is what Mesa uses (`bifrost_compile.c:4421`).

## 8. Not checked

- `nir_instrs_equal` (cse) was read for ALU; that it is an equivalence for
  every instruction kind (tex, intrinsics, phis) is assumed.
- The SPIR-V front end (`compiler/spirv`) is only the instrument's path;
  its two sorts (`spirv_to_nir.c:5603`, `vtn_structured_cfg.c:619`) were not
  studied.
- 4.5 rests on reading the two index-ordered passes; it was not tested by
  forcing a different creation order.
- 4.6's "not reachable" rests on reading `nir_lower_io_passes`; no
  shader was built to hit it.
- Threads: one shader is compiled on one thread; compile order across
  shaders in one process changes only addresses (U3), which section 5
  varied in another way.
- glibc's merge sort falls back to heap sort when the temporary buffer
  cannot be allocated (not stable); not reachable in practice, not tested.
