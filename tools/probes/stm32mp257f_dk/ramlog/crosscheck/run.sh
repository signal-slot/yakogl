#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh - host cross-check of the board lib's ramlog writer (ramlog.c,
# compiled unchanged with stub Zephyr headers) against ramlog_decode.py.
#
#   bash tools/probes/stm32mp257f_dk/ramlog/crosscheck/run.sh [--regen]
#
# RAMLOG_C=path overrides the writer source (to try a modified copy).
# Cases: short; wrap (128-byte region, 96-byte ring, wrapped more than five
# times); empty (zeroed DDR); garbage (cold power-up noise); torn (the last
# byte stored, head not yet updated). stdout of the decoder is diffed against
# expected/<case>.out; exit code and stderr line are checked here.
# Ends "N checks, 0 failed". --regen rewrites expected/ (review the diff).
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
src=${RAMLOG_C:-$here/../../../../../driver/zephyr/lib/board/common/ramlog.c}
dec=$here/../ramlog_decode.py
regen=0
[ "${1:-}" = --regen ] && regen=1
tmp=$(mktemp -d "${TMPDIR:-/tmp}/ramlog-cross.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
n=0; failed=0
ok() { n=$((n + 1)); echo "ok   $1"; }
bad() { n=$((n + 1)); failed=$((failed + 1)); echo "FAIL $1"; }

if ! gcc -std=gnu11 -Wall -Wextra -Werror -I"$here/stubs" \
    -DCONFIG_YAKOGL_RAMLOG_SIZE=128 -DCONFIG_YAKOGL_RAMLOG_ADDR=0x90000000 \
    -o "$tmp/harness" "$here/harness.c" "$src"; then
    bad "build of $src"
    echo "$n checks, $failed failed"
    exit 1
fi
ok "ramlog.c builds on the host"

check() { # case rc stderr
    local c=$1 rc=$2 err=$3 got
    "$tmp/harness" "$c" "$tmp/$c.bin" || { bad "$c: harness"; return; }
    python3 -I "$dec" "$tmp/$c.bin" >"$tmp/$c.out" 2>"$tmp/$c.err"
    got=$?
    if [ "$got" = "$rc" ]; then ok "$c exit $rc"; else bad "$c exit $got, want $rc"; fi
    if [ "$(cat "$tmp/$c.err")" = "$err" ]; then ok "$c stderr"; else bad "$c stderr: $(cat "$tmp/$c.err")"; fi
    [ $regen = 1 ] && cp "$tmp/$c.out" "$here/expected/$c.out"
    if cmp -s "$here/expected/$c.out" "$tmp/$c.out"; then ok "$c text"; else bad "$c text differs from expected/$c.out"; fi
}
check short 0 ''
check wrap 0 '[ramlog: 464 bytes lost]'
check empty 3 'ramlog: no ring'
check garbage 3 'ramlog: no ring'
check torn 0 ''
echo "$n checks, $failed failed"
[ "$failed" = 0 ]
