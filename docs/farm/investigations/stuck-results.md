# Stuck yakogl results (facts-stuck)

Status: **INCOMPLETE. The table of stuck results could not be produced from
XPS139340: it has no copy of yakogl's records (see below).**
Nothing was changed except this file.

## What the ccfarm tools returned (confirmed, 2026-10-08T13:56Z, run on XPS139340)

First attempt: all four read calls were refused. After a grant, the retry gave:

- `farm_status(detail=true)` worked. `generated_at` 2026-10-08T13:56:40Z.
- `list_agents` (no filter, limit 100; and states=completed, limit 300) returned
  `{"agents":[],"next_before":null}`.
- `list_decisions(state=all)` returned `shown 0, pending_total 0`.
- `get_updates` was denied ("nobody decided in time").
- After a second grant: `list_plans(state=all)` returned `{"plans":[]}`, and
  `get_agent_result` returned "no agent with id ..." for 01a11b9b-b871-7718-8b5a-3ae8342e65c6
  and 01a11a48-6b04-75b8-91a4-bc49d401177e. Both ids exist as git branches, so
  the agent store on XPS is empty. Plan #5 itself is not visible here either.

Reading of this (the empty lists are confirmed; the cause is inferred):
`farm_status.homes` says yakogl's records live on home `nuc`, `degraded: true`,
`seq 157714`, with the only follower XPS139340 at `applied_seq 0`,
`lag 157714`, `last_pull_secs null`. This worker runs on XPS139340, so it sees
no agents, no decisions, no reviews (`reviews.items: []`, `plans.plans: []`).
The agent list is empty because XPS has replicated nothing, not because there
are no results. So the stuck results cannot be listed from any machine other
than nuc until the replica works. This is itself the main finding for this item.

Other facts from `farm_status` at 13:56Z (confirmed):

- nuc: exhausted, `blocked_until` 2026-10-08T23:00Z, weekly utilisation 0.99,
  running 0, queued 7 (was 14 at 13:14Z; farm-wide queued also 7).
  five_hour `resets_at` 2026-10-07T16:30Z, `elapsed_pct` 100 (stale window).
- Lepton: exhausted until 2026-10-10T00:00Z, utilisation 0.99.
- system76: has quota (seven_day remaining 25%), running 8/8 slots
  (`concurrency_reason: load`). Only node whose yakogl env has
  `CC_aarch64_unknown_none`.
- XPS139340: quota 77% remaining, running 10/11. No `CC_aarch64_unknown_none`
  in its yakogl env; PATH has /usr/lib/llvm/22 and 23.
- Lepton-local (local model): its yakogl env lists DEQP_STAGE1, DEQP_WORK,
  GLMARK2_SRC, IMX8MP_OUT and others, but not `CC_aarch64_unknown_none`.
- `stages.missing_tools` is empty, `promotions` is empty, `pending_decisions` 0,
  `waiting` empty, `waiting_unclaimable` 0 (all as seen from XPS).
- verdin-imx8mp is leased (exclusive) by system76 for task
  01a11a48-6b04-75b8-91a4-bc49d401177e until 13:57:35Z; that is an unmerged
  candidate below.

## What git alone shows (confirmed as git facts, not as review states)

The workspace clone has read-only copies of the repository refs under
`refs/ccfarm/origin/heads/`. The integration tip there is
`7bc37edc0e4e2f473a2a4a4cc319e4be7fe106c8`.

Command:
`git for-each-ref refs/ccfarm/origin/heads/ccfarm --no-merged=refs/ccfarm/origin/heads/ccfarm/integration`

It lists many `ccfarm/<agent id>` branches that are not ancestors of the
integration tip. That does **not** mean they are stuck:

- ccfarm merges with its own merge commits and records abandonment only in
  its records.
- Branches of review agents ("ccfarm: review: ...", "strict review: ...",
  "resolve conflicts: ...") are reviewer or resolver turns, not results.
- Old branches of abandoned results stay unmerged.

So this list is a candidate set only. Candidates from the last day (newest
first; commit time is JST, +0900). Each needs a `get_agent_result` call to
learn its `review_state`.

