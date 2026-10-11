<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4 dEQP-GLES3 stage 1 (host only): built, image A1 links

Item `stage1-pi4` of plan #19 (OpenGL ES 3.0), 2026-10-11, on a farm worker
(12 cores). Result: **the dEQP-GLES3 stage 1 for `rpi_4b` is built, and image
A1 of `tools/cts/pi4-gles3-tree.txt` links and passes the conformance check
(`--gles3`) at 61.51 % of the 256 MB window.** No board was touched and nothing
was flashed. The commands are the ones `docs/es3-board-recipe-pi4.md` sections
2 and 3 give. The source tree is the `pi4-gles3-tree` result (`eb911ac`).

`<deps>` below is the worker's `$CCFARM_DEPS_DIR`.

## Step 1: the tree, as `pi4-gles3-tree` left it

Every piece already exists at `eb911ac` and is coherent. Nothing in it was
changed except the farm section of the recipe.

| piece | state | check |
|---|---|---|
| `tools/cts/pi4-gles3-tree.txt` | 91 labelled images | `python3 tools/cts/check-gles3-tree.py tools/cts/pi4-gles3-tree.txt`: rc 0, `91 images, 42810 matched once, 0 not matched, 0 matched more than once, 0 empty images` |
| `tools/cts/check-gles3-tree.py` | takes one or more tables, `--mustpass`, `--max`, `--self-test` | `--self-test`: `0 checks failed`; `check-deqp-images.py` runs it on both GLES3 tables (AM62P 25 images, Pi 4 91 images, both 42810 once) |
| `tools/cts/pi4-deqp-images.sh` | `DEQP_MODULE=gles3` adds `-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n` and checks `--gles3` | `bash -n` rc 0; `--dry-run` prints `A1 ... [-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n]` |
| `tools/cts/pi4-gles3-images.sh` | does not exist and is not needed: the GLES3 builder is `pi4-deqp-images.sh` with `DEQP_MODULE=gles3`, as on the AM62P | - |
| `docs/es3-board-recipe-pi4.md` | stage 1, images, run, tally, re-split, ES 2.0 rows | this item added "On the farm: `pi4-elegance`" (the exact `tools/board.sh` lines and verdicts) |

## Environment

The values of `docs/es3-zephyr-env.md` and `tools/cts/log/es3-host-toolchain.md`,
all inside `<deps>`:

| name | value |
|---|---|
| `ZEPHYR_WS` | `<deps>/zephyr-ws` (Zephyr `af17c0c8`) |
| `WEST_VENV` | `<deps>/zephyr-ws/.venv` (west, cmake 3.31) |
| `ZEPHYR_SDK_INSTALL_DIR`, `ZEPHYR_SDK` | `<deps>/zephyr-sdk-1.0.1` (gcc 14.3.0) |
| `CC_aarch64_unknown_none` | `<deps>/zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc` |
| `CTS` | `<deps>/VK-GL-CTS` at `93bca01861b0e3ef3c387027a9791e6d065f900c` |
| `RUSTUP_TOOLCHAIN` | `nightly` |
| `CMAKE_POLICY_VERSION_MINIMUM` | `3.5` |
| `PATH` | `$WEST_VENV/bin:$HOME/.cargo/bin:$PATH` (the venv's cmake configures CTS's zlib; cargo needs `rustc` beside it) |

## Stage 1

```sh
export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh "$ZEPHYR_WS")" \
    WORK=<deps>/deqp-stage1-rpi_4b-gles3/work BOARD=rpi_4b MODULE=gles3 \
    tools/cts/cts-build-zephyr.sh
```

- rc 0 in 203 s, cold, 453 ninja steps. The last line was
  `cts-build-zephyr: libraries in <deps>/deqp-stage1-rpi_4b-gles3/work/build`.
- `work/` is 2.0 GB: `build/` 101 MB, `flags/` 164 MB, and the rest is the
  `--shared` CTS clone `cts/` at `93bca018` with both yakogl patches and the
  GLES3 direct link appended.
- `work/build/modules/gles3/libdeqp-gles3-package.a` is 133,032 B. This is
  the file the image builder checks for.
  `work/build/modules/gles3/functional/libdeqp-gles3-functional.a` is
  12,588,308 B.
