#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/cts/t6-deqp-images.sh --plan: a known row, an unknown
# row, a missing variable, and that nothing is built. Needs no toolchain.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$HERE/mali-gles2-tree.txt
S=$HERE/t6-deqp-images.sh
W=$(mktemp -d "${TMPDIR:-/tmp}/t6dry.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
ROW=$(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l; exit }' "$T")
[ -n "$ROW" ] || { echo "no row in $T"; exit 1; }
export ZEPHYR_WS=ws-x ZEPHYR_SDK_INSTALL_DIR=sdk-x DEQP_WORK=work-x

out=$("$S" --plan "$T" "$W/img" "$ROW" 2>"$W/err"); rc=$?
if [ $rc = 0 ]; then ok "known row exits 0"; else bad "known row exit $rc: $(cat "$W/err")"; fi
if grep -q "^$ROW: filter " <<<"$out" && grep -q "ninja -C $W/img/$ROW" <<<"$out" &&
	grep -q "$W/img/$ROW/zephyr/zephyr.bin" <<<"$out"; then ok "commands and paths printed"; else bad "output: $out"; fi
if grep -q 'work-x\|ws-x\|sdk-x' <<<"$out"; then bad "a variable value was printed"; else ok "no values printed"; fi
if [ ! -e "$W/img" ]; then ok "nothing built"; else bad "image dir created"; fi

"$S" --plan "$T" "$W/img" NOSUCHROW >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ] && grep -q 'unknown row NOSUCHROW' "$W/err"; then ok "unknown row refused"; else bad "unknown row rc=$rc: $(cat "$W/err")"; fi

for v in ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK; do
	env -u "$v" "$S" --plan "$T" "$W/img" "$ROW" >"$W/o" 2>"$W/err"; rc=$?
	if [ $rc != 0 ] && grep -q "$v is not set" "$W/err"; then ok "missing $v refused"; else bad "missing $v rc=$rc: $(cat "$W/err")"; fi
done

"$S" --plan "$W/none.txt" "$W/img" >"$W/o" 2>"$W/err"; rc=$?
if [ $rc != 0 ]; then ok "missing table refused"; else bad "missing table rc=$rc"; fi
[ $fail = 0 ] && echo "t6-deqp-images-selftest: all ok"
exit $fail
