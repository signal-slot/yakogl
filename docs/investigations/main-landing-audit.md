<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Why main looked stuck: branches, the integration branch and the gate (2026-10-15)

Question from the owner: many ccfarm results are recorded as done, but main
seems to barely move. Where do they stop? Measured on a clone of the
repository's refs taken at main `81bffb65` (2026-10-14 18:22:52 UTC). `git
fetch` was refused on the host, so every number is as of that snapshot.

## 1. main moves

- Tip: `81bffb656038` 2026-10-14T18:22:52Z, "ccfarm: merge
  ccfarm/01a13642-... (#781)".
- Since 2026-09-15 (30 days): 1,282 commits, 438 of them on the
  first-parent line (the whole history starts on 2026-10-04). First-parent
  commits per day, 10-04 to 10-14: 2, 48, 26, 26, 37, 34, 27, 12, 73, 83, 70.
- 299 of those commits are ccfarm merges into main (`ccfarm: merge
  ccfarm/<id>`), the first on 2026-10-06.

So main was not stalled. What stalled is `ccfarm/integration`, and, since
2026-10-14 17:42Z, the gate on main (section 3).

## 2. The integration branch

`ccfarm/integration` tip `39bb67dc` (2026-10-11 22:58 +09:00): **117 commits
ahead of main, 775 behind**. It has not moved since: `0838de80` (2026-10-12,
on main) made results land as pull requests to main (`promote_to = main`, no
integration branch). Its work was re-landed on main by the owner's
`work/int-*` pull requests (`integration-main-overlap.md`).

The 117 commits, sorted against main (each commit's patch tried on main, and
each touched file compared at the merge base, integration and main):

| Kind | Commits | Taken | Why not |
|---|---|---|---|
| merge commits | 42 | 0 | a merge carries no change of its own beyond the commits below and their conflict resolutions |
| the same patch is on main | 3 (`d72ed694`, `b618d832`, `57085276`) | 0 | duplicate |
| the change is already in main's files | 6 (`d83dcfbd`, `efc1fb13`, `116aca62`, `a9149999`, `9b778704`, `1ff208c4`) | 0 | duplicate |
| touches only files main does not have | 7 (`32fa3678`, `05c27390`, `add776ef`, `cda96c78`, `6bdde8c4`, `17d4220c`, `c0733494`) | 0 | superseded on main, see below |
| touches files main changed too, or moved | 59 | 0 | main has its own, later version (re-landed, moved under `tools/cts/log/`, or a second implementation); merging would conflict or revert main |

The seven files only integration has, and what main has instead:

| Integration file | On main |
|---|---|
| `docs/board-sessions/pi4-es2-r2-baseline.md` | `tools/cts/log/board-sessions/pi4-es2-r2-baseline.md`, a superset (+69 lines) |
| `docs/investigations/es2-pvr-vertex-texture.md` | `tools/cts/log/investigations/es2-pvr-vertex-texture.md`, a superset (+36) |
| `docs/investigations/gc-msaa-record.md` | `tools/cts/log/investigations/gc-msaa-record.md`, a superset (+18) |
| `docs/boards/console-map.md` | `tools/cts/log/investigations/console-map.md`, with the addresses removed |
| `docs/es3-vertex-gap.md` | folded into `docs/es3-cts-failures.md` (Appendix A) |
| `docs/board-sessions/am62p-es2-tip-img1-7.md` | `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md` (rows 2026-10-11, 2026-10-12) |
| `tools/serial_capture.py` | none: main's `tools/board.sh` reads a bridge through `tools/am62p_netboot_uart.py capture` (`28fb55ae`); this would be a dead second reader |

**Nothing of the integration branch is left to land.** An earlier attempt at
this audit (ccfarm `01a13ba0-...`, `9cf766c2`) landed those seven files plus
a `prove.sh` `.rmeta` change; all of them were duplicates (`prove.sh` has had
the same fix since `39a4972e`), and `console-map.md` there brought back the
bridge addresses.

## 3. The gate on main was red

`tools/check.sh --quick` at main `81bffb65`: **116 ok, 5 failed, 0 gaps**
(3,537 s, 2 compiler slots). One cause for all five rows (clippy
`host-test,gpu-powervr`, test `host-test,gpu-powervr`, clippy
`...,test-knobs`, clippy `host-test,gpu-v3d,...,no-shader-table` and its
`report-es2` twin): `error[E0063]: missing fields generate_mipmap and
return_mipmap_chain` in `tests/es3_caps.rs` and `tests/es3_blit_ds.rs`.

- `tests/es3_caps.rs` came with #772 (`4a4fad60`, 16:18Z) and
  `tests/es3_blit_ds.rs` with #782 (`43fee551`, 15:53Z), both on
  2026-10-14.
- #787 (`7e8c0b37`, 17:42Z) added the two fields to `YakoglBackend`
  (`c6f17516`) and updated every literal its base had. Its CI ran on a base
  without the two new tests.

So main has failed to compile two test files since `7e8c0b37`, and every
later pull request whose rows build the tests (anything in `src/gl`,
`src/shaderc`, a back end, `Cargo.toml`, `tools/check.sh`) is red on the
merge with main through no fault of its own. Kind: a broken test (stale
struct literal), not a product defect. None of the five was a clippy
warning, a fmt drift, a timeout or a missing tool.

`ccfarm/integration` at `39bb67dc` with its own `check.sh --quick`: 106 ok,
0 failed.

## 4. The per-task branches

Of the 307 `origin/ccfarm/<id>` branches in the snapshot, 15 are merged into
main; 6 more were merged under a later tip and 4 more are patch-equivalent
to main. 26 others are in `ccfarm/integration` (re-landed through it). That
leaves **256 branches whose commits are on neither main nor integration**
(ahead of main by 1: 103, 2: 31, 3: 53, 4: 18, 5 or more: 51; last commit
dates 10-07 to 10-15, 84 on 10-14). The repository does not say why they
stopped; ccfarm's own state does (its `farm_status`, read on 2026-10-15 for
the last 24 hours):

- 86 results merged, 8 abandoned, 16 reworked (rework rate 17 %).
- Of the 50 latest review items: 13 merged, **15 approved and waiting for
  the CI on their pull request**, 10 being redone, 3 resolving conflicts
  with main, 3 held for a missing environment (`ZEPHYR_VENV`, `WEST_VENV`,
  `GLSLANG`; `shellcheck`; `MESA_V3D_TREE`), 2 in review, 3 cancelled, 1
  with the arbiter.
- At least four of the redos are "CI failed on pull request #767 / #791 /
  #795 / #820 ... closed; made again as a new pull request", and two review
  rounds failed on `cargo test --no-run ... error[E0...]`: the red main of
  section 3 sends results back to be redone.
- The promotions from `ccfarm/integration`: the last one that reached main
  was on 2026-10-09 (114 tasks, `2f69c7d1`); the next (2026-10-10, candidate
  `a35b4a68`) failed its checks (126 ok, 8 failed), so nothing more went
  from integration to main before the branch was dropped on 2026-10-12.
- `tools/handoff-notes.sh --check` is listed 14 times as failing on main
  too. It is not part of `check.sh` or the CI: it exits 1 by design while a
  note waits for the handoff-fold stage.

`docs/handoff/README.md` records one more cause: on 2026-10-13, 77 of 79
redos in 36 hours were conflicts, most in `docs/HANDOFF.md`.

After the snapshot main moved three more merges (#477, #813, #761, to
`769aed82` at 2026-10-14T20:37Z); the two test files are still unfixed
there, and none of the three touches a file this change does.

## 5. Every named branch against main `81bffb65`

`ahead behind` = commits the branch has that main lacks, and the reverse.
`heads/` are the owner's local branches, `remotes/origin/` the GitHub ones.
The per-task `ccfarm/<id>` branches are summarized in section 4.

| Branch | ahead behind | Last commit |
|---|---|---|
| heads/main | 0 0 | 2026-10-14 |
| remotes/origin/main | 0 0 | 2026-10-14 |
| remotes/origin/work/board-common-filter | 0 61 | 2026-10-14 |
| remotes/origin/work/gate-every-runner | 0 167 | 2026-10-14 |
| remotes/origin/work/check-compiler-slots | 0 249 | 2026-10-14 |
| remotes/origin/work/stage-after-note | 0 260 | 2026-10-13 |
| remotes/origin/work/handoff-notes | 0 266 | 2026-10-13 |
| remotes/origin/work/orion-gate | 1 271 | 2026-10-13 |
| remotes/origin/work/bgra8-pin | 0 272 | 2026-10-13 |
| remotes/origin/work/int-pr49 | 5 474 | 2026-10-12 |
| remotes/origin/work/ccfarm-direct-main | 0 473 | 2026-10-12 |
| remotes/origin/work/farm-records-20261012 | 0 470 | 2026-10-12 |
| remotes/origin/work/ci-docs-hx90 | 0 475 | 2026-10-12 |
| remotes/origin/work/gate-primary | 0 479 | 2026-10-12 |
| remotes/origin/work/prove-rmeta | 0 479 | 2026-10-12 |
| remotes/origin/work/issue-13 | 0 484 | 2026-10-12 |
| remotes/origin/work/issue-128 | 0 487 | 2026-10-12 |
| remotes/origin/work/issue-127 | 0 503 | 2026-10-12 |
| remotes/origin/work/issue-8 | 0 498 | 2026-10-12 |
| remotes/origin/work/issue-101 | 0 497 | 2026-10-12 |
| remotes/origin/work/issue-9-kickdiff | 0 502 | 2026-10-12 |
| remotes/origin/work/treekey-inputs | 0 504 | 2026-10-12 |
| remotes/origin/work/gate-main-cancel | 0 504 | 2026-10-12 |
| remotes/origin/work/issue-86 | 0 504 | 2026-10-12 |
| remotes/origin/work/issue-82 | 0 504 | 2026-10-12 |
| remotes/origin/work/ci-step4 | 0 543 | 2026-10-12 |
| remotes/origin/work/int-pr92 | 0 562 | 2026-10-12 |
| remotes/origin/work/int-pr64 | 0 562 | 2026-10-12 |
| remotes/origin/work/int-pr63 | 0 563 | 2026-10-12 |
| remotes/origin/work/int-pr57 | 0 562 | 2026-10-12 |
| remotes/origin/work/int-pr50 | 0 562 | 2026-10-12 |
| remotes/origin/work/int-pr46 | 0 573 | 2026-10-12 |
| remotes/origin/work/int-pr44 | 0 561 | 2026-10-12 |
| remotes/origin/work/int-pr34 | 0 562 | 2026-10-12 |
| remotes/origin/work/int-pr31 | 3 564 | 2026-10-12 |
| remotes/origin/work/int-5162a33 | 0 579 | 2026-10-12 |
| remotes/origin/work/int-cdb28c0 | 0 580 | 2026-10-12 |
| remotes/origin/work/int-4316994 | 0 581 | 2026-10-12 |
| remotes/origin/work/int-225d1ec | 0 580 | 2026-10-12 |
| remotes/origin/work/int-89d518e | 0 580 | 2026-10-12 |
| remotes/origin/work/int-3c317c4 | 0 583 | 2026-10-12 |
| remotes/origin/work/int-af9767d | 0 584 | 2026-10-12 |
| remotes/origin/work/int-bbe55cc | 0 586 | 2026-10-12 |
| remotes/origin/work/int-2e96561 | 0 592 | 2026-10-12 |
| remotes/origin/work/int-blocked | 0 586 | 2026-10-12 |
| remotes/origin/work/int-e3efeaa | 0 592 | 2026-10-12 |
| remotes/origin/work/int-b98a03a | 0 592 | 2026-10-12 |
| remotes/origin/work/int-db19f9f | 0 592 | 2026-10-12 |
| remotes/origin/work/int-e02b6ee | 1 593 | 2026-10-12 |
| remotes/origin/work/int-616485e | 0 592 | 2026-10-12 |
| remotes/origin/work/int-43c6a70 | 0 592 | 2026-10-12 |
| remotes/origin/work/int-64e1714 | 0 592 | 2026-10-12 |
| remotes/origin/work/int-e50d4ee | 1 593 | 2026-10-12 |
| remotes/origin/work/int-5eaa3cf | 0 592 | 2026-10-12 |
| remotes/origin/work/int-39bb67d | 0 592 | 2026-10-12 |
| remotes/origin/work/int-4451f13 | 1 593 | 2026-10-12 |
| remotes/origin/work/int-4535b16 | 1 593 | 2026-10-12 |
| remotes/origin/work/int-579181d | 0 592 | 2026-10-12 |
| remotes/origin/work/int-4007722 | 0 592 | 2026-10-12 |
| remotes/origin/work/ci-step6 | 0 619 | 2026-10-12 |
| remotes/origin/work/ci-fast | 0 623 | 2026-10-11 |
| heads/ccfarm/integration | 117 775 | 2026-10-11 |
| remotes/origin/ccfarm/integration | 117 775 | 2026-10-11 |
| remotes/origin/work/docs-phase2 | 0 605 | 2026-10-11 |
| remotes/origin/work/docs-tidy-driver | 0 610 | 2026-10-11 |
| remotes/origin/work/docs-tidy-tools | 0 638 | 2026-10-11 |
| remotes/origin/work/farm-board-tasks | 0 658 | 2026-10-11 |
| remotes/origin/work/ci-on-prs | 0 663 | 2026-10-11 |
| heads/work/es3-core-misc | 7 690 | 2026-10-11 |
| heads/work/gc-rgba16f-layout | 0 668 | 2026-10-11 |
| heads/work/pvr-arena-bigdraw | 1 690 | 2026-10-11 |
| heads/work/es3-fbo | 4 690 | 2026-10-11 |
| heads/work/ci-generic | 0 688 | 2026-10-11 |
| heads/work/land9 | 0 690 | 2026-10-11 |
| heads/work/gate-local | 1 691 | 2026-10-11 |
| heads/work/es3-pvr-2darray | 0 691 | 2026-10-11 |
| heads/work/es3-srgb | 1 694 | 2026-10-11 |
| remotes/origin/work/gate-host-libclang | 3 691 | 2026-10-11 |
| heads/work/gc-case44-board | 1 696 | 2026-10-11 |
| heads/work/es3-buffer-pbo | 3 702 | 2026-10-11 |
| heads/work/es3-blit | 1 702 | 2026-10-11 |
| heads/work/es3-tf-capture | 2 702 | 2026-10-11 |
| heads/work/land8 | 0 700 | 2026-10-10 |
| heads/work/v3d-cube-mipmap | 1 718 | 2026-10-10 |
| heads/work/tally-capture-gaps | 1 718 | 2026-10-10 |
| heads/work/land6 | 0 710 | 2026-10-10 |
| heads/work/land7 | 0 704 | 2026-10-10 |
| heads/work/pi4-tcp-uart | 2 720 | 2026-10-10 |
| heads/work/gles3-tree-split | 1 720 | 2026-10-10 |
| heads/work/pvr-depth-resolve | 1 734 | 2026-10-10 |
| heads/work/gc-case44 | 1 734 | 2026-10-10 |
| heads/work/land5 | 0 724 | 2026-10-10 |
| heads/work/pvr-a2cconst-hang | 1 737 | 2026-10-10 |
| heads/work/land4 | 0 737 | 2026-10-10 |
| heads/work/flaky-int-rt | 1 738 | 2026-10-10 |
| heads/work/gc-gltest-set | 1 749 | 2026-10-10 |
| heads/work/land2 | 0 750 | 2026-10-10 |
| heads/work/land3 | 0 739 | 2026-10-10 |
| heads/work/pvr-a2c | 1 752 | 2026-10-10 |
| heads/work/es3-pvr-formats | 5 766 | 2026-10-10 |
| heads/work/es3-shaderc-gaps | 6 766 | 2026-10-10 |
| heads/work/docs-numbers | 10 751 | 2026-10-10 |
| heads/work/es3-occlusion | 7 766 | 2026-10-10 |
| heads/work/pvr-vertex-texture | 1 766 | 2026-10-10 |
| heads/work/deqp-gles3-image | 1 766 | 2026-10-10 |
| heads/work/website | 9 766 | 2026-10-10 |
| heads/work/pvr-invariance | 0 765 | 2026-10-10 |
| heads/fix/check-sh-green | 3 779 | 2026-10-10 |
| heads/work/land | 0 766 | 2026-10-10 |
| heads/work/status-catchup | 0 1167 | 2026-10-06 |
| heads/land/cleanup-3 | 3 1168 | 2026-10-06 |
| heads/work/cleanup-1-vivante-knobs | 3 1169 | 2026-10-06 |
| heads/land/tools-cs-ours | 1 1168 | 2026-10-06 |
| heads/work/precompiled-1-generator | 1 1168 | 2026-10-06 |
| heads/work/pco-driver-programs | 1 1169 | 2026-10-06 |
| heads/land/precompiled-45 | 0 1169 | 2026-10-06 |
| heads/land/imx8mp-pixel-clock | 0 1172 | 2026-10-06 |
| heads/work/imx8mp-pixel-clock | 1 1175 | 2026-10-06 |
| heads/land/cleanup-2 | 0 1174 | 2026-10-06 |
| heads/land/cleanup-1 | 0 1175 | 2026-10-06 |
| heads/land/precompiled-2 | 0 1180 | 2026-10-06 |
| heads/work/pvr-if-branch | 0 1185 | 2026-10-06 |
| heads/work/pvr-branch-cost | 0 1192 | 2026-10-05 |
| heads/work/build-robustness | 0 1194 | 2026-10-05 |
| heads/work/pvr-blend-factor-once | 0 1204 | 2026-10-05 |
| remotes/origin/work/pvr-blend-factor-once | 1 1206 | 2026-10-05 |

126 rows: 82 are fully in main (ahead 0); the 44 others are the integration
branch (2 rows) and 42 owner branches 1 to 10 commits ahead, last touched
10-05 to 10-13 and 271 to 1,206 commits behind. Those 42 were not examined commit by
commit here; most are the owner's own work and landing branches whose work
went to main through other branches (`work/land*`, `work/int-*`).
