<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# CLAUDE.md

Read `CONTRIBUTING.md` (how a change reaches main) and `docs/HANDOFF.md`
(state of main, open work) first. This file is the short list.

## The check to run, every time

- `tools/check.sh --changed origin/main`: the rows your diff reaches. Run it
  against the **current** main tip, right before the result is handed in,
  not against the commit you started from (see the first failure below).
- `tools/check.sh --quick`: about 120 rows, the edit loop. Not the gate.
- `tools/check.sh --local <base>`: the local set when a diff reaches
  everything (`src/gl`, `src/shaderc`, `tools/check.sh`, `Cargo.toml`, ...).
- `tools/check.sh`: every row, what the CI (`.github/workflows/gate.yml`)
  runs on each pull request's reach, on each push to main and nightly.

Green is "N ok, 0 failed, 0 gap(s)". A run takes long (one `--quick` from a
cold tree with 2 compiler slots: 59 min); run it in the background and wait
for the summary line instead of a tool timeout. Exit 2 before any row: the
gate could not start (cargo, rustfmt or clippy missing from the toolchain;
the message says what to install). That is the host, not the code.

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
  needs a feature set. Use `tools/check.sh`, which supplies them.
- Clippy follows the host's nightly (no rust-toolchain file); a newer clippy
  flags old code. Fix the code; do not `allow` a lint blindly.
- `gap` rows (Zephyr images, hal_nxp, pco_shc, aarch64) need `ZEPHYR_WS`,
  `HAL_NXP_DIR`, `PCO_SHC` (`tools/setup-gate-host.sh --check`). A gap is
  not a pass; never silence it.
- `MESA_V3D_TREE` set to a Mesa tree without `build-v3d` fails two rows:
  `env -u MESA_V3D_TREE tools/check.sh --quick`.
- Generated files (`src/pvr/internal.rs`, `src/vivante/internal.rs`,
  `tests/reference/*.rs`, `tests/fixtures/`) are regenerated in the same
  commit as the change that moves them; the gate compares the bytes.
- A result never edits `docs/HANDOFF.md`; it adds a note under
  `docs/handoff/` (`docs/handoff/README.md`). Editing HANDOFF made most
  pull requests conflict and be redone.
- Board results replace their row in `tools/cts/RESULTS.md` (current
  results only, no run log) and the summary in `tools/cts/STATUS.md`. Do not
  add per-session or investigation records: what a session found goes into
  the commit message, RESULTS.md and the handoff note; a design or a finding
  that code relies on goes into `docs/notes/` (`docs/README.md`).

## Do not touch

- `third_party/` (vendored Mesa and DDK headers; sha256 in each `UPSTREAM`).
- `driver/src/v3d/generated/`, `driver/src/pvr/generated/` (generator output).
- `driver/src/fw/pvr_fw_blob.h` (gitignored, never committed).
- Board addresses, IPs, ports, UART paths, credentials: environment only
  (ccfarm env `BOARD_UART`, `ask_farm`), never in a file or a commit.
- `ccfarm/integration`: abandoned since 2026-10-12 (results land as pull
  requests to main, `0838de80`). Do not merge it into main or base work on it.
- Do not push main by hand, force-push, rewrite history or delete branches.
