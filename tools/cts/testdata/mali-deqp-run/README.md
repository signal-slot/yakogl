<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# `tools/cts/testdata/mali-deqp-run/`

The fixtures of `tools/cts/mali-deqp-selftest.sh`, which drives the NanoPC-T6
dEQP-GLES2 runner `tools/cts/mali-deqp-run.sh` without a board.

- `bin/ssh`, `bin/scp`: put first on the runner's PATH. They run the board's
  side on this host, in a directory standing for the board (`FAKE_BOARD`):
  `/root/yakogl-mali`, `/dev/kmsg` and `/proc/sys/kernel/random/boot_id` are
  mapped into it, and every call is logged (`FAKE_LOG`), so the self-test can
  prove that `--dry-run` and a refused commit make none. While
  `FAKE_BOARD/down` exists the board is gone (ssh exit 255, no output).
- `board-bin/`: the board's tools as the runner uses them (`dmesg -w` follows
  `FAKE_BOARD/kmsg`; `fuser` says nobody holds the render node; `pkill`,
  `pgrep` never touch this host's processes; `uname -r` and `df -Pk` answer
  as the T6's Armbian does).
- `fixture-build/`: a build directory in the layout `mali-deqp-build.sh`
  makes (`deqp-gles2`, `data/gles2/`, `build.txt` with `commit=`, `lib/` with
  `libGLESv2.so.2` and `libEGL.so.1`; not named `build/`, which `.gitignore`
  leaves out). Its `deqp-gles2` is a script that writes a `.qpa` from
  `qpa/pass.qpa` and `qpa/fail.qpa` and prints `yakogl-mali: loaded` and a
  `yakogl-mali-exit:` line when the staged `libGLESv2.so.2` is the fixture's.
- `scenarios/*.tsv`: what the fake `deqp-gles2` does to a case (`fail`,
  `crash`, `hang`, `jobtimeout` with `kmsg/job-timeout.txt` written to the
  board's kernel log, `lose-ssh`; `-<TAB>nolib` leaves out the loaded line).
- `tree.txt`: three rows (6, 6 and 5 cases) of the real
  `tools/cts/gles2-mustpass.txt`, in the table form of `mali-gles2-tree.txt`.

The host name `board.invalid`, the address `198.51.100.7` (TEST-NET-2) and the MAC
`02:00:5e:00:53:01` (documentation range) appear in the outputs on purpose:
the self-test checks that the slice keeps none of them.
