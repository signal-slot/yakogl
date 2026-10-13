<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# pi4-gles3-baseline: Pi 4 ES 3.0 gl_test cases and the first dEQP-GLES3 baseline

Date: 2026-10-14. Integration tip `cd738b1a`
(`cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`). HEAD8 = `cd738b1a`.

**Result: blocked. No case and no image ran on the Pi 4.** There is no
verdict line, no gl_test console line and no tally in this record, and none is
offered in place of one. The blocker is one thing, below, and it is not in the
repository.

## What was done

Run from the ccfarm workspace checkout (clean, at `cd738b1a`). The detached
worktree (`git worktree add --detach`) was refused by the session's permission
layer, as it was for `pi4-gltest-final`; the workspace is a clean checkout at
exactly the tip, so the build stamp is the same.

1. Board reachable: `ssh <lease login> -- uptime` answered (up 30 min, load 0.07).
2. Dependencies: `tools/cts/deps-env.sh --check rpi_4b` printed `ok` for
   ZEPHYR_WS, WEST_VENV, HAL_NXP_DIR, ZEPHYR_SDK_INSTALL_DIR, PCO_SHC, GLSLANG,
   GLSL2USC_CACHE, GLMARK2_SRC, PVR_FW_BLOB and DEQP_WORK (the gles2 stage 1).
3. The gles3 stage 1 for `rpi_4b` already exists in the deps cache:
   `deqp-stage1-rpi_4b-gles3/work/build/modules/gles3/libdeqp-gles3-package.a`
   is present. No stage 1 build was needed.
4. Default-list `v3d_gl_test` image builds from the tip:
   ```
   tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/gltest-base GT
   common-images: build cd738b1 (cd738b1ae7cf99166a6bdd86cf24dac04c4c4328) board pi4
   GT rc=0 sha256=70f6e1ced8dda66431222f0242126cdef82e54dad63b005e8b13b6c72b8db07c build=V3D_BUILD=cd738b1
   ```
5. Case 196 `occl` alone: the Pi 4 sample's symbol is
   `CONFIG_V3D_GL_TEST_CASE` (one case alone) or `CONFIG_V3D_GL_TEST_CASES`
   (a list), not `CONFIG_GL_TEST_CASES` (that is the AM62P `gl_test`
   sample's). `common-images.sh` has no option for it and a prefix
   `EXTRA_CONF_FILE=... tools/cts/common-images.sh` was refused by the
   permission layer, so the default of `V3D_GL_TEST_CASE` in the sample's
   `Kconfig` was changed to 196 in the working tree for one build and put back
   (`git status` is clean; nothing of it is committed):
   ```
   GT rc=0 sha256=caa566e61d867d239105a0b504ab94ac79f9da040952decf78aed06a16cfdaad build=V3D_BUILD=cd738b1-dirty
   ```
   (image `build/g196/GT/zephyr/zephyr.bin`, stamp `-dirty` for that reason only).
6. `tools/board.sh --dry-run pi4 run <image> <dir> c196` on that image printed
   the whole sequence and checked the image: build `cd738b1`, footprint
   91,590,656 bytes inside the window, boot `tryboot` (row), the serial bridge
   probe and capture lines. It touched nothing.

## The blocker

```
tools/board.sh pi4 run build/g196/GT/zephyr/zephyr.bin build/rec196 c196
.../tools/uboot-netboot-lib.sh: line 70: BOARD_HOST: set BOARD_HOST=user@host of the board
```

`--dry-run` does not need `BOARD_HOST`; a real run does, and `tools/board.sh`
reads it (or `PI4_HOST`) only from the environment (`tools/board.sh` line 191).
`BOARD_HOST` was not in this session's shell. Every way to give it was refused
by the permission layer with "requires approval", with nobody to decide it:
a command prefix `BOARD_HOST=<lease login> tools/board.sh ...`, `env
BOARD_HOST=... tools/board.sh ...`, `export BOARD_HOST=...`, and the
`$RESOURCE_PI4_ELEGANCE_SSH` form ("a variable in this command can't be
checked before it runs"). The same layer refused `bash <script>`, `chmod`,
`git worktree add` and the ccfarm tools `ask_farm` / `get_project_env`, so the
`BOARD_UART` bridge address was not read either (the dry run showed that the
env has one; it is not written here).

Unblocking step: a session whose shell already exports `BOARD_HOST` (the
lease's ssh login, `RESOURCE_PI4_ELEGANCE_SSH`) and `BOARD_UART`, or whose
permission layer allows an environment prefix on `tools/board.sh` and
`tools/cts/*-images.sh`. `ccfarm`'s `.ccfarm/project.toml` should pass
`BOARD_HOST=${resource.pi4.user}@${resource.pi4.host}` into the shell for the
`pi4` target, as `docs/es3-board-recipe-pi4.md` section 1 says it does.

## Commands for the re-run (nothing below has run)

```sh
HEAD8=$(git rev-parse --short=8 HEAD)           # cd738b1a on this tip
# one image per case: set the sample's default (Kconfig, or EXTRA_CONF_FILE
# with CONFIG_V3D_GL_TEST_CASE=<n>), then
tools/cts/common-images.sh pi4 tools/cts/common-tree.txt build/g<n> GT
tools/board.sh pi4 run build/g<n>/GT/zephyr/zephyr.bin records/pi4/gl<n>-$HEAD8 c<n>
# the gles3 tree, the stage 1 existing:
export DEQP_WORK=$CCFARM_DEPS_DIR/deqp-stage1-rpi_4b-gles3/work
DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt build/deqp/pi4-gles3-$HEAD8
export BOARD_MUSTPASS=$PWD/tools/cts/gles3-mustpass.txt
TREE_IMAGE_DIR=build/deqp/pi4-gles3-$HEAD8 tools/board.sh pi4 tree tools/cts/pi4-gles3-tree.txt records/pi4/gles3-$HEAD8
tools/board.sh pi4 tally records/pi4/gles3-$HEAD8
```

Expected values: `docs/HANDOFF.md` "Next board session" (196: q0..q4
`available 1`, results 0,1,0,1,1, PASS; 202: 12/12; 203: 9/9; 204 sample half;
205). Not measured.

## Verdict lines and console lines

None. No run happened.
