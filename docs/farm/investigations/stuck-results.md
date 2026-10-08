# Stuck yakogl results (facts-stuck)

Status: **INCOMPLETE. The table of stuck results could not be produced.**
Nothing was changed except this file.

## What blocked the investigation (confirmed)

Every ccfarm MCP call this item needed was refused by the permission layer
in this worker session (2026-10-08):

- `mcp__ccfarm__farm_status` (detail=true)
- `mcp__ccfarm__list_agents` (states=completed, and a plain retry with limit 5)
- `mcp__ccfarm__list_decisions` (state=all)
- `mcp__ccfarm__get_updates` (peek)

The error was "Claude requested permissions to use ... but you haven't
granted it yet". `get_agent_result`, `list_plans` and `get_plan` were never
called, because with `list_agents` refused there was no id list to feed them.

Without these calls the facts the item needs are unknown: each result's
`review_state`, its stage, and what it waits on. No agent id has been
classified as stuck, merged or abandoned from ccfarm's own records.
Everything below is a git-only substitute.

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

| agent id | branch tip subject | tip commit time |
|---|---|---|
| 01a11bc4-6674-707f-a6a3-b0d9e34902e4 | Add public ROADMAP.md with the five stages | 10-08 22:48 |
| 01a11b9b-b871-7718-8b5a-3ae8342e65c6 | Internal roadmap status: plans #2/#3 (turn 1) | 10-08 22:45 |
| 01a11b9b-b591-7256-8d18-d1329773a7c7 | docs: CHANGELOG.md brought up to the integration tip | 10-08 22:11 |
| 01a1193c-ca43-7171-9286-5ff44665535f | docs: imx8mp-frame-floor host syntax-check evidence | 10-08 22:10 |
| 01a11866-5c1a-7250-ae4d-e314889f9e2c | imx8mp gles3: 133-entry table, gles3 conformance check | 10-08 22:10 |
| 01a11a09-e3dc-7565-8cb4-af6595bf6f65 | v3d chain: attribute_locations doc | 10-08 21:48 |
| 01a11afd-f67e-76dc-854d-65cb9ae89e5e | tools/cts: executable bit on check-gles3-tree.py | 10-08 21:47 |
| 01a11ab1-df5e-7058-9bec-0b619ff3a607 | docs(board-sessions): am62p-gles3-percase (no dEQP results) | 10-08 21:45 |
| 01a11ab1-dd1d-7473-a776-70a296935307 | pvr pool: replay terrain's NPOT mip chains | 10-08 21:27 |
| 01a11a48-6b04-75b8-91a4-bc49d401177e | docs: imx8mp-gc-case10: 40 verdict lines per run | 10-08 18:28 |
| 01a11ab1-df7f-7782-9c9a-d8215b5a8512 | docs(am62p): scan-out view investigation | 10-08 18:26 |
| 01a11a03-c215-717c-871f-85fd9c7d5998 | tests: es3_shaderc_pp: one #[test] | 10-08 17:34 |
| 01a119ca-708f-7040-9874-ea491427be1e | docs: es3-rest-failure-map | 10-08 17:27 |
| 01a119e0-9d60-7386-ada8-85bccbc131bc | gl: ES 3.0 integer colour render targets | 10-08 16:36 |
| 01a1194b-7f12-7095-8268-f7cc6253246d | docs: es3 shader failure map | 10-08 14:45 |
| 01a11974-b6fe-71bd-a9f3-3972ec8e3fdf | shaderc (PowerVR): centroid fragment inputs | 10-08 13:07 |
| 01a11989-d714-7269-8d79-06f1264220b5 | docs: es3-texture-failure-map | 10-08 12:30 |
| 01a11829-4308-7096-b6fb-ec317a4bb7d6 | AM62P board session: plan #1 confirmation | 10-08 11:09 |

The "three dEQP docs waiting for 12:00 quota" in the plan background are
possibly among the docs rows above. That is **unconfirmed**.

## Unconfirmed (all of the following)

- Which of the candidates are stuck, versus merged, abandoned or still in
  review. The expected count is 5 to 8.
- The stage, the thing waited on and the "since when" of each.
- Which are blocked only by nuc (records on nuc, quota exhausted until
  2026-10-08T23:00Z, 14 queued) or only by the single check machine
  (system76 is the only node with `CC_aarch64_unknown_none`). These two
  claims come from the plan text only; I did not re-check them.
- Whether any review decisions are pending.

## Safest action per stage (generic, to apply after classification)

| stage found | safest action | risk |
|---|---|---|
| gate waiting for the check node/tool | wait, or `recheck_results` once a second check machine is registered (item checkenv-install) | none; recheck only re-evaluates |
| review waiting for a slot or quota | wait for the reset (nuc 23:00Z), or `recheck_results` | none |
| decision pending (review, not human_only) | approve if the verdict is approve; deny with a fix note otherwise | an approve merges into `ccfarm/integration`; promotion to main is off (`promote_to = ""`) |
| merge conflict | let ccfarm's conflict resolver run again (recheck) | one more resolver turn; no integration change until reviewed |
| waiting on nuc records | nothing from a worker: needs nuc back or the replica working (items facts-replica, replica-restart) | re-dispatching elsewhere would read nothing |

## Redo this item

Run it from a session where the ccfarm read tools are allowed:

1. `farm_status(detail=true)`.
2. `list_agents(states=["completed"], limit=500)`, filtered to yakogl.
3. `get_agent_result` for each id above; read `review_state`.
4. `list_decisions(state="all")`, `get_updates(since=0, peek=true)`.
5. Fill a table with: agent id, branch, plan/item, stage, waits for, since,
   evidence line, nuc-only or check-machine-only.
