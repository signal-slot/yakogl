<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/mali-rerun-e2e/`

Fixtures of `tools/cts/mali-rerun-e2e-selftest.sh`.

- `mustpass.txt`: ten cases.
- `slice-a/`, `slice-b/`: first-run slices reduced to the one file
  `mali-rerun-tree.py` reads, `results.tsv`. Between them: Pass, Fail, and
  Lost with chunk-hang, chunk-crash, not-run and board-lost (`flush_finish.wait`
  is Lost in both, for different reasons).
- `scenarios/clean.tsv`: the fake board's scenario for the rerun, in which
  every case passes. The board itself is `../mali-deqp-run/`.
