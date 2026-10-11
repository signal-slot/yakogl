<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (old PR #92, ccfarm `01a12b02-b874-70e1-9511-b33ca0b91da5`, commits `9aaa16c0c`, `9ae0ab5fb`). Written on 2026-10-11 against the integration tip `76d349c4` and origin/main `5ad2fa0e`; the text is as written, and the paths in it are the ones of that time (on main, `docs/investigations/` and `docs/board-sessions/` files live under `tools/cts/log/`). The re-landing it recommends was done on 2026-10-12: integration was abandoned, and its results went to main one pull request each (PRs #102 to #126, and #129 to #137 with this record for the result PRs that were still open, #29 to #92). Read the pull requests for what landed, not this audit.
# Integration against origin/main: which ES 2.0 and publication items main already covers

Item `es2-invariance` of plan #20, 2026-10-11. A read-only audit; no source,
test or other document changed. Reading only: `git diff`, `git log`, `git show`
between the integration tip `76d349c4` and `origin/main` `5ad2fa0e` (fetched
2026-10-11). The common ancestor is `2f69c7d1`; main has 147 commits
integration lacks and integration has 108 main lacks. Nothing was built or
run here, so every "compiles" or "passes" below is a reading of the diffs, not
a result. Each claim says which commits it rests on. A trial merge of the
two tips was run (`git merge-tree --write-tree 76d349c4 origin/main`, result
tree `416e2549`, nothing written to a ref); its conflict list is section 3.3
and section 7.

Labels: **[read]** read from a diff or file; **[inferred]** follows from what
was read, not run; **[merge-tree]** from the trial merge.

Plans #137 and #139 were read through the plan tool (their state at
2026-10-11 ~13:00): #139 `pvr-vertex-2d-fix` is `ready` but held, its
`waiting_reason` being "touches overlap with pvr-pool-npot (in review, plan
18)"; #137 `pi4-console-tcp2` is `running` (agent `01a12af1-d009-73b3-ba27-e4389254185f`,
touches `tools/board.sh`, `tools/board_selftest.sh`,
`tools/am62p_netboot_uart.py`, `tools/serial_capture.py`), with
`pi4-console-probe2` running and `pi4-es2-tip-r2` waiting on it. Neither
item has a result branch yet, so there is no commit to compare; the
statements below are against the item descriptions. Plan #136 was not read.

## 0. Summary table

