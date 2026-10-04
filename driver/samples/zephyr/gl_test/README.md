# gl_test (Zephyr)

Runs `tools/pvrlab/gl_test.c`, the on-silicon YakoGL test suite of the Linux
harness, on Zephyr: the same source file (printk instead of pr_info), every
case reading its pixels back and printing `gl_test: case N <name>: PASS/FAIL`,
then `gl_test: all: N of 24 cases failed` (the regression set; the header comment of
`gl_test.c` lists every case). Verified on the Verdin AM62P on 2026-09-14: 0 of 23
cases failed (0 of 20 before cases 21..24 were added; case 34 came later and was
verified through the Linux harness).

```sh
# once per checkout: embed the board's DDK firmware (TI/Img property, never
# committed, and .gitignored -- so a FRESH clone or git worktree has nothing
# here and the build stops at `fatal error: fw/pvr_fw_blob.h`)
driver/tools/mkfwblob.sh /path/to/rgx.fw.36.53.104.796   # -> driver/src/fw/pvr_fw_blob.h
# ...or, when the firmware itself is not on the host any more, copy the
# generated header from another checkout: it is a build artefact of the same
# firmware and does not depend on the branch.
# from the top of a west workspace made from yakogl's west.yml (../README.md)
west build -p always -b verdin_am62p/am62p54/a53 -d build/gl_test \
    yakogl/driver/samples/zephyr/gl_test
BOARD_HOST=torizon@<board> yakogl/driver/tools/kexec-zephyr/kexec_run.sh \
    build/gl_test/zephyr/zephyr.bin
```

Kconfig: `GL_TEST_CASE` (7 = all), `GL_TEST_PANEL` (1 = demo scene, 2 = spinning
MSAA cube on the DSI panel, shown through the DSS application plane),
`GL_TEST_DEBUG` (1 = command-stream dumps, 2 = depth/stencil dumps),
`GL_TEST_PRETRANSFORM`. The shared board glue (`samples/zephyr/common`: hold,
reset, RTI watchdog feed, DSS plane) is the same as in `gl_triangle`.
