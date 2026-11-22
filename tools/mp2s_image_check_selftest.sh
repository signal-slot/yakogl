#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_image_check_selftest.sh - tools/mp2s_image_check.py without a board
# (plan #362, item image-check).
#
#   tools/mp2s_image_check_selftest.sh
#
# Synthetic arm64 Images (built here: image_size le64 at byte 16, "ARM\x64"
# at byte 56, an optional marker at byte 0x40, the rest sparse zeros), one
# per verdict:
#   - accepted: valid image with free space and marker, and without them;
#     image_size above the file size but inside the window (the file is only
#     its first part); image_size and the file exactly at the window; a file
#     exactly half the free space; a multi-word marker;
#   - refused: bad magic, image_size one above the window, image_size 4 GiB,
#     file below the 0x40-byte header, file one above the window, file one
#     above half the free space, marker absent, missing file, a directory;
#   - lifetime marker (yakogl-lifetime-s=N): none, 120, 0, two disagreeing,
#     two equal, with and without --require-bounded;
#   - usage errors (exit 2, no "refused:" line, empty stdout): a
#     non-numeric --free-bytes, an empty --require-marker.
# Every verdict pins the streams: exit 0 keeps stdout's "ok:"/"skip:" lines
# and an empty stderr; exit 2 puts the single "refused: <why>" line on
# stderr with the pinned wording and stdout never says "refused".
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_image_check_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-image-check-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
WANT_CHECKS=28

# mkimg FILE SIZE IMAGE_SIZE [MAGIC ok|bad] [MARKER]
# SIZE and IMAGE_SIZE are decimal or 0x hex; the marker, if given, goes at
# byte 0x40. The file is sparse past the header and the marker. Prints
# FILE, so a case can take its image from $(mkimg ...).
mkimg() {
	python3 - "$@" <<'PY'
import struct, sys
path, size, imgsz = sys.argv[1], int(sys.argv[2], 0), int(sys.argv[3], 0)
magic = b"ARM\x64"
if len(sys.argv) > 4:
    magic = b"ARM\x64" if sys.argv[4] == "ok" else b"xxxx"
marker = sys.argv[5].replace("\\0", "\0").encode() if len(sys.argv) > 5 else None
if marker is not None and size < 0x40 + len(marker):
    sys.exit("mkimg: the marker does not fit the file")
buf = bytearray(0x40)
buf[0x10:0x18] = struct.pack("<Q", imgsz)
buf[0x38:0x3c] = magic
with open(path, "wb") as f:
    f.write(buf)
    if marker is not None:
        f.write(marker)
    f.truncate(size)
PY
	echo "$1"
}

report() { echo "FAIL $1"; sed 's/^/  out| /' "$T/out"; sed 's/^/  err| /' "$T/err"; fail=$((fail + 1)); }

# expect_ok DESC WANT_OK WANT_SKIP ARGS...
expect_ok() {
	local what=$1 want_ok=$2 want_skip=$3
	shift 3
	if python3 "$here/mp2s_image_check.py" "$@" >"$T/out" 2>"$T/err"; then
		local got_ok got_skip
		got_ok=$(grep -c '^ok: ' "$T/out" || true)
		got_skip=$(grep -c '^skip: ' "$T/out" || true)
		if [ "$got_ok" = "$want_ok" ] && [ "$got_skip" = "$want_skip" ] \
			&& ! grep -q '^refused' "$T/out" && [ ! -s "$T/err" ]; then
			pass=$((pass + 1))
			return
		fi
		report "$what (want $want_ok ok and $want_skip skip lines, empty stderr)"
	else
		report "$what (exit $?)"
	fi
}

# expect_refuse DESC NEEDLE ARGS...
expect_refuse() {
	local what=$1 needle=$2
	shift 2
	local rc=0
	python3 "$here/mp2s_image_check.py" "$@" >"$T/out" 2>"$T/err" || rc=$?
	if [ "$rc" = 0 ]; then
		report "$what (accepted)"
		return
	fi
	local n
	n=$(grep -c '^refused: ' "$T/err" || true)
	if [ "$rc" = 2 ] && [ "$n" = 1 ] && grep -qF "$needle" "$T/err" && ! grep -q 'refused' "$T/out"; then
		pass=$((pass + 1))
	else
		report "$what (want one 'refused:' naming '$needle' on stderr)"
	fi
}

# expect_usage DESC ARGS...  (exit 2, empty stdout, no "refused:" line)
expect_usage() {
	local what=$1
	shift
	local rc=0
	python3 "$here/mp2s_image_check.py" "$@" >"$T/out" 2>"$T/err" || rc=$?
	if [ "$rc" = 2 ] && [ ! -s "$T/out" ] && ! grep -q 'refused' "$T/err"; then
		pass=$((pass + 1))
	else
		report "$what (want exit 2, empty stdout, no refused line)"
	fi
}

GOOD=$(mkimg "$T/good.bin" 0x2000 0x40000 ok MP2-IWDG-REFRESH)

