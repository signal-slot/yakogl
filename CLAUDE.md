<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# CLAUDE.md

Read `CONTRIBUTING.md` (how a change reaches main) first, and the open
GitHub issues of signal-slot/yakogl: they hold all open work, the known
defects and the next board sessions. This file is the short list.

## What to run locally

Locally you run only what you added or changed (owner's decision of
2026-11-12):

- The tests you added or changed: `cargo test --test <name>` with the
  feature sets named in that test's header.
- `cargo fmt --check` on the files you touched; `cargo clippy` on the crates
  you touched, if it is quick. A docs-only change needs none of this.
- Do **not** run `tools/check.sh` (`--changed`, `--quick`, `--local` or full)
  locally. A full or `--quick` run takes from tens of minutes to over an hour
  and the pull request's CI runs the same rows. The only exception is a task
  whose stated purpose is a gate walk (for example an acceptance run of the
  gate itself).

The check is the CI gate: `.github/workflows/gate.yml` runs `tools/check.sh`
on every pull request (`--changed <merge base> --no-clippy`: the rows the diff reaches, no clippy),
on each push to main as its header says, and nightly (every row). How to read
its log:

- Green is "N ok, 0 failed, 0 gap(s)". A `gap` row (Zephyr images, hal_nxp,
  pco_shc, aarch64) is a step that could not run; it is not a pass.
- `--quick` is most of the full plan's rows; `--local <base>` is the local
  set for a diff that reaches everything (`src/gl`, `src/shaderc`,
  `tools/check.sh`, `Cargo.toml`, ...). The CI does not use them.
- Exit 2 before any row: the gate could not start (cargo, rustfmt or clippy
  missing from the toolchain). That is the runner, not the code.

A pull request's CI gate runs `--changed <merge base> --no-clippy`: no
clippy row (owner, 2026-11-13). The clippy rows run in the nightly gate on
main, which is clippy's gate. Run clippy yourself on the crates you touched
when it is quick (`cargo clippy -p <crate> --all-targets`, with the crate's
feature set from `tools/check.sh`); `tools/check.sh --no-clippy ...` leaves
the rows out locally too, counted as "clippy rows left to the nightly".

## Common failures and how to avoid them

- **Two green pull requests, red main.** Each pull request's CI checks it
  against the main it was based on. On 2026-10-14 #787 added two fields to
  `YakoglBackend` (`generate_mipmap`, `return_mipmap_chain`) while #772 and
  #782 had just added `tests/es3_caps.rs` and `tests/es3_blit_ds.rs` with
  struct literals that lacked them: main then failed to compile those tests
  in 5 rows of `--quick`. When you add a field to a table struct, grep
  `tests/` for every literal of it (`grep -rl 'YakoglBackend {' tests src`)
  and recheck against the latest main before merging.
- Bare `cargo clippy --all-targets` fails with hundreds of errors: the crate
  needs a feature set. The CI gate supplies them; locally use the feature
  sets named in the test's header.
- Clippy follows the host's nightly (no rust-toolchain file); a newer clippy
  flags old code. Fix the code; do not `allow` a lint blindly.
- `gap` rows (Zephyr images, hal_nxp, pco_shc, aarch64) need `ZEPHYR_WS`,
  `HAL_NXP_DIR`, `PCO_SHC` (`tools/setup-gate-host.sh --check`). A gap is
  not a pass; never silence it.
- `MESA_V3D_TREE` set to a Mesa tree without `build-v3d` makes the three v3d
  rows one `gap` naming `MESA_V3D_TREE` (not a pass, the gate stays red).
  Unset it to run the vendored reference build, or point it at a built tree
  (`tools/v3d/README.md`).
- Generated files (`src/pvr/internal.rs`, `src/vivante/internal.rs`,
  `tests/reference/*.rs`, `tests/fixtures/`) are regenerated in the same
  commit as the change that moves them; the gate compares the bytes.
- Open work, known defects and board to-dos are GitHub issues, never a
  file in the tree (`docs/HANDOFF.md` was retired on 2026-10-19). A result
  that finishes, changes or finds one updates the issue: a comment with the
  commit and the record, closed when done, a new issue for a new item.
- Board results replace their row in `tools/cts/RESULTS.md` (current
  results only, no run log) and the summary in `tools/cts/STATUS.md`. Do not
  add per-session or investigation records: what a session found goes into
  the commit message, RESULTS.md and the issue; a design or a finding
  that code relies on goes into `docs/notes/` (`docs/README.md`).

## Do not touch

- `third_party/` (vendored Mesa and DDK headers; sha256 in each `UPSTREAM`).
- `driver/src/v3d/generated/`, `driver/src/pvr/generated/` (generator output).
- `driver/src/fw/pvr_fw_blob.h` (gitignored, never committed).
- Board addresses, IPs, ports, UART paths, credentials: environment only
  (ccfarm env `BOARD_UART`, `ask_farm`), never in a file or a commit.
- `ccfarm/integration`: abandoned since 2026-10-12 (results land as pull
  requests to main, `0838de80`). Do not merge it into main or base work on it.
- Do not push main by hand, force-push or rewrite main or someone else's
  branch, or delete branches. Your own topic branch is rebased onto
  origin/main and pushed with `--force-with-lease` (CONTRIBUTING.md).
