<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-gltest-final: AM62P gl_test default list and cases 198, 197, 171 from the origin/main tip

Date: 2026-10-14. Images built from the `origin/main` tip `cd738b1a`
(`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`, "ccfarm: merge
ccfarm/01a13623-0a20-7578-bba9-a6ab4290251a (#533)"). They were built in the
ccfarm workspace checkout, which was clean and at exactly this commit (a
detached worktree was not available at the time). `board.sh` printed `build
2f8ccfda` for itself and the second image reports `PVR_SAMPLE_TREE=2f8ccfda`,
because the workspace then held this record's own commit (documents only) on
top of `cd738b1a`; the code is `cd738b1a`'s. Boot: `uboot-mmc`; the console
was captured on the bridge port wired to main_uart0 (bridge port 33).

**Result: 53 of 53 cases passed on the AM62P. Default list: 50 of 50. Cases
198, 197, 171: 3 of 3. 0 failed, 0 timed out, 0 skipped, 0 resets.**

## Build

```
tools/cts/common-images.sh am62p tools/cts/common-tree.txt build/gltest-final GT
common-images: build cd738b1a (cd738b1ae7cf99166a6bdd86cf24dac04c4c4328) board am62p
GT rc=0 sha256=e8cc385a24e930bbd85c4725983f7e2746d014bb996ba54c54cd2e6eb4bda6bd build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=cd738b1a
common-images: 0 row(s) not built
```

Cases 198, 197, 171 (image `C3`): built by hand as in `am62p-es2-r1c.md` for
case 171, with `west build` of `gl_test` for `verdin_am62p/am62p54/a53`, the
same am62p arm options (`CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`,
`CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n`) and
`-DCONFIG_GL_TEST_CASES="198,197,171"`: `rc=0`, sha256
`50d7e3244c50f2f283fd8c8c3699eeebd975dab22ed7f5bd85764154d5356f7c`.

Dependencies: `tools/cts/deps-env.sh --check am62p` printed `ok` for every
entry. The firmware blob was copied into `driver/src/fw/` (git-ignored, not
committed).

## Verdict lines (verbatim)

Run with `tools/board.sh am62p tree <table> <outdir>`, `BOARD_BOOT=uboot-mmc`,
`TREE_IMAGE_DIR` and (GT) `TREE_ROWS=4`. Default list, row GT:

```
board.sh am62p GT rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=91s
```

Cases 198, 197, 171, row C3. The first attempt did not reach the console:

```
capture: cannot reach <bridge>: timed out
board.sh am62p C3 rc=4 verdict=scan_failed boot=uboot-mmc row boot_verdict=scan_failed result=none resets=absent record=absent back_after=424s
```

(`<bridge>` stands for the serial bridge address, left out on purpose.) That
run produced no result; the bridge capture timed out and the board went back
to Linux by itself. It is not a case failure. The second attempt, with
nothing changed, printed:

```
board.sh am62p C3 rc=0 verdict=done boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all resets=0 record=absent back_after=69s
```

`record=absent`: the image prints no `yk-record:` line; the verdict comes from
the console's own end line.

## Per case

Console summaries:

```
gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 208
gl_test: all: 0 of 3 cases failed (3 passed; 0 timed out; first failure case -1 at index -1; second failure case -1; 0 skipped, first skip case -1); precompiled hits 0, compiler runs 19
```

Default list (50 `gl_test: case N name: PASS` lines, 0 other result
lines), in run order: 1 uniform, 2 texture, 3 depth, 4 cull, 5 scissor,
6 fbo, 8 blend, 9 stencil, 10 prims, 22 vao, 11 persist, 34 unishare,
12 stencil2, 23 winding, 13 zls, 40 qtstrip, 14 api, 24 compact, 58 clearscis,
59 clearmask, 60 wrapmode, 61 texincomp, 15 egl, 16 msaa, 19 es100, 20 etc,
41 es3cube, 42 es3array, 43 es3vid, 46 es3size, 47 es3inst, 48 es3vol,
49 es3facing, 45 es3dyn, 44 es3hdr, 172 swapzs, 173 splitzs, 181 fusemad,
182 fsat, 183 unipad, 188 predif, 189 mulone, 190 blendonce, 191 varyfma,
192 predlive, 197 vtxgrid, 199 es3trunc, 21 rgb565, 17 ctx2, 18 image:
**all PASS.** (Case 197 is also in the default list, so it ran in both
images.)

Requested extra cases (image C3, console `gl_test: the list 198,197,171
(3 cases) on one context`):

| Case | Name | Result |
|---|---|---|
| 198 | a2cconst | PASS |
| 197 | vtxgrid | PASS |
| 171 | viewwin | PASS |

## Failing cases

None. No failure line; `first failure case -1` in both summaries.

## How the run went

- Console bridge: the lease carried only the mcu_uart0 bridge port; the
  main_uart0 port (bridge port 33) and the board login came from
  `ask_farm`. The addresses are not recorded here.
- `board.sh am62p tree` first refused (rc 3) because the board still had a
  one-shot image staged by an earlier run that had not cleaned up (dated
  2026-10-13). `board.sh am62p disarm` removed it, which is the runner's own
  advice; then the runs above followed. A second refusal was only because the
  record directory of the refused try existed ("a record is never
  overwritten"); a fresh output directory was used.
- The board was left in Linux. Records stay under the gitignored `records/`
  of the workspace (directories `am62p-gltest-final-2` for GT and
  `am62p-gltest-final-4` for C3); they are not committed.
- `docs/investigations/am62p-console-empty.md` and
  `docs/board-sessions/zephyr-ws-setup.md` named by the task do not exist at
  `cd738b1a`; `am62p-es2-r1c.md` step 1 was followed instead.
