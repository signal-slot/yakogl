<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP benchmark baseline (before plan #2): NOT MEASURED

Status: **no row was built or run; there is no benchmark number.** No host-only
number is offered in place of a board result.

- Date: 2026-10-08. Board: verdin-imx8mp (Verdin i.MX8M Plus, GC7000UL),
  reached as `torizon@verdin-imx8mp`; `ssh ... uname -a` answered
  `Linux verdin-imx8mp-08600488 6.6.142-7.7.0 #1-Torizon SMP PREEMPT ... aarch64`
  (the board is up and the farm key works).
- Commit meant to be built: origin/main. `git fetch origin` was refused
  ("This command requires approval"), so only the ref copy
  `refs/ccfarm/origin/remotes/origin/main` = `16a53f8dedecde8bdd7519c84d48354b0fcefd50`
  is known; the session's checkout is the integration tip `98ab5fae1470`.
  Nothing was built from either.

## Rows

| Row | Verdict / tally line |
|-----|----------------------|
| GM glmark2 | not run: `tools/cts/common-images.sh --dry-run imx8mp tools/cts/common-tree.txt img` -> `This command requires approval` (refused on every retry, also as `bash tools/...`) |
| SO slint | not run: same error |
| SV slint-view | not run: same error |
| CL cluster | not run: same error |
| GT gc_gl_test | not run: same error |

glmark2 per-scene scores and the final score: none (GM did not run).

## What the session found (build inputs, deps cache `/home/tasuku/.local/share/ccfarm/deps/yakogl`)

- `zephyr-ws/` holds a west workspace (`.west/config`: manifest `yakogl/west.yml`,
  `zephyr/`, `modules/hal/nxp`, `modules/hal/ti`, `modules/fs/fatfs`,
  `modules/lib/picolibc`), `.venv/bin/west` and a `build-proof/imx8mp` build
  directory, apparently from the zephyr-ws-setup item (in review). `west list`
  was not run (refused), so the zephyr pin `af17c0c8` was not verified.
- `zephyr-sdk-1.0.1/` holds `gnu/aarch64-zephyr-elf`, `cmake`, `hosttools`.
- `glmark2-src/` is an **empty** directory; `git clone git@github.com:glmark2/glmark2.git`
  into the deps cache was refused too. The pin is `22c527cb0556f3a1ac4445aaa52cc532760928d5`.
- `femtovg-0.26.0/` exists; no Slint no_std tree, no NOSTD_PATCH_DIR, no
  agl-slint-cluster clone, no Noto Sans were found.
- Every Bash command that is not read-only (`git fetch`, `git clone`, the repo
  scripts, `west list`, even `bash -c 'echo hi'`) was refused with "This command
  requires approval" in this session.

## To do on a re-run with commands permitted

1. Obtain GLMARK2_SRC (clone, checkout `22c527cb`), SLINT_NOSTD_DIR (slint
   `1007c032` + `slint-nostd.patch`) and NOSTD_PATCH_DIR (femtovg/glow patches),
   AGL_SLINT_CLUSTER_DIR, NOTO_SANS_DIR as `driver/samples/zephyr/slint/nostd/README.md` says.
2. `tools/cts/common-images.sh imx8mp tools/cts/common-tree.txt <img-dir>`
   with ZEPHYR_WS, WEST_VENV, ZEPHYR_SDK_INSTALL_DIR, HAL_NXP_DIR set.
3. `tools/cts/cts-common.sh imx8mp run` / `tools/board.sh tree`, then
   `tools/cts/common-tally.py`, and quote the verdict lines here.
