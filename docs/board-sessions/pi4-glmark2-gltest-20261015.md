<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-glmark2-gltest: Pi 4 glmark2 (GM) and v3d_gl_test, blocked

Date: 2026-10-15. Tip of `origin/main` after `git fetch origin`: `aed962e2`
(`aed962e2c2a9d519a6afcb4de83ca409319e0235`, "ccfarm: merge
ccfarm/01a13ec8-c443-7094-8c94-50ccf55dcfa4 (#886)"). **Nothing ran on the
board, and no image was built.** This file has no verdict lines, no glmark2
score and no gl_test result. Every item of the session is still open:
"Waiting for a board" in `docs/HANDOFF.md` is unchanged.

## Blocker

1. **No ssh to the Pi 4.** The session's lease named one resource, `usb1`
   (target `rpi4`), reachable by ssh as `usb1`. `ssh usb1 -- <command>` was
   tried three times (`hostname; uname -a; ...`, `true`, `uptime`). Each time:
   `ssh: connect to host usb1 port 22: Connection refused`, exit 255. Zero
   bytes came back. `tools/cts/STATUS.md` calls `usb1` the node of the TCP
   serial bridge, so `usb1` is the console bridge and not the Pi's Linux.
   The lease gave no other host. The board's Linux login (`BOARD_HOST` for
   `tools/board.sh`) was not available, so `board.sh pi4 run` could not
   start: it needs ssh and `sudo -n` on the board to stage the image and
   `tryboot.txt`.
2. **No console address.** `BOARD_UART` (the bridge's `host:port`) had no
   value on the worker, and ccfarm's workspace text said so. The last session
   that read the bridge got the port from the farm's notes (`ask_farm`,
   `pi4-console-probe.md`). Here the request to `ask_farm` was refused by
   the permission layer, and so was `get_project_env`. The port was not
   guessed and no port was probed. Bytes read from the bridge: 0 (no
   connection was made).
3. **No image worktree.** `git worktree add --detach <sweep> origin/main` was
   refused by the permission layer, three times with three different
   target paths. No GM glmark2 image and no `v3d_gl_test` image was built
   from `aed962e2`. With 1 and 2 also in the way, the images could not have
   run anyway.

### Retry once the permissions were granted (10:03 to 10:10 UTC)

- `ask_farm` was allowed this time. The farm's notes give the Pi 4's Linux
  login as the resource `pi4-elegance` (`tailscale_direct`), and the console
  as a TCP bridge port on the `usb1` node. One node's notes have the bridge
  port; the other two do not. The lease was now on `pi4-elegance`
  (exclusive).
- **ssh to the Pi's Linux login: `Connection timed out` on port 22,
  three times** (10:03, and twice more within the next few minutes).
  Exit 255, 0 bytes. One node's access record already calls `pi4-elegance`
  unreachable from this worker.
- **The console bridge, from the farm's notes: accepted.**
  `python3 -I tools/am62p_netboot_uart.py probe <bridge>` exited 0, so the
  bridge is not busy and did not close. A 20 s read with the socket open:
  **0 bytes**. That is expected while the board idles in Linux
  (`pi4-console-probe.md`), but with no ssh the marker test
  (`echo ... > /dev/serial0`) could not be made. Whether the board is up at
  all is unknown.
- `git worktree add` was refused again. With the board unreachable, no
  image was built by any other route.

So the blocker is the same: **the Pi 4's Linux does not answer ssh from
this worker.** `board.sh pi4 run` needs it for the lock pre-checks, staging
and `tryboot`.

Board state: not touched. No image or `tryboot.txt` was staged, no reboot
was made and the board lock was never taken, so nothing needs cleaning up.
Whether an earlier run left files on the boot partition was not checked,
because there was no ssh.

Dependencies on the worker (`tools/cts/deps-env.sh --check rpi_4b`): every
entry `ok` except `DEQP_WORK`, which this session does not need (no dEQP
image).

## Still to run (unchanged)

- GM glmark2 (`tools/cts/common-tree.txt`, `driver/samples/zephyr/glmark2`
  with `rpi_4b.conf`, 96 MiB carve-out). Expected: 33 scenes, 0 failed;
  `refract` not refused (#4); `terrain` not black (#1), with the
  incomplete-texture-draw and GL-error counters (#13) at 0 for every scene;
  no refused allocation on the memory lines (#5).
- `v3d_gl_test`: the default list (12 / 12 as on `1cf946a9`), then 196
  `occl`, 202 `blitconv`, 203 `tfcap`, 204 `srgb`, 205 `mappbo` one at a
  time, and 199 to 201 where the V3D runs them.

What the next session needs: a worker that reaches the Pi 4's Linux login
over ssh (or someone to check that the board is powered and on the
tailnet), `BOARD_UART` set on that worker to the Pi's bridge (not the
AM62P's), and permission for `git worktree add`.
