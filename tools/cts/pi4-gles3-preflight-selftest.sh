#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# pi4-gles3-preflight-selftest.sh - selftest of pi4-gles3-preflight.sh on a fake layout of empty stub files under
# $TMPDIR (removed at the end): all good, an image missing, an image with rc=1, an image from another commit,
# a dirty build, a row range past the table end, BOARD_UART unset, DEQP_MODULE=gles2, no stage 1; that BOARD_UART's
# value never appears and that the script writes nothing. The worker's own env is replaced (env -i).
#   TMPDIR=<dir> tools/cts/pi4-gles3-preflight-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory}"
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
W=$(mktemp -d "$TMPDIR/preflight.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fails=0
UARTVAL=selftest-uart-value-7
SHA=0123456789abcdef0123456789abcdef01234567
SHORT=012345678
SUM=0000000000000000000000000000000000000000000000000000000000000007
rc=0
L=(A1 B1 C1) # the first three labels of the real table
for l in "${L[@]}"; do grep -q "^# image $l:" "$HERE/pi4-gles3-tree.txt" || { echo "table has no image $l" >&2; exit 2; }; done

mk() { # mk <dir>: a fake layout of empty stub files; three images built from $SHA
	mkdir -p "$1/work/cts" "$1/work/build/modules/gles3" "$1/ws" "$1/img"
	: > "$1/work/build/modules/gles3/libdeqp-gles3-package.a"
	for l in "${L[@]}"; do
		echo "$l rc=0 sha256=$SUM build=$SHORT" > "$1/img/$l.built"
	done
}
run() { # run <dir> <uart> <module> args...: output in $W/log, status in $rc
	local d=$1 uart=$2 mod=$3; shift 3
	local e=(PATH="$PATH" HOME="$W" DEQP_WORK="$d/work" ZEPHYR_WS="$d/ws" DEQP_MODULE="$mod")
	if [ -n "$uart" ]; then e+=(BOARD_UART="$uart"); fi
	env -i "${e[@]}" bash "$HERE/pi4-gles3-preflight.sh" --img-dir "$d/img" "$@" > "$W/log" 2>&1
	rc=$?
}
expect() { # expect <name> <want rc> <pattern>
	local good=1
	[ "$rc" -eq "$2" ] || good=0
	grep -q -- "$3" "$W/log" || good=0
	if grep -q -- "$UARTVAL" "$W/log"; then good=0; echo "  FAIL BOARD_UART value in output"; fi
	if grep -q -- "$W" "$W/log"; then good=0; echo "  FAIL a path of the layout in output"; fi
	if [ $good -eq 1 ]; then echo "  ok   $1"
	else echo "  FAIL $1 (rc=$rc)"; sed 's/^/       /' "$W/log"; fails=$((fails + 1)); fi
}

mk "$W/a"
find "$W/a" | sort > "$W/before"
run "$W/a" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "all good" 0 "preflight: all ok"
grep -q "ok   BOARD_UART is set" "$W/log" || { echo "  FAIL BOARD_UART reported as set"; fails=$((fails + 1)); }
grep -q "ok   all 3 built image(s) are commit 01234567" "$W/log" || { echo "  FAIL commit check line"; fails=$((fails + 1)); }
run "$W/a" "$UARTVAL" gles3 --commit 01234567 --rows 0,2
expect "8-digit commit prefix, rows 0,2" 0 "preflight: all ok"

mk "$W/b"; rm "$W/b/img/B1.built"
run "$W/b" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "image missing" 1 "FAIL image(s) missing .*B1"

mk "$W/c"; echo "B1 rc=1 sha256=none build=none" > "$W/c/img/B1.built"
run "$W/c" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "image with rc=1" 1 "FAIL image(s) not built .*B1(rc=1"

mk "$W/d"; echo "C1 rc=0 sha256=$SUM build=fedcba987" > "$W/d/img/C1.built"
run "$W/d" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "image from another commit" 1 "FAIL image(s) not built from commit 01234567: C1(build fedcba987)"

mk "$W/g"; echo "A1 rc=0 sha256=$SUM build=$SHORT-dirty" > "$W/g/img/A1.built"
run "$W/g" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "dirty build" 1 "FAIL image(s) not built from commit 01234567: A1(build .*-dirty"

mk "$W/h"; echo "A1 rc=0 sha256=$SUM build=unknown" > "$W/h/img/A1.built"
run "$W/h" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "commit not recorded" 1 "FAIL commit not recorded for: A1"

run "$W/a" "$UARTVAL" gles3 --commit "$SHA" --rows 80-95
expect "rows past the table end" 1 "FAIL rows 80-95 reach past the table end"

mk "$W/e"
run "$W/e" "" gles3 --commit "$SHA" --rows 0-2
expect "BOARD_UART unset" 1 "FAIL BOARD_UART is unset"

run "$W/a" "$UARTVAL" gles2 --commit "$SHA" --rows 0-2
expect "DEQP_MODULE=gles2" 1 "FAIL DEQP_MODULE is 'gles2'"

mk "$W/f"; rm "$W/f/work/build/modules/gles3/libdeqp-gles3-package.a"
run "$W/f" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2
expect "missing stage 1" 1 "FAIL stage 1 package missing"

run "$W/a" "$UARTVAL" gles3 --commit "$SHA" --rows 0-2 --table "$W/nonexistent"
expect "missing table" 1 "FAIL table file not found"

env -i PATH="$PATH" bash "$HERE/pi4-gles3-preflight.sh" --rows 5-1 > "$W/log" 2>&1; rc=$?
expect "backwards rows is a usage error" 2 "usage:"

env -i PATH="$PATH" bash "$HERE/pi4-gles3-preflight.sh" > "$W/log" 2>&1; rc=$?
expect "no environment fails with FAIL lines" 1 "FAIL DEQP_WORK is not set"
if grep -qi 'traceback' "$W/log"; then echo "  FAIL traceback with no environment"; fails=$((fails + 1)); fi

find "$W/a" | sort > "$W/after"
if cmp -s "$W/before" "$W/after"; then echo "  ok   read-only (file listing unchanged)"
else echo "  FAIL read-only"; diff "$W/before" "$W/after"; fails=$((fails + 1)); fi

if [ $fails -eq 0 ]; then echo "preflight selftest: all ok"; exit 0; fi
echo "preflight selftest: $fails FAILED"
exit 1