| # | Topic | Main | Integration (or review branch) | Where it is | Verdict |
|---|---|---|---|---|---|
| 1 | PowerVR invariance (9 AM62P `common_subexpression_1/2/3`) | `f9688bc5`, merged by `e17fb6a7`; board-confirmed (image 1 on `e17fb6a7`, 1,379 / 0) | `e0473221` + `aa750648` (`tests/es2_invariance.rs`, `docs/investigations/es2-invariance.md`), branch `origin/ccfarm/01a11d80-bb8a-7598-8efd-71e4b54b42be`, item es2-invariance-repro in review | main fixes; integration has only a reproduction with 9 `#[ignore]`d cases and a different proposed fix | **resolved on main**; integration's fix proposal **replaced**; its tests **still needed** (un-ignore) |
| 2 | AM62P `texture.vertex.2d` (45) | `a6fc6f14`: `draw_indexed_pieces` in `src/pvr/backend.rs`; board confirmation pending | `e4bee1ba` (`docs/investigations/es2-pvr-vertex-texture.md`, `tests/es2_pvr_vertex_texture.rs`); fix proposed, not written: own staging memory (`extra`); plan #139 `pvr-vertex-2d-fix` ready, held behind plan 18 `pvr-pool-npot` | main fixes the cases; integration has diagnosis, GL-core tests and a different, unwritten fix | **replaced by main**; the diagnosis tests **still needed**; the plan #139 fix item is **a duplicate** |
| 3 | report-es2 refusals (ES 2.0 error rules) | `4e9397c6` (common to both), later `fdc3b692`, `8b3e7039`, `03840710`, `3d1e5ba1`, `6c9e9eea` | `289fb9c4`, `a54bbdec`, `b618d832`, `57085276`, `8928d47d`, `e8e6029a`; review: `d70f8ee7`, `933b4283`, `82cec4f4` | two commit pairs are patch-identical, three are the same intent with different text, the rest is integration only | **conflicting** in `vertex.rs`, `framebuffer.rs`, `es2_error_rules_vertex.rs`, `es2_report_guard.rs` (trial merge), otherwise still needed (see 3.3, 3.4) |
| 4 | Pi 4 serial-bridge console in `board.sh` | `28fb55ae` (+ `c605d8ad`, `469875d8`) | none: `tools/board.sh` has no `host:port` Pi 4 path at the tip; plan #137 `pi4-console-tcp2` (running, no result yet) | main only | **replaced by main**; `pi4-console-tcp2` is a duplicate |
| 5a | NPOT (`GL_OES_texture_npot`, 18 CTS cases) | `9fdc9bcb` (section H of `docs/es2-cts-failures.md`; investigation file removed) | `f889ed90` (merged research), `b738368b`/`3eac13d0`/`a9574460`/`adfe9be5` (`tests/es2_npot.rs`, doc section 8; item es2-npot-cases in review) | tests are integration only; the doc is deleted on main | **still needed** (tests only); the doc part **conflicts** (modify/delete) |
| 5b | Preprocessor | none: HANDOFF records "no defect" (log `tools/cts/log/investigations/es2-preprocessor.md`) | `c344cf6e` (diagnosis, merged), `f44a0ff0` (`rescan_tail` fix + `tests/es2_preprocessor.rs`, review) | main: no CTS defect; integration: one extra C99 fix with tests | **resolved on main** for dEQP; `f44a0ff0` **still needed** only as optional hardening |
| 5c | Offline table (#22) and the 8 offline GL tests | `9224abaf` (8 tests run without table); #22 still open in HANDOFF; `746b24cc` removes the investigation | `7151a12a`, `c2c31372` (same 8 tests); `56491243`, `cc9b3846`, `f8bb9576` (GC7000 table rows, review) | tests: both, same change; table rows: integration only | tests **resolved on main**, drop; table rows **still needed** |
| 5d | Fragdata | `4e9397c6` (common) | `4fa00f0a` (`tests/es2_fragdata.rs`, doc; review) | fix is common; tests integration only | **resolved**; tests only |
| 5e | V3D `glClear` | none | `21c3acb8`, `3c317c4a` (`tests/v3d_clear.rs`, merged), `es2-v3d-clear-jobs` ready | integration only | **still needed** (tests only) |
| 6 | Website, CHANGELOG, README, ROADMAP | `c4ccc966` (website), `0964d2d3`, `8ebd9ca5` (CHANGELOG), README/CONTRIBUTING/DESIGN edits | `pub-changelog` merged earlier, `4e44e14f` checklist, `2afaf0b9` / `02017674` ROADMAP (review) | website: main only; ROADMAP: integration only; HANDOFF goals: both, conflicting | see section 6 |

## 1. PowerVR invariance (main `e17fb6a7`, integration es2-invariance-repro)

**Main.** `f9688bc5` (merged by `e17fb6a7`, 2026-10-10) stops `fusion_pairs`
from fusing a sum on the way to an `invariant` output into an `fmad`. It
follows Mesa's rule (propagate_invariance marks the assignments feeding an
invariant output; no `ffma` on exact ALU instructions). It adds
`ir::IoDecl::invariant` (`invariant gl_Position;`, `invariant varying`,
`varying v; invariant v;`, `#pragma STDGL invariant(all)`), `mark_exact` in
`powervr/prepare.rs`, and `chain.rs` passing the module's invariant slots to
`prepare_fused`. Tests are in main's own source: `chain.rs`
`an_invariant_position_is_computed_alike_whatever_else_the_shader_computes`
(dEQP's 4 pairs x 3 precisions), `prepare.rs`
`a_sum_an_invariant_output_reads_is_not_fused`, `lower.rs`
`an_output_declared_invariant_is_marked_in_the_declarations`. Board:
`tools/cts/RESULTS.md` "AM62P: image 1 on `e17fb6a7`": 1,379 pass / 0 fail
(record `records/am62p/deqp-e17fb6a7/img1`). **[read]**

**Integration.** `e0473221` (2026-10-09) adds `tests/es2_invariance.rs`
(traces `gl_Position` back to the inputs as a canonical expression on each
back end; PowerVR 9 cases `#[ignore]`d with the cause, GC7000 and V3D 12
pass) and `docs/investigations/es2-invariance.md`; `aa750648` is a comment
fix. The doc proposes "fusion as a function of the sum's own expression" and
lists Mesa's rule only as the alternative (its section 5). Neither file is on
main. No `src/` change. **[read]**

**State:** the fix is on main only; the reproduction is on integration only.
They do not touch the same lines (integration changed nothing under
`src/shaderc/online/powervr/`).

**Compatibility of the test against main [inferred].** The test calls
`lower::compile_to_ir`, `powervr::chain::build_vertex`,
`select::select` and `v3d::prepare::prepare_vertex`; their signatures are the
same on main (`lower.rs:563`, `chain.rs:65`, `v3d/prepare.rs:2304`). It does
not construct `ir::IoDecl`, so the new `invariant` field does not break it.
The cases it marks `#[ignore]` should pass once main's fix is in; whether the
test's "fmad rounded once" model agrees with main's change was not run.

**Verdict:** **resolved on main** (cause fixed, board-confirmed). The
integration proposal is **replaced** (main took the alternative the doc
listed). Still needed: `tests/es2_invariance.rs` as a host guard, after
removing the 9 `#[ignore]` lines and running it against main; its comment
"the fix (plan item es2-invariance) removes the ignore" then holds. The
markdown doc should not land as written: its fix section describes a change
main did not make.

## 2. AM62P `texture.vertex.2d` (main `a6fc6f14`, plan #139 `pvr-vertex-2d-fix`)

**Main.** `a6fc6f14` (2026-10-10): `draw_dispatch` sends an indexed draw
whose staged bytes (client indices plus staged attributes) exceed half an
empty GENERAL arena to `draw_indexed_pieces` (`src/pvr/backend.rs`, `:4951`
on main), which cuts the index list on primitive boundaries into pieces whose
copies each fit half an arena and copies only the vertex window each piece
reads. dEQP's 127 x 127 grid becomes three pieces (41,385 + 41,385 + 14,004
indices). A fan, a loop, a second index list, a primitive whose window alone
is over half an arena, or more than `MAX_PIECES` (64) pieces are still
refused by name. Tests: `src/pvr/backend_tests.rs`
`a_deqp_vertex_texture_grid_of_client_arrays_is_drawn_in_index_pieces` and
three more; gl_test 197 `vtxgrid`. The arena stays 1 MiB
(`ARENA_GEN_BYTES`, main `src/pvr/backend.rs:271`). **Board confirmation is
pending** (HANDOFF "State of main", row for the 45 cases; image 9 of the
AM62P tree). A sequential-draw twin, `528d4260` (gl_test 209 `bigdraw`), is
only on a work branch (`work/pvr-arena-bigdraw`), not on main. **[read]**

