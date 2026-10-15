<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 stage 1 and image A1 (item stage1-pi4)

Host only. Nothing was run on or sent to a board.

**Status: built on 2026-10-14 from main `cd738b1` on the ccfarm worker node.** Stage 1 and image A1
were built from scratch in this session (both rc=0). The artefacts live in that node's
`$CCFARM_DEPS_DIR`. They are not in the repository and
not on other nodes; on another machine run the commands below to make them. Only A1 was built: that is
enough to prove the GLES3 link works. The other 90 images were not built.

## Step 1: the tree, the script and the recipe

All four pieces are on main; no change was needed.

```sh
python3 tools/cts/check-gles3-tree.py tools/cts/pi4-gles3-tree.txt
# check-gles3-tree: tools/cts/pi4-gles3-tree.txt: 91 images, 42810 matched once,
#   0 not matched, 0 matched more than once, 0 empty images      (exit 0)
bash -n tools/cts/pi4-deqp-images.sh                              # exit 0
```

- `tools/cts/pi4-deqp-images.sh` with `DEQP_MODULE=gles3` adds
  `-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n` to every image and runs
  the post-build check with `--gles3`. Unset, nothing changes (ES 2.0 stays the default).
- `docs/es3-board-recipe-pi4.md` has the `tools/board.sh pi4 tree` command lines for `pi4-elegance`
  and the verdict lines to expect.
- There is no separate `tools/cts/pi4-gles3-images.sh`: the GLES3 support is inside
  `pi4-deqp-images.sh`, as for the AM62P.

## Step 2: environment

`docs/es3-zephyr-env.md` and `docs/es3-host-toolchain.md` are not on main. The values come from the
ccfarm environment of the node (`D` = `$CCFARM_DEPS_DIR`):

| variable | value |
|---|---|
| `ZEPHYR_WS` | `D/zephyr-ws` |
| `WEST_VENV` | `D/zephyr-ws/.venv` |
| `ZEPHYR_SDK_INSTALL_DIR` | `D/zephyr-sdk-1.0.1` |
| `CC_aarch64_unknown_none` | `D/zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc` |
| `CTS` | `D/VK_GL_CTS` (underscores), VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c`, `external/fetch_sources.py` already run |
| Rust | `RUSTUP_TOOLCHAIN=nightly` |

If the CTS is missing on a node:

```sh
git clone https://github.com/KhronosGroup/VK-GL-CTS.git "$CCFARM_DEPS_DIR/VK_GL_CTS"
git -C "$CCFARM_DEPS_DIR/VK_GL_CTS" checkout 93bca01861b0e3ef3c387027a9791e6d065f900c
python3 "$CCFARM_DEPS_DIR/VK_GL_CTS/external/fetch_sources.py"
```

## Commands (run from the repository root)

```sh
D=$CCFARM_DEPS_DIR
S=$D/deqp-stage1-rpi_4b-gles3          # must not exist yet: a clean build
mkdir -p $S/logs
export PATH=$D/zephyr-ws/.venv/bin:$HOME/.cargo/bin:$PATH RUSTUP_TOOLCHAIN=nightly
export ZEPHYR_WS=$D/zephyr-ws ZEPHYR_BASE=$D/zephyr-ws/zephyr WEST_VENV=$D/zephyr-ws/.venv
export ZEPHYR_SDK_INSTALL_DIR=$D/zephyr-sdk-1.0.1
export CC_aarch64_unknown_none=$D/zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc
export CMAKE_POLICY_VERSION_MINIMUM=3.5 CTS=$D/VK_GL_CTS

# stage 1: the dEQP libraries, MODULE=gles3, board rpi_4b
ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")" \
    WORK=$S/work BOARD=rpi_4b MODULE=gles3 \
    tools/cts/cts-build-zephyr.sh > $S/logs/stage1.log 2>&1

# one image from the tree: A1
DEQP_WORK=$S/work DEQP_MODULE=gles3 \
    tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt $S/img A1 > $S/logs/images.log 2>&1
```

Timing on the worker node (64 threads): stage 1 02:51:46 to 02:53:42, A1 02:53:42 to 02:54:41 (JST).

## Result (paths under `D`, read with `ls -l` and `sha256sum` after the build)

| artefact | path | size |
|---|---|---|
| stage 1 log (453 of 453 ninja steps, rc=0) | `deqp-stage1-rpi_4b-gles3/logs/stage1.log` | 1,444,777 B |
| `libdeqp-gles3-functional.a` | `deqp-stage1-rpi_4b-gles3/work/build/modules/gles3/functional/` | 12,588,308 B |
| `libdeqp-gles3-package.a` | `deqp-stage1-rpi_4b-gles3/work/build/modules/gles3/` | 133,032 B |
| A1 build log | `deqp-stage1-rpi_4b-gles3/img/A1.log` | 28,945 B |
| A1 `zephyr.bin` | `deqp-stage1-rpi_4b-gles3/img/A1/zephyr/zephyr.bin` | 16,000,984 B |
| A1 `zephyr.elf` | `deqp-stage1-rpi_4b-gles3/img/A1/zephyr/zephyr.elf` | 37,817,464 B |

`img/A1.built`:

```
A1 rc=0 sha256=cc58ac6ded82c8d5f1aff7124faefbc20713c45c2b4e658217f7286e1bc6bb0f build=cd738b1
```

`logs/images.log` starts with `pi4-deqp-images: build from cd738b1` and
`DEQP_MODULE=gles3 (-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n)`.
Zephyr reports `RAM: 161288 KB of 256 MB (61.53%)` for A1. Its `.config` has
`# CONFIG_DEQP_REPORT_ES2 is not set`, `# CONFIG_YAKOGL_REPORT_ES2 is not set` and the A1 filter of
`tools/cts/pi4-gles3-tree.txt` (`dEQP-GLES3.functional.attribute_location.*,...,dEQP-GLES3.functional.prerequisite.*`),
so it is an ES 3.0 image. `check-deqp-images.py --gles3` passed on it.

An earlier build of the same directory (from `a7575a3`, by an earlier attempt of this item) was moved to
`D/deqp-stage1-rpi_4b-gles3.prev-a7575a3` before this clean build; it can be deleted.

## Failures

None in the build. Not done: the other 90 images, and any board run (no board access in this item).
