<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Plan status: the roadmap on one page

Written 2026-10-08 (plan #4, item `plan-status-doc`); updated 2026-10-09
(plan #4, item `plan-status-r2`): the plan #4 items for stages 1 and 3 with
their states, the execution order, and the proposals. The owner's view of the
roadmap fixed 2026-10-08, in order:

1. dEQP-GLES2: zero failures on TI AM62P (PowerVR), NXP i.MX8MP (GC7000) and
   Raspberry Pi 4 (V3D).
2. The benchmarks (glmark2, Slint, the AGL cluster sample) correct on the
   same three boards.
3. Public release: CHANGELOG, README, licences, a public roadmap.
4. ES 3.0 (host work may run ahead of the boards).
5. More boards.

How to read this file. "Merged" means the commit is an ancestor of HEAD (the
integration tip `ecfff113908c`); "unmerged" means it lives only on a
`ccfarm/*` branch not merged into integration. The state was reconstructed
from git (`git log`, `git log --all --not HEAD`, `git merge-base
--is-ancestor`), not from memory. This edition was written in a clone taken
at `ecfff113908c`; a result merged after that snapshot is named by its report
path (the es2-invariance diagnosis). Two results were merged and then vetoed
by a confirming review and reverted (`3f78982`, `a35095b`); they are listed as
stopped. Hashes are short abbreviations and resolve with `git show` in this
clone, except run records named in quotes: `d60b9d76`, `d8583f20`,
`fe5e8e73`, `396d667a` and `29460f96` name runs on main before this clone's
root commit (`afb1633`, 2026-10-04), and `8abeaabb` is the origin/main tip
named by the 2026-10-08 records; those objects are not in this clone (the
read-only origin/main ref copy here is `16a53f8`, 2026-10-06). Board
addresses, UART paths and credentials never appear here (the rule at the head
of docs/HANDOFF.md).

## Stage 1 — dEQP-GLES2: zero failures on the three boards

Criterion. Each board runs the mustpass list `tools/cts/gles2-mustpass.txt`
(17,165 cases) with 0 Fail. NotSupported and QualityWarning are acceptable
where docs/es2-cts-failures.md section C lists them as acceptable (the
compressed-texture cases, clip_control, multisample at 1x, the warnings).

State.

- Last full trees (docs/es2-cts-failures.md, summary table): AM62P "d60b9d76"
  2026-10-02, 17,141 pass / 0 fail / 19 NotSupported / 5 warnings; i.MX8MP
  "d8583f20" 2026-10-01, 0 fail (17,112 to 17,116 pass, 30 to 34
  NotSupported, 19 warnings); Pi 4 "fe5e8e73" 2026-10-01, 25 fail (all cube
  cases), 17,075 pass. No board has run a full tree since.
- Plan #1's fixes are merged: `4e9397c` (DEPTH_COMPONENT24 gated on
  GL_OES_depth24; `gl_MaxDrawBuffers` is 1 in ES 2.0 builds), `c3e4146` /
  `ec5ba4f` / `53ef6d7` / `d8ce931` (V3D non-seamless cube, the 25 cases),
  `6bddfc5` (gl_test case 16 harness), `8409419` (the #9/#25 board recipe),
  `c6aa6ba` (PowerVR vertex attribute wrong-code fix), `e6e5f14` /
  `292310c` / `688a574` (dEQP image recipe checks).
- i.MX8MP: the plan #1 session ran at integration `b20e4d11`
  (docs/board-sessions/imx8mp-plan1.md, merged). `4e9397c` is confirmed on
  the board: shaders tail 1512/1512/0 and fbo 711/711/0, back at the
  "29460f96" baseline numbers. Case 16 is now a by-name skip, not a pixel
  failure. The 4x MSAA A/B failed in both arms at check 4 (got 0x00000000):
  bit 3 does not fix case 16, defect #8 stays open. The #9/#25 repeat runs
  (60 boots): base 4/40, page-aligned 1/10, no-reuse 2/10 — the rate fell
  from 14/30, but no knob removed the failure and the table cannot yet
  separate the hypotheses. The gc-case10 session (docs/board-sessions/
  imx8mp-gc-case10.md, merged `1ad4695`) re-measured #9/#25 (gl_test cases
  10 and 2): case 10 failed 1 of 30 boots (page arm), case 2 0 of 40; the
  earlier rate did not repeat, and the three hypotheses are still not
  separated — the KICK_DIFF second read-back instrument (gc-flake-instrument)
  is the next step.
- AM62P: the plan #1 session ran images 1 to 7 at integration `b83216d4`:
  10,092 pass / 9 fail. `shaders.fragdata.invalid_assign_to_1` moved
  NotSupported to Pass, as predicted. The 9 failures are new regressions
  (`shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}`),
  not yet in docs/es2-cts-failures.md. The record `bf5c575` is unmerged: it
  names the board login and the serial bridge, so it must re-land redacted
  (am62p-plan1-doc); the merged docs/board-sessions/am62p-plan1.md still
  records the earlier attempt that ran nothing. Images 8 to 17 and the ms4
  image have not run: the round-1 session am62p-es2-r1 is running now.
- Pi 4: no dEQP image has run since "fe5e8e73" (2026-10-01). The round-1
  session pi4-es2-r1a merged without running anything (docs/board-sessions/
  pi4-es2-r1.md, merged `c4a4d7f`): the worker had no Zephyr workspace, no
  SDK 1.0.1 and no dEQP stage 1, and rows 0-14 of tools/cts/pi4-full-tree.txt
  are all unrun. Rows 0-14 still need a session on a machine with that
  environment (the doc names system76). Rows 15-28 run now (pi4-es2-r1b).
- gl_test case 171 (stopped item): the fix `41f56eb` is merged; the board
  check is pending.
- Plan #4's host diagnoses are landing. es2-invariance-diag is done
  (docs/investigations/es2-invariance.md, merged after this clone's
  snapshot). es2-preprocessor-diag is merged (`ecfff11`, docs/investigations/
  es2-preprocessor.md) and found no preprocessor defect: the two
  `definitions.define_value_and_function_{fragment,vertex}` failures come
  from one stale row of tools/cts/RESULTS.md (line 7623, measured at
  `e31bd3e`, 2026-09-23), so es2-preprocessor reduces to a regression guard
  (tests/es2_preprocessor.rs). The npot, v3d-clear, error-rules,
  offline-table, v3d-fbo-combos and host-baseline diagnoses are pending (in
  review or running); see the item tables below.
- The gate's clippy row is host-dependent (the repository pins no
  toolchain; Cargo.toml sets only `rust-version = 1.88`). `141dcfa` cleared
  needless_range_loop for the 2026-10-06 toolchain; the plan record reports
  the `tools/check.sh --quick` clippy row still red at the tip in
  src/gl/draw.rs, src/gl/texture.rs and src/shaderc/online/sema.rs. Measured
  on this host (clippy 0.1.98): `tools/check.sh --quick` is clean at
  `ecfff113908c` (99 ok, 0 failed, 0 gaps). clippy-gate-fix makes the row
  green on every farm host.

