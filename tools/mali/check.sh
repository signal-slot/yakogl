#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The local gate for Mali work: exactly the rows of the `mali` scope of
# tools/check.sh, and nothing else. It does not list the rows itself; it asks
# tools/check.sh for the plan of a diff that touches only tools/mali, so the
# rows here and the rows CI runs for a Mali pull request (`tools/check.sh
# --changed <base>`) are the same by construction:
#
#   cargo fmt --check
#   clippy and the --lib mali:: tests of host-test,gpu-mali,shaderc-online,
#     no-shader-table (zero warnings; the exact test count is the pin in
#     tools/mali/gate.env)
#   the two generated tables, regenerated and compared byte for byte
#   the third_party upstream hashes, the pan_ref_diff self-test
#   cargo check of osal-zephyr,gpu-mali,... for aarch64-unknown-none
#   yakogl-mali-panthor: clippy, tests, clippy for aarch64-unknown-linux-gnu
#   pan_shc on the tri pair when MESA_PAN_TREE is set; otherwise a `gap`
#     line, which is not a pass and makes the exit status non-zero
#
# Usage: tools/mali/check.sh [--jobs N]
set -u
top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel) || exit 2
cd "$top" || exit 2
pwd
git rev-parse --abbrev-ref HEAD
git rev-parse --short HEAD
YAKOGL_CHECK_PATHS=tools/mali/gate.env tools/check.sh "$@" --changed HEAD
rc=$?
if [ "$rc" -eq 0 ]; then echo "tools/mali/check.sh: mali scope green"; else echo "tools/mali/check.sh: mali scope NOT green (rc=$rc)"; fi
exit "$rc"
