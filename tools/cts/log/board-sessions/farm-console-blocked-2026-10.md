<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# Farm board sessions blocked by the console path (2026-10-10 to 2026-10-11)

Five ccfarm results on `ccfarm/integration` record sessions in which no
image was built and no case ran, all for one reason: the session could not
read the board's console. They are folded into this one record on the
owner's decision (2026-10-12). The full texts stay in git history on
`ccfarm/integration` at the merges named below.

**Status: closed.** The AM62P fault was the lease: it named the bridge port
wired to mcu_uart0 (Linux ttyS3), while U-Boot, the getty and Zephyr print
on main_uart0 (ttyS2). AM62P runs since 2026-10-11 use the main_uart0 port
and capture (`tools/cts/log/investigations/am62p-console-empty.md`, Status).
`tools/board.sh pi4` reads a `host:port` serial bridge since main
`f2a13877e` (`uart_is_bridge`, `console_from_ts`). The harness refusal of
`ssh` is covered by the recipe allow list in `.ccfarm/project.toml` (#56).

## The attempts

| Date | Item (ccfarm id) | Tip | What was tried | Why nothing ran | Record (integration merge) |
|---|---|---|---|---|---|
| 2026-10-11 | am62p-gles3-r2, item oq-gc (`01a12913-d2bb-7273-8c81-c39d627b45c0`) | `f013dbbb` | `echo probe \| sudo tee /dev/ttyS2` over ssh, then each tty in turn, reading the leased port after each write | Nothing from ttyS0, ttyS1, ttyS2; ttyS3 (mcu_uart0) gave 30 NUL bytes for a 9-byte write (it was at 9600 baud, the bridge reads 115200). The leased port is mcu_uart0 and can never show U-Boot or Zephyr output. | `am62p-gles3-r2.md` at `4535b161b` |
| 2026-10-11 | pi4-es2-final-b (`01a12995-4abd-73f6-b141-27dd7de8f4ff`) | `e3efeaad` | The lease gave ssh to the Pi 4 only (board idle, load 0.01) | `BOARD_UART` was the `host:port` of a TCP serial bridge on another lab host, outside the lease; `board.sh pi4` had no TCP form then (the AM62P side had one). Rows 0-28 of `pi4-full-tree.txt` unrun. | `pi4-es2-r1-tip.md` at `4451f136c` |
| 2026-10-11 | am62p-es2-tip-img1-7 and its -r2 (`01a12a30-fd0e-75ca-865a-b9cf588de092`, `01a12a4c-f9db-7301-920f-df768126e9b0`) | `89d518ee`, `64e1714e` (same code) | One CRLF on the leased channel, 8 s wait for `login:|#|=>|\w` | `matched:false`, 0 bytes: the lease gave the mcu_uart0 channel. `ssh` was refused by the harness (`Denied Bash(ssh:*) (nobody decided in time)`), so the ttyS2 probe could not run. The 9 `shaders.invariance.*.common_subexpression_*` cases and `shaders.fragdata.invalid_assign_to_1`: not determined. | `am62p-es2-tip-img1-7.md` at `e50d4ee2d` and `76d349c46` |
| 2026-10-11 | pi4-console-probe (`01a12aff-138a-71f2-8410-f8160906a73f`) | `76d349c4` | TCP connect to the bridge (python3 `-I`, 5 s timeout), 40 s read with the board idle in Linux, then `echo PROBE_MARKER_42 > /dev/ttyAMA0` over ssh with the socket open | Connection accepted, 0 bytes while idle; after the marker, 2 non-printing bytes and no marker text. Not distinguished: adapter not on the Pi's ttyAMA0 pins, a baud mismatch, or the console UART muxed away under Linux. | `pi4-console-probe.md` at `39bb67dc5` |
| 2026-10-11 | am62p console lease: owner brief (`01a129da-6ab2-75a1-a32d-b1fde7f017c9`) | n/a | The brief for the board owner: one fault, two fixes (rewire the leased channel to main_uart0, or point the `usb-serial-1` device at the port that carries it), one check | `echo probe \| sudo tee /dev/ttyS2` over ssh must show `probe` on `$BOARD_UART`. Done: the lease names the main_uart0 port now. | `am62p-console-lease.md` at `e02b6ee63` |

## What these records are evidence of

- A session that cannot read the console must say so and run nothing; all
  five did (no verdict line, no tally, no number offered in place of a
  device result).
- The same wiring fact was found four times by four sessions because the
  lease, not the repository, held the fault. The fix was on the lease side.
- The Pi 4 bridge probe is still the only measurement of that bridge path
  from the farm; a Pi 4 session from the farm proves its console by the
  dEQP image's banner over the bridge before a long run.