Remaining. The stage 1 work is plan #4's item list and execution order below.

## Stage 2 — the benchmarks correct on the three boards

Criterion. Per board, the common set (GM glmark2, SO/SV Slint, CL cluster,
GT gl_test; `tools/cts/common-tree.txt`) builds, runs to a `tools/board.sh`
verdict of done with the pictures right — glmark2's terrain and refract
draw correctly, Slint and cluster PROBES pass — and the numbers are recorded
in docs/benchmarks-baseline/<board>.md.

State.

- i.MX8MP: measured 2026-10-08 (docs/benchmarks-baseline/imx8mp.md, merged;
  `2606b25`): GM score 42 (33 scenes ran, 0 failed, 0 unsupported), SO, SV
  and CL probes pass, GT 46/48 (case 16). The board has no console: no
  per-scene scores, no fps.
- AM62P: NOT MEASURED (docs/benchmarks-baseline/am62p.md; `6441595`,
  `c658d8a`, `15a8f46`, `4b41de4`): all five images built; GT ran with an
  empty console; GM and SO were interrupted; SV and CL never started.
- Pi 4: not run (docs/benchmarks-baseline/pi4.md; `b32b3f4`): the build and
  board variables were missing.
- Host-side work merged, no board run yet: `GL_OES_texture_npot` in ES 2.0
  builds (`2673197`, `77693ff`; the 18-case prediction in
  docs/es2-cts-failures.md section H); the PowerVR pool fix so refract's
  7.6 MB depth buffer fits (`541c9d3`); the Pi 4 carve-out raised to 96 MiB
  (`ca4074a`, host replay `ac0c254`); glmark2's GL stream on the host
  harness (`69a14e2`, `e7c314a`, `7f424b3`); the cluster joined the common
  set (`c7c80ba`); femtovg 0.26.0 provenance (`2563c27`, `8d09278`).
