# imx8mp-gltest-final, 2026-10-14, origin/main cd738b1a

## Next board session
The i.MX8MP gl_test default list from origin/main has still not been run: the
`imx8mp-gltest-final` session built the GT image (`cd738b1a`, sha256 `c04fd86a...`) but
the worker was not allowed to start `tools/board.sh` with `BOARD_HOST`/`BOARD_BOOT` set,
see `docs/board-sessions/imx8mp-gltest-final.md`. Redo it on a worker that may run
`BOARD_HOST=<lease> BOARD_BOOT=uboot-mmc tools/board.sh imx8mp run ...`; report cases
#8 (16, run alone), #9 (10) and #25 (2) explicitly.
