<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-final-a: AM62P dEQP-GLES2 images 1-9 at origin/main `cd738b1a` (built; BLOCKED, not run)

Date: 2026-10-14. Board: Verdin AM62P, lease `usb-serial-1` (serial console
only).

## Result in short

- **Commit.** `git fetch origin` worked. The newest `origin/main` tip is
  **`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`** (2026-10-13, "ccfarm: merge
  ccfarm/01a13623-0a20-7578-bba9-a6ab4290251a (#533)"). This is the same
  commit as this item's workspace (clean, no local changes).
- **Images 1-9 were built (all `rc=0`) but none was run.** There is no
  `board.sh` verdict line and no TALLY line for this commit. No number below
  is a device result.
- The newest whole-tree device result is still
  `docs/board-sessions/am62p-es2-final-r2.md`: `63efed8e`, 17 images,
  17,125 pass / 0 fail, ms4 16 / 16. Cite that record until this item is
  re-run.

## Blocker (exact)

1. **The leased console is silent.** The MCP console client of
   `usb-serial-1` reported `connected`, offset 0, 0 bytes buffered. Two
   writes (CR LF, then CR alone) and two `serial_expect` waits (10 s on
   `login:|# |\$ |=>|\w`, 30 s on any byte) gave 0 bytes. A final
   `serial_read` from offset 0 also gave 0 bytes. This is the same result as
   the attempts in `tools/cts/log/board-sessions/farm-console-blocked-2026-10.md`,
   where the leased bridge port was the one wired to mcu_uart0. Zephyr and
   U-Boot print on main_uart0
   (`tools/cts/log/investigations/am62p-console-empty.md`). In the second
   round `ask_farm` confirmed it from the farm's resource records: the port
   leased as `usb-serial-1` is **mcu_uart0**, and the board's console
   (main_uart0) is on another port of the same bridge. The lease does not
   name that port, so this session did not use it. **Fix (farm side):**
   point `usb-serial-1` at the main_uart0 port, or add that port as the
   AM62P console resource.
2. **ssh to the board was refused.** `ssh -o BatchMode=yes verdin-am62p ...`
   returned "This command requires approval" twice, and the farm then
   denied `Bash(ssh:*)` ("nobody decided in time"). `BOARD_BOOT=uboot-mmc`
   stages the image on the eMMC over ssh, so `tools/board.sh am62p tree`
   cannot run without it (pre-check "ssh and sudo -n").
3. (Solved in the second round.) The build wrapper and `git worktree add`
   were refused at first and allowed later. The images were then built; see
   "Image build lines".

## What was checked

`tools/cts/am62p-deqp-images.sh --dry-run tools/cts/am62p-full-tree.txt
records/final-a/images cd738b1a` lists the 17 images at this commit.
Images 1-9 (rows 0-8) are this item's:

```
1 img1-cd738b1a shaders.{struct, keywords, scoping, linkage, conditionals, return, qualification_order, reserved_operators, discard, invariance, builtin_variable, texture_functions, declarations, function, algorithm, fragdata, misc, functions, constant_expressions, constants}.*
2 img2-cd738b1a shaders.{matrix, invalid_implicit_conversions}.*
3 img3-cd738b1a shaders.{swizzles, conversions}.*
4 img4-cd738b1a shaders.{indexing, preprocessor}.*
5 img5-cd738b1a shaders.{loops, random}.*
6 img6-cd738b1a shaders.operator.binary_operator.*
7 img7-cd738b1a shaders.operator.{unary_operator, common_functions, angle_and_trigonometry, geometric, exponential, float_compare, int_compare, selection, sequence, bool_compare}.*
8 img8-cd738b1a texture.{completeness, filtering, format}.*
9 img9-cd738b1a texture.{mipmap, specification, vertex, wrap}.*
```

(The dry run prints full `dEQP-GLES2.functional.` filters. They are
shortened here.) The split is the same as in final-r2, so the per-image
results can be compared one to one.

## Image build lines

Built from a detached worktree of `origin/main` at `cd738b1a`, in the
yakogl deps directory (`wt-am62p-final-cd738b1a/`). The command was
`TREE_ROWS=0-8 tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt
<deps>/am62p-final-cd738b1a-images cd738b1a`, with the environment from
`tools/cts/deps-env.sh am62p` and the firmware header copied to
`driver/src/fw/` (git-ignored). The build log line was
`build HEAD=cd738b1ae7cf99166a6bdd86cf24dac04c4c4328 status=[0 changed]`.
`build.out`:

```
img1 rc=0 bin=bb7438158eecadfc
img2 rc=0 bin=6ce69557a3895de9
img3 rc=0 bin=1f763469ac76862d
img4 rc=0 bin=feb3d2fc30c122fe
img5 rc=0 bin=88b8fefded184d63
img6 rc=0 bin=eebf683bc05f1f93
img7 rc=0 bin=56e4db97fc4f0750
img8 rc=0 bin=98516ce87d982e33
img9 rc=0 bin=edb31ef2003f6e1d
```

The images stay in that deps directory, so a re-run on this machine can use
them without a rebuild. A fresh checkout has no `driver/src/fw/` directory,
so create it before you copy the header.

## Device verdict lines and TALLY

None. No image was run.

## Failing cases

Not determined at `cd738b1a`. At `63efed8e` (final-r2) images 1-9 had
0 failures (11,028 pass, 0 NotSupported).

## Comparison with `63efed8e`

Not possible without a run.

## Commands still to run

Prerequisites: a console lease on the board's **main_uart0** port (a CR on
it must return a U-Boot prompt or a login prompt), and ssh with `sudo -n` to
the board's Linux as `BOARD_HOST`. Then, from the worktree at
`cd738b1ae7cf`:

```bash
# <script>.sh, run as: bash <script>.sh   (records/ is git-ignored)
set -euo pipefail
OUT=<deps dir>/am62p-final-cd738b1a-images   # the built images 1-9 (or rebuild, see above)
export BOARD_BOOT=uboot-mmc BOARD_HOST=<user@board> BOARD_UART=<bridge host:port of main_uart0>
tools/board.sh am62p disarm                  # remove any one-shot an earlier stopped run left
export TREE_IMAGE_DIR=$OUT TREE_TAG=cd738b1a TREE_ROWS=0-8
tools/board.sh am62p tree tools/cts/am62p-full-tree.txt records/final-a/tree
tools/board.sh am62p tally records/final-a/tree
```

Before the tree, prove the console: write a CR on the lease and see the
U-Boot or login prompt.

---

# am62p-es2-final-b: images 10-17 and ms4 (BLOCKED, not run)

Date: 2026-10-14. Commit of the workspace: `db39c3c1eda6`; the images stay
pinned to `cd738b1a`, final-a's commit.

## Result in short

- **Nothing was run on the device.** There is no `board.sh` verdict line for
  images 10-17 or for ms4, and no whole-tree total for the 17,165 cases.
  Nothing in this section is a device result. Final-a's images 1-9 are also
  unrun, so no half-tree number exists either.
- Stage-1 criterion (fail 0, no result 0, every NotSupported in an accepted
  bucket): **not decided by this item.** The newest device result stays
  `docs/board-sessions/am62p-es2-final-r2.md` (`63efed8e`: 17,125 pass /
  0 fail / 34 NotSupported at 1x, ms4 16 / 16).

## Why it stopped

- On this worker neither the board's ssh name nor the serial bridge's host
  name resolves ("Name or service not known"; `ssh -o BatchMode=yes` and
  `tools/am62p_netboot_uart.py probe` on the bridge's port 33 (main_uart0)
  both failed before connecting). The leased console tool reports
  `connected` but is the bridge's mcu_uart0 port and delivers 0 bytes, as in
  final-r2. `BOARD_BOOT=uboot-mmc` needs ssh with `sudo -n`, so
  `tools/board.sh am62p tree` cannot start.
- Cause (farm notes): the worker that ran this item cannot reach the board; it
  is reachable only from other farm nodes (this worker has no tailnet DNS).
  The board must be placed on a node that reaches it.
- An earlier attempt of this item (not kept) did run images 10 and 11 with
  the bridge's port 32 (mcu_uart0): both `verdict=empty`, 0 console bytes.
  That is a wrong-port result, not a case result.

## To finish

Needs a worker that resolves the board and the bridge, and a console on the
bridge's port 33 (main_uart0); prove it with a CR that returns a U-Boot or
login prompt. Then, from a worktree at `cd738b1a`:

```bash
export BOARD_BOOT=uboot-mmc BOARD_HOST=<user@board> BOARD_UART=<bridge host:port of main_uart0>
tools/board.sh am62p disarm
# build rows 9-16 and the ms4 image (the ms4 image into its own directory)
export TREE_IMAGE_DIR=<out> TREE_TAG=cd738b1a TREE_ROWS=9-16
tools/board.sh am62p tree tools/cts/am62p-full-tree.txt records/final-b/tree
tools/board.sh am62p tree tools/cts/am62p-ms4-tree.txt records/final-b/ms4
python3 tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles2-mustpass.txt <consoles of both halves>
```

Then check the NotSupported list against section C of
`docs/es2-cts-failures.md` plus the npot cases.

Assumptions: the farm notes say main_uart0 is the bridge's port after the leased one; the main_uart0 port is the bridge's port 33, as in final-r2 and
am62p-es2-r1c; no port was used successfully in this attempt.

## Retry on the same worker (2026-10-14)

The item was resumed after the farm reported the node-local values as set.
The worker was the same one: the board's ssh name still did not resolve
("Name or service not known"), and the leased console tool still showed
`connected` with 0 bytes buffered. No `board.sh` verdict line exists for
images 10-17 or ms4, and no 17,165-case total; the section above stands.
The item has to be run on a node that reaches the board, with the console on
port 33 (main_uart0).
