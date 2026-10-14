# Pi 4 plan #1 confirmation from the integration image: not run (no console)

HEAD8=0175bac1 (integration tip, ccfarm/integration)

Status: **no board result exists**. Nothing below was measured; every
expectation of HANDOFF "Next board session" > "Raspberry Pi 4" is **not
verified** (neither met nor failed).

## What was checked

```
ssh -o BatchMode=yes "$BOARD_HOST" true                 # rc 0, board login works
ssh -o BatchMode=yes "$BOARD_HOST" -- ls /dev/serial/by-id
  ls: cannot access '/dev/serial/by-id': No such file or directory
test -d /dev/serial/by-id                               # on the build host: no
```

## Why it stopped

`tools/board.sh pi4 run|tree` requires `BOARD_UART`, a character device on the
machine that runs it (board.sh lines 751 and 1417-1432): the Pi 4's dEQP and
v3d_gl_test records come back only on that UART. The lease `pi4-elegance`
gives ssh only. Neither the build host nor the board's Linux has the Raspberry
Pi Debug Probe (no `/dev/serial/by-id`), so there is no console to capture and
no verdict line can be produced. `BOARD_HDMI_TIMINGS` (tools/cts/README.md,
the 1920x720 panel's 17 fields) is a lab value that is also not in this
environment.

Running the images without a capture would only give "result none", so no run
was attempted and the board was left untouched (idle).

## Expectations

| Expectation (HANDOFF) | State |
| --- | --- |
| v3d_gl_test 21 / 21 (GT) | not run |
| T1 `texture.vertex.cube.filtering.*` 10 and `wrap.*` 8 pass | not run |
| T3 `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` 6 pass | not run |
| TF `texturecubelod` 1 passes | not run |
| D `fragdata.invalid_assign_to_1` NotSupported -> Pass | not run |
| FB1 27 `attachment_combinations` pass | not run |
| everything else as RESULTS.md "Pi 4: sweep 30" | not run |

## To run it

On a machine with the Debug Probe attached and ssh to the board:

```sh
export BOARD_HOST=<user@board>   # from the lease
export BOARD_UART=/dev/serial/by-id/usb-Raspberry_Pi_Debug_Probe__CMSIS-DAP__<serial>-if01
export BOARD_HDMI_TIMINGS='1920 1 50 44 50 720 1 10 20 50 0 0 0 60 0 99070000 1'
export DEQP_WORK=$CCFARM_DEPS_DIR/yakogl/deqp-stage1-rpi_4b/work   # exists on a farm node
HEAD8=$(git rev-parse --short=8 HEAD)
tools/board.sh --dry-run pi4 tree tools/cts/pi4-full-tree.txt $TMPDIR/dry
tools/cts/common-images.sh pi4 tools/cts/common-tree.txt <img-gt> GT
tools/board.sh pi4 run <img-gt>/GT/zephyr/zephyr.bin <rec>/gt gt-$HEAD8
RUSTUP_TOOLCHAIN=nightly tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt <img> D FB1 T1 T3 TF
TREE_IMAGE_DIR=<img> TREE_ROWS=1,4,19,21,24 tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt <rec>/cube
tools/board.sh pi4 tally <rec>/cube
```
