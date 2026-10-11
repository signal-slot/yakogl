<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Integration, the ccfarm item branches and origin/main: what main already covers (2026-10-12)

Item `es2-invariance` of plan #20 (an audit; the id is reused). Read-only: `git log`, `git show`,
`git diff`, trial merges with `git merge-tree --write-tree` (nothing written to a ref) and the
plan states of #136, #137 and #139 from ccfarm's plan tool. Nothing was built or run. Every
claim cites a sha.

Labels:

- **[read]**: read from a diff, a file or a plan.
- **[merge-tree]**: the conflict list of a trial merge. A file not listed merged cleanly.
- **[inferred]**: follows from what was read (struct literals, hunk ranges); not compiled.

## 0. What changed since the first audit

The first attempt at this item (ccfarm `01a12b02-b874-70e1-9511-b33ca0b91da5`, commits `9aaa16c0`,
`9ae0ab5f`) compared integration `76d349c4` with main `5ad2fa0e` on 2026-10-11. The owner closed
it on GitHub and re-landed its text on `work/int-pr92` (`171681be`, `62c70c5c`, not yet on main) as
`tools/cts/log/investigations/integration-main-overlap.md`. Its header note says
**integration was abandoned on 2026-10-12** and its results went to main, one pull request each [read]:

- **merged on main:** PRs #102 to #126;
- **still open:** #129 to #138, the `work/int-*` branches below.

This file is that audit redone at the current tips:

| Ref | Sha | Date |
|---|---|---|
| origin/main | `9af3abba` | 2026-10-12 05:16 |
| `ccfarm/integration` (the last ccfarm merge) | `39bb67dc` | 2026-10-11 22:58 |
| this workspace's base, the old main | `db3545fa` | an ancestor of origin/main; it holds every main commit the task names (`e17fb6a7`, `a6fc6f14`, `4e9397c6`, `28fb55ae`, `c4ccc966`) |

Integration and main share base `2f69c7d1`; main has 248 commits integration lacks, and
integration has 121 main lacks. Most of those 121 now reach main as `work/int-*` re-landings,
under new shas. That is why the table below compares content, not ancestry.

