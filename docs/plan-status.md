<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Plan status: the roadmap on one page

Written 2026-10-08 (plan #4, item `plan-status-doc`). The owner's view of the
roadmap fixed 2026-10-08, in order:

1. dEQP-GLES2: zero failures on TI AM62P (PowerVR), NXP i.MX8MP (GC7000) and
   Raspberry Pi 4 (V3D).
2. The benchmarks (glmark2, Slint, the AGL cluster sample) correct on the
   same three boards.
3. Public release: CHANGELOG, README, licences, a public roadmap.
4. ES 3.0 (host work may run ahead of the boards).
5. More boards.

How to read this file. "Merged" means the commit is an ancestor of HEAD (the
integration tip `7bc37edc`); "unmerged" means it lives only on a `ccfarm/*`
branch not merged into integration. The state was reconstructed from git
(`git log afb1633..HEAD`, `git log --all --not HEAD`,
`git merge-base --is-ancestor`), not from memory. Two results were merged and
then vetoed by a confirming review and reverted (`3f78982`, `a35095b`); they
are listed as stopped. Hashes are short abbreviations and resolve with
`git show` in this clone, except run records named in quotes: `d60b9d76`,
`d8583f20`, `fe5e8e73`, `396d667a` and `29460f96` name runs on main before
this clone's root commit (`afb1633`, 2026-10-04), and `8abeaabb` is the
origin/main tip named by the 2026-10-08 records; those objects are not in
this clone (the read-only origin/main ref copy here is `16a53f8`, 2026-10-06).
Board addresses, UART paths and credentials never appear here (the rule at
the head of docs/HANDOFF.md).

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
  cases), 17,075 pass.
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
  separate the hypotheses.
- AM62P: the session ran images 1 to 7 at integration `b83216d4`: 10,092
  pass / 9 fail. `shaders.fragdata.invalid_assign_to_1` moved NotSupported
  to Pass, as predicted. The 9 failures are new regressions
  (`shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}`),
  not yet in docs/es2-cts-failures.md. The record `bf5c575` is unmerged: it
  names the board login and the serial bridge, so it must re-land redacted;
  the merged docs/board-sessions/am62p-plan1.md still records the earlier
  attempt that ran nothing. Images 8 to 17 and the ms4 image have not run.
- Pi 4: no dEQP image has run since "fe5e8e73" (2026-10-01). The session
  attempt (unmerged `dbd1e88`) ran nothing: the Zephyr workspace, SDK, dEQP
  stage 1 and the board variables were missing on that worker.
- gl_test case 171 (stopped item): the fix `41f56eb` is merged; the board
  check is pending.

Remaining. Investigate the 9 AM62P invariance regressions (bisect against
"d60b9d76"); run AM62P images 8 to 17 plus ms4, or the whole tree; run the
Pi 4 session (the 25 cube cases, fragdata, the 27 fbo
`attachment_combinations`, `v3d_gl_test`); the i.MX8MP whole tree is
optional, plus the #8 follow-up (telling resolve from draw needs scratch
probes in a record, or a console) and a re-measure for #9/#25 against the
commit the 14/30 rate was taken at. Then replace the rows in
docs/es2-cts-failures.md, tools/cts/STATUS.md and tools/cts/RESULTS.md with
the new numbers.

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
that board's stage 1 tree is done (see the proposal); a common-set round
with the CL row; GT case 16 on the i.MX8MP.

## Stage 3 — public release

Criterion. The repository is public with: a current CHANGELOG, a README,
CONTRIBUTING.md and SECURITY.md, a licence/SPDX audit with no unresolved
finding, a redaction audit showing no board address, credential or internal
record in the tree or history, a website consistent with the repository's
status sources, and this roadmap published.

State.

- Merged: CHANGELOG.md drafted from the git history (`4b60fb6`);
  CONTRIBUTING.md and SECURITY.md drafts (`b4ec427`); the release building
  guide (docs/release/building.md; `f3da7e9`, `2560595`, `2606688`).
