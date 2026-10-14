# imx8mp-gltest-final, 2026-10-14, origin/main e0eb883d

## State of main
i.MX8MP gl_test default list at `e0eb883d` (two boots, identical): 48 of 49 run
passed, case 44 (#6) fails, case 16 skipped (`docs/board-sessions/imx8mp-gltest-final.md`).
Case 16 (#8) passes when run alone (`SEL=16`); cases 10 (#9) and 2 (#25) passed on both
boots. No GPU resets.

## Known defects
- gl_test case 44 (#6) fails on the i.MX8MP default list at `e0eb883d`
  (`failing 1: first case 44`); the same on both boots.

## Next board session
`common-images.sh` cannot pass `-pipe`, and the worker's `/tmp` quota stopped the
compile of `gl_test.c`; on a worker with enough `/tmp` the GT image builds without the
workaround. Cases 10 and 2 are flaky (`docs/investigations/gc-kick-diff.md`): more boots
are needed before calling them fixed.
