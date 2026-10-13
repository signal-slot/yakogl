<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-gltest-final: AM62P gl_test default list and cases 198, 197, 171 (blocked, not run)

Date: 2026-10-14. Built from the `origin/main` tip `cd738b1a`
(`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`, "ccfarm: merge
ccfarm/01a13623-0a20-7578-bba9-a6ab4290251a (#533)"), in the ccfarm workspace
checkout, which was clean and at exactly this commit (the `git worktree add`
under `/tmp` was refused by the session's permission layer).

**Result: no device result. The board was not run.** No `board.sh am62p ...
verdict=` line was printed in this session, so none is quoted. Nothing below
is a pass or a fail of any gl_test case.

## What was done

Dependencies: `tools/cts/deps-env.sh --check am62p` printed `ok` for every
entry (Zephyr workspace, west venv, SDK 1.0.1, `PCO_SHC`, glslang,
`GLSL2USC_CACHE`, `PVR_FW_BLOB`, `DEQP_WORK`). The firmware blob was copied
into `driver/src/fw/` (git-ignored, not committed).

Built, default list (row GT):

```
tools/cts/common-images.sh am62p tools/cts/common-tree.txt build/gltest-final GT
common-images: build cd738b1a (cd738b1ae7cf99166a6bdd86cf24dac04c4c4328) board am62p
GT rc=0 sha256=e8cc385a24e930bbd85c4725983f7e2746d014bb996ba54c54cd2e6eb4bda6bd build=PVR_SAMPLE_TABLE=c95547a1,PVR_SAMPLE_TREE=cd738b1a
common-images: 0 row(s) not built
```

## Blockers

1. **Board login and serial bridge unknown.** `tools/board.sh` needs
   `BOARD_HOST` (the board's ssh login) and `BOARD_UART` (host:port of the
   serial bridge, port 33 = main_uart0). The lease text carries only the MCP
   serial console (`usb-serial-1`), not these two values. The calls that could
   have found them were refused: `mcp__ccfarm__ask_farm` (not granted) and
   reading the environment (`printenv`). Without them `board.sh am62p tree`
   cannot start, and the wrong guess (the MCP console, which is not
   proven to be port 33) could put the capture on mcu_uart0, which stays empty
   (see `docs/board-sessions/am62p-es2-r1c.md`, step 1).
2. **Image for cases 198, 197, 171 not built.** The `west build` of
   `gl_test` with `-DCONFIG_GL_TEST_CASES="198,197,171"` (r1c's recipe for
   case 171) was refused twice by the permission layer, as a script run from
   the workspace's gitignored `build/`.
3. `docs/investigations/am62p-console-empty.md` and
   `docs/board-sessions/zephyr-ws-setup.md`, named by the task, do not exist
   in this checkout at the `cd738b1a` tip.

## Per case

Not run. Default-list cases (50 in `am62p-es2-r1c.md` at `40dd2107`) and cases
198, 197, 171: no result in this session.

## Failing cases

None observed, because nothing ran.

## To finish

Rerun with `BOARD_HOST`, `BOARD_UART` (port 33) and `BOARD_BOOT=uboot-mmc`
in a wrapper outside the repository, the GT image above, and the 198,197,171
image built as r1c built case 171. Then run `tools/board.sh am62p tree` for
each, and record the verdict lines.
