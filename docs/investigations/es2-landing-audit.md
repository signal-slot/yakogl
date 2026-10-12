<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 2.0 landing audit: eight finished result branches against origin/main and ccfarm/integration

Item `am62p-es2-r1a2` of plan #20. Host-only, document only: nothing was
merged, rebased, cherry-picked or pushed; the only change is this file.

## 1. What was checked against what

| Name | Commit | Notes |
|---|---|---|
| `origin/main` | `c345ab74` (2026-10-12, merge of PR #177) after `git fetch origin` | the landing branch; `db3545fa` (this workspace's start) is its ancestor, 282 commits behind it |
| `origin/ccfarm/integration` | `39bb67dc` (2026-10-11, merge of `01a12aff`, #93) | ccfarm's `merge_into` branch (`.ccfarm/project.toml`); `promote_to` is empty, so it never reached main |
| merge base of the two | `2f69c7d1` | main is 282 commits ahead of it, integration 121 |

The two lineages are **diverged**, and that decides most rows below. Main
carries, and integration lacks, among others `2241cd84` (the back end's
`query_begin`/`query_end`/`query_result` hooks, which every new
`YakoglBackend` literal in the tests names), `f9688bc5` (the PowerVR
invariance fix), `1bad5754` (the state tranche), `9224abaf`, `fdc3b692`;
`git merge-base --is-ancestor <c> origin/ccfarm/integration` is false for each.
Main also moved `docs/investigations/` to `tools/cts/log/investigations/`;
integration still has `docs/investigations/`.

The brief says each branch is based on `db3545fa`. That holds for six of the
eight. Two are not: the vertex/VAO branch is based on integration's `e984a4c8`,
and the offline no-table tests branch on `2e75b565`, a commit both lineages
contain.

### Commands

```text
git fetch origin
git log --format='%h %s' db3545fa..origin/main                    # what main added
git log --format='%h %s' db3545fa..origin/ccfarm/integration
git merge-base origin/main origin/ccfarm/integration              # 2f69c7d1
git merge-base <branch> origin/main | origin/ccfarm/integration   # each branch's base
git log --right-only --cherry-pick --format='%h %s' origin/main...<branch>        # patch-id containment
git log --right-only --cherry-pick --format='%h %s' origin/ccfarm/integration...<branch>
git merge-base --is-ancestor <commit> origin/main | origin/ccfarm/integration
git diff --stat <base> <target> -- <files the branch changes>     # empty = target has the base's blob
git diff <base> <branch> -- <file>; git diff <base> <target> -- <file>   # hunk overlap
git diff origin/main <branch> -- <file>                           # content equivalence
git grep -c / -n <pattern> <ref> -- <file>; git cat-file -e <ref>:<path>
git log --format=... <a>..<b> -- <paths>
```

**Limit of the method.** `git merge-tree`, `git worktree add` and writing a
patch file were all refused by this sandbox's permission layer, so no trial
merge was run. "Applies cleanly" below is **predicted** from the commands above:
a file is clean when the target still has the branch base's blob, or when the
target's hunks and the branch's do not touch the same lines; anything else is
named as a conflict. Where only the file set was compared, the cell says so.
Patch-id containment (`--cherry-pick`) said "not contained" for all eight
branches, because every landed equivalent was re-done (resolved, re-based on
main's layout), so containment was decided by content, per row.

## 2. Summary

| # | Branch (item) | Commits | Contained in origin/main | Contained in integration | Applies onto origin/main | Applies onto integration | Recommendation |
|---|---|---|---|---|---|---|---|
| 1 | `01a12489-2eba` (es2-error-rules, vertex/VAO) | 4: `8c052337` `d70f8ee7` `b9116dc8` `209b8c89` | partial: the tranche yes (`8b3e7039`, `fdc3b692`); these 4 cosmetic commits no | partial: the tranche yes (`289fb9c4`); the 4 commits no | conflict: `src/gl/shader.rs`, `src/gl/vertex.rs`, the renamed test, the doc (moved) | conflict: `src/gl/shader.rs` only | **obsolete** |
| 2 | `01a12c92-c0d5` (changelog-es2-final, state/program, `fe45a813`) | 1 | **yes**, by equivalent content (`1bad5754` + `47ab9b2c`) | no | conflict: the same 10 files, hunks already applied in another shape | conflict (predicted): `draw.rs`, `es3.rs`, `framebuffer.rs`, `program.rs`, `state.rs`, `tf_tests.rs`; and the tests do not compile there | **already landed** |
| 3 | `01a12c67-d898` (es2-offline-table, GC7000 table, `83d971fe`) | 1 | no | no | conflict: `tools/cts/log/investigations/es2-offline-table.md` (main deleted it); rest clean | doc path conflict (`docs/investigations/`); code text-clean, count pins not meaningful (3.3) | **re-land** after its doc is resolved |
| 4 | `01a12c82-a318` (es2-npot-cases, `50dd2f1d`) | 1 | partial: the test yes (`10b20bd0` + `2f2227ca`, one header comment differs); the doc section 8 no | no | conflict: `tests/es2_npot.rs` add/add (comment only); doc clean | test does not compile there (`query_begin`); doc path conflict | **re-land the doc part** (test already landed) |
| 5 | `01a12c84-4055` (es2-fragdata-guard, `bafbf5e5`) | 1 | partial: same 7 cases covered by an earlier result (`a1189436` + `53fe3973`) with a different test; this one adds the GL-API replay and a fuller doc | no | conflict: `tests/es2_fragdata.rs` and `tools/cts/log/investigations/es2-fragdata.md`, add/add | test: no conflict, but needs main's back end literal (`query_begin`) to build (3.5); doc path | **optional re-land** after deciding which test stays |
| 6 | `01a12c70-a6f9` (es2-invariance-repro, `1c574bb6`) | 1 | **yes**, by equivalent content (`dadda3bf` + `2b76b6b1` + `13eb8a90`) | no | conflict: add/add, comments only | applies, but its 9 PowerVR cases would fail there (no `f9688bc5`) | **already landed** |
| 7 | `01a11e01-9ded` (es2-offline-tests, `7151a12a`) | 1 | partial: all but `src/gl/draw.rs` (`9224abaf`, `82bffc50`, the `mesa_reference` cfgs) | partial: all but `src/gl/draw.rs` (`c2c31372`, said "without the draw.rs part") | clean (predicted): the 5 `draw.rs` attribute lines | clean (predicted) | **re-land the `draw.rs` part only**, if the 5 tests still fail |
| 8 | `01a12c93-41ce` (es2-invariance, overlap audit, `2bc29a6e`) | 3: `6ce2f169` `d0bafcdc` `2bc29a6e` | partial: the 2026-10-11 version is on main (`171681be`, `62c70c5c`); this redo is not | no | clean as a new file, but at a path main removed | clean | **obsolete** as a work item; land only as a record |

## 3. One section per branch

### 3.1 `01a12489-2eba-761e-8af0-da26cade3d06`: vertex attribute and VAO error rules

- **Commits** (on `e984a4c8`, an integration merge of 2026-10-10): `8c052337`
  rename the test to `tests/es2_error_rules.rs`; `d70f8ee7` (turn 3);
  `b9116dc8` gate `use std::format` in the `shader.rs` tests; `209b8c89` fix a
  stale test header and one overlong comment line.
- **Files, net of the four**: `docs/investigations/es2-error-rules.md` (37
  lines), `src/gl/shader.rs` (8), `src/gl/vertex.rs` (5),
  `tests/es2_error_rules_vertex.rs` renamed to `tests/es2_error_rules.rs` (9).
  `git diff e984a4c8 209b8c89 -- src tests` shows a comment re-wrap in
  `vertex.rs`, an import gate in the `shader.rs` tests and a doc comment in the
  test: no behaviour.
- **The work itself is already in both**: integration has `fdc3b692`'s
  equivalent `289fb9c4` (`git log origin/ccfarm/integration -- tests/es2_error_rules_vertex.rs`),
  main has `8b3e7039` and `fdc3b692`, and main's
  `tests/es2_error_rules_vertex.rs` is that test.
- **The import gate is in both, written differently**: main
  `src/gl/shader.rs:563-570` and integration `:565-574` already gate
  `glGetProgramInfoLog` and `format` (`82bffc50`; `e9300f9c`).
- **Clean apply**: onto integration only `src/gl/shader.rs` conflicts
  (`e9300f9c` rewrote the same import block; no other commit since `e984a4c8`
  touched the four paths). Onto main: `shader.rs`, `vertex.rs`, the doc (now at
  `tools/cts/log/investigations/es2-error-rules.md`, rewritten by `47ab9b2c`)
  and the renamed test (main's `es2_error_rules_vertex.rs` is a different
  file) all conflict.
- **Recommendation: obsolete.** Do not re-land. The rename to
  `es2_error_rules.rs` is also wrong for the tree as it is: main and the open
  tranches name their files `es2_error_rules_<group>.rs` (`_state`, `_texture`,
  and `_buffer`, `_fbo` on `01a12b76`).

### 3.2 `01a12c92-c0d5-740e-abf8-4d87de08c151`: state/program tranche (`fe45a813`)

- **Files** (10, +753/-57): `src/gl/{draw,es3,framebuffer,names,program,state,texture,tf_tests}.rs`,
  `tests/es2_error_rules_state.rs` (522 lines), `tests/es3_tf_api.rs`.
- **Contained in main by equivalent content.** Main's `1bad5754` (a cherry-pick
  of the older result `01a12a21-934e`, `933b4283`) changes the same 10 files
  for the same rows 14-17 and 19-21 of the ES 2.0 audit, and `47ab9b2c`
  adapted its test to main's backend fields. Both test files hold 7 `#[test]`s
  with the same row names (`row14_enable_disable_...` to
  `row21_get_programiv_...`). `fe45a813` is a second, independent
  implementation of the same task, written after `01a12a21` had landed.
- **Not in integration**: `git merge-base --is-ancestor 1bad5754` is false and
  integration has no `tests/es2_error_rules_state.rs`.
- **Clean apply**: onto main every one of the 10 files conflicts (the same
  hunks, applied once already). Onto integration the six source files
  integration changed since the merge base (`draw.rs`, `es3.rs`,
  `framebuffer.rs`, `program.rs`, `state.rs`, `tf_tests.rs`) are predicted to
  conflict (file set only; hunks not compared), and
  `tests/es2_error_rules_state.rs` names `query_begin`, which integration's
  back end contract lacks.
- **Recommendation: already landed.** Nothing to do. Its test names are the
  only difference; if the owner wants them, rename in a one-line follow-up.

### 3.3 `01a12c67-d898-751e-a3ba-e8c0de1a9803`: GC7000 offline table (`83d971fe`)

- **Files** (19, +53486/-25045): `tests/reference/vivante_blobs.rs`
  (+78091 lines, generated), `tools/vivante/{gen_shader_table.py,glsl2vivante.py,es100.py,etna_shc.c,check_gl_test_blobs.py,README.md}`,
  `src/shaderc/es100_twins.rs`, `src/shaderc/online/select.rs` (one hunk),
  `src/shaderc/online/vivante/roundtrip.rs`, nine files under `tests/`, and
  `tools/cts/log/investigations/es2-offline-table.md` (+140).
- **Contained**: no, in either. Main has no commit after `db3545fa` touching
  `tools/vivante/` sources or `tests/reference/vivante_blobs.rs`;
  `git diff --stat db3545fa origin/main` over the 18 code files is empty
  except `tools/vivante/README.md`.
- **Clean apply onto main**: the 18 code files are byte-identical to
  `db3545fa` on main, so they apply; `tools/vivante/README.md` differs on main
  (SPDX header, today's paths) but its hunks (lines 1-31, 97-139, 331) do not
  meet the branch's (line 51): clean. **Conflict: the doc.** `746b24cc`
  (2026-10-11) deleted `tools/cts/log/investigations/es2-offline-table.md`
  ("its open part goes to HANDOFF #22"), so the branch's +140 lines are a
  modify/delete.
- **Onto integration**: the file blobs equal `db3545fa` except `select.rs` and
  `tests/backend_differential.rs`, where integration removed the `invariant`
  field (different hunks from the branch's), so a text merge is clean; but
  `backend_differential.rs` pins population counts (244/347, 126/209) measured
  on a tree with main's changes, so they are not meaningful on integration.
  The doc is at `docs/investigations/es2-offline-table.md` there: a path
  conflict.
- **The gate**: `tests/reference/vivante_blobs.rs` is generated and the gate
  reproduces it byte for byte; this audit did not run the gate.
- **Recommendation: re-land onto main as is**, taking the 18 code files and
  re-homing the doc: either as a new record under
  `tools/cts/log/investigations/`, or folded into HANDOFF #22 (lines 140
  and 287) in the same commit, as the repository's landing rule asks. The
  README of `tools/vivante` already says "the product compiles with our own
  compiler" and that the table is "kept for measurement", so the owner should
  confirm that closing #22 is still wanted; that is a decision, not a defect
  in the branch.

### 3.4 `01a12c82-a318-7709-87c5-a68408a9c086`: NPOT host replay (`50dd2f1d`)

- **Files**: `tests/es2_npot.rs` (650 lines) and
  `tools/cts/log/investigations/es2-npot-cases.md` (+51, a new section 8 and
  a follow-up list of stale NPOT comments in `src/`).
- **Contained in main: the test yes.** `10b20bd0` + `2f2227ca` give
  `tests/es2_npot.rs`; `git diff origin/main 50dd2f1d -- tests/es2_npot.rs`
  is one comment line (the header's pointer: main says
  `docs/es2-cts-failures.md` H.2/H.3, the branch says the investigation's
  section 3.1). **The doc's section 8 is not in main**: main's file ends at
  the old section 7 and has no `es2_npot` text (`git grep` empty).
- **Integration**: neither the test nor the doc section is there, and the test
  names `query_begin`, so it would not compile on integration's back end.
- **Clean apply**: onto main the test is an add/add that differs by that one
  comment; the doc hunk (an append at the end, file unchanged on main since
  `db3545fa`) is clean. Onto integration: the test file is new but does not
  build; the doc is at `docs/investigations/es2-npot-cases.md` there (path
  conflict).
- **Another copy exists**: `origin/ccfarm/01a12e48-056d-778b-9b0c-451f27e10ea7`
  ("tests: replay the 18 NPOT dEQP-GLES2 setups on the host", 2026-10-12
  13:09) carries a 650-line `tests/es2_npot.rs` and a 40-line doc change
  against main: a third run of the same task. It is not audited further here.
- **Recommendation: re-land the doc part only** (`50dd2f1d`'s doc hunk; take
  the branch's one-line header so the test points at the section). The test is
  already landed.

### 3.5 `01a12c84-4055-7540-95be-b0427d4fad52`: fragdata guard (`bafbf5e5`)

- **Files**: `tests/es2_fragdata.rs` (432 lines),
  `tools/cts/log/investigations/es2-fragdata.md` (199 lines).
- **Main has the same cases from an earlier result** (`a1189436`, `53fe3973`,
  re-landed from `01a11d2b`): 3 `valid_*` cases, `invalid_assign_to_1`, 3
  `write_fragcolor_and_fragdata_*` and a `report-es2` `gl_MaxDrawBuffers`
  test, run through the front end only (`compile_to_ir`), with the GL half
  pinned in `src/gl/state.rs`. This branch replays the 7 cases through the
  public GL API on a context (6 tests, per its commit message) and records the
  CTS lines (`glsShaderLibraryCase.cpp:1041-1052`) and the per-board result.
  So the coverage is equal; the way in differs and the doc is fuller
  (`git diff --stat origin/main bafbf5e5`: 482 test lines, 281 doc lines).
- **Integration**: not there; the test is new on integration, and needs the
  back end literal of main (`query_begin`) to build.
- **Clean apply**: onto main both files are add/add conflicts. Onto
  integration, no code conflict; the doc is under `docs/investigations/`
  there.
- **Recommendation: optional re-land after an owner or reviewer choice**
  between main's front-end test and this API-level one (they cannot both be
  `tests/es2_fragdata.rs`); the doc is the part worth keeping. No CTS failure
  depends on it: the source change the branch says is needed is none.

### 3.6 `01a12c70-a6f9-77d9-a216-3fd43553540a`: invariance test (`1c574bb6`)

- **Files**: `tests/es2_invariance.rs` (642 lines),
  `docs/investigations/es2-invariance.md` (242 lines).
- **Contained in main by equivalent content**: `dadda3bf`, `2b76b6b1` and
  `13eb8a90` (the last "the nine PowerVR cases run (main fixed them); the doc
  as a record"). `git diff origin/main 1c574bb6 -- tests/es2_invariance.rs` is
  14 lines, all comments (the branch's header cites commit `e17fb6a7` where
  main's cites `f9688bc5`, and a macro doc line). The doc is main's
  `tools/cts/log/investigations/es2-invariance.md` (237 lines).
- **Integration**: neither file; and without `f9688bc5` the 9 PowerVR cases
  fail there, so landing it on integration would turn the tree red.
- **Clean apply**: onto main add/add, comments only; onto integration text
  clean.
- **Recommendation: already landed.**

### 3.7 `01a11e01-9ded-7172-9d12-f09a91212514`: offline-table GL tests without a table (`7151a12a`)

- **Files** (6, +254/-58): `src/gl/{draw,program,shader}.rs`,
  `src/testutil.rs`, `src/shaderc/online/vivante/{alloc,roundtrip}.rs`. Based
  on `2e75b565`, which both lineages contain.
- **Per file**: `program.rs` (no-shader-table count 7 to 20), `testutil.rs`
  (6 to 7) and `alloc.rs`/`roundtrip.rs` (`mesa_reference` cfg 7 to 12 and 1
  to 2) reach the branch's counts on **both** main and integration, so those
  are in, through `9224abaf`/`82bffc50` and `c2c31372`. `shader.rs` has more
  gates than the branch on both. **`src/gl/draw.rs` is the gap**: the branch
  has 15 `no-shader-table` mentions against 9 on both targets; the five
  attribute changes (`a_fifth_blend_state_evicts_the_least_recently_used_variant`,
  `front_facing_key` and its caller, `blend_state_without_a_variant_refuses_the_draw_and_logs_once`,
  `a_refused_draw_leaves_its_pending_clear_for_the_next`,
  `odd_sized_vs_block_pads_the_fs_block_to_16_bytes`) are in neither
  (`git grep -F` finds the old attribute comments one for one on both). The
  integration commit says "without the draw.rs part" itself.
- **Clean apply**: the five old attribute lines are present verbatim on main
  and integration, and neither changed those lines; predicted clean on both
  (offset hunks).
- **Recommendation: re-land the `draw.rs` part only**, after one check that
  the five tests still fail or are skipped wrongly under
  `--features host-test,gpu-powervr,no-shader-table` on main (the branch
  claims they do). No other hunk is needed.

### 3.8 `01a12c93-41ce-756f-bf77-6d0f47ef9869`: integration/main overlap audit (`2bc29a6e`)

- **Files**: `docs/investigations/integration-main-overlap.md` (325 lines,
  three commits `6ce2f169`, `d0bafcdc`, `2bc29a6e`).
- **Main has the earlier audit**: `171681be` + `62c70c5c`, 477 lines under
  `tools/cts/log/investigations/integration-main-overlap.md`, "the audit of
  2026-10-11 ... unchanged". This branch is the redo at main `9af3abba`
  with the trial-merge results and the plan #136/#137/#139 states; its text
  ("pi4-es2-tip-r2 finishes as a record only") is not in main.
- **Clean apply**: a new file on both, so no text conflict, but at a path
  main removed.
- **Recommendation: obsolete as work.** Its recommendation, re-landing, was
  carried out (PRs #102-#138 on main), and this audit supersedes it for the
  eight branches above. If a record is wanted, land it as
  `tools/cts/log/investigations/integration-main-overlap-2026-10-12.md`, not
  under `docs/investigations/`.

## 4. Overlap with the items in review

The four items are not on any `origin` ref this clone can see
(`git log --remotes -i --grep=preprocessor --grep=attachment_combinations`
finds nothing newer than main), so their file sets are taken from the item
titles and from the closest branches that exist.

| In-review item | Files (source) | Overlaps which branch |
|---|---|---|
| `cts-tree-diff-readme` (buffer/framebuffer tranches), `01a12b76-651f` | `src/gl/{buffer,framebuffer,draw,es3,names,state,texture,tf_tests}.rs`, `tests/es2_error_rules_{buffer,fbo}.rs`, `tests/es2_report_guard.rs`, `tests/es3_tf_api.rs`, `tools/cts/log/investigations/es2-error-rules.md`, `docs/HANDOFF.md` (`git diff --stat origin/main...<it>`) | #2 state tranche on 8 source files, `tests/es3_tf_api.rs` and the error-rules doc, but #2 is obsolete; #1 on the error-rules doc (obsolete); #7 on `src/gl/draw.rs` (different hunks: 1 line vs five attribute lines, no conflict predicted); `docs/HANDOFF.md` with #3's #22 text |
| `es2-fbo-combos-test` | its own test file (title) | none |
| `es2-preprocessor` | the GLSL ES 1.00 front end (title; no branch visible) | #3 touches `src/shaderc/es100_twins.rs` and `src/shaderc/online/select.rs`: only if the preprocessor change reaches them; unknown |
| `clippy-gate-fix` | `tools/cts/host-deqp-run.sh`, `docs/investigations/es2-host-deqp-v3d-tip.md` (`01a12e19`) | none |

## 5. Order and what is left

1. Nothing needs to land for rows 2 and 6 (and 1, 8 are obsolete).
2. Row 7 (`draw.rs` five attributes) and row 4 (doc section 8) are small and
   independent of the open items: land first.
3. Row 3 touches `docs/HANDOFF.md` #22 (and `es2-docs-final` is the item
   that rewrites HANDOFF): land with or just before `es2-docs-final`.
4. Row 5 waits for a choice of test.
5. Integration is not a target worth landing on: it lacks the back end hooks
   and the invariance fix that four of these tests assume. Rows 3, 4 and 7 are
   the only ones that make sense there at all, and only for code.

What this audit could not establish: a real trial merge (`merge-tree` and
`worktree` were refused), whether the row 3 count pins and the generated
`vivante_blobs.rs` pass the gate, and the branches of the two in-review items
that are not visible.
