# i.MX8MP common set, 2026-10-16, ab43a20c

## State of main
The i.MX8MP common set (5 rows, U-Boot from eMMC, `tools/cts/board-stage.sh imx8mp common`)
on `ab43a20c`: four rows green, GT red. GM glmark2 score 43 (33 scenes, 0 failed, 0 unsupported),
SO and SV slint probes pass (crc16 c592), CL cluster probes pass (crc16 b107). GT gl_test:
48 of 49 run passed, 1 skipped (case 16, #8), 1 failing: case 17 (`ctx2`), not case 44
(record `594b010e 00110107 ffff0030 00100031`). Case 44 (es3hdr, #6) is not among the failing
cases, so it passed in this run (a63fcc5b: 48 of 49 with case 44 failing). No resets.
Records: `records/imx8mp/common-ab43a20c/` (gitignored); `tools/cts/RESULTS.md` has the table.

## Open work
Run GT again, and case 17 alone (`SEL=17`), to tell a flake from a regression: the board has no
console, so the record carries only the case number.

## Known defects
Case 44 (es3hdr, #6) passed on i.MX8MP at `ab43a20c` (GT record `20261016-1155-GT-5e76d1d3`,
failing case 17 only). One boot is not enough to close #6 for this board: it failed on `cd738b1a`,
`f2a13877`, `a63fcc5b` and `e0eb883d`; close it after a second clean GT boot on a main that
contains the same driver code. New: case 17 (`ctx2`) failed once at `ab43a20c`; no issue yet.

## Next board session
Rerun the common set (or GT alone) on the next main that moves `src/pvr` or `driver/`; rerun
`SEL=17` and `SEL=44` alone to settle both.
