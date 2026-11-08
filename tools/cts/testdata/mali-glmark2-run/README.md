<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/mali-glmark2-run/`

The fixtures of `tools/cts/mali-glmark2-selftest.sh`, which drives the
NanoPC-T6 glmark2-es2 runner `tools/cts/mali-glmark2-run.sh` without a board.

- `bin/ssh`, `bin/scp`: put first on the runner's PATH. They run the board's
  side on this host: the login's home is `FAKE_BOARD/home`, a real directory,
  so the stage path the runner resolves is the path it stages to and the
  path the fake program's `lib=` names; `/proc/sys/kernel/random/boot_id` is
  mapped to `FAKE_BOARD/boot_id`. Every call is logged (`FAKE_LOG`), so the
  self-test can prove that `--dry-run` and a refused build make none. While
  `FAKE_BOARD/down` exists the board is gone (ssh exit 255, no output).
- `board-bin/`: the board's tools the host may lack or must not answer for
  the host: `dmesg` prints `FAKE_BOARD/kmsg` (the ring), `fuser` says nobody
  holds the render node unless `FAKE_BOARD/busy` exists, `uname -r` answers
  as the T6's Armbian does. `setsid`, `timeout`, `pkill`, `pgrep` and `df`
  are the host's own; `pkill` and `pgrep` only ever match the per-run stage
  path under the self-test's directory.
- `fixture-build/`: a build directory in the layout `mali-glmark2-build.sh`
  makes (`bin/glmark2-es2-mali`, `lib/libGLESv2.so.2` and `libEGL.so.1`,
  `share/glmark2/`, `BUILD-INFO`; the self-test rewrites `commit=` and
  `dev=`). Its `glmark2-es2-mali` is a script that takes `--size 800x600 -b
  <scene>`, prints glmark2's result line, the pair's `yakogl-mali: loaded`
  line when the `libGLESv2.so.2` its `LD_LIBRARY_PATH` finds is the
  fixture's, a `glmark2-picture:` line and a `yakogl-mali-exit:` line.
- `scenarios/*.tsv`: what the fake program does to a scene
  (`<scene><TAB><action>`): `glerror`, `oom`, `refused`, `gpufault` and
  `jobtimeout` (with `kmsg/gpu-fault.txt` or `kmsg/job-timeout.txt` written
  to the board's ring), `setupfail`, `crash`, `hang`, `fpsonly`, `lostssh`,
  `reboot`, `nolib`, `foreignlib`; a scene without a line draws cleanly.
- `scenes.txt`: a `--scenes` file (an index, a scene with `;`, a scene
  without options).

The host name `board.invalid`, the address `198.51.100.7` (TEST-NET-2) and the
MAC `02:00:5e:00:53:01` (documentation range) appear in the inputs on
purpose: the self-test checks that the run directory keeps none of them.
