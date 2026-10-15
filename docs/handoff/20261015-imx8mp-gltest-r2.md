# i.MX8MP gl_test #9/#25 KICK_DIFF arms, 2026-10-15, e40e3835

## State of main
No change to the i.MX8MP row's figures. Add to its gl_test cell: KICK_DIFF round on
`e40e3835` (2026-10-15, image `kd-e40e3835`, 120 interleaved boots;
`docs/board-sessions/imx8mp-gltest-r2.md`): base arm case 10 failed 4 of 40, case 2 0 of 40;
KICK_DIFF arms case 10 1 of 40, case 2 1 of 40, both `stable`; 0 GPU resets.

## Open work
- `imx8mp-gltest-r2` (the #9/#25 KICK_DIFF arms) is done: the line "i.MX8MP #8 (4x MSAA) and
  #9 / #25 (cases 10 and 2): the next instruments are in Known defects" keeps #8 only for the
  instrument; #9/#25's next arm is the K1 arm (below).

## Known defects
- #9 / #25 (i.MX8MP gl_test cases 10 and 2), replace the rate and add the verdict: at
  `e40e3835`, 40 interleaved boots per arm from one image (`kd-e40e3835`), base case 10 failed
  4 of 40 and case 2 0 of 40 (the "about 1 run in 3" is no longer the measured rate: 4 of 32
  at `e3efeaad`, 4 of 40 here). The KICK_DIFF arms (`SEL=4586`, `SEL=4578`, a second read-back
  2000 us after the fence) failed 1 of 40 each (kd10-16, kd2-28), both **stable**: no word
  changed between the two reads, the case failed at check 23. 0 `late` checks in all 80
  KICK_DIFF runs. So the pixels never appeared within 2 ms: the write was lost (or more than
  2 ms late), not a fence that fired before the PE write-back. Next: the K1 arm (etnaviv flush
  rule in `emit_flushes`, `gc-kick-diff.md` "How it combines with K1"); a longer
  `KICK_DIFF_DELAY_US` arm only if "lost vs very late" must be separated.
- The KICK_DIFF Zephyr build (`gc_gl_test` with the `gl_test.c` call site) compiles
  (`rc=0`) and arms on the board; the "never been built" caveat is closed.
- `gc-kick-diff.md` corrected (both the short guide and
  `tools/cts/log/investigations/gc-kick-diff.md`): the KICK_DIFF selectors are list 1
  (`0x11ea`, `0x11e2`), not list 0; case 10 is bit 10 of the SWEEP pass bits, not bit 9; the
  sample console lines say 2000 us, not 10000 us. The "use the values in Known defects, not
  the comments in gc-kick-diff.md" caveat can go.

## Next board session
- i.MX8MP: the #9/#25 KICK_DIFF arms are done; drop that part of `imx8mp-gltest-r2` from the
  i.MX8MP section. A K1 + KICK_DIFF image needs the K1 change in `src/vivante/backend.rs`
  first; the same selectors and loop then apply.
- One boot of `tools/board.sh imx8mp run` with `BOARD_BOOT=uboot-mmc` took about 2 to 2.5
  minutes in this session, so 120 boots need about 4.5 hours; a worker whose background
  commands stop at 2 hours must split the loop and check that an interrupted boot left no
  one-shot image (`fw_printenv yako_once`, `/var/yakogl/`) before resuming.
