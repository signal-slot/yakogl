# pi4-es2-r1: Pi 4 dEQP-GLES2 tree, rows 0-14 (BLOCKED, nothing run on the board)

Date: 2026-10-08. Integration tip 7bc37edc0e4e. Any later result of this item is
an integration image, not a landing proof.

Result: no image was built and no case ran on the Pi 4. There are no board.sh
verdict lines, no per-row tallies and no gl_test count. Rows 0-14 of
`tools/cts/pi4-full-tree.txt` are all unrun; the reason is below. No host result
is offered in place of a device result.

## Why it could not run

- This worker has none of the inputs. The deps dir
  (`<deps>`) holds only an empty
  `VK-GL-CTS/` (a `.git` with no checkout) and `plan4-v3d-es2-lib.txt`. There is no
  Zephyr west workspace, no Zephyr SDK 1.0.1, no dEQP stage 1 for `rpi_4b`
  (`DEQP_WORK`).
- Building them needs the network, and every such command was refused in this
  session ("This command requires approval"): `python -m pip install west`,
  `git ls-remote https://github.com/...`, `curl -I` on the SDK release.
  (Same refusal as tools/cts/log/board-sessions/zephyr-ws-setup.md recorded on host A.)
- `tools/board.sh --dry-run pi4 tree ...` (with `BOARD_HOST` set inline) was also
  refused, so the runner itself could not be exercised.
- The farm's `ask_farm` was not granted either, so the node-local copies on
  other workers (see pi4-deqp-stage1.md) could not be located from here.

## What was checked

- The board answers ssh with the leased key (login `true` exit 0).
- It is idle: `uptime` showed load average 0.07 0.09 0.03, up 19 h.
- A venv was created at `deps/yakogl/zephyr-ws/.venv` (empty, no west).

## Next session

Run it on a worker that has the inputs (host A or host B per
pi4-deqp-stage1.md: `DEQP_WORK`, `ZEPHYR_WS`, `WEST_VENV`,
`ZEPHYR_SDK_INSTALL_DIR` listed there), or let this worker run `pip install west`,
`west init/update` and the SDK download, then follow HANDOFF 'Next board session'
-> 'Raspberry Pi 4' and tools/cts/README.md unchanged.

## Retry after the permission decisions (same day)

- `ask_farm` answered: the usable copies (workspace, venv, SDK, `DEQP_WORK`) are
  node-local to host A (confidence 0.6, from the farm's environment, not checked against
  af17c0c8); host B and host C have the SDK or a pinned tree but no venv or
  `DEQP_WORK`. This worker (host C) has none of them.
- `pip install` into the venv stayed denied; system python 3.14 lacks `pykwalify`
  (west's dependency), so west cannot be run from a plain git clone either.
- The deps volume has 34 GiB free (99% used), too little for a Zephyr tree, the
  SDK and a CTS stage 1 build.
- Still no image built, no board run. Rerun on host A (or give this worker
  pip access and disk).

## rows 15-28 (pi4-es2-r1b: BLOCKED, nothing run on the board)

Date: 2026-10-09. Integration tip c4a4d7fa2691 (the tip at the start of this
item). pi4-es2-r1a ran nothing, so there is no common commit with a first half.

Result: no image was built and no case ran on the Pi 4. There are no board.sh
verdict lines for rows 15-28 (T1 19, T3 21, TF 24 included), no tally, and no
verdict for the 25 cube cases (neither Pass nor Fail). The HANDOFF 'Reading a
failure' branches could not be applied. There is no whole-tree total: neither
half ran. No host result is offered in place of a device result.

Reason: the same missing inputs as above. This worker holds in the deps cache
only `VK-GL-CTS/`, `zephyr-ws/` (a bare `.venv` with pip, no `west`, no Zephyr
checkout), `sdk-dl/` and `plan4-v3d-es2-lib.txt`. There is no Zephyr SDK, no
west workspace and no dEQP stage 1 (`DEQP_WORK`), so
`tools/cts/pi4-deqp-images.sh` cannot build an image. The earlier session found
the network installs that would create them refused and the volume nearly full.
`BOARD_HDMI_TIMINGS` and the other node-local variables are unset here too.

Checked: the Pi 4 answers ssh with the leased key, is idle (load average
0.02 0.03 0.00) and runs a 6.12 aarch64 Linux kernel. Nothing was copied to it
or run on it.

Next session: run on host A (pi4-deqp-stage1.md lists its `DEQP_WORK`,
`ZEPHYR_WS`, `WEST_VENV`, `ZEPHYR_SDK_INSTALL_DIR`) at the then-current tip,
with `TREE_ROWS=15-28`. If both halves then run at one commit, add the
whole-tree total with
`tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles2-mustpass.txt`.
