#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_uimg_check_selftest.sh - tools/mp2s_uimg_check.py without a board
# (plan #362, item uimg-check).
#
#   tools/mp2s_uimg_check_selftest.sh
#
# Fixtures in $TMPDIR: one written by Python (always) and one by mkimage
# (when it is on PATH; otherwise that part is skipped). Cases: a good image
# (with --expect and --sha256); a flipped header byte; a flipped data byte;
# type kernel; arch arm; a truncated file; a body that differs from
# --expect; a sha256 mismatch. A corruption exits 1 and prints a
# "mp2s-uimg: FAIL <check>" line naming the check.
#
# A tools/check.sh row waits for plan #341, so it is run by hand.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-uimg-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
chk="$here/mp2s_uimg_check.py"

printf 'echo mp2s: hello\nsetenv mp2s_left 1\nsaveenv\n' >"$T/s.cmd"
printf 'echo other\n' >"$T/other.cmd"

# mkpy OUT [TYPE] [ARCH]: the image mkimage writes for s.cmd
mkpy() {
	python3 - "$T/s.cmd" "$@" <<'PY'
import struct, sys, zlib
body = open(sys.argv[1], "rb").read()
out = sys.argv[2]
typ = int(sys.argv[3]) if len(sys.argv) > 3 else 6
arch = int(sys.argv[4]) if len(sys.argv) > 4 else 22
data = struct.pack(">II", len(body), 0) + body
h = struct.pack(">IIIIIIIBBBB32s", 0x27051956, 0, 1700000000, len(data), 0, 0,
                zlib.crc32(data) & 0xFFFFFFFF, 5, arch, typ, 0, b"mp2s")
h = h[:4] + struct.pack(">I", zlib.crc32(h) & 0xFFFFFFFF) + h[8:]
open(out, "wb").write(h + data)
PY
}

# flip OUT IN OFFSET: copy IN with the byte at OFFSET inverted
flip() {
	python3 - "$@" <<'PY'
import sys
b = bytearray(open(sys.argv[2], "rb").read())
b[int(sys.argv[3])] ^= 0xFF
open(sys.argv[1], "wb").write(b)
PY
}

# run DESC WANT_EXIT WANT_LINE ARGS...: WANT_LINE is a grep -E on stdout
run() {
	local what=$1 want=$2 line=$3 rc=0
	shift 3
	python3 "$chk" "$@" >"$T/out" 2>"$T/err" || rc=$?
	if [ "$rc" = "$want" ] && { [ -z "$line" ] || grep -Eq -- "$line" "$T/out"; }; then
		pass=$((pass + 1))
	else
		echo "FAIL $what (exit $rc, want $want; line /$line/)"
		sed 's/^/  out| /' "$T/out"
		sed 's/^/  err| /' "$T/err"
		fail=$((fail + 1))
	fi
}

# cases IMAGE LABEL: the case set for an image that is good
cases() {
	local g=$1 l=$2 sha
	sha=$(sha256sum "$g" | cut -d' ' -f1)
	echo "$sha  boot.scr.uimg" >"$T/good.sha"
	echo "0000000000000000000000000000000000000000000000000000000000000000  boot.scr.uimg" >"$T/bad.sha"
	run "$l good" 0 '^mp2s-uimg: ok sha256$' "$g" --expect "$T/s.cmd" --sha256 "$T/good.sha"
	if grep -q FAIL "$T/out"; then
		echo "FAIL $l good prints FAIL"
		fail=$((fail + 1))
	fi
	flip "$T/hdr.img" "$g" 40
	run "$l header byte" 1 '^mp2s-uimg: FAIL hcrc: ' "$T/hdr.img"
	flip "$T/dat.img" "$g" 70
	run "$l data byte" 1 '^mp2s-uimg: FAIL dcrc: ' "$T/dat.img"
	head -c 80 "$g" >"$T/trunc.img"
	run "$l truncated" 1 '^mp2s-uimg: FAIL size: ' "$T/trunc.img"
	head -c 30 "$g" >"$T/short.img"
	run "$l short" 1 '^mp2s-uimg: FAIL size: ' "$T/short.img"
	run "$l body mismatch" 1 '^mp2s-uimg: FAIL expect: ' "$g" --expect "$T/other.cmd"
	run "$l sha mismatch" 1 '^mp2s-uimg: FAIL sha256: ' "$g" --sha256 "$T/bad.sha"
}

mkpy "$T/py.img"
cases "$T/py.img" python
mkpy "$T/kernel.img" 2 22
run "type kernel" 1 '^mp2s-uimg: FAIL type: ' "$T/kernel.img"
mkpy "$T/arm.img" 6 2
run "arch arm" 1 '^mp2s-uimg: FAIL arch: ' "$T/arm.img"
run "missing file" 2 "" "$T/none.img"
run "no args" 2 ""

if command -v mkimage >/dev/null 2>&1; then
	mkimage -A arm64 -O linux -T script -C none -n mp2s -d "$T/s.cmd" "$T/mk.img" >/dev/null
	cases "$T/mk.img" mkimage
else
	echo "skip: mkimage not on PATH, Python fixtures only"
fi

echo "mp2s_uimg_check_selftest: $pass ok, $fail failed"
[ "$fail" = 0 ]