| agent id | branch tip subject | tip commit time | safest action and risk (until the state is known) |
|---|---|---|---|
| 01a11bc4-6674-707f-a6a3-b0d9e34902e4 | Add public ROADMAP.md with the five stages | 10-08 22:48 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11b9b-b871-7718-8b5a-3ae8342e65c6 | Internal roadmap status: plans #2/#3 (turn 1) | 10-08 22:45 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11b9b-b591-7256-8d18-d1329773a7c7 | docs: CHANGELOG.md brought up to the integration tip | 10-08 22:11 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a1193c-ca43-7171-9286-5ff44665535f | docs: imx8mp-frame-floor host syntax-check evidence | 10-08 22:10 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11866-5c1a-7250-ae4d-e314889f9e2c | imx8mp gles3: 133-entry table, gles3 conformance check | 10-08 22:10 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11a09-e3dc-7565-8cb4-af6595bf6f65 | v3d chain: attribute_locations doc | 10-08 21:48 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11afd-f67e-76dc-854d-65cb9ae89e5e | tools/cts: executable bit on check-gles3-tree.py | 10-08 21:47 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11ab1-dd1d-7473-a776-70a296935307 | pvr pool: replay terrain's NPOT mip chains | 10-08 21:27 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11ab1-df7f-7782-9c9a-d8215b5a8512 | docs(am62p): scan-out view investigation | 10-08 18:26 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11a03-c215-717c-871f-85fd9c7d5998 | tests: es3_shaderc_pp: one #[test] | 10-08 17:34 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a119ca-708f-7040-9874-ea491427be1e | docs: es3-rest-failure-map | 10-08 17:27 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a119e0-9d60-7386-ada8-85bccbc131bc | gl: ES 3.0 integer colour render targets | 10-08 16:36 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a1194b-7f12-7095-8268-f7cc6253246d | docs: es3 shader failure map | 10-08 14:45 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11974-b6fe-71bd-a9f3-3972ec8e3fdf | shaderc (PowerVR): centroid fragment inputs | 10-08 13:07 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11989-d714-7269-8d79-06f1264220b5 | docs: es3-texture-failure-map | 10-08 12:30 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |
| 01a11829-4308-7096-b6fb-ec317a4bb7d6 | AM62P board session: plan #1 confirmation | 10-08 11:09 | read `get_agent_result` on nuc first; then apply the stage table below. No action is safe to pick before that: a `recheck_results` on a merged result is harmless, but settling a decision or re-merging without the verdict could merge unreviewed work into `ccfarm/integration` |

Removed from this list: 01a11a48-6b04-75b8-91a4-bc49d401177e and
01a11ab1-df5e-7058-9bec-0b619ff3a607 (both hold board leases on system76 at
14:26Z, so they are running, not finished). The agent 01a11b9b-b871-... and
01a11a48-... were also not found by `get_agent_result` on XPS (empty store).

The "three dEQP docs waiting for 12:00 quota" in the plan background are
possibly among the docs rows above. That is **unconfirmed**.

## Unconfirmed

- Which of the 15 remaining candidates are stuck, versus merged, abandoned or
  still in review. The expected count is 5 to 8 (plan text).
- The stage, the thing waited on, "since when" and evidence line of each.
- Which are blocked only by nuc or only by the single check machine. The
  facts are confirmed (see the sections above): the records live on nuc, nuc is
  blocked until 2026-10-08T23:00Z with 10 queued at 14:26Z, and system76 is the
  only node with `CC_aarch64_unknown_none`. What is unconfirmed is which result
  waits on which of them.
- Whether any review decisions are pending on nuc. `list_decisions` on XPS
  shows 0 only because XPS has no records.
- Whether the three "12:00 quota" dEQP docs are really quota-limited
  (that is item facts-health).

## Safest action per stage (generic, to apply after classification)

| stage found | safest action | risk |
|---|---|---|
| gate waiting for the check node/tool | wait, or `recheck_results` once a second check machine is registered (item checkenv-install) | none; recheck only re-evaluates |
| review waiting for a slot or quota | wait for the reset (nuc 23:00Z), or `recheck_results` | none |
| decision pending (review, not human_only) | approve if the verdict is approve; deny with a fix note otherwise | an approve merges into `ccfarm/integration`; promotion to main is off (`promote_to = ""`) |
| merge conflict | let ccfarm's conflict resolver run again (recheck) | one more resolver turn; no integration change until reviewed |
| waiting on nuc records | nothing from a worker: needs nuc back or the replica working (items facts-replica, replica-restart) | re-dispatching elsewhere would read nothing |

## Redo this item