**Integration.** `e4bee1ba` (merged through `225d1ec8`, item pub-license-audit)
adds `docs/investigations/es2-pvr-vertex-texture.md` and
`tests/es2_pvr_vertex_texture.rs`. The doc reaches the same cause
(1,225,804 bytes against a 1 MiB arena, `dcaafa41`) but proposes a different
fix in `src/pvr/backend.rs`: a per-set `extra: OsalGpuMem` plus `extra_used`
for any draw whose staged copies do not fit an empty arena, modelled on the
GC7000's `ensure_extra`, bounded by `ARENA_GEN_MAX`. It is "written in the
doc, no src change". The test file replays the cells at the GL core: a 2D
cell stages more than the default arena, a cube cell fits, the vertex-stage
binding and the compiled vertex program are right. **[read]**

**State:** both sides agree on the diagnosis; main has a fix, integration has a
different, unwritten one. The test file's assertions are about the GL core
(staged bytes against `zephyr/Kconfig`'s `YAKOGL_PVR_ARENA_GEN_KB` default),
which main did not change, so they should still hold **[inferred]**.

**Plan #139 `pvr-vertex-2d-fix` (ready, held behind plan 18's
`pvr-pool-npot`; its goal text expects `#[ignore]`d tests in
`tests/es2_pvr_vertex_texture.rs`, which the file at the tip does not have: its
four tests assert GL-core facts and already pass)** would write the `extra`
staging memory into `src/pvr/backend.rs`: `FrameSet` (`:1142-1157` in the
doc), `draw_part` (`:4220`), `record_draw` copy sites (`:4602-4673`),
shutdown and pool-owner counts. Main's `a6fc6f14` changed the same region
(`draw_dispatch`, `record_draw`, new `draw_indexed_pieces`,
`lists_with_streams`); the doc's line numbers are stale against main and an
`extra`-arena patch written from the integration tip would conflict with it,
and, applied after it, be largely dead code for the one case it was written
for (an indexed client draw is now cut before it reaches `draw_part`).

**Verdict:** **replaced by main** for the 45 cases. What the plan #139 fix would
still add is the cases main refuses by name (a fan, a line loop, a primitive
whose window is over half an arena) and the sequential draw (main:
`528d4260` on a branch). No dEQP-GLES2 case is known to reach them, so that is
not needed for the ES 2.0 goal.

## 3. report-es2 refusals (main `4e9397c`, the es2-error-rules tranches)

### 3.1 What `4e9397c6` is

`4e9397c6` (2026-10-07, "ES2 context: refuse DEPTH_COMPONENT24 renderbuffer,
no MAX_DRAW_BUFFERS...") is an ancestor of both sides (it is in the common
base). It is not a diff between main and integration. The overlap is main's
later re-landings of the audit work, which carry different shas.

### 3.2 Pairs, by `git log --cherry-mark` (patch-identical) and by diff

| Change | Main | Integration | Relation |
|---|---|---|---|
| `glDrawBuffers` checks n before the enums | `3d1e5ba1` | `57085276` | same patch (`=`) |
| `glUnmapBufferOES`, `glGetBufferPointervOES` take every target in an ES 3.0 build | `03840710` | `b618d832` | same patch (`=`) |
| cts-selftest check 19 after 18 | `eb911acb` | `d72ed694` | same patch (`=`) |
| 8 offline-table GL tests run without a table | `9224abaf` | `c2c31372`, `7151a12a` | same files, `testutil.rs` 5 lines apart |
| vertex rows: report-es2 refuses `GL_INT` / `GL_UNSIGNED_INT` in `glVertexAttribPointer` | `fdc3b692` | `289fb9c4` | same intent, same tests file name, different comment text and test text |
| ES 2.0 report guard `tests/es2_report_guard.rs` | `6c9e9eea` | `8928d47d` | both add the file (740 / 739 lines); main marks four open pins `BUG:` |
| pure-integer current values, ES 2.0 refuses the types | `8b3e7039` | `a54bbdec` | same intent |
| `docs/investigations/es2-error-rules.md` audit | `tools/cts/log/investigations/es2-error-rules.md` (moved, `6eeba5a5`) | `docs/investigations/es2-error-rules.md` (edited) | rename on main, edit on integration |

