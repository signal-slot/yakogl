# Stuck yakogl results (facts-stuck)

Investigated 2026-10-08, 13:56Z to 16:45Z, from worker XPS139340.
Nothing was changed except this file: no recheck, decision, merge or retry
was run.

Status: **partial.** The table below lists each unmerged yakogl result, its git
state, how long it has been idle and the safest action with its risk. It is
built from git (the snapshot of nuc's checkout plus a fetch of GitHub). The
ccfarm review stage of each result (`review_state`, the reviewer verdict, the
gate log) could **not** be read, because XPS139340 has none of yakogl's
records. Columns that need those records are marked *unconfirmed*.

## 1. Why the ccfarm records are not readable here (confirmed)

| time (UTC) | call (on XPS139340) | result |
|---|---|---|
| 13:56 | `farm_status(detail=true)` | `homes`: yakogl home `nuc`, `degraded: true`, `seq 157714`; only follower XPS139340 `applied_seq 0`, `lag 157714`, `last_pull_secs null`, `candidate false` |
| 13:56 | `list_agents` (states=completed, limit 300; and no filter) | `{"agents":[],"next_before":null}` |
| 13:56 | `list_decisions(state=all)` | `shown 0, pending_total 0` |
| 13:56 | `get_updates` | denied by the permission layer ("nobody decided in time") |
| ~14:00 | `list_plans(state=all)` | `{"plans":[]}` (plan #5 itself is not visible) |
| ~14:00 | `get_agent_result` 01a11b9b-b871…, 01a11a48-6b04… | "no agent with id …" (both exist as branches) |
| 14:26 | `farm_status` | `seq 163115`, follower still `applied_seq 0` |
| 16:34 | `farm_status` | `seq 182744`, follower still `applied_seq 0`, `last_pull_secs null` |
| 16:35 | `ask_farm` (project yakogl; asked XPS139340, Lepton, Lepton-local, nuc, system76) | `"answers":[]`, "No node of the farm knows this from its notes" |
| 16:35 | `get_plan(5)`; `get_agent_status` 01a11974-b6fe… | "no plan with id 5"; "no agent with id" |

The empty store on XPS comes from the replica, not from an absence of results:
the home's sequence grew by 25,000 entries in 2.6 h while the follower applied
none. Diagnosing the replica is item facts-replica.

## 2. How the result list was built (confirmed git facts)

1. Candidates: `ccfarm/<id>` branches of the snapshot (`refs/ccfarm/origin/heads`,
   nuc's checkout at integration `7bc37ed`, 2026-10-08 13:02Z) that are not
   ancestors of integration, with a commit in the last day. Branches whose
   subject starts with "ccfarm: review:", "strict review:", "confirming review:"
   or "resolve conflicts:" were left out. Those are reviewer or resolver agents,
   not results.
2. `git ls-remote origin` and
   `git fetch origin 'refs/heads/ccfarm/01a1*:refs/remotes/gh/01a1*'` plus
   `refs/heads/ccfarm/integration`. GitHub's integration is at
   **`334d4e6`** (2026-10-08 16:24Z). Its first-parent merges after `7bc37ed`:

   | merge | time (UTC) | merged result |
   |---|---|---|
   | c9c2cd3 | 14:31 | 01a11b9b-b591-7256-8d18-d1329773a7c7 |
   | 9ad5922 | 14:36 | 01a11b9b-b871-7718-8b5a-3ae8342e65c6 |
   | 2c7f887 | 14:46 | 01a11a03-c215-717c-871f-85fd9c7d5998 |
   | bc833b3 | 15:31 | 01a11bc4-636e-7458-b008-c229bf07a874 |
   | 334d4e6 | 16:24 | 01a11bec-8e9e-715a-b54d-af6be7fba045 |

   So ccfarm kept reviewing and merging yakogl results while nuc's Claude quota
   was exhausted (blocked until 23:00Z). The merge pipeline is **not** fully
   stopped by nuc's quota.
3. For each remaining candidate,
   `git log --cherry-mark --right-only --no-merges gh/integration...gh/<id>`.
   Every one still has at least one `+` commit: its content is not in
   integration, not even as a cherry-pick.
4. Running, not finished: 01a11a48-6b04-75b8-91a4-bc49d401177e (verdin-imx8mp
   lease, held by system76 at 13:56Z and 14:26Z) and
   01a11ab1-df5e-7058-9bec-0b619ff3a607 (verdin-am62p and usb-serial-1 leases at
   14:26Z). Both are left out of the table.

## 3. A shared blocker: the gate is red on integration itself (confirmed in part)

- Confirmed: the automatic review of this item's own markdown-only result ran
  `tools/check.sh --quick` and reported one failure, `FAIL no table in the
  artefact` ("the control found no table symbols with the table compiled in
  (0): the pattern no longer matches"). About 80 other steps passed. A
  markdown diff cannot cause it, so the integration base itself fails the gate
  on that reviewer's machine.
- Confirmed: commit `3985bee` (2026-10-08 16:02Z) on branch
  `ccfarm/01a119e0-9d60-7386-ada8-85bccbc131bc` explains and fixes it. Its
  message: "the release profile's LTO leaves the rlib's object as LLVM bitcode,
  which GNU nm 2.46 reads as no symbols at all, so the control arm found 0 and
  failed". The same commit clears clippy `needless_range_loop` at 10 sites. It
  also names one gap it does not fix: the "board.sh dry-run matrix" row needs
  `gawk`, which that host lacks.
- Confirmed: `tools/check.sh` is identical in `7bc37ed` and `334d4e6`, so the
  fix is not in integration.
- Confirmed: XPS139340 has `GNU nm (Gentoo 2.47 p1) 2.47.0` (`nm --version`,
  2026-10-08). Unconfirmed: whether 2.47 reads LTO bitcode rlibs, and which nm
  the review host that failed had.
- Unconfirmed: that this red gate is what holds the results below. Results did
  merge after 14:31Z (table above), so either those gates ran on a host whose
  `nm` reads LTO bitcode, or the failure is judged per host. The gate log of each
  result is in nuc's records.

## 4. The unmerged results (13)

"Idle since" is the commit time of the result branch's newest commit (UTC). It
is the latest moment the agent touched the branch, a confirmed git fact. The
stage after that is in nuc's records. The "stage" column gives the stage that
git supports and marks the rest *unconfirmed*.

The columns "nuc only?" and "check machine only?" answer whether that machine
alone holds the result. *nuc*: only nuc can say the stage (records), and only
nuc's checkout has the newer tip when marked so. *check machine*: the gate needs
`CC_aarch64_unknown_none`, which only system76 has (farm_status 13:56Z). That
applies to code changes whose gate has to build the no_std archive. Docs-only
changes still run the full gate (`commands = ["tools/check.sh"]`), so it applies
to them too.

| # | agent id / branch `ccfarm/<id>` | change (files) | idle since (UTC) | stage: confirmed / *unconfirmed* | waits on | nuc only? | check machine only? |
|---|---|---|---|---|---|---|---|
| 1 | 01a119e0-9d60-7386-ada8-85bccbc131bc | GL ES 3.0 integer render targets; plus `3985bee` gate fix (check.sh, clippy) | 10-08 16:02 (GitHub tip `3985bee`) | finished turn with a rework commit after the snapshot tip `b9b2ac0` (07:36). *Review of the rework not seen* | review of the reworked branch; *possibly a red gate before the fix* | records: yes | gate: yes (code change) |
| 2 | 01a11bc4-6674-707f-a6a3-b0d9e34902e4 | public ROADMAP.md, HANDOFF goal list | 10-08 13:48 | finished, not merged. *Review stage unknown* | review or gate | records: yes | gate: yes |
| 3 | 01a1193c-ca43-7171-9286-5ff44665535f | imx8mp board_show without per-frame VSYNC wait (F1/F2) + doc | 10-08 13:10 | finished, 2 unmerged commits. *Stage unknown* | review or gate (driver code) | records: yes | gate: yes (Zephyr/no_std) |
| 4 | 01a11866-5c1a-7250-ae4d-e314889f9e2c | imx8mp dEQP-GLES3 table (119 → 133 entries), gles3 check | 10-08 13:10 | finished, 2 unmerged commits. *Stage unknown* | review or gate | records: yes | gate: yes |
| 5 | 01a11a09-e3dc-7565-8cb4-af6595bf6f65 | V3D shaderc matrix attributes, Ishl/Ftrunc, es3_shaderc_v3d test | 10-08 12:48 | finished, 4 unmerged commits (an agent turn commit "ccfarm: V3D shaderc … (turn 1)" plus 3 follow-ups: test-rule and clippy fixes, a doc) | review of the follow-ups; *the clippy follow-up suggests an earlier gate failure* | records: yes | gate: yes (code) |
| 6 | 01a11afd-f67e-76dc-854d-65cb9ae89e5e | Pi 4 dEQP-GLES3 tree, image builder, exec bits | 10-08 12:47 | finished, 2 unmerged commits on base `e5eff04`. `c905ac6` is a fresh single-commit redo of the Pi 4 tree; it does **not** contain 01a11866-5412 (also unmerged; `merge-base --is-ancestor` false). The snapshot also had a resolver branch for it, 01a11ac1-d205 ("resolve conflicts: Pi 4: dEQP-GLES3 tree…"), absent on GitHub | review of the redo; *01a11866-5412 is likely superseded* | records: yes | gate: yes |
| 7 | 01a11ab1-dd1d-7473-a776-70a296935307 | pvr pool: replay terrain's NPOT mip chains | 10-08 12:27 | finished, 1 unmerged commit. *Stage unknown* | review or gate (code) | records: yes | gate: yes |
| 8 | 01a11ab1-df7f-7782-9c9a-d8215b5a8512 | docs(am62p): scan-out view investigation | 10-08 09:26 | finished, 1 unmerged commit, idle 7 h. *Stage unknown* | review or gate | records: yes | gate: yes |
| 9 | 01a119ca-708f-7040-9874-ea491427be1e | docs: es3-rest-failure-map (dEQP doc) | 10-08 08:27 | finished, 4 unmerged commits (turn commit plus fixes: "drop scratch files", "correct the none row total"); idle 8 h. *Stage unknown* | re-review; *candidate for the "12:00 quota" dEQP docs* | records: yes | gate: yes |
| 10 | 01a1194b-7f12-7095-8268-f7cc6253246d | docs: es3 shader failure map (dEQP doc) | 10-08 05:45 (nuc tip `960e6d8`); GitHub has the older `6a046ea` (02:45) | finished, newer tip only in nuc's checkout; idle 11 h. *Stage unknown* | review; *candidate for the "12:00 quota" dEQP docs* | **yes: newest commit not on GitHub** | gate: yes |
| 11 | 01a11974-b6fe-71bd-a9f3-3972ec8e3fdf | shaderc PowerVR centroid inputs, es3_shaderc_infolog test | 10-08 04:07 | finished, 1 unmerged commit; idle 12.5 h. *Stage unknown* | review or gate (code) | records: yes | gate: yes |
| 12 | 01a11989-d714-7269-8d79-06f1264220b5 | docs: es3-texture-failure-map (dEQP doc) | 10-08 03:30 | finished, 1 unmerged commit; idle 13 h. *Stage unknown* | review; *candidate for the "12:00 quota" dEQP docs* | records: yes | gate: yes |
| 13 | 01a11829-4308-7096-b6fb-ec317a4bb7d6 | docs: AM62P plan #1 board session | 10-08 02:09 (nuc tip `a4b59bc`); GitHub has a different lineage `c2b6294` (00:37) | finished; nuc's branch was re-created from `b83216d`, so GitHub's copy is a different lineage; idle 14.5 h. *Stage unknown* | review; *possibly superseded by 01a119a2 (strict review of a re-run, also unmerged)* | **yes: nuc's tip differs from GitHub's** | gate: yes |

(Rows 10, 12 and 9 are the three dEQP failure-map docs. Matching
them to the plan's "three dEQP docs waiting for 12:00 quota" is *unconfirmed*.
Item facts-health checks it.)

The plan expected 5 to 8 stuck results. Git shows 13 finished, unmerged result
branches with content not in integration. Some may be **abandoned** in ccfarm's
records, which git cannot show. That is the most likely reason the count is
higher than 5 to 8.

## 5. Safest action per result, with its risk

ccfarm's own tools are the only safe movers. None was run (fact-finding only).
Every action assumes it runs where the records are readable (nuc, or XPS after
replica-restart).

| # | agent id | safest action | risk |
|---|---|---|---|
| 1 | 01a119e0-9d60… | `get_agent_result` → if it waits on review, let it be reviewed **first**: its `3985bee` fixes the shared red gate (§3). Then `recheck_results` for the others | low: review runs the normal gate; the fix touches check.sh, so a wrong fix could hide a real "table in the artefact" regression. The reviewer should check the archive-bytes fallback (measured 3 with table, 0 without) |
| 2 | 01a11bc4-6674… | `get_agent_result`; if gate red on "no table", `recheck_results` after #1 merges | none (recheck only re-evaluates) |
| 3 | 01a1193c-ca43… | as #2; the gate must build the Zephyr images, so it needs a node with ZEPHYR_WS and the cross CC | none; may wait for system76 (only CC_aarch64_unknown_none) |
| 4 | 01a11866-5c1a… | as #3 | as #3 |
| 5 | 01a11a09-e3dc… | `get_agent_result`; if a review decision is pending and its verdict is approve, approve it; else deny with the reviewer's note | approve merges into `ccfarm/integration` (promotion to main is off, `promote_to = ""`, so main is unaffected; revertible with ccfarm's revert) |
| 6 | 01a11afd-f67e… | `get_agent_result` for 01a11afd and 01a11866-5412; let ccfarm review the redo (01a11afd) and abandon 01a11866-5412 through ccfarm if superseded, not by deleting branches | a re-merge or resolve turn costs quota; deleting branches by hand would lose work, so do not |
| 7 | 01a11ab1-dd1d… | as #2 | none |
| 8 | 01a11ab1-df7f… | as #2 | none |
| 9 | 01a119ca-708f… | as #5 (its follow-up fixes suggest it went through at least one review round) | as #5 |
| 10 | 01a1194b-7f12… | as #2, **on nuc** (the newest commit `960e6d8` is only there) | re-dispatching it elsewhere would review the older GitHub tip `6a046ea` and lose `960e6d8` |
| 11 | 01a11974-b6fe… | as #2 | none |
| 12 | 01a11989-d714… | as #2 | none |
| 13 | 01a11829-4308… | `get_agent_result` for it and for 01a119a2-7d7e ("strict review: AM62P board session (re-run where the inputs exist)", in the snapshot only). If the re-run superseded it, ccfarm abandons it with that reason | abandoning wrongly drops a board-session record; check that the re-run's doc covers images 1-7 first |

General rule: `recheck_results` is the action with no risk (it re-evaluates
only). Settling a decision and re-merging change `ccfarm/integration`. Never
merge, rebase or delete branches by hand (ccfarm's MCP instructions).

## 6. Confirmed vs unconfirmed

Confirmed:
- XPS139340 has no yakogl records (`applied_seq 0`, lag 157,714 → 182,744
  between 13:56Z and 16:34Z). Every record-reading call returns empty or "no
  agent / no plan".
- nuc: exhausted, `blocked_until` 2026-10-08T23:00Z, queued 7 (13:56Z) → 10
  (14:26Z). system76 is the only node with `CC_aarch64_unknown_none` for yakogl.
  system76 was `taking_work: false` at 14:26Z and took work again by 16:34Z.
- 5 results were merged into GitHub's integration between 14:31Z and 16:24Z
  (§2). The merge pipeline runs while nuc's quota is exhausted.
- The 13 branches in §4 are finished (no lease, an agent commit as tip) and
  their content is not in integration `334d4e6`.
- Two of them (rows 10 and 13) have their newest commits only on nuc.
- The integration gate fails `no table in the artefact` on at least one review
  host; `3985bee` (row 1) fixes it and is not merged.

Unconfirmed (needs nuc's records):
- The ccfarm stage of each of the 13: `review_state`, waiting for a review
  slot, gate red, a decision pending, merge conflict, or abandoned.
- How long each has been in that stage (git only gives the idle-since time).
- Whether the red gate in §3 is what holds them.
- Which three are the "12:00 quota" dEQP docs (rows 9, 10 and 12 are the
  likely ones).
- Whether pending review decisions exist (`list_decisions` on XPS is empty only
  for lack of records).

## 7. Reproduce

On nuc, or on XPS after the replica has applied entries:

1. `get_agent_result` for each id in §4; record `review_state` and the reason.
2. `list_decisions(state=all)`, `get_updates(since=0, peek=true)`.
3. Fill the *unconfirmed* cells in §4 and drop the rows found merged or
   abandoned.

From any machine with GitHub access (git only):

```
git fetch origin 'refs/heads/ccfarm/*:refs/remotes/gh/*'
git log --first-parent --format='%h %cI %s' refs/remotes/gh/integration
git log --oneline --cherry-mark --right-only --no-merges refs/remotes/gh/integration...refs/remotes/gh/<id>
```