Run it on nuc (the home of yakogl's records), or on XPS139340 after the replica
has applied entries, with `get_agent_result`, `list_plans` and `get_updates`
allowed:

1. `farm_status(detail=true)`.
2. `list_agents(states=["completed"], limit=500)`, filtered to yakogl.
3. `get_agent_result` for each id above; read `review_state`.
4. `list_decisions(state="all")`, `get_updates(since=0, peek=true)`.
5. Fill a table with: agent id, branch, plan/item, stage, waits for, since,
   evidence line, nuc-only or check-machine-only.

## Re-check at 2026-10-08T14:26Z (confirmed, still on XPS139340)

Done after the review asked for a re-run. The replica has still not moved:

- `farm_status` homes: yakogl home nuc, degraded, `seq 163115` (was 157714 at
  13:56Z), follower XPS139340 `applied_seq 0`, `lag 163115`,
  `last_pull_secs null`. The records on nuc keep growing; XPS applies nothing.
- `list_agents` (limit 50): `{"agents":[]}`. `list_plans`: `{"plans":[]}`.
- nuc: still exhausted until 2026-10-08T23:00Z, running 0, **queued 10**
  (7 at 13:56Z, 14 at 13:14Z). Its queue is not draining.
- system76 is now listed in `exhausted_nodes` and `quota_first.low_nodes`
  (`remaining_pct` 25, `taking_work` false); only XPS takes work. So the one
  machine with `CC_aarch64_unknown_none` currently takes no new work.
  Worker slots with quota: 3 of 3 busy, 10 agents waiting (bottleneck hint).
- Board leases held by system76 (so those agents are running, not finished):
  01a11a48-6b04-75b8-91a4-bc49d401177e (verdin-imx8mp) and
  01a11ab1-df5e-7058-9bec-0b619ff3a607 (verdin-am62p, usb-serial-1).
  Remove both from the candidate table once the records can be read.

## Why the per-result table is not filled

I disagree with the review only in that this cannot be fixed from this
machine: the per-result fields (stage, what it waits on, since when, evidence
line, review_state) exist only in the agent and review records on nuc. XPS holds
0 of 163,115 entries, and nuc is not reachable from a worker. Filling the table
from git refs would be a guess, which the task forbids. The table must be built
by a run on nuc, or on XPS after replica-restart. This file stays
`INCOMPLETE`, with the candidate list and the redo steps above.

## Re-check at 2026-10-08T16:34Z (confirmed, review round 4, still on XPS139340)

- `farm_status` homes: yakogl home nuc, degraded, `seq 182744`; follower
  XPS139340 `applied_seq 0`, `lag 182744`, `last_pull_secs null`. The lag grew
  by about 19,600 entries in 2 h 8 min (14:26Z to 16:34Z).
- `list_agents` (limit 20): `{"agents":[]}`.
- nuc: still exhausted (`exhausted_nodes: ["nuc","Lepton"]`), next capacity
  reset 2026-10-08T23:00Z. system76 takes work again (not in `low_nodes`;
  weekly remaining 19%). Farm: 21/21 slots busy, 5 queued.
- `ask_farm`, `get_plan` and `get_agent_status` were refused in this session,
  and `git ls-remote origin` needed approval, so there was no other route to
  the records or to a newer integration tip.

The per-result table therefore still cannot be built here. All three reviews
also say it must be built on nuc or after replica-restart.

## New routes tried at 2026-10-08T16:35Z, after more tools were granted

- `ask_farm` (project yakogl) went to XPS139340, Lepton, Lepton-local, nuc and
  system76. The result was `"answers":[]`: "No node of the farm knows this from
  its notes". So nuc did not answer from its records through this route.
- `get_plan(5)` returned "no plan with id 5", and `get_agent_status` for
  01a11974-b6fe-71bd-a9f3-3972ec8e3fdf returned "no agent with id". Same
  cause: the store on XPS is empty.
- `git ls-remote origin` (GitHub) worked. GitHub's `ccfarm/integration` is at
  `334d4e658772f466e2625b8de1d512f0fc6afd2f`, not the snapshot's `7bc37ed`.
  The commit is not in this clone; `git fetch` needed approval, so I could not
  test which candidates it contains.

Candidate branch tips on GitHub compared with the workspace snapshot (confirmed
from the `ls-remote` output; all 15 candidates still exist on GitHub):

| agent id | snapshot tip | GitHub tip | reading |
|---|---|---|---|
| 01a11b9b-b871-7718-8b5a-3ae8342e65c6 | 34e6055 | e9c748e | moved: a later turn (likely rework after review) |
| 01a119e0-9d60-7386-ada8-85bccbc131bc | b9b2ac0 | 3985bee | moved: a later turn |
| 01a1194b-7f12-7095-8268-f7cc6253246d | 960e6d8 | 6a046ea | moved: a later turn |
| 01a11829-4308-7096-b6fb-ec317a4bb7d6 | a4b59bc | c2b6294 | moved: a later turn |
| the other 11 candidates | same | same | no new turn since the snapshot |

"A later turn" is confirmed (the branch moved). That it was rework after a
review is unconfirmed. The other 11 have had no new commit, so if they are not
merged they are waiting on something outside the agent: review, gate, decision
or merge.

Not confirmed: whether GitHub's integration `334d4e6` merges any of them.
Command to check it, once allowed:
`git fetch origin refs/heads/ccfarm/integration:refs/remotes/gh/integration`
then `git branch -r --contains` / `git merge-base --is-ancestor <tip> gh/integration`
for each candidate tip.
