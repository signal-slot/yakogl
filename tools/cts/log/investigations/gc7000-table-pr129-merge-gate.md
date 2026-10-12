# GC7000 table, PR #129 merge gate on the local merge 63940012: nothing ran

Summary: the host gate did not run on the PR #129 + main 2fe4b879 merge; no check.sh row has a result.

## Commits

- PR #129 head: `419b0159d2eab3e1b0ed7e8e376441beea4fdc6e`, tree `f9bd713431fb35251f1b5120f20a7c6e25eaa417` (present as origin/work/int-pr31 after `git fetch origin`).
- main: `2fe4b8798b5faf2807f31dd486296f21045f4abf`.
- Local merge `63940012a0aea051d3e110f3dd837d3e1a9bccb3`: not obtainable here. It is not in this clone, `git fetch origin ccfarm/01a12f0d-cec7-706f-9402-20bc63f69e6c` found no such ref, and `fetch_from_farm` of the board-merge workspace answered "no online node exports" the path.

## Re-creating the merge

Every command that writes to the repository was refused with "This command requires approval": `git checkout -b`, `git switch -c`, `git checkout --detach`, `git branch`, `git worktree add`, `git merge-tree --write-tree`, `git clone`. Read-only git commands worked. With no way to create the merge commit or a checkout of it, the merge was not made, its tree is unknown, and its equality with 63940012's tree is unestablished.

## Gate rows

No `tools/check.sh` invocation was made (the working tree is the integration tip db3545fa, not the merge). Every row is: not run.

Env names that rows will need once a checkout exists (not checked here): PCO_SHC, GLSL2USC_CACHE.