### 3.3 Where they meet

The trial merge **[merge-tree]** reports content conflicts in these files of
this topic: `src/gl/vertex.rs`, `src/gl/framebuffer.rs`, `src/gl/draw.rs`,
`src/gl/mod.rs`, `src/gl/tf.rs`, `tests/es2_error_rules_vertex.rs`,
`tests/es2_report_guard.rs`, and (ES 3.0 re-landings of the same work)
`tests/es3_gl_core.rs`, `tests/es3_int_rt.rs`, `tests/es3_vertex_api.rs`.
`src/gl/state.rs`, `program.rs`, `buffer.rs`, `texture.rs`, `shader.rs`,
`tf_tests.rs`, `testutil.rs` and the integration-only
`tests/es2_error_rules_texture.rs`, `tests/es2_pvr_vertex_texture.rs`,
`tests/v3d_clear.rs` merge without a textual conflict (so the compile break
of section 7, item 1 is the only problem there). Detail:

- `tests/es2_error_rules_vertex.rs`, `tests/es2_report_guard.rs`: both sides
  add them (add/add). Main's differ from integration's by the three new
  `YakoglBackend` fields (`query_begin`, `query_end`, `query_result`), a path
  in a comment, and the `BUG:` prefix on four `#[ignore]` reasons in the guard
  (lines ~436, 492, 576, 659 on main). Take main's file, then re-apply any
  integration-only test (none found).
- `src/gl/vertex.rs`: both change the `match type_` arm of
  `glVertexAttribPointer` the same way; the comment text differs
  (main differs from the tip by a 10-line comment rewording). One hunk
  conflict, resolved by taking either.
- `docs/investigations/es2-error-rules.md`: modify (integration, 21 lines
  added by `289fb9c4`) against the move on main. The 21 lines have to be
  carried into `tools/cts/log/investigations/es2-error-rules.md`.
- `src/gl/shader.rs`: the review commit `d70f8ee7` and the tip both gate the
  `glGetProgramInfoLog` import; main has the same gate with a different
  comment. The trial merge of the tips took it without a conflict.
- `src/gl/framebuffer.rs` (conflict in the trial merge): the tranche
  `82cec4f4` edits the same functions as main's ~400 lines; expect it to
  conflict again when rebased.

### 3.4 Tranches in review and what main already did

- `d70f8ee7` (first tranche; only `src/gl/shader.rs` remains in the
  branch): subsumed by `289fb9c4` and main's `fdc3b692`. Drop.
- `933b4283` (state/program tranche: `tests/es2_error_rules_state.rs`,
  `src/gl/state.rs` +116, `program.rs`, `tf_tests.rs`, `es3_tf_api.rs`).
  Main's `state.rs` change is a comment edit near line 102 and its
  `program.rs` change adds integer-sampler classes at line 2038; the tranche
  hunks (state.rs 423 onward, program.rs 2709) do not overlap them
  **[inferred]**. It also fixes pins that main still lists as `BUG:`
  (`es3_only_queries_are_invalid_enum`, `es3_only_caps_are_invalid_enum`):
  **still needed**. The file `tests/es2_error_rules_state.rs` needs the three
  `YakoglBackend` fields (section 7).
