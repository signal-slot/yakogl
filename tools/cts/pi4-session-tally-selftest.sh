#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pi4-session-tally-selftest.sh
#
# Hermetic check of tools/cts/pi4-session-tally.sh on a scratch copy of the
# testdata consoles: a tally per console, no absolute path in it, an unknown
# label refused with nothing written, no other file changed.
set -u
unset BOARD BOARD_UART DEQP_MODULE DEQP_STAGE1 DEQP_STAGE1_GLES3 DEQP_WORK \
    IMX8MP_OUT MESA_V3D_TREE PI4_TREE PI4_MUSTPASS ZEPHYR_WS LANGUAGE LC_ALL
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$HERE/testdata/gles3-notpass-pi4/s1-lo
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

T=$(mktemp -d "${TMPDIR:-/tmp}/pi4-session-tally-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
S=$T/session
mkdir "$S"
cp "$SRC/A1.console.txt" "$SRC/README.md" "$S/"
# B1: a finished image of two real mustpass cases, one failing
CASE=dEQP-GLES3.functional.fragment_ops.scissor.clear_int_buffer
{
    echo "deqp: Pass ${CASE}.a"
    echo "deqp: Fail ${CASE}.b"
    echo "deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1"
    echo "deqp: summary: 2 cases begun, 2 with a result, 0 without"
} > "$S/B1.console.txt"
# C1: an image that did not finish, cut off before the summary
head -n 3 "$SRC/A1.console.txt" > "$S/C1.console.txt"

out=$(bash "$HERE/pi4-session-tally.sh" "$S" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "session tallied" || bad "rc=$rc: $out"
for l in A1 B1 C1; do
    [ -s "$S/$l.tally.txt" ] && ok "$l.tally.txt written" || bad "$l.tally.txt missing"
done
case "$out" in *"B1: "[0-9]*" pass, "[0-9]*" fail, "[0-9]*" notsupported, "[0-9]*" other"*) ok "summary line of B1" ;; *) bad "B1 summary: $out" ;; esac
case "$out" in *"C1: aborted"*) ok "C1 reported aborted" ;; *) bad "C1 not aborted: $out" ;; esac
if grep -Eq '(^|[ :=(])/(home|tmp|Users|var|root)/' "$S"/*.tally.txt; then
    bad "an absolute path in a tally"
else ok "no absolute path in the tallies"; fi
grep -q 'A1.console.txt' "$S/A1.tally.txt" && ok "tally names the relative console" || bad "relative console name missing"
[ "$(ls "$S" | wc -l)" -eq 7 ] && ok "only the three tallies were added" || bad "files: $(ls "$S")"
cmp -s "$SRC/A1.console.txt" "$S/A1.console.txt" && ok "console unchanged" || bad "console changed"

# an unknown label is refused, naming it, and writes nothing
U=$T/unknown
mkdir "$U"
cp "$SRC/A1.console.txt" "$U/ZZ99.console.txt"
cp "$SRC/A1.console.txt" "$U/A1.console.txt"
out=$(bash "$HERE/pi4-session-tally.sh" "$U" 2>&1); rc=$?
[ $rc -ne 0 ] && ok "unknown label refused" || bad "unknown label accepted"
case "$out" in *ZZ99*) ok "message names ZZ99" ;; *) bad "no label in: $out" ;; esac
[ ! -e "$U/A1.tally.txt" ] && [ ! -e "$U/ZZ99.tally.txt" ] && ok "nothing written on refusal" || bad "tally written on refusal"

if [ $fail -eq 0 ]; then
    echo "pi4-session-tally-selftest: all checks passed"
else
    echo "pi4-session-tally-selftest: $fail failed"
    exit 1
fi
