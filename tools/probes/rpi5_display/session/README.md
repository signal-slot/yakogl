# U3 board session driver

`run_u3.sh [--dry-run] <image>` runs the single Raspberry Pi 5 display session
(`driver/docs/rpi5-display.md`) in one go: undo commands printed first, a
sha256 manifest of the `/boot` files it may touch, `../linux_ref.sh` over ssh,
`tools/board.sh pi5 run` (unchanged: staging, tryboot, console capture, wait
for the PM watchdog reset), the manifest again with a byte-for-byte restore of
what differs, `../verdicts.sh`, and the record through `../record.sh`
(get_throttled before and after, redaction, check).

Environment only (never in a file): `U3_SSH` or `BOARD_HOST` (ssh target from
the lease), `BOARD_UART`. Optional: `U3_NAME`, `U3_BOOT_DIR`,
`U3_RETURN_WAIT` (420 s), `U3_POLL`.

Files touched on the board: `tryboot.txt` and `yakogl-<name>.bin` under the
firmware partition. `config.txt` and `cmdline.txt` are in the manifest only as
guards; a difference there is reported (exit 6), never written.

Exit codes: 0 done; 4 the probe's verdict is not done; 3 the board did not come
back, printed as `u3 session: board needs a person` (no retry: stop, release the
board, report); 6 restore or record check failed; 2 refused before any change;
1 other board.sh result.

Self-test: `bash tools/probes/rpi5_display/session/run_u3_selftest.sh` (fake
boot directory, fake ssh, board.sh, record.sh; no board).

Caveats: `record.sh` is called as `new <name>` (last stdout line is the
directory), `throttled <dir> <label>` (value on stdin), `redact <dir>`,
`check <dir>`. board.sh does not remove a `tryboot.txt` that did not exist
before the run; this driver does, after verifying the board is back.