- `82cec4f4` (buffer and framebuffer tranches, item cts-tree-diff-readme,
  running): main changed `src/gl/buffer.rs` by about 460 lines
  (`unmap_buffer`, `glGetBufferParameteriv`, `glCopyBufferSubData`,
  `glUnmapBufferOES`) and `src/gl/framebuffer.rs` by about 400 lines
  (`check_status`, `renderbuffer_bits`, `is_renderbuffer_color_format`),
  which are the functions the tranche edits; `es3_only_renderbuffer_formats_are_invalid_enum`
  is main's open `BUG:` pin. **Still needed and likely to conflict**; the
  branch should be rebased on main, not merged as it is. (This item does not
  touch the running item's files.)
- `e8e6029a` (texture rules, table A rows 24-27, tests only, in
  `tests/es2_error_rules_texture.rs`): main has no such file, but changed
  `src/gl/texture.rs` by 137 lines; whether the tests still pass there was
  not run.

**Verdict:** the vertex rows are **resolved on main** (and on integration) and
conflict only textually; the state, program, buffer, framebuffer and texture
rows are **still needed**, because main still carries four `BUG:` pins for
them (`63139491`: "four BUG pins open"). Recommended: rebase the tranches on
main and let their commits flip main's four `#[ignore]` pins.

## 4. Pi 4 serial-bridge console (main `28fb55ae`, plan #137 `pi4-console-tcp2`)

**Main.** `28fb55ae` (2026-10-10): with `BOARD_UART` matching
`^[^/:]+:[0-9]+$` the pi4 tryboot path skips the `/proc` fd scan and `stty`,
captures with `tools/am62p_netboot_uart.py capture` (raw TCP; `console.raw`,
`console.ts`, exit 4 on "busy"), derives `console.txt` with
`console_from_ts`, and retries a busy bridge 12 times at 10 s. `board_selftest`
pins 221 checks, including a replay of the `29460f96` GT21 record through a
bridge `console.ts`. Follow-ups on main: `c605d8ad` (an end line followed by
more than a pipe's worth of console), `469875d8` (a scan cut short prints
nothing), `4ed3ff5f` / `c17714e9` (a capture that lost lines reads
Lost / `done_lossy`). Main's own board use: the Pi 4 runs of 2026-10-10/11
(`records/pi4/deqp-a27d69c0`, `deqp-1cf946a9`) went through the usb1 TCP
bridge. **[read]**

**Integration.** `tools/board.sh` has no Pi 4 `host:port` path: the board
records say so (`935f5fb8`: "Pi 4 tool takes a device, not host:port";
`433a7a07`, `ce60c5e2`: the Pi 4 whole tree is blocked, "TCP console, not a
device file"). The pi4-es2-final-b record (`docs/board-sessions/pi4-es2-r1-tip.md`)
and the pi4-es2-r1a/b records ran nothing for this reason. Integration's
changes to `board.sh` are: a bash `printf %(..)T` fallback in
`console_from_ts` (`126a36ce`), `BOARD_MUSTPASS` (`8a7222fc`; main has the
same, `board.sh:150`). The branch `pi4-console-tcp2` is not in the visible
refs; it has not produced a commit. **[read]**

**State:** the feature is on main only. The one real textual conflict is
`console_from_ts` (integration: bash loop fallback; main `d78b5c38` /
`760b6581`: `python3 -I`, around main's `board.sh:974`). Take main's; the
bash fallback is redundant once python3 is required.

**Plan #137.** `pi4-console-tcp2` is running now (see the top of this file).
Its goal text asks for exactly what `28fb55ae` does: `BOARD_UART=host:port`
for `pi4`, reuse `tools/am62p_netboot_uart.py capture`, the 12-try "busy"
handling, dry-run selftest rows in `tools/board_selftest.sh`, and the header's
`BOARD_UART` line. It is written from the integration tip, which has none of
that. The trial merge already conflicts in `tools/board.sh` and the file
`tools/board_selftest.sh` is main-only-changed (+89 lines), so the result of
this item will add/add-conflict with main in both.

**Verdict:** **replaced by main**. `pi4-console-tcp2` is a duplicate of
`28fb55ae`; `pi4-es2-tip-r2` (plan #137) and `pi4-es2-final-a` (plan #20) need
only "a `board.sh` that accepts `host:port`", which main has.

## 5. NPOT, preprocessor, offline table, fragdata, V3D clear

**NPOT.** Main: `9fdc9bcb` removed the investigation file and moved its
lasting part into section H of `docs/es2-cts-failures.md`: the CTS source
shows none of the 18 cases becomes NotSupported (they must Pass); the boards
agree (the i.MX8MP passes all 18; the AM62P and Pi 4 trees name none among
failures or NotSupported). Main lists "no host replay of the CTS sequences"
as still open. Integration: `f889ed90` (the research, merged) and the review
branch `b738368b`, `3eac13d0`, `adfe9be5`, `a9574460`: `tests/es2_npot.rs`
(host replay of the 18 setups) and doc section 8. Verdict: tests **still
needed** (they close what main lists as open); the doc edit **conflicts**
(`docs/investigations/es2-npot-cases.md` is deleted on main by `9fdc9bcb`
and `6eeba5a5`); put the record in `tools/cts/log/` or section H instead.
`tests/es2_npot.rs` has two `YakoglBackend` literals (section 7).

**Preprocessor.** Main: HANDOFF says the two `define_value_and_function`
failures are one stale `RESULTS.md` row and all 534 preprocessor shaders get
the CTS's compile outcome; the log `tools/cts/log/investigations/es2-preprocessor.md`
(diagnosis, "finished, no defect") says `tests/es2_preprocessor.rs` does not
exist. Integration: the same diagnosis (`c344cf6e`) and the review commit
`f44a0ff0` (`rescan_tail`: `#define G F` then `G(2)`; `tests/es2_preprocessor.rs`,
100 lines). Its `pp.rs` hunks (+822, +845, +849) do not overlap main's
(`pp.rs` +76, +142, +153, +271, +381, the invariant pragma), so it should
apply with offsets **[inferred]**. Verdict: **resolved on main** for the
dEQP-GLES2 goal; `f44a0ff0` is optional hardening, not a stage-1 item.

**Offline table (#22).** Main: `9224abaf` runs the 8 GL tests in
no-shader-table builds; `746b24cc` deletes the investigation; HANDOFF line 266
keeps "#22 GC7000 table still lags on `matattr`" open. Integration:
`c2c31372` / `7151a12a` are the same 8 tests (same five files,
`alloc.rs` and `roundtrip.rs` blobs identical on both sides,
`testutil.rs` `shader_source` hunk identical); `56491243`, `cc9b3846`,
`f8bb9576` regenerate the GC7000 table (129 + 218 rows, `texsize` replaced;
`tests/reference/vivante_blobs.rs`, `tools/vivante/*`), which main has not
touched (`HEAD` and main have the same `vivante_blobs.rs`). Verdict: the 8
tests are **resolved on main** (drop the integration copy when merging; same
change); the table rows are **still needed** for #22 and apply cleanly.

**Fragdata.** `shaders.fragdata.invalid_assign_to_1` is fixed by the common
`4e9397c6` and passes on all three boards on main builds (AM62P
`32778b57`, i.MX8MP `32778b57`, Pi 4 `a27d69c0`). `4fa00f0a`
(`tests/es2_fragdata.rs` + doc, review) adds only a host pin. Verdict:
**resolved**; keep the test file, put the doc under `tools/cts/log/`.

**V3D clear.** Main has no `tests/v3d_clear.rs` and no clear change under
`src/v3d` (its V3D hunks are occlusion counter and cube level:
`V3dJobState.oq_addr`, `V3dJobTmu.width/height`). Integration:
`21c3acb8` / `3c317c4a` (merged) and `es2-v3d-clear-jobs` (ready). The test
uses `V3dJobState` and `state::{ClearProg, QuadBuffers}` through
struct-update syntax, which tolerates the two new fields **[inferred]**.
Main's HANDOFF says the V3D already draws every scissored or masked clear
the TLB cannot as a quad, and the board passed those groups. Verdict:
**still needed**, tests only; no clash.

## 6. Website, CHANGELOG, README, ROADMAP

- **Website.** `website/` exists on main only (`c4ccc966`, merge of
  `work/land2`: `905866fe` ... `4d732523` Pages workflow,
  `tools/website-check.sh`). Integration has no `website/`. Verdict:
  **on main only**; nothing to merge, but the item pub-website of this
  plan is not a website item (its title is the offline-table tests). The
  site's figures live in `website/data.js` with commit and record. One row is
  stale on main: `deqp2.am62p.ms4` still reads 14 pass / 2 fail
  (`32778b57`), while HANDOFF says the `ms4` image confirmed the fix on
  `452d6c31` and `a27d69c0` (16 / 16). `website/data.js` last changed in
  `0a7607b2`.
- **CHANGELOG.** Main `0964d2d3` (2026-10-10, ES 3.0 transform feedback
  state, formats, UBO tests) and `8ebd9ca5` (the 32778b57 runs). Integration
  did not change CHANGELOG.md after the base (`pub-changelog` is already in
  the base). `git diff HEAD origin/main -- CHANGELOG.md` is 10 lines:
  **main only; take main's**.
- **README, CONTRIBUTING, DESIGN.** Edited on main (`6468142f`,
  `26aac6cc`, ...), not on integration since the base. Integration's
  walkthrough doc (`docs/release/readme-walkthrough.md`), checklist
  (`docs/release/checklist.md`, `4e44e14f`) and `docs/plan-status.md` are
  integration only, and `docs/release/building.md` is main only
  (13 lines apart). No conflict.
- **ROADMAP.md.** On neither tip. The review branch has it (`2afaf0b9`,
  `02017674`; item pub-roadmap), together with edits to `README.md` (one link
  line) and `docs/HANDOFF.md`'s "Goal, in order" list. Main rewrote the same
  HANDOFF list (website as goal 3, ES 3.0 goal 4, ES 3.1 / 3.2 goal 5, no
  "more boards"), so the HANDOFF hunk will conflict at lines 11-26; the
  ROADMAP's own "State" paragraphs quote 2026-10-02 / 10-01 trees and are
  stale against section 8 below. **Still needed**, rebase on main and rewrite
  the state paragraphs from section 8.

## 7. Other collisions found

1. **Backend literal.** Main added three fields to `YakoglBackend`
   (`query_begin`, `query_end`, `query_result`, `src/backend.rs:1261`).
   Every integration test file that spells a `YakoglBackend { ... }` literal
   without them stops compiling on main: at the tip
   `tests/es2_error_rules_texture.rs` and `tests/es2_pvr_vertex_texture.rs`;
   on the review branches `tests/es2_error_rules_state.rs` (`933b4283`),
   `tests/es2_error_rules_buffer.rs`, `tests/es2_error_rules_fbo.rs`
   (`82cec4f4`) and `tests/es2_npot.rs` (`b738368b`, two literals). Main's
   `es2_report_guard.rs`, `es2_error_rules_vertex.rs`, `es3_*.rs` and
   `depth_resolve_pvr.rs` already carry them. Each needs the three
   `query_* : None` lines. **[read]**
2. **Docs location.** `6eeba5a5` moved `docs/investigations/*` that
   mattered into `tools/cts/log/investigations/` (6 files remain) and
   `docs/investigations/` is empty on main; every integration document of
   that directory (es2-error-rules, es2-host-deqp-v3d, es2-npot-cases,
   es2-pvr-vertex-texture, am62p-console-empty, gc-*, this file) lands in a
   path main removed. Main's header rule: a finished investigation says its
   conclusion at its top.
3. **`HANDOFF.md`** changed on both sides (integration: `am62p-es2-tip`,
   `pi4-es2` notes, the Pi 4 recipe; main: the whole "State of main"). The
   trial merge conflicts in it; take main's text and re-apply only
   integration's board readings.
   **Other conflicts of the trial merge [merge-tree]** (all of the 108
   integration commits against main, not only this audit's topics):
   `capi/yakogl_capi.cmake`, `tools/board.sh`, `tools/check.sh`,
   `tools/cts/check-deqp-images.py`, `tools/cts/pi4-deqp-images.sh`,
   `tools/cts/testdata/README.md`,
   `tools/cts/log/hostharness/glmark2/FINDINGS-refract-budget.md`,
   `tools/cts/log/investigations/am62p-console-empty.md`, and add/add on
   `docs/es3-board-recipe-pi4.md`, `docs/es3-cts-failures.md`,
   `docs/es3-vertex-gap.md`. Modify/delete: `.ccfarm/project.toml`
   (deleted on integration, edited on main) and
   `docs/investigations/es2-host-deqp-v3d.md` (deleted on main). File
   location: every `docs/board-sessions/*` and `docs/investigations/*` file
   integration added lands in a directory main renamed to
   `tools/cts/log/board-sessions/` and `tools/cts/log/investigations/`
   (`am62p-es2-r1c`, `am62p-es2-tip-img1-7`, `am62p-gles3-r2`,
   `imx8mp-common-int`, `imx8mp-es2-r1`, `pi4-deqp-stage1-gles3`,
   `pi4-es2-r1-tip`, `pi4-es2-r2-baseline`, `es2-pvr-vertex-texture`,
   `gc-kick-diff`, `gc-msaa-record`).
4. **`tools/cts/*`.** Both sides changed `am62p-deqp-images.sh`,
   `check-deqp-images.py`, `check-gles3-tree.py`, `cts-selftest.sh`,
   `pi4-deqp-images.sh`, `pi4-gles3-tree.txt`, `testdata/README.md`,
   `testdata/deqp-config/pi4-gles3-sweep.config` (integration `752508a4`,
   `34d79ead`, `5584c7ba`, `1ff208c4`, `a9947724` family: defects found by the
   round-1 sessions; main `c0247221`, `dd2554be`, `ea984992`, `ecb9e162`).
   Not read hunk by hunk here.

## 8. Main's dEQP-GLES2 board numbers (mustpass list, 17,165 cases)

Source: `docs/HANDOFF.md` ("State of main", the 2026-10-10 and 10-11
updates; Pi 4 recipe section) and `docs/es2-cts-failures.md` (summary table
and section A), main `5ad2fa0e`. The detailed record is
`tools/cts/RESULTS.md`; `records/...` paths are gitignored, so only their
names are quoted.

| Board | Commit (image built from) | Date | Pass | Fail | NotSupported | Warnings | Record |
|---|---|---|---:|---:|---:|---:|---|
| AM62P | `32778b57`, whole tree, 17 images, 0 resets, 0 without result | 2026-10-10 | 17,070 | **54** | 34 | 5 Quality + 2 Compatibility | `records/am62p/deqp-32778b57/full/`; RESULTS "AM62P: the whole dEQP-GLES2 tree on origin/main `32778b57`: 54 fail" |
| AM62P, image 1 only | `e17fb6a7` | 2026-10-10 | 1,379 | 0 | 0 | 0 | `records/am62p/deqp-e17fb6a7/img1`; RESULTS "image 1 on `e17fb6a7`": the 9 invariance cases pass |
| AM62P, `ms4` image (16 `multisample.*`) | `32778b57`; fix confirmed on `452d6c31` and `a27d69c0` | 2026-10-10 | 14 (was 16 at `d60b9d76`) | 2 (`constancy_alpha_to_coverage`, `constancy_both_inverted`) -> 0 after `a4f64950` | n/a | n/a | `records/am62p/deqp-32778b57/ms4`, `ms4-rerun`; `records/am62p/deqp-452d6c31/ms4/` (16 / 16) |
| i.MX8MP | `32778b57`, 29 selectors | 2026-10-10 | 17,112-17,116 (17,318 of 17,371 begun, 206 outside the list) | **0** | 30-34 | 19 Quality | `records/imx8mp/deqp-32778b57/full`; identical per selector to `d8583f20` (2026-10-01) |
| Pi 4 | composed: whole tree on `a27d69c0` (17,083 / 9 / 34, 34 lost in capture) + six rows on `1cf946a9` (934 / 934) + F1 rerun | 2026-10-10 / 10-11 | 17,126 | **0** | 34 | 5 (3 Quality + 2 Compatibility) | `records/pi4/deqp-a27d69c0/full`, `.../F1-rerun2`, `records/pi4/deqp-1cf946a9/rows`; RESULTS "Pi 4: the whole dEQP-GLES2 tree, 0 fail" |

Reading the table:

- The AM62P whole tree (54) predates both fixes. The 9 invariance cases are
  confirmed fixed by image 1 on `e17fb6a7`. The 45 `texture.vertex.2d` cases
  (`a6fc6f14`, image 9) have **no board result yet**; so the AM62P is 0 fail
  only by inference. The `ms4` row on `452d6c31` is a HANDOFF statement
  (lines 101 and 1065); the website still shows 14 / 2.
- The Pi 4 figure is a composition of three records at three commits, not one
  image set; HANDOFF asks for one whole tree from one main commit to replace it.
- The i.MX8MP has not been re-run since `32778b57`, and its gl_test case 44
  (`es3hdr`, #6) fails on `f2a13877`; its dEQP-GLES2 is 0 fail.
- Stale figures to ignore: the summary table at the top of
  `docs/es2-cts-failures.md` still prints AM62P `d60b9d76` (17,141 / 0) and
  i.MX8MP `d8583f20`; HANDOFF's "State of main" lines 33-35 (`d60b9d76`,
  `d8583f20`, `fe5e8e73`) are superseded by its own 10-10 and 10-11 updates.
- Integration's board records are older and weaker: `am62p-es2-r1c` measured
  images 8-17 at integration `f013dbbb` (6,978 pass / 45 fail / 34
  NotSupported / 5 Quality / 2 Compatibility; ms4 14 / 2; gl_test 48 / 48) and
  the Pi 4 records ran nothing.

## 9. Owner section: recommended action per duplicate

| Duplicate | Action | Why |
|---|---|---|
| Plan #139 `pvr-vertex-2d-fix` (ready, held behind `pvr-pool-npot`; own staging memory in `src/pvr/**`) against main `a6fc6f14` | **Drop** the fix item. Keep `tests/es2_pvr_vertex_texture.rs` as tests only (add the three `YakoglBackend` fields). Re-open only if `am62p-es2-final-a` shows image 9 still failing | main already draws the 45 cases' grid in pieces; a second, different mechanism in the same functions would conflict and add dead code. The doc's fix section should be marked replaced |
| Plan #137 `pi4-console-tcp2` (running, agent `01a12af1`) against main `28fb55ae` | **Drop** it, or let it finish and then **rebase onto main** and keep only what main lacks (none known). Run `pi4-es2-tip-r2` and `pi4-es2-final-a` from a main-based tree | `board.sh` `host:port` for the Pi 4 exists on main, with 221 selftest checks; a second implementation conflicts in `board.sh` and `board_selftest.sh` |
| Integration `es2-invariance-repro` against main `f9688bc5` | **Keep as tests only**: `tests/es2_invariance.rs`, un-ignored, rebased on main; do not land the doc's fix section | main took the alternative; the test is still the only host guard for GC7000 and V3D |
| `d70f8ee7`, `289fb9c4`, `a54bbdec`, `8928d47d`, `c2c31372`/`7151a12a` against main `fdc3b692`, `8b3e7039`, `6c9e9eea`, `9224abaf` | **Drop** the integration copies; carry the 21-line audit addition and any test not on main | same change on both sides (3 patch-identical, the rest the same intent) |
| `933b4283`, `82cec4f4`, `e8e6029a` (state, program, buffer, framebuffer, texture rules) | **Rebase onto main**; they flip main's four `BUG:` pins | not on main; buffer.rs and framebuffer.rs changed heavily there |
| `b738368b` family (`tests/es2_npot.rs`) | **Keep as tests only**; drop the `docs/investigations/es2-npot-cases.md` edit | the doc is deleted on main |
| `f44a0ff0` (preprocessor `rescan_tail`) | **Keep as tests only plus the small fix**, low priority; not stage 1 | no dEQP case reaches it |
| `56491243`, `cc9b3846`, `f8bb9576` (GC7000 table, #22) | **Rebase onto main** | only on integration |
| `4fa00f0a` (fragdata), `3c317c4a` (V3D clear) | **Keep as tests only** | no clash |
| `2afaf0b9` / `02017674` (ROADMAP.md) | **Rebase onto main**, rewrite state paragraphs from section 8, take main's HANDOFF goal list | HANDOFF hunk conflicts |
| Website | **None** on integration; fix `deqp2.am62p.ms4` on main | main only |

Order that avoids repeated work: land (or rebase on) main first, then replay
the integration-only tests; run `am62p-es2-final-a` from origin/main, which
contains both `a6fc6f14` and `e17fb6a7`.

## 10. What was not checked

- No build or test run on either tip; compatibility statements are
  [inferred] from signatures and hunk ranges.
- The trial merge compares the two tips only. It says nothing about the
  review branches (`933b4283`, `82cec4f4`, `f44a0ff0`, ROADMAP, the table
  rows), whose conflicts in sections 3.4, 5 and 6 are from hunk ranges.
- Plan #136 was not read; `pvr-vertex-2d-fix` and `pi4-console-tcp2` have no
  commit to compare (section 2 and 4 describe them from the plan items).
- The `tools/cts/*` script overlap (section 7, item 4) was not diffed hunk by
  hunk.
