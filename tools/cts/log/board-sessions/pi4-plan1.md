<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 board session, plan #1 confirmation: not run

Status: **nothing ran.** The precondition in the task (plan #1's commits
are ancestors of origin/main; images may only be built from origin/main)
did not hold, so the board was not touched.

## Precondition check

origin/main as visible in this workspace (read-only ref copy, HEAD8
`16a53f8d`, "docs: HANDOFF as of the session's end ..."):

| commit | meaning | ancestor of origin/main |
|---|---|---|
| `ec5ba4f` | non-seamless cube (the 25 cube cases) | no (18 commits not on main) |
| `4e9397c` | fragdata invalid_assign_to_1 | no (5 commits not on main) |

Both commits are reachable only from the integration branch and the
agents' result branches (checked with `git merge-base --is-ancestor` and
`git branch --all --contains`; no `main` ref contains them).
`git fetch origin` was refused in this unattended session, so the
snapshot of origin/main may be stale; if main has since received plan
#1, re-run this item.

## Rows

| row | tally | verdict lines | expectation (HANDOFF) |
|---|---|---|---|
| D 1, FB1 4, T1 19, T3 21, TF 24 | not run | not run | sweep 30 tallies, 25 cube cases Fail to Pass, fragdata, fbo 27 |
| v3d_gl_test | not run | not run | |

No record directories exist. No 'Reading a failure' branch applies.

## To run when plan #1 is on origin/main

Follow docs/HANDOFF.md 'Next board session', then 'Raspberry Pi 4':
`tools/board.sh --dry-run` first, board idle, build from origin/main only.
