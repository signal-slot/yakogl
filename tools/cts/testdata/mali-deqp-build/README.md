<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# mali-deqp-build: fixtures of tools/cts/mali-deqp-build-selftest.sh

`tools/cts/mali-deqp-build.sh` takes its repository from its own location, so
the self-test copies the script, `cts-yakogl-target.patch` and the stand-ins
below into `tools/cts/` of a scratch git repository and dry-runs the copy
there. Fakes on `PATH` log to `called.log` and fail if called.

| path | what |
| --- | --- |
| `deps-env.sh` | the stand-in for `tools/cts/deps-env.sh`: `--layout` prints `$FAKE_DEPS`, anything else logs to `$FAKE_CALLED` and fails (a dry run must only print the `--build` step, not run it) |
| `mustpass.txt` | a three-line stand-in for `gles2-mustpass.txt` (the dry run only names it; the self-test edits it to make the tree dirty) |

The script's symbol-floor helpers (`max_of`, `ver_le`) are cut out of the
script with `grep` and run on version lists written by the self-test. The
floors check itself (step 4) needs a built program and is not run on the host.
