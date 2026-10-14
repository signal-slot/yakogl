<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# main-gate-es3-tests, 2026-10-15, on main 81bffb65

## State of main

- main `81bffb65` failed `tools/check.sh --quick` (116 ok, 5 failed, 0 gaps)
  since `7e8c0b37` (#787): `tests/es3_caps.rs` and `tests/es3_blit_ds.rs`
  lacked the `YakoglBackend` fields `generate_mipmap` and
  `return_mipmap_chain`. Fixed by this result (both set to `None`).
- `tools/check.sh` stops with exit 2 and names what to install when cargo,
  rustfmt or clippy is missing, instead of failing every row.
- `ccfarm/integration` (117 ahead, 775 behind main) has nothing left to
  land: `docs/investigations/main-landing-audit.md`.
