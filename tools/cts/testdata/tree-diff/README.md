<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/tree-diff/`

Fixtures for `tools/cts/tree-diff.py`, run by `tools/cts/tree-diff-selftest.sh`.
The cases are scaffolding under `dEQP-GLES2.functional.treediff.*`; each name
says what it exercises.

- `mustpass.txt`: the eight-case mustpass list; `functional.other.never_run`
  is outside every fixture's case filter.
- `console-a.txt`, `console-b.txt`: the `deqp-console` form, A with the Pi 4
  recorder's epoch time stamps, B with its `HH:MM:SS` ones. A regression
  (`regress_fail` Pass to Fail), a fix (`fixed_case`), a case missing from B
  (`missing_case`), one that still fails (`still_fail`), a Pass that becomes
  NotSupported (`accepted_ns`) and a result outside the mustpass.
- `statuses-a.txt`, `statuses-b.txt`: the `deqp-uart-tally.py --statuses`
  form, with `NoResult` and `Lost`. Only `accepted_ns` regresses, so with
  `accepted.txt` the pair is clean (exit 0).
- `accepted.txt`: an `--accepted` list, one glob with its reason.
- `a-qpa.txt`, `b-qpa.txt`: the `qpa` form, B with a
  `#terminateTestCaseResult Crash` and a case never closed. Named `*-qpa.txt`,
  not `*.qpa`: `cts-selftest.sh` check 16 runs `cts-refusals.py` over every
  `.qpa` under `tools/cts/testdata/` and pins its counts. The selftest copies
  `b-qpa.txt` to a scratch `c_001.qpa` for the directory check.
- bad input (exit 2): `bad-input.txt` (no per-case form), `bad-status-qpa.txt`
  (an unknown `StatusCode`), `results-tsv.txt` (the i.MX8MP selector form,
  per row: refused), `empty-mustpass.txt`.
