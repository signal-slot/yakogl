# am62p dEQP-GLES2 tree, 2026-10-16, 7e1dd598

## State of main
AM62P dEQP-GLES2, 17 images at 1x on `7e1dd598`: 17,330 pass, 0 fail, 34 not
supported, 5 QualityWarning, 2 CompatibilityWarning, 17,371 cases begun, every
case with a result (`records/am62p/deqp-7e1dd598/`). Same 34 NotSupported as
`e9901869a`; no GPU resets seen.

## Open work
None from this run.

## Known defects
None new.

## Next board session
A one-shot image left in `/var/yakogl` by an interrupted run blocks
`board.sh tree`; `board.sh am62p disarm` only cleared the U-Boot env here, the
file had to be removed by hand.
