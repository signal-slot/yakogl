<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# am62p-es2-final-a: AM62P dEQP-GLES2 images 1-9 at origin/main `cd738b1a` (BLOCKED, not run)

Date: 2026-10-14. Board: Verdin AM62P, lease `usb-serial-1` (serial console
only).

## Result in short

- **Commit.** `git fetch origin` worked. The newest `origin/main` tip is
  **`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`** (2026-10-13, "ccfarm: merge
  ccfarm/01a13623-0a20-7578-bba9-a6ab4290251a (#533)"). This is the same
  commit as this item's workspace (clean, no local changes).
- **No image was built and no image was run.** There is no `board.sh` verdict
  line and no TALLY line for this commit. No number below is a device result.
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
   (`tools/cts/log/investigations/am62p-console-empty.md`). A silent
   console can also mean the board is off, or is sitting in a finished
   Zephyr image that does not echo. Without ssh (point 2) these causes
   cannot be told apart from here.
2. **ssh to the board was refused.** `ssh -o BatchMode=yes verdin-am62p ...`
   returned "This command requires approval" twice. `BOARD_BOOT=uboot-mmc`
   stages the image on the eMMC over ssh, so `tools/board.sh am62p tree`
   cannot run without it (pre-check "ssh and sudo -n").
3. **The build wrapper was refused.** The deps layout is complete
   (`tools/cts/deps-env.sh --check am62p`: every entry `ok`). But the
   wrapper script that exports that environment and calls
   `tools/cts/am62p-deqp-images.sh` returned "This command requires
   approval" three times (background, foreground, relative path). So did
   `git worktree add --detach` under the deps directory. The image script
   without the environment runs only as `--dry-run` (below).
4. The farm tools `ask_farm` and `get_project_env` were not granted, so the
   lease's UART wiring and `BOARD_HOST` could not be confirmed.

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

None. No image was built.

## Device verdict lines and TALLY

None. No image was run.

## Failing cases

Not determined at `cd738b1a`. At `63efed8e` (final-r2) images 1-9 had
0 failures (11,028 pass, 0 NotSupported).

## Comparison with `63efed8e`

Not possible without a run.

## Commands still to run

Prerequisites: a lease whose console shows the board (a CR on it must
return a U-Boot prompt or a login prompt), ssh with `sudo -n` to the
board's Linux as `BOARD_HOST`, and permission to run a script from the
workspace. Then, from a checkout at `cd738b1ae7cf`:

```bash
# records/final-a/run.sh   (records/ is git-ignored)
set -euo pipefail
export CCFARM_DEPS_DIR=<deps dir>          # the yakogl deps layout
set -a; eval "$(tools/cts/deps-env.sh am62p)"; set +a
cp "$PVR_FW_BLOB" driver/src/fw/pvr_fw_blob.h       # git-ignored
OUT=records/final-a/images; mkdir -p "$OUT"
TREE_ROWS=0-8 tools/cts/am62p-deqp-images.sh tools/cts/am62p-full-tree.txt "$OUT" cd738b1a >> "$OUT/build.out"
export BOARD_BOOT=uboot-mmc BOARD_HOST=<user@board> BOARD_UART=<bridge host:port of main_uart0>
export TREE_IMAGE_DIR=$OUT TREE_TAG=cd738b1a TREE_ROWS=0-8
tools/board.sh am62p tree tools/cts/am62p-full-tree.txt records/final-a/tree
tools/board.sh am62p tally records/final-a/tree
```

Run it with `bash records/final-a/run.sh`. Before the tree, prove the
console: write a CR on the lease and see the U-Boot or login prompt. Then
`tools/board.sh am62p disarm` removes any one-shot that an earlier stopped
run left behind. The tree can start on image 1 while later images build
(it waits for each `img<N> rc=` line).
