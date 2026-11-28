#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of tools/cts/pi5-deqp-images.sh --dry-run over the repository's own
# pi5-full-tree.txt: one build is listed per "# image <LABEL>:" block, with and
# without a LABEL argument, the board is rpi_5, a label the table lacks lists
# nothing, and nothing is built (no image directory appears). Needs no
# ZEPHYR_WS, no stage-1 tree, no toolchain and no board.
#
#   sh tools/cts/pi5-images-selftest.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/pi5-deqp-images.sh
table=$here/pi5-full-tree.txt
w=$(mktemp -d "${TMPDIR:-/tmp}/pi5-images-selftest.XXXXXX") || exit 2
trap 'rm -rf "$w"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
unset ZEPHYR_WS ZEPHYR_SDK_INSTALL_DIR DEQP_WORK DEQP_STAGE1 DEQP_MODULE

labels=$(awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); print l }' "$table")
n=$(printf '%s\n' "$labels" | grep -c .)
[ "$n" -gt 0 ] || { echo "FAIL no image block in $table"; exit 1; }
first=$(printf '%s\n' "$labels" | sed -n 1p)

img=$w/img
sh_out=$("$tool" --dry-run "$table" "$img" 2>"$w/err"); rc=$?
if [ $rc = 0 ]; then ok "no LABEL: exit 0 without ZEPHYR_WS"; else bad "no LABEL: rc $rc: $(cat "$w/err")"; fi
lines=$(printf '%s\n' "$sh_out" | grep -c .)
if [ "$lines" = "$n" ]; then ok "no LABEL: $lines builds listed for $n image blocks"
else bad "no LABEL: $lines lines for $n image blocks"; fi
missing=
for l in $labels; do
	c=$(printf '%s\n' "$sh_out" | grep -c "^$l ")
	[ "$c" = 1 ] || missing="$missing $l($c)"
done
if [ -z "$missing" ]; then ok "every label is listed exactly once"; else bad "label counts not 1:$missing"; fi
if printf '%s\n' "$sh_out" | grep -q "^$first dEQP-GLES2\."; then ok "a line is '<LABEL> <filter>'"
else bad "first line: $(printf '%s\n' "$sh_out" | sed -n 1p)"; fi

one=$("$tool" --dry-run "$table" "$img" "$first" 2>"$w/err"); rc=$?
if [ $rc = 0 ] && [ "$(printf '%s\n' "$one" | grep -c .)" = 1 ] && printf '%s\n' "$one" | grep -q "^$first "; then
	ok "LABEL $first: exactly its own build"
else bad "LABEL $first: rc $rc: $one"; fi

# the board is rpi_5: the wrapper names no other and the builder refuses a
# wrong one, so the table is not an rpi_4b one
if grep -q 'deqp-tree-images.sh" rpi_5 ' "$tool"; then ok "the wrapper passes board rpi_5"
else bad "pi5-deqp-images.sh does not pass rpi_5"; fi
if grep -q 'rpi_4b\|nanopc_t6' "$tool"; then bad "the wrapper names another board"; else ok "the wrapper names no other board"; fi

# an unknown label: nothing listed (dry-run exits 0 for it, see the report; the
# exit status is not asserted)
none=$("$tool" --dry-run "$table" "$img" NOSUCHLABEL 2>/dev/null)
if [ -z "$none" ]; then ok "an unknown label lists no build"; else bad "unknown label listed: $none"; fi

# a missing table is refused
"$tool" --dry-run "$w/no-such-table" "$img" >/dev/null 2>&1; rc=$?
if [ $rc != 0 ]; then ok "a missing table: nonzero exit ($rc)"; else bad "a missing table exited 0"; fi

if [ ! -e "$img" ] && [ -z "$(ls -A "$w" | grep -v '^err$')" ]; then ok "nothing is built: no image directory"
else bad "dry run left files: $(ls "$w")"; fi

if [ $fail = 0 ]; then echo "pi5-images-selftest: all ok"; else echo "pi5-images-selftest: FAILED"; fi
exit $fail
