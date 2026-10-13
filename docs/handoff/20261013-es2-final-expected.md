# es2-final-expected, 2026-10-14, main fa4751b6573f (plan #20 item es2-invariance)

## State of main

Add under "ES 2.0, per board":

- The expected reading of every image or row of the final dEQP-GLES2 trees is
  in `tools/cts/log/board-sessions/es2-final-expected.md`: one table per board
  (AM62P 17 images and the ms4 image, i.MX8MP 29 entries, Pi 4 29 rows), with
  the case count of each filter, the last recorded reading with its commit and
  record, the expected final reading (Fail 0; the NotSupported and
  QualityWarning cases only in the buckets of `docs/es2-cts-failures.md`
  section C), and the fix commit where a row changes. Each board's rows add up
  to 17,165.
- The invariance fix is in at the tip: `tests/es2_invariance.rs` reads 12
  passed, 0 failed, 0 ignored in all six feature sets (PowerVR, GC7000, V3D,
  each with and without `report-es2`).
- Whole-tree records of 2026-10-12 exist for every board and read 0 Fail:
  AM62P `63efed8e` (`docs/board-sessions/am62p-es2-final-r2.md`), i.MX8MP
  `c345ab74` (`docs/board-sessions/imx8mp-es2-final-r2.md`), Pi 4 `d56e79e5`
  (`docs/board-sessions/pi4-es2-final-r2.md`). `docs/es2-cts-failures.md` and
  `tools/cts/STATUS.md` do not carry them yet (`es2-docs-r1`).

## Open work

- `es2-invariance` (plan #20): done. The PowerVR fix (`f9688bc5`, merged by
  `e17fb6a7`) needs no further source change.
- The final sessions (`am62p-es2-final-a`, `am62p-es2-final-b`,
  `imx8mp-es2-final`, `pi4-es2-final-a`, `pi4-es2-final-b`) and ccfarm's
  post-promote `deqp-tree` stages compare their per-image or per-row counters
  with `es2-final-expected.md` and name every difference by case.

## Next board session

Compare the counters with `tools/cts/log/board-sessions/es2-final-expected.md`
(the Begun column is the basis of the board's own counter; the mustpass-list
tally is the Totals table). A row that differs from "Expected final" is news.
`flush_finish.{flush, flush_wait}` may read CompatibilityWarning (0 to 2 per
board) and is not a difference. A capture with Lost cases is a rerun.
