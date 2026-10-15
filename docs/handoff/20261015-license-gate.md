<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pub-license-gate hand-over (2026-10-15)

Does not edit `docs/HANDOFF.md`; fold it in when the owner next consolidates.

## State of main
- `tools/cts/cts-selftest.sh` has three new checks, run by `tools/check.sh`
  through its existing cts-selftest step (`tools/check.sh` is unchanged):
  - 23: `tools/cts/tree-diff-selftest.sh` (17 checks pass at the tip);
  - 24: `python3 -I tools/license_audit.py --self-test` (2 fixture checks);
  - 25: the audit in report mode over the repository. Fails on a crash or
    `unknown-id` > 0; prints `missing-spdx` (597 at `d2a07a33`).
- Broken fixtures (an expected finding renamed; a bad line in the tree-diff
  mustpass), tried locally and not committed, make checks 24 and 23 print FAIL.

## Open work
- Owner decision: headers or allowlist rules for the ~597 missing-spdx files
  (`docs/release/license-audit.md`). Then switch check 25 to `--strict` or a
  baseline.

## Known defects
- `docs/release/license-audit.md` tables still carry the `79c73ea` figures
  (587 missing-spdx); the tip is 597. Re-measure when the decision is taken.
- Check 25 greps the report's `unknown-id:` / `missing-spdx:` lines; a change
  of the report format makes it print FAIL ("no ... line"), not pass silently.

## Next board session
None needed; host-only.