expect_ok "valid image, --free-bytes and --require-marker given" 6 1 \
	"$GOOD" --free-bytes 0x2200000 --require-marker MP2-IWDG-REFRESH
expect_ok "valid image, free space and marker not asked for" 4 3 \
	"$GOOD"
expect_refuse "bad magic refused" "no ARM64 Image magic at byte 56" \
	"$(mkimg "$T/badmagic.bin" 0x1000 0x1000 bad)"
expect_refuse "image_size one above the window refused" "image takes" \
	"$(mkimg "$T/imgsz-over.bin" 0x1000 0x10000001)"
expect_refuse "image_size 4 GiB or more refused" "4 GiB or more" \
	"$(mkimg "$T/imgsz-4g.bin" 0x1000 0x100000000)"
expect_refuse "file below the 0x40-byte header refused" "below the 0x40-byte Image header" \
	"$(mkimg "$T/tiny.bin" 0x3f 0x1000)"
expect_ok "0x40-byte file with image_size exactly the window accepted (the file is only its first part)" 4 3 \
	"$(mkimg "$T/head-only.bin" 0x40 0x10000000)"
expect_ok "image_size exactly the window accepted" 4 3 \
	"$(mkimg "$T/imgsz-edge.bin" 0x1000 0x10000000)"
expect_refuse "file one above the window refused" "above the 0x10000000-byte window" \
	"$(mkimg "$T/win-over.bin" 0x10000001 0x1000)"
expect_ok "file exactly the window accepted" 4 3 \
	"$(mkimg "$T/win-edge.bin" 0x10000000 0x1000)"
expect_refuse "file one above half the free space refused" "more than half the" \
	"$(mkimg "$T/free-over.bin" 0x200001 0x1000)" --free-bytes 0x400000
expect_ok "file exactly half the free space accepted, --free-bytes decimal" 5 2 \
	"$(mkimg "$T/free-edge.bin" 0x200000 0x1000)" --free-bytes 4194304
expect_refuse "marker absent refused" "does not contain the marker" \
	"$GOOD" --require-marker NOT-IN-THE-IMAGE
expect_ok "multi-word marker found" 5 2 \
	"$(mkimg "$T/multi.bin" 0x1000 0x1000 ok "mp2 iwdg refresh v1")" --require-marker "mp2 iwdg refresh v1"
expect_refuse "missing file refused" "not a readable file" \
	"$T/none.bin"
expect_refuse "a directory refused" "not a readable file" \
	"$T"
expect_usage "non-numeric --free-bytes is a usage error" \
	"$GOOD" --free-bytes abc
expect_usage "empty --require-marker is a usage error" \
	"$GOOD" --require-marker ""

# Lifetime marker "yakogl-lifetime-s=N" NUL (a literal \0 in mkimg's text).
L120=$(mkimg "$T/l120.bin" 0x1000 0x1000 ok 'yakogl-lifetime-s=120\0')
L0=$(mkimg "$T/l0.bin" 0x1000 0x1000 ok 'yakogl-lifetime-s=0\0')
LBAD=$(mkimg "$T/lbad.bin" 0x1000 0x1000 ok 'yakogl-lifetime-s=120\0yakogl-lifetime-s=0\0')
LSAME=$(mkimg "$T/lsame.bin" 0x1000 0x1000 ok 'yakogl-lifetime-s=120\0yakogl-lifetime-s=120\0')
has() { grep -qxF "$1" "$T/out"; }
expect_ok "lifetime: no marker prints skip" 4 3 "$GOOD"
has "skip: lifetime (no marker)" || report "no-marker skip line"
expect_ok "lifetime: no marker, --require-bounded does not refuse" 4 3 "$GOOD" --require-bounded
has "skip: lifetime (no marker)" || report "no-marker skip line with --require-bounded"
expect_ok "lifetime: 120" 5 2 "$L120"
has "ok: lifetime 120 s" || report "lifetime 120 line"
expect_ok "lifetime: 120 with --require-bounded" 5 2 "$L120" --require-bounded
expect_ok "lifetime: two equal markers with --require-bounded" 5 2 "$LSAME" --require-bounded
expect_ok "lifetime: 0 without --require-bounded is reported" 5 2 "$L0"
has "ok: lifetime 0 s" || report "lifetime 0 line"
expect_refuse "lifetime: 0 with --require-bounded refused" "says 0 s" "$L0" --require-bounded
expect_ok "lifetime: disagreeing markers without --require-bounded" 4 3 "$LBAD"
expect_refuse "lifetime: disagreeing markers with --require-bounded refused" "disagree" "$LBAD" --require-bounded
expect_refuse "lifetime: marker check comes first" "does not contain the marker" "$L0" --require-bounded --require-marker NOPE

echo "mp2s_image_check_selftest: $pass checks, $fail failed (pinned: $WANT_CHECKS)"
[ "$fail" = 0 ] && [ "$pass" = "$WANT_CHECKS" ]
