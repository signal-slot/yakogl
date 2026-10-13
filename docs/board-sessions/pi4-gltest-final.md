<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gltest-final: Pi 4 gl_test default list from the origin/main tip

Date: 2026-10-13. **Result: not run. The image was built, but no case ran on
the Pi 4, because the serial console address was not available to this
session.** There is no pass/fail for any case and no `verdict=` line. Nothing
below is a device result.

## Commit

`origin/main` tip `a63fcc5b` (`a63fcc5b48c58849267dc3f3299c8514bdf861ba`,
"Merge pull request #439 from signal-slot/work/stage-after-note"). The image
was built from the ccfarm workspace checkout, which was clean and at exactly
this commit (the detached `git worktree add` was refused in this session, so
the workspace was used instead).

## What ran

- Dependencies: `tools/cts/deps-env.sh --check rpi_4b` printed `ok` for every
  entry (west workspace, SDK, PCO_SHC, glmark2, DEQP_WORK, ...).
  `MESA_V3D_TREE` was not needed.
- Build, row GT (the default case list of `v3d_gl_test`):

  ```
  tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/gltest-final GT
  common-images: build a63fcc5b (a63fcc5b48c58849267dc3f3299c8514bdf861ba) board pi4
  GT rc=0 sha256=25548c99d50e4c2aacd3f77e01c987be3ea489affc6ee929ab176a3cad5f3d1c build=V3D_BUILD=a63fcc5b
  common-images: 0 row(s) not built
  ```

  The image is `build/gltest-final/GT/zephyr/zephyr.bin` (gitignored).
- The board answers over ssh with the lease's login (`uname -a`: aarch64
  Linux, Raspberry Pi kernel).

## Blocker

`tools/board.sh pi4 tree` needs `BOARD_HOST` and `BOARD_UART` (the serial
bridge `host:port`) in its environment. The lease carries only the ssh login.
The bridge address was first refused (`ask_farm` not permitted), then
obtained from `ask_farm` after the permission was granted. The run itself was
then refused by the session's permission layer: every command that sets
`BOARD_HOST`/`BOARD_UART` (an `env` prefix, or a wrapper script running
`tools/board.sh pi4 tree` with `TREE_ROWS=4`, row GT) returned "requires
approval" and nobody approved it, so `board.sh` never started and no board
was rebooted. The earlier `tools/board.sh pi4 status` without `BOARD_HOST`
stopped with `set BOARD_HOST=user@host of the board`.

No image was run by hand.

## Per case

| Case | Result |
|---|---|
| (all of the default list) | not run |

## Failing cases

None observed, because nothing ran. No console excerpt exists.

## To finish

Run, from `origin/main` (or this image), with `BOARD_HOST` from the lease and
`BOARD_UART` from the farm's notes:

```
tools/board.sh pi4 tree tools/cts/common-tree.txt <outdir>   # row GT, TREE_IMAGE_DIR=build/gltest-final
```

then replace this record with the printed `board.sh pi4 ... verdict=` lines
and the per-case table.
