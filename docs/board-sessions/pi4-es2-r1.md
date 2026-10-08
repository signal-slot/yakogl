# pi4-es2-r1: Pi 4 dEQP-GLES2 tree, rows 0-14 (BLOCKED, nothing run on the board)

Date: 2026-10-08. Integration tip 7bc37edc0e4e. Any later result of this item is
an integration image, not a landing proof.

Result: no image was built and no case ran on the Pi 4. There are no board.sh
verdict lines, no per-row tallies and no gl_test count. Rows 0-14 of
`tools/cts/pi4-full-tree.txt` are all unrun; the reason is below. No host result
is offered in place of a device result.

## Why it could not run

- This worker has none of the inputs. The deps dir
  (`/home/tasuku/.local/share/ccfarm/deps/yakogl`) holds only an empty
  `VK-GL-CTS/` (a `.git` with no checkout) and `plan4-v3d-es2-lib.txt`. There is no
  Zephyr west workspace, no Zephyr SDK 1.0.1, no dEQP stage 1 for `rpi_4b`
  (`DEQP_WORK`).
- Building them needs the network, and every such command was refused in this
  session ("This command requires approval"): `python -m pip install west`,
  `git ls-remote https://github.com/...`, `curl -I` on the SDK release.
  (Same refusal as docs/board-sessions/zephyr-ws-setup.md recorded on system76.)
- `tools/board.sh --dry-run pi4 tree ...` (with `BOARD_HOST` set inline) was also
  refused, so the runner itself could not be exercised.
- The farm's `ask_farm` was not granted either, so the node-local copies on
  other workers (see pi4-deqp-stage1.md) could not be located from here.

## What was checked

- The board answers ssh with the leased key (login `true` exit 0).
- It is idle: `uptime` showed load average 0.07 0.09 0.03, up 19 h.
- A venv was created at `deps/yakogl/zephyr-ws/.venv` (empty, no west).

## Next session

Run it on a worker that has the inputs (system76 or Lepton-local per
pi4-deqp-stage1.md: `DEQP_WORK`, `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR` listed there), or let this worker run `pip install west`,
`west init/update` and the SDK download, then follow HANDOFF 'Next board session'
-> 'Raspberry Pi 4' and tools/cts/README.md unchanged.
