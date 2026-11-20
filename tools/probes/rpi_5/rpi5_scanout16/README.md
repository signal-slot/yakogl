# rpi5_scanout16

The Raspberry Pi 5 probe that adopts the firmware's running RGB565 plane,
draws text and a pattern into it and holds (`docs/notes/rpi5-display.md`
section 3, question Q2). This directory has the build skeleton: `src/main.c`
prints the build line and the read-only register dump (`src/dump.h`: HVS
VERSION, channels 0 to 2 control/status with FRCNT and list pointer, and
pixel valve 0 and 1 on separate lines with PV_CONTROL and PV_V_CONTROL), then
`rpi5_scanout16: skeleton done`. It writes no register. `src/hold_check.h`
(host test in `test/`) and `verdicts.sh` belong to the later stages.

## Build

    west build -p always -b rpi_5 -d build/rpi5_scanout16 \
      yakogl/tools/probes/rpi_5/rpi5_scanout16 -- -DYAKOGL_WERROR=ON

The gate's `zephyr_rpi5_probe_images` row builds every directory under
`tools/probes/rpi_5/` that has a CMakeLists.txt and a prj.conf, this one
included.

Host self-test of the hold decision: `sh hold_check_selftest.sh`.

## On the board

Not run yet.
