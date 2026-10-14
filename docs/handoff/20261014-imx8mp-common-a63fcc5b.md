# i.MX8MP common set, 2026-10-14, a63fcc5b

## State of main
i.MX8MP common set on `a63fcc5b` (2026-10-14; `tools/cts/RESULTS.md`): GM glmark2 score 43,
33 ran / 0 failed / 0 unsupported; SO, SV, CL probes pass; GT gl_test 48 of 49 run passed,
1 skipped (case 16, #8), 1 failing (case 44, #6). Replaces the glmark2 figure of 42 and the
gl_test cell "47 / 48 run" in the i.MX8MP row.

## Known defects
No change: #6 (case 44) and #8 (case 16) are still the only gl_test failure and skip on the i.MX8MP.

## Next board session
A board left a staged one-shot image (`/var/yakogl/once-*.bin`) after an interrupted run; if a
run is refused with "holds a one-shot image of a run that did not clean up", `board.sh disarm`
(or remove the file) before retrying. The cleanup of such an image on a killed run is not done by
`board.sh` itself.
