# AM62P dEQP stage 1 (items am62p-deqp-stage1, stage1-am62p)

Status: **not built; blocked on command approval.** Every attempt to run the
build (`sh <script>` and individual `git -C`, `ls` with variables or pipes) was
answered "This command requires approval" and nobody could approve it in the
unattended run. No stage 1 output exists, so there is no link result, no
duration, no size and no missing-symbol list. Nothing was sent to a board.

## Inputs found in the deps cache (2026-10-08, worker system76)

`CCFARM_DEPS_DIR=/home/tasuku/.local/share/ccfarm/deps/yakogl` now holds:

- `VK-GL-CTS` (task statement: pinned `93bca01`; not re-read here, `git -C`
  was refused),
- `zephyr-ws/{zephyr,modules,yakogl,.venv}` with `west` in `.venv/bin`,
- `zephyr-sdk-1.0.1/gnu/aarch64-zephyr-elf`.

`docs/es3-zephyr-env.md` is not on this branch (zephyr-env is on its own
branch `ccfarm/01a11864-6c3b-7286-9e40-1e2123e11ef2`); its env lines were not
available, the ones below follow the task statement.

`driver/src/fw/pvr_fw_blob.h` is **absent** (`find` over the deps cache for
`pvr_fw_blob*`: no hit; `driver/src/` has no `fw/`). Image links of the deqp
sample therefore cannot run; only the stage-1 build and the configure step
could.

## Commands to run (not yet run)

```sh
D=$CCFARM_DEPS_DIR
export CTS=$D/VK-GL-CTS ZEPHYR_BASE=$D/zephyr-ws/zephyr
export ZEPHYR_SDK=$D/zephyr-sdk-1.0.1 ZEPHYR_SDK_INSTALL_DIR=$ZEPHYR_SDK
export PATH=$D/zephyr-ws/.venv/bin:$PATH RUSTUP_TOOLCHAIN=nightly
export ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh $D/zephyr-ws)"
export BOARD=verdin_am62p/am62p54/a53
for m in gles3 gles2; do
  time MODULE=$m WORK=$D/deqp-stage1-am62p-$m tools/cts/cts-build-zephyr.sh
done
# link check per module, once the firmware blob exists:
west build -b $BOARD driver/samples/zephyr/deqp -- \
    -DZEPHYR_MODULES="$ZEPHYR_MODULES" \
    -DDEQP_SRC=$D/deqp-stage1-am62p-$m/cts -DDEQP_BUILD=$D/deqp-stage1-am62p-$m/build
```

For gles3, every `undefined reference to 'gl*'` at that link is a finding
(missing ES 3.0 entry point; do not change `src/` or `capi/` in this item).

The runner script used for the attempt is
`$CCFARM_DEPS_DIR/scratch/s1-run.sh`; it runs the loop above with timing and
logs to `$CCFARM_DEPS_DIR/scratch/s1/<module>.log`. Running
`sh $CCFARM_DEPS_DIR/scratch/s1-run.sh` on a worker that may execute it
completes the item; then fill in CTS commit, durations, `du -sh` of each
`build`, and the link results here.
