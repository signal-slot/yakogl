#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-common-selftest.sh
#
# Host-only checks of tools/cts/x2-common.sh (its --self-test: arguments, the
# ancestor check, the record layout and the measured-on marker, with stub
# board.sh and west in a scratch git repository), plus a syntax check and
# shellcheck when it is installed. No board, no network, no environment.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
S=$HERE/x2-common.sh
fail=0
bash -n "$S" || fail=1
if command -v shellcheck > /dev/null; then
    shellcheck -x "$S" "$0" || fail=1
else
    echo "x2-common-selftest: shellcheck not installed, skipped"
fi
env -i PATH="$PATH" HOME="${HOME:-/}" TMPDIR="${TMPDIR:-/tmp}" bash "$S" --self-test || fail=1
[ "$fail" = 0 ] && echo "x2-common-selftest: all ok" || echo "x2-common-selftest: FAILED"
exit "$fail"
