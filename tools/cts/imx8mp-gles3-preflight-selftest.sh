#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-preflight-selftest.sh - selftest of imx8mp-gles3-preflight.sh on a fake layout of empty stub files
# under $TMPDIR (removed at the end): all good, no stage 1, no image, wrong commit, BOARD_UART unset, and that
# BOARD_UART's value never appears in the output.
#   TMPDIR=<dir> tools/cts/imx8mp-gles3-preflight-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory}"
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
W=$(mktemp -d "$TMPDIR/preflight.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fails=0
SECRET=ttyUSB-selftest-secret-7
SHA=0123456789abcdef0123456789abcdef01234567
rc=0

mk() { # mk <dir>: a fake layout of empty stub files
	mkdir -p "$1/out/es3-01234567/zephyr" "$1/stage1/build/modules/gles3" "$1/ws" "$1/hal"
	: > "$1/out/es3-01234567/zephyr/zephyr.bin"
	: > "$1/stage1/build/modules/gles3/libdeqp-gles3-package.a"
}
run() { # run <dir> <uart> args...: output in $W/log, status in $rc
	local d=$1 uart=$2; shift 2
	local e=(PATH="$PATH" HOME="$W" IMX8MP_OUT="$d/out" DEQP_STAGE1="$d/stage1" ZEPHYR_WS="$d/ws" HAL_NXP_DIR="$d/hal")
	if [ -n "$uart" ]; then e+=(BOARD_UART="$uart"); fi
	env -i "${e[@]}" bash "$HERE/imx8mp-gles3-preflight.sh" "$@" > "$W/log" 2>&1
	rc=$?
}
expect() { # expect <name> <want rc> <pattern>
	local good=1
	[ "$rc" -eq "$2" ] || good=0
	grep -q -- "$3" "$W/log" || good=0
	if grep -q -- "$SECRET" "$W/log"; then good=0; echo "  FAIL BOARD_UART value in output"; fi
	if [ $good -eq 1 ]; then echo "  ok   $1"
	else echo "  FAIL $1 (rc=$rc)"; sed 's/^/       /' "$W/log"; fails=$((fails + 1)); fi
}

mk "$W/a"
run "$W/a" "$SECRET" --commit "$SHA"
expect "all good" 0 "preflight: all ok"
grep -q "ok   BOARD_UART is set" "$W/log" || { echo "  FAIL BOARD_UART reported as set"; fails=$((fails + 1)); }
run "$W/a" "$SECRET" --commit 01234567
expect "8-digit commit prefix" 0 "preflight: all ok"

mk "$W/b"; rm "$W/b/stage1/build/modules/gles3/libdeqp-gles3-package.a"
run "$W/b" "$SECRET" --commit "$SHA"
expect "missing stage 1" 1 "FAIL stage 1 package missing"

mk "$W/c"; rm -r "$W/c/out/es3-01234567"
run "$W/c" "$SECRET" --commit "$SHA"
expect "missing image" 1 "FAIL no table image"

mk "$W/d"
run "$W/d" "$SECRET" --commit fedcba9876543210
expect "wrong commit" 1 "FAIL no image is named after commit fedcba98"

mk "$W/e"
run "$W/e" "" --commit "$SHA"
expect "BOARD_UART unset" 1 "FAIL BOARD_UART is unset"

run "$W/a" "$SECRET" --commit "$SHA" --table "$W/nonexistent"
expect "missing table" 1 "FAIL table file not found"

# X2 images live in <out>/x2/es3-<hash8> (imx8mp-env.sh): a newer one next to the Verdin's is never picked up,
# neither by the preflight nor by the glob imx8mp-gles3-board.sh takes its default image from
mk "$W/f"; mkdir -p "$W/f/out/x2/es3-fedcba98/zephyr"; : > "$W/f/out/x2/es3-fedcba98/zephyr/zephyr.bin"
sleep 1; touch "$W/f/out/x2/es3-fedcba98/zephyr/zephyr.bin"
run "$W/f" "$SECRET" --commit "$SHA"
expect "newer X2 image beside the Verdin's" 0 "table image exists (es3-01234567)"
run "$W/f" "$SECRET" --commit fedcba98
expect "X2 image is not a Verdin image" 1 "FAIL no image is named after commit fedcba98"
pick=$(ls -t "$W"/f/out/es3-*/zephyr/zephyr.bin 2>/dev/null | head -1)
if [ "$pick" = "$W/f/out/es3-01234567/zephyr/zephyr.bin" ]; then echo "  ok   board script's glob picks the Verdin image"
else echo "  FAIL board script's glob picked '$pick'"; fails=$((fails + 1)); fi

if [ "$(find "$W/a" -type f | wc -l)" -eq 2 ]; then echo "  ok   read-only"
else echo "  FAIL read-only"; fails=$((fails + 1)); fi

if [ $fails -eq 0 ]; then echo "preflight selftest: all ok"; exit 0; fi
echo "preflight selftest: $fails FAILED"
exit 1
