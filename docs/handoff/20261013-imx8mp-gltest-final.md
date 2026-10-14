# imx8mp-gltest-final, 2026-10-14, origin/main e0eb883d

## Next board session
The i.MX8MP gl_test default list from origin/main has still not been run
(`docs/board-sessions/imx8mp-gltest-final.md`). Two things blocked the session: the worker
could not start `tools/board.sh` with `BOARD_HOST`/`BOARD_BOOT` set, and the GT image
would not build there because the sandbox's `/tmp` quota ran out while compiling
`gl_test.c` (`common-images.sh` cannot pass `-pipe`; a case-16 image built with
`imx8mp-gltest-build.sh ... -DEXTRA_CFLAGS=-pipe` did). Redo it on a worker that may run
`BOARD_HOST=<lease> BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run ...` and can write
enough to `/tmp`; report cases #8 (16, run alone), #9 (10) and #25 (2) explicitly.