- The log is `<deps>/deqp-stage1-rpi_4b-gles3/logs/stage1.log`. The only
  line containing `error` is the source name `pngerror.c`.

This stage 1 is the gles3 one. The gles2 stage 1 stays at
`<deps>/deqp-stage1-rpi_4b/work` (`DEQP_WORK`), and the ES 2.0 regression rows
still need it.

## Image A1 (link proof)

```sh
export DEQP_WORK=<deps>/deqp-stage1-rpi_4b-gles3/work
DEQP_MODULE=gles3 tools/cts/pi4-deqp-images.sh tools/cts/pi4-gles3-tree.txt <deps>/deqp-stage1-rpi_4b-gles3/img A1
```

- The builder printed `A1 rc=0 sha256=9a60e3fe376e9fbd2543c6051598997dda5e89b69df37d72dbb70c9107fa82f8 build=unknown`
  and took 57 s, cold.
- The end of `A1.log`:

  ```
               RAM:     161256 KB       256 MB     61.51%
  check-deqp-images: the dEQP image recipes build what conformance needs
  ```

- `zephyr.bin` is 15,976,048 B and `zephyr.elf` is 37,322,736 B. The build
  directory is 313 MB.
- The resolved `.config` has `# CONFIG_DEQP_REPORT_ES2 is not set` and
  `# CONFIG_YAKOGL_REPORT_ES2 is not set`, so the context reports ES 3.0.
  `CONFIG_DEQP_CASE` is A1's filter.
- Every ES 3.0 entry point that dEQP's table names resolves. There was no
  undefined reference.
- Size: the gles2 image D linked at 59.30 % (155,440 KB). The gles3 A1 is
  2.2 points larger, so the 91 images fit the window (the sizes differ by
  filter only, and the libraries and test data are the same).

A `tools/board.sh --dry-run pi4 tree tools/cts/pi4-gles3-tree.txt <rec>` with
`TREE_IMAGE_DIR=<deps>/deqp-stage1-rpi_4b-gles3/img TREE_ROWS=0` and a
placeholder board ran A1's full sequence and ended `=== A1 rc=0`. Its output
included `tree: 91 rows`, `footprint 165126144` and the tryboot staging. The
dry run accepts the footprint.

The first build showed `build=unknown` because its tree was a copy of
`eb911ac` without `.git`. I rebuilt A1 from a `git worktree` at `eb911ac`
into `<deps>/deqp-stage1-rpi_4b-gles3/img-eb911ac`. That build took 21 s,
warm (ccache), and gave:

```
pi4-deqp-images: build from eb911ac (eb911acb93af598c286996d5baa49795c7aa7b42)
A1 rc=0 sha256=9a37f65153fc1e75dfa1348be08b21b5953c27ba57f62ed16d101daf24cd731c build=eb911ac
```

Its RAM line (61.51 %), its size and its verdict are the same as the first
build's. Only the embedded build string, and with it the hash, changed.
`board-pi4` should still build its images in its own checkout, so that
`build=` names the commit under test.

## Checks

- `python3 tools/cts/check-gles3-tree.py tools/cts/pi4-gles3-tree.txt`: rc 0,
  42,810 matched once.
- `python3 tools/cts/check-deqp-images.py`: passed (both GLES3 tables).
- `bash -n`: rc 0 for `tools/cts/pi4-deqp-images.sh`,
  `tools/cts/cts-build-zephyr.sh` and `tools/board.sh`.

## Artefacts (kept)

| path | what |
|---|---|
| `<deps>/deqp-stage1-rpi_4b-gles3/work` | `DEQP_WORK` for `DEQP_MODULE=gles3` (proposed farm name `DEQP_WORK_GLES3`) |
| `<deps>/deqp-stage1-rpi_4b-gles3/img-eb911ac/A1` | the link-proof image, `build=eb911ac` |
| `<deps>/deqp-stage1-rpi_4b-gles3/img/A1` | the first build of A1, `build=unknown` |
| `<deps>/deqp-stage1-rpi_4b-gles3/logs/` | `stage1.log`, `images.log` |