- Unmerged (in progress): the Slint/cluster host replay and its findings
  (`44b9592`); the EGL configs of the three samples on all three GPUs
  (`8ec4612`); common-tally judging glmark2 per scene (`217fff4`); the
  i.MX8MP frame-floor code F1/F2 (`a4c074d`; the investigation `e7a92d3` is
  merged); the AM62P scan-out view finding — the DSS already scans the
  PowerVR window, so there is no copy to remove (`e730fe6`); the errno audit
  (`81f3b2a`; a merged part found the numbers already agree, `06a81bd`); a
  pool replay showing that terrain's own set-up no longer fits after the
  npot mip chains (`459343d`) — a new gap.

Remaining. Board verification of npot (terrain's picture), refract (AM62P)
and the carve-out (Pi 4); the AM62P and Pi 4 baselines, which wait until
that board's stage 1 tree is done (see the proposals); a common-set round
with the CL row; GT case 16 on the i.MX8MP.

## Stage 3 — public release

Criterion. The repository is public with: a current CHANGELOG, a README,
CONTRIBUTING.md and SECURITY.md, a licence/SPDX audit with no unresolved
finding, a redaction audit showing no board address, credential or internal
record in the tree or history, a website consistent with the repository's
status sources, and this roadmap published.

State.

- Merged: CHANGELOG.md drafted from the git history (`4b60fb6`) and refreshed
  to the integration tip (`e6b84f7`, merged — the item pub-changelog is
  done); CONTRIBUTING.md and SECURITY.md drafts (`b4ec427`); the release
  building guide (docs/release/building.md; `f3da7e9`, `2560595`, `2606688`).
- Running: pub-roadmap — a public ROADMAP.md with the five stages and their
  criteria, aligned with README.md and docs/HANDOFF.md. ROADMAP.md is not in
  HEAD yet.
- Never merged, to redo: the licence/SPDX audit tool and report (`ec9f59c`,
  decided `34d7dc0`); the redaction audit (`3a4e55f`, decided `4b37a25`);
  the website update (`058e6fc`, `b3cc30e`, decided `e4fe79a`). website/
  exists from earlier work but does not yet match the status sources.
- Not started: the release checklist (pub-checklist) and the tree-diff tool
  (cts-tree-diff).

Remaining. The stage 3 items and their order are in the plan #4 section
below. The promotion to main and the publication itself are the owner's
decisions.

## Stage 4 — ES 3.0 (host work may run ahead)

Criterion. dEQP-GLES3 mustpass (42,810 cases) per board. The measured
starting point is the AM62P full tree at "396d667a" (2026-09-26): 31,278
pass (73.1%), 8,835 fail, 2,636 NotSupported; 56 lifetime cases do not run
because transform feedback is not implemented (tools/cts/STATUS.md).

State.

- Merged design and host work: es3-occlusion-query (`d303712`, `e37662e`),
  es3-pvr-2darray (`0edefef`), es3-pvr-formats (`6abe43d`, `8a1260b`,
  `a11d99a`; the doc check resolved as `7dedef7`, `a43ec9b`, `710934d`),
  es3-host-env (`675b447`), es3-host-toolchain (`6b99cff`), the GLSL ES
  3.00 shader-compiler coverage test over the dEQP gles3 corpus (`4ab482e`),
  the fbo / fragment_ops / fragment_out failure map (`53c7ba3`, `3cdbe16`),
  the AM62P dEQP stage 1 built for gles3 and gles2 (`2adce0b`), the
  transform-feedback and rasterizer-discard design (docs/es3-transform-
  feedback.md; `f818f84`, merged `eac9dec`), and the GLSL ES 3.00
  preprocessor tests (`79fe171`, `ed0c81d`).
- Unmerged (in progress): the shaders (`6a046ea`), texture (`b15aff0`) and
  remaining-groups (`1fc9d47`, `960e6d8`, `a3de654`, `c387185`) failure
  maps; the i.MX8MP GLES3 table (`c1f4503`, `473aa2b`) and the Pi 4 GLES3
  tree (`dbbdf79`, `c905ac6`, `88befa8`); the AM62P per-case GLES3 session
  (blocked on a silent UART; `46680ac` through `64f04b3`); code: ES 3.0
  integer colour render targets (`b9b2ac0`), sized internal formats at
  upload (`888af5d`), centroid fragment inputs (`56fa3df`), V3D matrix
  attributes / Ishl / Ftrunc (`72e0c5c`).
- Stopped: es3-caps (`b6004e1` merged, reverted by `3f78982` — vetoed by a
  confirming review after it was merged without a consultation; the earlier
  variants `52393d2`, `2128040`, `858bf09` remain unmerged) and the es2
  report guard (`75dedad` merged, reverted by `a35095b`, same reason). Both
  need a redo that goes through the consultation.

Remaining. Re-land es3-caps and the es2 report guard; finish the failure
maps; the board GLES3 sessions wait until each board's stage 1 tree is done
(see the proposals).

## Stage 5 — more boards

Criterion. A fourth board runs the stage 1 and stage 2 suites the way the
three current boards do.

State. Not started as a stage. Groundwork exists for the Armadillo-X2 (an
i.MX8MP-class board): the board layer `driver/boards/atmark/armadillo_x2`,
the direct-boot design (docs/direct-boot.md), `tools/x2-mkflash.py`. Its
first gc_slint run had no picture yet, and its direct boot was built but not
run (tools/cts/RESULTS.md, the "Armadillo-X2" sections).

Remaining. Nothing scheduled; the stage starts after stages 1 to 3 are done.

## Plan #4 — stages 1 and 3: items, states, execution order

Stage 1 criterion (as it stands): the mustpass list
`tools/cts/gles2-mustpass.txt` (17,165 cases) runs on each of the three
boards with 0 Fail, and the only NotSupported / QualityWarning results are
the acceptable buckets of docs/es2-cts-failures.md section C.

Stage 3 criterion (as it stands): the Stage 3 criterion above — a public
repository with a current CHANGELOG, README, CONTRIBUTING.md and SECURITY.md,
a clean licence/SPDX audit, a clean redaction audit, a website consistent
with the status sources, and the roadmap published.

States: done = merged; running = a session is active; pending = the result
is in review or the agent is still running, and its report file is not
merged; ready = not started, no blocker; waiting = not started, depends on
an earlier step.

### Stage 1 items

Diagnoses (host only):

| item | state | finding / evidence |
|---|---|---|
| es2-invariance-diag | done | docs/investigations/es2-invariance.md (merged; landed after this clone's snapshot) — host diagnosis of the 9 AM62P `shaders.invariance.*.common_subexpression_{1,2,3}` regressions |
| es2-preprocessor-diag | done | docs/investigations/es2-preprocessor.md (merged `ecfff11`): no preprocessor defect — the two `definitions.define_value_and_function_{fragment,vertex}` cases come from one stale row of tools/cts/RESULTS.md (line 7623, measured at `e31bd3e`, 2026-09-23); all 534 preprocessor shaders match the CTS verdict on the host |
| es2-npot-research | pending (in review) | GL_OES_texture_npot: replace the predicted case list (docs/es2-cts-failures.md section H) with what the CTS source checks; report docs/investigations/es2-npot-cases.md, not merged |
| es2-v3d-clear-diag | pending (in review) | scissored and masked glClear on V3D against the dEQP clear and fragment-ops cases; report docs/investigations/es2-v3d-clear.md, not merged |
| es2-error-rules-diag | pending (in review) | static audit of ES 3-only enums and formats a report-es2 build accepts, against the dEQP negative_api cases; report docs/investigations/es2-error-rules.md, not merged |
| es2-offline-table-diag | pending (running) | the offline shader table gaps: the GL tests the gate skips without a table (8 of the 11 ignored in the no-shader-table build), and the GC7000 table lag (#22, docs/es2-cts-failures.md); report docs/investigations/es2-offline-table.md, not merged |
| es2-v3d-fbo-combos-diag | pending (running) | the 27 Pi 4 `fbo.completeness.attachment_combinations.*_tex` cases; report docs/investigations/es2-v3d-fbo-combos.md, not merged |
| es2-host-deqp-baseline | pending (running) | host dEQP-GLES2 compile/link baseline at the integration tip, recorded without tool changes; report docs/investigations/es2-host-deqp-baseline.md, not merged |

Fixes:

| item | state | note |
|---|---|---|
| es2-invariance | ready | fix the 9 regressions: deterministic mul-add fusion on PowerVR; follows es2-invariance-diag (done) |
| es2-preprocessor | running | reduced to a regression guard: tests/es2_preprocessor.rs (not in the tree yet) |
| es2-npot-cases | ready | texture wrap and completeness under GL_OES_texture_npot, from the CTS-source case list; follows es2-npot-research |
| es2-v3d-clear | ready | scissored and masked glClear drawn correctly (host proof for dEQP color_clear / fragment_ops); follows es2-v3d-clear-diag |
| es2-error-rules | ready | refuse ES 3-only enums and formats in report-es2 builds; follows es2-error-rules-diag |
| es2-offline-table | ready | cover the no-table path in the GL suite and settle the GC7000 table lag (#22); follows es2-offline-table-diag |
| es2-host-deqp | ready | the host dEQP-GLES2 compile/link and error-rule run on the PowerVR and GC7000 back ends, committed as a record; follows es2-host-deqp-baseline |
| cts-tool-fixes | ready | board tooling defects recorded by the merged docs/board-sessions/imx8mp-gc-case10.md (`1ad4695`): the sweep-list `;` quoting (CMake drops bare separators) and the missing WDOG self-arm option in the gl_test recipe |
| clippy-gate-fix | ready | make the `tools/check.sh --quick` clippy row green on every farm host; see the clippy note in Stage 1 |

Round-1 board sessions:

| item | state | note |
|---|---|---|
| am62p-es2-r1 | running | dEQP-GLES2 images 8-17 plus ms4 at the integration tip, plus gl_test; record docs/board-sessions/am62p-es2-r1.md not merged yet |
| imx8mp-es2-r1 | running | the whole tree (29 entries); record docs/board-sessions/imx8mp-es2-r1.md not merged yet |
| pi4-es2-r1a | done — ran nothing | docs/board-sessions/pi4-es2-r1.md (merged `c4a4d7f`): no image built, no case ran; the worker lacked the Zephyr workspace, the SDK and dEQP stage 1 |
| pi4-es2-r1b | running | rows 15-28 of tools/cts/pi4-full-tree.txt |
| Pi 4 rows 0-14 (new session) | needed | on a machine with the Zephyr workspace, SDK 1.0.1 and dEQP stage 1; docs/board-sessions/pi4-es2-r1.md names system76 |

After round 1:

| item | state | note |
|---|---|---|
| es2-docs-r1 | waiting | fold the round-1 results into docs/es2-cts-failures.md, tools/cts/STATUS.md and tools/cts/RESULTS.md |
| am62p-es2-final-a | waiting | final tree, images 1-9 |
| am62p-es2-final-b | waiting | final tree, images 10-17 and ms4, whole-tree total |
| imx8mp-es2-final | waiting | final tree, run twice |
| pi4-es2-final-a | waiting | final tree, rows 0-14 |
| pi4-es2-final-b | waiting | final tree, rows 15-28, whole-tree total |
| es2-docs-final | waiting | record the final trees: inventory, HANDOFF, STATUS, RESULTS and the README conformance table |
| changelog-es2-final | waiting | CHANGELOG.md records the board-confirmed ES2 results |

gl_test side work:

| item | state | note |
|---|---|---|
| gc-flake-instrument | ready | the KICK_DIFF second read-back instrument for #9/#25 (gl_test cases 10 and 2); last measurement docs/board-sessions/imx8mp-gc-case10.md (`1ad4695`): case 10 failed 1/30 boots, case 2 0/40 |
| gc-msaa-record | waiting | record the 4x MSAA scratch probes for #8 without a console; the harness skip is merged (`6bddfc5`), defect #8 open (docs/board-sessions/imx8mp-plan1.md) |
| imx8mp-gltest-r2 | waiting | the #9/#25 instrument runs and the #8 probe runs on the board |

Publication record (feeds stage 3):

| item | state | note |
|---|---|---|
| am62p-plan1-doc | ready | re-land the AM62P images 1-7 board record redacted; the merged docs/board-sessions/am62p-plan1.md records an attempt that ran nothing, and the unmerged record `bf5c575` names the board login and serial bridge |

### Stage 3 items

| item | state | note |
|---|---|---|
| pub-roadmap | running | public ROADMAP.md with the five stages and their criteria; also aligns README.md and docs/HANDOFF.md; ROADMAP.md is not in HEAD yet |
| pub-changelog | done | `e6b84f7` merged: CHANGELOG.md current to the integration tip |
| pub-license-audit | ready (redo) | licence/SPDX audit tool and report; never merged (`ec9f59c`, decided `34d7dc0`) |
| pub-redaction-audit | waiting (redo) | redaction audit of the tree and history; never merged (`3a4e55f`, decided `4b37a25`); runs after am62p-plan1-doc |
| pub-website | ready (redo) | website/ consistent with the repository's current state; never merged (`058e6fc`, `b3cc30e`, decided `e4fe79a`) |
| pub-checklist | waiting | release checklist with measurable criteria and the owner's decisions; after the stage 1 final trees and the audits |
| cts-tree-diff | ready | tools/cts/tree-diff.py: per-case diff of two dEQP-GLES2 trees against the mustpass list; feeds es2-docs-r1 and es2-docs-final |

### Execution order

1. clippy-gate-fix. No dependency. It unblocks the review gate on the farm's
   newer-clippy hosts; every later change passes through that gate.
2. Land the pending host diagnoses, in parallel (all host-only):
   es2-npot-research, es2-v3d-clear-diag, es2-error-rules-diag,
   es2-offline-table-diag, es2-v3d-fbo-combos-diag, es2-host-deqp-baseline.
3. Host fixes as their diagnoses land: es2-invariance and es2-preprocessor
   can run now (their diagnoses are done); es2-npot-cases, es2-v3d-clear,
   es2-error-rules and es2-offline-table each follow its diagnosis from
   step 2; es2-host-deqp follows es2-host-deqp-baseline.
4. cts-tool-fixes — before any further board run.
5. am62p-plan1-doc — re-land the redacted AM62P images 1-7 record.
6. Round-1 board sessions: am62p-es2-r1, imx8mp-es2-r1 and pi4-es2-r1b (all
   running) plus a new Pi 4 rows 0-14 session on a machine with the Zephyr
   workspace, SDK and dEQP stage 1 (docs/board-sessions/pi4-es2-r1.md names
   system76). A session started after step 3 runs at a tip that holds steps
   1, 3 and 4.
7. es2-docs-r1 — after all of step 6.
8. gl_test chain: gc-flake-instrument, then gc-msaa-record (host-side, any
   time after step 3), then imx8mp-gltest-r2 on the i.MX8MP board — between
   step 6 and step 9, so the board is free for its final tree.
9. Final trees, after steps 3, 6 and 7: am62p-es2-final-a then
   am62p-es2-final-b; imx8mp-es2-final (twice); pi4-es2-final-a then
   pi4-es2-final-b. Stage 1 is met when each board's tree shows 0 Fail with
   only the section C buckets.
10. es2-docs-final, then changelog-es2-final.
11. Stage 3 host items overlap steps 2-10: pub-roadmap (running),
    pub-license-audit, pub-website and cts-tree-diff any time;
    pub-redaction-audit after step 5; pub-checklist after step 10 and the
    audits. The promotion to main and the publication itself stay the
    owner's decisions.

## Plan #2 and plan #3 items, classified

State: done = merged and its measurable part finished; in progress = work
exists unmerged, or merged but its board or measurable part is pending;
stopped = vetoed, abandoned, or blocked with no open attempt.

| item | plan | stage | state | evidence |
|---|---|---|---|---|
| baseline-am62p | 2 | 2 | in progress (measured nothing; redo) | docs/benchmarks-baseline/am62p.md |
| baseline-imx8mp | 2 | 2 | done (measured; GT row fails case 16) | docs/benchmarks-baseline/imx8mp.md, `2606b25` |
| baseline-pi4 | 2 | 2 | in progress (not run; redo) | docs/benchmarks-baseline/pi4.md, `b32b3f4` |
| npot (GL_OES_texture_npot) | 2 | 2 | in progress (claim merged; board rerun pending) | `2673197`, docs/es2-cts-failures.md section H |
| carve-out (Pi 4, 96 MiB) | 2 | 2 | in progress (merged; no board run) | `ca4074a` |
| refract (PowerVR pool) | 2 | 2 | in progress (merged; no board run) | `541c9d3` |
| terrain (black texture) | 2 | 2 | in progress (npot merged; new pool gap `459343d`) | docs/benchmarks.md gap 1 |
| glmark2 host harness | 2 | 2 | done | `69a14e2`, `e7c314a`, `7f424b3` |
| Slint/cluster host replay | 2 | 2 | in progress | `44b9592` (unmerged) |
| cluster in the common set | 2 | 2 | in progress (merged; no round run) | `c7c80ba` |
| EGL configs host test | 2 | 2 | in progress | `8ec4612` (unmerged) |
| i.MX8MP frame floor | 2 | 2 | in progress (investigation merged; F1/F2 unmerged) | `e7a92d3`, `a4c074d` |
| AM62P scan-out view | 2 | 2 | in progress | `e730fe6` (unmerged) |
| common-tally per scene | 2 | 2 | in progress | `217fff4` (unmerged) |
| femtovg provenance | 2 | 2 | done | `2563c27`, `8d09278` |
| errno audit | 2 | 2 | in progress | `81f3b2a` (unmerged), `06a81bd` |
| release building guide | 2 | 3 | done | docs/release/building.md |
| CHANGELOG | 2 | 3 | done (draft and refresh merged) | `4b60fb6`, `e6b84f7` |
| CONTRIBUTING / SECURITY | 2 | 3 | done | `b4ec427` |
| licence/SPDX audit | 2 | 3 | stopped (never merged; redo) | `ec9f59c`, `34d7dc0` |
| redaction audit | 2 | 3 | stopped (never merged; redo) | `3a4e55f`, `4b37a25` |
| website update | 2 | 3 | stopped (never merged; redo) | `058e6fc`, `b3cc30e`, `e4fe79a` |
| gl_test case 171 | 1 | 1 | stopped (fix merged; board check pending) | `41f56eb` |
| PVR formats doc check | 3 | 4 | done (resolved) | `7dedef7`, `a43ec9b`, `710934d` |
| es3 design docs | 3 | 4 | done | `d303712`, `0edefef`, `6abe43d`, `675b447`, `6b99cff` |
| es3 shaderc coverage | 3 | 4 | done | `4ab482e` |
| es3 fbo/fragops failure map | 3 | 4 | done | `53c7ba3` |
| es3 shaders / texture / rest maps | 3 | 4 | in progress | `6a046ea`, `b15aff0`, `1fc9d47` |
| GLES3 trees (i.MX8MP, Pi 4) | 3 | 4 | in progress | `c1f4503`, `473aa2b`, `dbbdf79`, `c905ac6` |
| AM62P GLES3 per-case session | 3 | 4 | in progress (blocked: silent UART) | `46680ac` … `64f04b3` |
| integer render targets | 3 | 4 | in progress | `b9b2ac0` |
| sized formats | 3 | 4 | in progress | `888af5d` |
| centroid | 3 | 4 | in progress | `56fa3df` |
| V3D shaderc ES 3.0 gaps | 3 | 4 | in progress | `72e0c5c` |
| transform feedback design | 3 | 4 | done (design merged) | `f818f84`, `eac9dec` |
| es3-caps | 3 | 4 | stopped (merged, vetoed, reverted) | `b6004e1`, `3f78982` |
| es2 report guard | 3 | 4 | stopped (merged, vetoed, reverted) | `75dedad`, `a35095b` |

## Proposals (labelled proposals, not decisions)

- Proposal: plan #3 (ES 3.0, stage 4) continues host-only at lower priority.
  Its board-side items — the AM62P per-case GLES3 session and the i.MX8MP
  and Pi 4 GLES3 trees, listed under Stage 4 — take no board leases until
  stage 1's final trees (execution step 9) are done. Its host work (failure
  maps, shader-compiler gaps, the transform-feedback design) continues.
  Reason: stage 1 is the roadmap's first stage, and the three boards are the
  scarce resource; ES 3.0 host work does not compete for them.
- Proposal: plan #2's remaining benchmark board runs — the AM62P and Pi 4
  baselines and the board verification of npot (`2673197`), refract
  (`541c9d3`) and the Pi 4 carve-out (`ca4074a`) — come after stage 1's
  round-1 sessions (execution step 6), so they do not compete for the three
  boards. Reason: each baseline should measure the board in its ES 2.0 state
  after the round-1 fixes, and the boards are already leased to the round-1
  sessions.
- No existing plan is cancelled.
