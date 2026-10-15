<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Hand-over: README walkthrough findings and CONTRIBUTING N10 fixed

## State of main
`548860df`. README.md carries the ten findings of
`docs/release/readme-walkthrough.md` ("README findings", now marked fixed);
its Status tables are unchanged. CONTRIBUTING.md step 1 of "How a change
reaches main" says an untracked file outside the ignored paths widens the
`--changed` scope to all (finding N10). No script changed.

## Open work
- N9 (`MESA_V3D_TREE` without `build-v3d` fails `tools/check.sh --quick`) stays
  open here; it belongs to item gate-v3d-tree-gap.
- N8 (docs-only change "runs two rows" in CONTRIBUTING.md:16) is untouched.

## Known defects
- The corrected commands were checked by reading the sources and running
  `python3 tools/api_coverage.py` (304 names, source fallback); the cross
  build, bare-metal and compare.sh commands were not run (no toolchain file,
  no aarch64-none-elf GCC, 17 to 50 GB of disk).
- The README names two open `ignore = "BUG` pins; one (`tests/gc_npot_repeat.rs`)
  may not be listed in `docs/HANDOFF.md` known defects.

## Next board session
None needed; host-only docs change.
