#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_record_check_selftest.sh - gate row for tools/mp2s_record_check.py (plan #362).
#
#   tools/mp2s_record_check_selftest.sh
#
# Runs the checker's built-in --self-test (synthetic DK standalone console
# logs, no board) and expects "all ok". tools/check_selftests.py gives the gate
# a row only for files named *selftest.sh / *selftest.py, so the built-in
# --self-test needs this wrapper to run in CI.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
if ! out=$(python3 -I "$here/mp2s_record_check.py" --self-test 2>&1); then
	printf '%s\n' "$out"
	echo "mp2s_record_check_selftest: FAIL (--self-test exited non-zero)" >&2
	exit 1
fi
printf '%s\n' "$out"
case $out in
*"self-test: all ok"*) ;;
*)
	echo "mp2s_record_check_selftest: FAIL (no 'all ok' line)" >&2
	exit 1
	;;
esac
echo "mp2s_record_check_selftest: ok"
