<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gl_test (Zephyr, Verdin AM62P)

Runs `driver/tools/pvrlab/gl_test.c`, YakoGL's on-silicon test suite, on the
Verdin AM62P's PowerVR. Every case reads its pixels back and prints
`gl_test: case N <name>: PASS/FAIL`; a run ends with
`gl_test: all: N of M cases failed`. The header comment of `gl_test.c` lists
every case. The same file runs on the i.MX8MP (`gc_gl_test`) and the Pi 4
(`v3d_gl_test`). Current results are in `tools/cts/STATUS.md`.

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
yakogl/tools/board.sh am62p run build/gl_test/zephyr/zephyr.bin <logdir>
```

`tools/board.sh` needs an image with the RTI self-arm
(`CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y`); `tools/cts/common-images.sh` builds
the common-set image that way.

Kconfig: `GL_TEST_CASE` (7 = all), `GL_TEST_CASES`, `GL_TEST_PANEL` (1 = demo
scene, 2 = spinning MSAA cube on the DSI panel, shown through the DSS
application plane), `GL_TEST_DEBUG` (1 = command-stream dumps, 2 =
depth/stencil dumps), `GL_TEST_PRETRANSFORM`. The board glue (watchdog, hold
and reset, the DSS plane) is the board layer, `driver/zephyr/lib/board/`.