The owner's re-landing branches that are still open (`git for-each-ref --no-merged=origin/main
refs/remotes/origin/work/`) [read]:

| Branch | Tip | Re-lands | Files |
|---|---|---|---|
| `work/int-pr31` | `419b0159` | GC7000 table rows #22 (`7898ab09` = `cc9b3846`, `d6951f1c` = `f8bb9576`) | `tests/reference/vivante_blobs.rs`, `tools/vivante/*`, `tools/cts/log/investigations/gc7000-table-rows.md` |
| `work/int-pr44` | `13eb8a90` | es2-invariance-repro (`dadda3bf`, `2b76b6b1`), the 9 `#[ignore]` removed | `tests/es2_invariance.rs`, `tools/cts/log/investigations/es2-invariance.md` |
| `work/int-pr50` | `2f2227ca` | es2-npot-cases (`10b20bd0`) | `tests/es2_npot.rs`, `docs/es2-cts-failures.md` H.5 |
| `work/int-pr57` | `53fe3973` | es2-fragdata (`a1189436` = `4fa00f0a`) | `tests/es2_fragdata.rs`, `tools/cts/log/investigations/es2-fragdata.md` |
| `work/int-pr64` | `47ab9b2c` | state/program tranche (`1bad5754` = `933b4283`), with main's backend fields, two guard pins run | `src/gl/state.rs`, `program.rs`, `names.rs`, `tests/es2_error_rules_state.rs`, `tests/es2_report_guard.rs`, ... |
| `work/int-5162a33` | `05641216` | texture rules tests (`e8e6029a`), the five open rows pinned at today's answers | `tests/es2_error_rules_texture.rs` |
| `work/int-cdb28c0` | `52c322fe` | ms4 constancy diagnosis | `tests/es2_pvr_ms4_constancy.rs`, `tools/cts/log/investigations/es2-pvr-ms4-constancy.md` |
| `work/int-4451f13`, `int-e50d4ee`, `int-e02b6ee`, `int-4535b16`, `int-pr63` | | board records: Pi 4 tip tree blocked, AM62P images 1-7 blocked, console lease, AM62P GLES3 r2 blocked, i.MX8MP gl_test r2 | `tools/cts/log/board-sessions/*`, `investigations/*` |
| `work/int-pr34`, `int-pr49` | | ES 3.0 test fixes | outside this audit |
| `work/int-pr92` | `62c70c5c` | the first attempt of this audit | `tools/cts/log/investigations/integration-main-overlap.md` |

Integration merges that have **no** re-landing branch: `a25712f2` (`pi4-console-tcp2`, section
4), `76d349c4` (`17d4220c`, an earlier AM62P images 1-7 record superseded by `int-e50d4ee`) and
`77c86005` (`9625abd0`, readme walkthrough host-harness step).

## 1. Summary

"Where" uses these labels:

- **main**: on origin/main `9af3abba`.
- **owner PR**: on an open `work/int-*` branch.
- **ccfarm**: on `ccfarm/integration` or on an unmerged ccfarm item branch.

| # | Topic | Main | Integration / ccfarm branch (plan item) | Where | Verdict |
|---|---|---|---|---|---|
| 1 | PowerVR invariance, 9 AM62P `common_subexpression_1/2/3` | `f9688bc5` via `e17fb6a7` (fix); board image 1 on `e17fb6a7`: 1,379 / 0 | `1c574bb6` on `01a12c70-a6f9-...` (es2-invariance-repro, running; tests only, based on `db3545fa`); older `aa750648` | fix: main. Tests: ccfarm and owner PR `int-pr44` `13eb8a90`, the same test except comments | **resolved on main**; the tests are a **duplicate** of `int-pr44` |
| 2 | AM62P `texture.vertex.2d` (45) | `a6fc6f14` (`draw_indexed_pieces`); `985592b5` (#122) re-landed integration's tests and diagnosis with "the fix proposed below was not taken" | `e4bee1ba` (merged into integration by `225d1ec8`); plan #139 `pvr-vertex-2d-fix` (not started, no branch) | main | **replaced by main**; `pvr-vertex-2d-fix` is a **duplicate** |
| 3a | report-es2, vertex/VAO tranche | `fdc3b692`, `8b3e7039`, `6c9e9eea`, `3d1e5ba1`, `03840710` (`4e9397c6` is in both bases) | `289fb9c4`, `a54bbdec`, `57085276`, `b618d832`, `8928d47d` (integration); `209b8c89` on `01a12489-2eba-...` (es2-error-rules) | both, same change | **resolved on main**; `01a12489` **conflicting** (renames main's `tests/es2_error_rules_vertex.rs`) |
| 3b | state/program tranche | none; main's guard still carries `BUG:` pins at `tests/es2_report_guard.rs:436` and `:492` | `933b4283` on `01a12a21-934e-...` (changelog-es2-final, running) | owner PR `int-pr64` `47ab9b2c` and ccfarm | **replaced by `int-pr64`** once merged; until then still needed |
| 3c | buffer/framebuffer tranche | none; `BUG:` pin at `es2_report_guard.rs:576` | `195fa350` on `01a12b76-651f-...` (based on `db3545fa`); older `82cec4f4` | ccfarm only | **still needed**; **conflicting** with main and `int-pr64` (3.3) |
| 3d | texture tranche | none; `BUG:` pin at `es2_report_guard.rs:659` | tests `e8e6029a` (integration `5162a33d`); guards `336b3dc9` + `00e91bdf` on `01a12b4e-a143-...` (based on `39bb67dc`) | tests: owner PR `int-5162a33`. Guards: ccfarm only | tests **replaced by `int-5162a33`**; guards **still needed** |
| 4 | Pi 4 console over a serial bridge | `28fb55ae` (+ `c605d8ad`, `469875d8`, `d78b5c38`, `c17714e9`) | `db5928d1`, `2ff18444`, `c0733494`, merged into integration by `a25712f2` (plan #137 `pi4-console-tcp2`) | both, two implementations | **replaced by main**; **conflicting** in `tools/board.sh`, `tools/board_selftest.sh`; `tools/serial_capture.py` is integration only |
| 5a | NPOT, 18 CTS cases | `9fdc9bcb` (section H of `docs/es2-cts-failures.md`; the investigation removed) | `50dd2f1d` on `01a12c82-a318-...` (es2-npot-cases, running) | owner PR `int-pr50` (`10b20bd0`, `2f2227ca`) and ccfarm | **duplicate**; `50dd2f1d` **conflicts** (modify/delete of `es2-npot-cases.md`) |
| 5b | Preprocessor | record `tools/cts/log/investigations/es2-preprocessor.md` ("no defect"); no `rescan` in main's `pp.rs` | `f44a0ff0` on `01a11c0d-ccc3-...` (es2-preprocessor, review; `rescan_tail` + `tests/es2_preprocessor.rs`) | ccfarm only | **resolved on main** for dEQP; `f44a0ff0` **still needed** only as optional C99 hardening |
| 5c | Offline table #22 and the 8 offline GL tests | `9224abaf` (the 8 tests); `746b24cc` (investigation removed, #22 open in HANDOFF) | `7151a12a`, `c2c31372` (8 tests); `cc9b3846`, `f8bb9576`, `20a10e4a` on `01a1246d-a277-...`; older `56491243`, `e1ecccd7` on `01a12324-635c-...` (es2-offline-table, running) | 8 tests: both. Table: owner PR `int-pr31` and ccfarm | 8 tests **resolved on main**; the table is a **duplicate** of `int-pr31` |
| 5d | Fragdata `invalid_assign_to_1` | `4e9397c6` (in both bases); passes on the three boards | `4fa00f0a` on `01a11d2b-2352-...` (es2-fragdata-guard, running) | owner PR `int-pr57` and ccfarm | **resolved on main**; the tests are a **duplicate** of `int-pr57` |
| 5e | V3D `glClear` | `4f77ae26` (#120, `tests/v3d_clear.rs`); `358ca532` removed `es2-v3d-clear.md` ("no defect") | `21c3acb8` (integration `3c317c4a`); `es2-v3d-clear-jobs` (ready) | main | **resolved on main**; `es2-v3d-clear-jobs` is optional; the doc sections it cites are gone on main |
| 6 | Website, CHANGELOG, README, ROADMAP | `c4ccc966` (website), `02d3e514` (README, CHANGELOG), `8de69f69` / `0a7607b2` (`website/data.js`); no `ROADMAP.md` | ROADMAP `02017674`, `455a8241` (pub-roadmap, review); no CHANGELOG/README/website change on integration since the base | website, CHANGELOG, README: main only. ROADMAP: ccfarm only | **still needed**: ROADMAP (rebase); `data.js` ms4 row stale on main |

## 2. PowerVR invariance (main `e17fb6a7`)

**Main.** `f9688bc5`, merged by `e17fb6a7` (2026-10-10), stops `fusion_pairs` from fusing a sum on
the way to an `invariant` output into an `fmad`: `mark_exact` in `powervr/prepare.rs`, and
`ir::IoDecl::invariant`. The board confirms it: `tools/cts/RESULTS.md` line 17065, "AM62P: image 1
on `e17fb6a7`", 1,379 pass / 0 fail [read].

**ccfarm.** `1c574bb6` (2026-10-12, branch `01a12c70-a6f9-77d9-a216-3fd43553540a`, item
es2-invariance-repro) adds `tests/es2_invariance.rs` (642 lines, no `#[ignore]`) and
`docs/investigations/es2-invariance.md`, which says the fix is `e17fb6a7`. It changes no
`src/`. Its base is `db3545fa` [read].

**Owner PR.** `work/int-pr44` (`dadda3bf`, `2b76b6b1`, `13eb8a90`) lands the same test. The
whole difference from `1c574bb6` is 14 lines of comments (the doc's path, the fix named as
`f9688bc5`) [read]. The doc goes to `tools/cts/log/investigations/es2-invariance.md`.

**Verdict:** resolved on main. `1c574bb6` duplicates `int-pr44`. On its own it merges onto main
cleanly. Against `int-pr44` it gives an add/add conflict in `tests/es2_invariance.rs`, from the
comment lines [merge-tree]. It would also leave a second copy of the doc under
`docs/investigations/`.

## 3. report-es2 refusals (`4e9397c6` and the tranches)

`4e9397c6` (2026-10-07) is an ancestor of both main and integration. The overlap lies in the later
re-landings.

### 3.1 Vertex and VAO (resolved)

Main has `tests/es2_error_rules_vertex.rs` and `tests/es2_report_guard.rs` (`fdc3b692`,
`6c9e9eea`, `8b3e7039`). Branch `01a12489-2eba-...` (`8c052337`, `d70f8ee7`, `b9116dc8`,
`209b8c89`, based on integration `e984a4c8`) renames `es2_error_rules_vertex.rs` to
`tests/es2_error_rules.rs` (14 changed lines against main's copy). Against main, its `src/gl/vertex.rs`
differs only in comments:

- the `glVertexAttribPointer` type match, about line 47;
- two `docs/es3-vertex-gap.md` references, at lines 16 and 358.

Main has no `docs/es3-vertex-gap.md`; it cites `docs/es3-cts-failures.md` Appendix A [read].

Verdict: **resolved on main**. A trial merge of the branch's own four commits onto main (merge
base `e984a4c8`) conflicts in `src/gl/vertex.rs` only [merge-tree]. Git takes the test rename
silently, which would leave the dangling `docs/es3-vertex-gap.md` reference on main.

### 3.2 State/program (owner PR `int-pr64`)

`int-pr64` re-lands `933b4283` as `1bad5754`, then `47ab9b2c`. That commit adds main's three
`YakoglBackend` fields (`query_begin`, `query_end`, `query_result`; `src/backend.rs:1261` on main)
and drops the `BUG:` ignores at `es2_report_guard.rs:436` and `:492` [read]. `933b4283` itself
(branch `01a12a21-934e-...`) lacks those fields. Its `tests/es2_error_rules_state.rs` spells a
full `YakoglBackend { ... }` literal ending at `depth_resolve: None`, so it does not compile on main
(E0063) [inferred]. The running item changelog-es2-final changes the same files.

### 3.3 Buffer/framebuffer (ccfarm only; conflicting)

`195fa350` (2026-10-12, branch `01a12b76-651f-...`, based on `db3545fa`) refuses ES 3-only buffer
and framebuffer values. It also drops the `BUG:` pin at `es2_report_guard.rs:576`. No owner branch
carries it. Main changed no `YakoglBackend` field after `db3545fa` (`git diff db3545fa origin/main
-- src/backend.rs` adds no `pub` field), so its test literals compile there [inferred]. Trial
merges [merge-tree]:

| Merged with | Conflicts | Clean |
|---|---|---|
| origin/main `9af3abba` | `docs/HANDOFF.md`; `src/gl/buffer.rs` (one hunk, `glGetBufferParameteriv`, lines 629-642 of the merged file) | `framebuffer.rs`, `draw.rs`, `es3.rs`, `state.rs`, `texture.rs`, `tf_tests.rs`, `tests/es3_tf_api.rs`, `es2-error-rules.md` |
| `work/int-pr64` `47ab9b2c` | the two above, plus `src/gl/names.rs` (both insert after `reset_context`, line 191) and `tools/cts/log/investigations/es2-error-rules.md` | `tests/es2_report_guard.rs`, `tf_tests.rs`, `tests/es3_tf_api.rs` |

Verdict: **still needed**. Rebase onto main after `int-pr64` lands. The superseded `82cec4f4`
(branch `01a12a21-932c-...`) is to be dropped.

### 3.4 Texture (tests replaced, guards still needed)

`int-5162a33` re-lands `e8e6029a` as `c0fc580b` and `05641216`. It pins the five open rows at
today's answers instead of ignoring them. The guards are `336b3dc9` and `00e91bdf` (branch
`01a12b4e-a143-...`, based on integration `39bb67dc`): a new `src/gl/es2_texture_rules.rs` with
predicates, plus `tests/es2_texture_rules_pred.rs`. Neither main nor any owner branch has them,
and they are not yet wired into `texture.rs`. Integration's own `tests/es2_error_rules_texture.rs`
lacks the three `query_*` fields and does not compile on main [inferred].

The guards' two commits merge onto main without a conflict (merge base `39bb67dc`)
[merge-tree]. Plan #136's item `es2-error-rules-tex` writes the guards into `texture.rs`. It is
`ready`, waiting on `tex3d-finish` (plan 19), and its `handoff_from` is `es2-tex-rules-pred` (that
branch) [read]. Its goal still says to remove `#[ignore]` marks. `int-5162a33` replaced those marks
with pins at today's answers, so the guard has to flip pins, not ignores.

Verdict: the tests are **replaced by `int-5162a33`**. The guards are **still needed**. A guard
must then flip the pins of `int-5162a33` and the `BUG:` pin at `es2_report_guard.rs:659`
(`tex_image_2d`).

## 4. Pi 4 serial-bridge console (main `28fb55ae`; plan #137 `pi4-console-tcp2`)

**Main.** `28fb55ae` (2026-10-10). `uart_is_bridge` (`tools/board.sh:251`) matches
`^[^/:]+:[0-9]+$`. `bridge_capture` (`:253-264`) captures through `tools/am62p_netboot_uart.py
capture` into `console.raw` and `console.ts`, retrying "busy" 12 times at 10 s. `pi4_uart_free`
(`:794-806`) skips the `/proc` scan for a bridge, and `pi4_capture_start` (`:878`) calls
`bridge_capture`. The header (`:134-135`) documents `BOARD_UART`. `board_selftest.sh` pins
`WANT_CHECKS=227`. Later fixes: `c605d8ad`, `469875d8`, `d78b5c38` (`console_from_ts` in python3)
and `c17714e9` (`done_lossy`). The Pi 4 rows of `a27d69c0` and `1cf946a9` ran through this path
[read].

**Integration.** Plan #137's `pi4-console-tcp2` ran and was merged: agent `01a12af1-d009-...`,
commits `db5928d1`, `2ff18444`, `c0733494`, merge `a25712f2` (#94). It is a second implementation:

- `pi4_uart_tcp` and a new reader, `tools/serial_capture.py` (163 lines, `probe` / `capture`,
  exit 3/4/5);
- inline busy retries in `pi4_capture_start`, `console.txt` written directly (no
  `console.raw`/`.ts`);
- `WANT_CHECKS=210` [read].

The owner made no `work/int-*` branch for `a25712f2` [read].

**Verdict:** **replaced by main**, and **conflicting** if ever merged. A trial merge of
`a25712f2`'s own change onto main (merge base `cdb28c04`) gives three conflict hunks in
`tools/board.sh` [merge-tree]:

- the `BOARD_UART` header, merged-file lines 134-144;
- `pi4_uart_free`, 806-829;
- `pi4_capture_start`, 906-932.

It also gives one in `tools/board_selftest.sh` (`WANT_CHECKS` 227 against 210, lines 49-53), and
`tools/serial_capture.py` would land as a dead second reader.

**Plan #137 now** [read]:

- `pi4-console-tcp2` is `done`, merged into integration.
- `pi4-console-tcp-verify` is done, with no changes.
- `pi4-es2-tip-r2` is `running` (agent `01a12b18-f2ee-73d4-a52a-be060e767504`). It was
  redefined to probe the bridge with `tools/serial_capture.py probe` and to run the tree at the
  integration tip. Its record therefore measures the integration tree with the integration
  console, not main.
- Its only prebuilt images are `pi4-4451f136`, an integration commit.

## 5. NPOT, preprocessor, offline table, fragdata, V3D clear

- **NPOT.** Main `9fdc9bcb` folded the case list into section H of `docs/es2-cts-failures.md`. It
  deleted `tools/cts/log/investigations/es2-npot-cases.md`. `50dd2f1d` (branch `01a12c82-...`,
  running es2-npot-cases) adds `tests/es2_npot.rs` and edits that deleted doc (+51 lines):
  modify/delete. `int-pr50` carries the same test; the whole difference is one comment line, which
  points at H.2/H.3. It also adds H.5 to the inventory [read]. Trial merges [merge-tree]:
  - `50dd2f1d` onto main: modify/delete on `es2-npot-cases.md`.
  - `50dd2f1d` against `int-pr50`: the same, plus an add/add conflict on `tests/es2_npot.rs`.

  Verdict: **duplicate of `int-pr50`**.
- **Preprocessor.** Main has only the record `tools/cts/log/investigations/es2-preprocessor.md`.
  Its `src/shaderc/online/pp.rs` has no `rescan` [read]. `f44a0ff0` (branch `01a11c0d-ccc3-...`,
  based on integration `2c7f8871`, review) fixes `#define G F` / `G(2)` (C99 6.10.3.4) with
  `rescan_tail` and adds `tests/es2_preprocessor.rs`. No dEQP-GLES2 case reaches it (its own
  message: 532 of 532 preprocessor cases as expected). Its own commit merges onto main without
  a conflict (merge base `2c7f8871`) [merge-tree]. Verdict: **resolved on main for dEQP**.
  `f44a0ff0` is **still needed** only as optional hardening; it is not a stage-1 item.
- **Offline table (#22).** Main `9224abaf` runs the 8 GL tests without a table, the same change
  as `7151a12a` / `c2c31372`. Main HANDOFF lines 274-278 keep #22 open: 146 of gl_test's 192
  shaders have no row, 325 rows. `int-pr31` re-lands `cc9b3846` and `f8bb9576` with a record
  `gc7000-table-rows.md`. Branch `01a1246d-a277-...` carries the same two commits plus `20a10e4a`,
  on integration `e984a4c8`. Verdict: the 8 tests are **resolved on main**. The running
  es2-offline-table item is a **duplicate of `int-pr31`**. The plan item `pub-website` ("Re-land
  es2-offline-tests") is resolved by `9224abaf`.
- **Fragdata.** The fix is `4e9397c6` (common). `RESULTS.md` records the case as passing on the
  Pi 4 (`a27d69c0`), and the AM62P and i.MX8MP `32778b57` trees have 0 fail outside the 54 named
  cases. `4fa00f0a` (running es2-fragdata-guard) and `int-pr57` (`a1189436`, `53fe3973`) differ
  only in one comment line. Verdict: **resolved on main; duplicate of `int-pr57`**.
- **V3D clear.** `tests/v3d_clear.rs` is on main (`4f77ae26`, #120, re-land of `21c3acb8`).
  `358ca532` removed `es2-v3d-clear.md`: no defect, the Pi 4 tree is 0 fail. Verdict: **resolved
  on main**. `es2-v3d-clear-jobs` (ready) is optional extra coverage; the "doc 5.1 #7, 5.2" it
  cites exists only on integration.

## 6. Website, CHANGELOG, README, ROADMAP

- **Website.** On main only: `c4ccc966` (`work/land2`); figures in `website/data.js`, last
  changed in `0a7607b2`. Integration has no `website/`. One row on main is stale: `deqp2.am62p.ms4`
  reads 14 pass / 2 fail at `32778b57` (`data.js:94-100`), while HANDOFF line 47 gives 16 / 16 on
  `452d6c31` and `a27d69c0`, with "no RESULTS.md entry". Fixing it needs a RESULTS.md entry first.
- **CHANGELOG, README.** Main `02d3e514` (2026-10-11) and `0964d2d3`. Integration has not changed
  either since `e6b84f7d` (pub-changelog). Take main's.
- **ROADMAP.md.** On neither tip. Review branches: `01a11bc4-6674-...` (`02017674`) and
  `01a11d2d-0998-...` (`455a8241`: `ROADMAP.md`, one `README.md` line, a `docs/HANDOFF.md` goal
  list, and an unrelated `src/shaderc/online/sema.rs` line). Both branches conflict with main in
  `docs/HANDOFF.md` only, from main's rewrite `b6d86120`; `README.md` merges [merge-tree]. Verdict: **still needed**. Rebase onto main, drop the HANDOFF and `sema.rs`
  hunks, and take the state from section 7.

## 7. Main's dEQP-GLES2 numbers per board (mustpass 17,165)

Read from origin/main `9af3abba`. The table is `docs/HANDOFF.md` lines 45-49, last rewritten in
`b6d86120`. The summary is `docs/es2-cts-failures.md` lines 25-31 (last data commit `0a7607b2`,
then `9fdc9bcb`). The records are `tools/cts/RESULTS.md`; the `records/...` directories are
gitignored.

| Board | Built from | Date | Pass | Fail | NotSupported | Warnings | Record (RESULTS.md section) |
|---|---|---|---:|---:|---:|---|---|
| AM62P | `32778b57`, 17 images | 2026-10-10 | 17,070 | **54** | 34 | 5 Quality + 2 Compatibility | `records/am62p/deqp-32778b57/full/`; RESULTS:16974 |
| AM62P, image 1 only | `e17fb6a7` | 2026-10-10 | 1,379 | 0 | 0 | 0 | `records/am62p/deqp-e17fb6a7/img1`; RESULTS:17065 (the 9 invariance cases pass) |
| AM62P, ms4 image (16 `multisample.*`) | `32778b57` | 2026-10-10 | 14 | 2 | | | RESULTS:17043; HANDOFF: 16 / 16 on `452d6c31` and `a27d69c0` after `a4f64950`, no RESULTS entry |
| i.MX8MP | `32778b57`, 29 selectors | 2026-10-10 | 17,112-17,116 (17,318 of 17,371 begun) | **0** | 30-34 (34 of the begun) | 19 Quality | `records/imx8mp/deqp-32778b57/full`; RESULTS:17076 |
| Pi 4 | composed: tree `a27d69c0` (17,083 / 9 / 34, 34 lost in capture) + six cube rows on `1cf946a9` (934 / 934) + F1 rerun | 2026-10-10/11 | 17,126 | **0** | 34 | 3 Quality + 2 Compatibility | `records/pi4/deqp-a27d69c0/full`, `.../F1-rerun2`, `records/pi4/deqp-1cf946a9/rows`; RESULTS:17171 |

Reading:

- The AM62P 54 predate both fixes. The 9 invariance cases are board-confirmed on `e17fb6a7`.
  The 45 `texture.vertex.2d` cases (`a6fc6f14`, image 9) have no board result. HANDOFF "Waiting
  for a board" asks for image 9 and then one whole tree with ms4 from one main commit.
- The Pi 4 row combines three records; HANDOFF asks for one tree from one commit.
- No main tree contains `a6fc6f14` and `e17fb6a7` together. The plan #20 items
  `am62p-es2-final-a/b`, `imx8mp-es2-final` and `pi4-es2-final-a` are the runs that would.
- On origin/main, the first line of `docs/es2-cts-failures.md` still says "Written 2026-10-07".
  Its summary rows are current (`32778b57`, `a27d69c0`).

## 8. Owner section: recommended action per duplicate

| Duplicate (ccfarm item / branch) | Main or owner PR | Action |
|---|---|---|
| **Plan #139 `pvr-vertex-2d-fix`** (`ready`, held behind plan 18's `pvr-pool-npot`; own `extra` staging memory in `src/pvr/backend.rs`; no branch) | main `a6fc6f14`; `985592b5` marks that fix "not taken" | **Drop** (cancel the item, then the plan). Its goal expects `#[ignore]`d tests in `tests/es2_pvr_vertex_texture.rs`; the file has none, here or on main. Reopen only if AM62P image 9 from a main tree still fails. Main refuses by name a fan, a line loop, a primitive whose window exceeds half an arena, and more than 64 pieces (`MAX_PIECES`, `src/pvr/backend.rs:4462`); no dEQP-GLES2 case is known to reach them |
| **Plan #137 `pi4-console-tcp2`** (done, merged into integration: `a25712f2` = `db5928d1`, `2ff18444`, `c0733494`) | main `28fb55ae` and its follow-ups | **Drop** (do not re-land): main has the feature, with `console.ts` and lossy-capture handling that integration's version lacks |
| Plan #137 `pi4-es2-tip-r2` (running, integration tip) | plan #20 `pi4-es2-final-a` (origin/main) | **Let it finish as a record only** (the default taken on 2026-10-12, when no owner answer came): its tree is integration's, not main's, so it does not count for stage 1. `pi4-es2-final-a` from origin/main is the stage-1 result. The owner may still cancel it if it has not reached the board |
| Plan #136 `es2-error-rules-tex` (ready) | tests in `int-5162a33` | **Rebase onto main** after `int-5162a33`; start from `00e91bdf` (clean onto main); flip pins, not ignores |
| es2-invariance-repro `1c574bb6` | owner PR `int-pr44` | **Drop**; keep `int-pr44` (tests only) |
| es2-npot-cases `50dd2f1d` | owner PR `int-pr50` | **Drop**; keep `int-pr50` (tests only) |
| es2-fragdata-guard `4fa00f0a` | owner PR `int-pr57` | **Drop**; keep `int-pr57` (tests only) |
| es2-offline-table (`cc9b3846`, `f8bb9576`, `20a10e4a`; `56491243`, `e1ecccd7`) | owner PR `int-pr31` | **Drop**; keep `int-pr31` |
| changelog-es2-final, state tranche `933b4283` | owner PR `int-pr64` | **Drop**; keep `int-pr64` |
| es2-error-rules, vertex tranche `01a12489` (`209b8c89`) | main `fdc3b692`, `8b3e7039`, `6c9e9eea` | **Drop** |
| `es2-offline-tests` / `pub-website` (re-land `7151a12a`) | main `9224abaf` | **Drop** |
| Buffer/framebuffer tranche `195fa350` | none | **Rebase onto main** after `int-pr64`; fix the `names.rs`, `es2-error-rules.md`, `HANDOFF.md`, `buffer.rs` clashes (3.3) |
| Texture guards `336b3dc9`, `00e91bdf` | tests only in `int-5162a33` | **Rebase onto main** after `int-5162a33`; the guard has to flip that branch's five pins and `es2_report_guard.rs:659` |
| Preprocessor `f44a0ff0` | none | **Rebase onto main**, low priority (no dEQP case) |
| `es2-v3d-clear-jobs`, `es2-fbo-combos-test` (`06ae06ab`) | main `4f77ae26`; `358ca532` removed the fbo-combos and clear docs | **Keep as tests only** if wanted; not stage-1 work |
| ROADMAP `02017674` / `455a8241` | none | **Rebase onto main**; drop the HANDOFF and `sema.rs` hunks |
| This file, against `int-pr92` | owner PR `int-pr92` (2026-10-11 text) | This file replaces the 2026-10-11 text for current decisions; it uses another path (`docs/investigations/`), so no add/add conflict |

Order that avoids repeated work:

1. Merge the open owner PRs (`int-pr31/44/50/57/64`, `int-5162a33`, `int-cdb28c0`, the records).
2. Cancel the ccfarm items that duplicate them.
3. Start the remaining ccfarm work (3.3, 3.4 guards, ROADMAP, preprocessor) from origin/main,
   not from `ccfarm/integration`.
4. Build the final board trees from origin/main.

## 9. Not checked

- Nothing was compiled. The trial merges show textual conflicts only; a clean merge may still
  fail to build (for example the `YakoglBackend` literals of 3.2 and 3.4).
- Plan states are as of the plan tool's read on 2026-10-12.
- GitHub could not be queried, so the PR numbers are those quoted in the commit messages; the
  `work/int-*` branches are named instead.
- `work/int-pr34`, `int-pr49` (ES 3.0) and the `tools/cts/*` script overlaps were not read hunk by
  hunk.