- In progress: the CHANGELOG refresh to the integration tip (`e6b84f7`,
  unmerged; the item pub-changelog owns CHANGELOG.md).
- Never merged, to redo: the licence/SPDX audit tool and report (`ec9f59c`,
  decided `34d7dc0`); the redaction audit (`3a4e55f`, decided `4b37a25`);
  the website update (`058e6fc`, `b3cc30e`, decided `e4fe79a`). website/
  exists from earlier work but does not yet match the status sources.

Remaining. Re-land the three items above; refresh the CHANGELOG; a README
pass; publish the roadmap (this file, once merged, is its first draft). The
promotion to main and the publication itself are the owner's decisions.

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
  and the AM62P dEQP stage 1 built for gles3 and gles2 (`2adce0b`).
- Unmerged (in progress): the shaders (`6a046ea`), texture (`b15aff0`) and
  remaining-groups (`1fc9d47`, `960e6d8`, `a3de654`, `c387185`) failure
  maps; the i.MX8MP GLES3 table (`c1f4503`, `473aa2b`) and the Pi 4 GLES3
  tree (`dbbdf79`, `c905ac6`, `88befa8`); the AM62P per-case GLES3 session
  (blocked on a silent UART; `46680ac` through `64f04b3`); code: ES 3.0
  integer colour render targets (`b9b2ac0`), sized internal formats at
  upload (`888af5d`), centroid fragment inputs (`56fa3df`), V3D matrix
  attributes / Ishl / Ftrunc (`72e0c5c`), the transform-feedback design
  (`f818f84`).
- Stopped: es3-caps (`b6004e1` merged, reverted by `3f78982` — vetoed by a
  confirming review after it was merged without a consultation; the earlier
  variants `52393d2`, `2128040`, `858bf09` remain unmerged) and the es2
  report guard (`75dedad` merged, reverted by `a35095b`, same reason). Both
  need a redo that goes through the consultation.

Remaining. Re-land es3-caps and the es2 report guard; finish the failure
maps; the board GLES3 sessions wait until each board's stage 1 tree is done
(see the proposal).

## Stage 5 — more boards

Criterion. A fourth board runs the stage 1 and stage 2 suites the way the
three current boards do.

State. Not started as a stage. Groundwork exists for the Armadillo-X2 (an
i.MX8MP-class board): the board layer `driver/boards/atmark/armadillo_x2`,
the direct-boot design (docs/direct-boot.md), `tools/x2-mkflash.py`. Its
first gc_slint run had no picture yet, and its direct boot was built but not
run (tools/cts/RESULTS.md, the "Armadillo-X2" sections).

Remaining. Nothing scheduled; the stage starts after stages 1 to 3 are done.

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
| CHANGELOG | 2 | 3 | in progress (draft merged; refresh unmerged) | `4b60fb6`, `e6b84f7` |
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
| transform feedback design | 3 | 4 | in progress | `f818f84` |
| es3-caps | 3 | 4 | stopped (merged, vetoed, reverted) | `b6004e1`, `3f78982` |
| es2 report guard | 3 | 4 | stopped (merged, vetoed, reverted) | `75dedad`, `a35095b` |

## Proposal (plan order)

- Plan #3 (ES 3.0) continues at lower priority. Its board sessions (the
  AM62P per-case GLES3 run, the i.MX8MP and Pi 4 GLES3 trees) are paused
  until each board's ES 2.0 final tree (stage 1) is done; its host work
  (failure maps, shader-compiler gaps, design docs) continues.
- Plan #2's host work (host harnesses, tallies, guides, audits) continues.
  Its AM62P and Pi 4 board baselines wait until that board's stage 1 ES 2.0
  tree is done, so each baseline measures the board in its ES 2.0-finished
  state.
- No existing plan is cancelled.
