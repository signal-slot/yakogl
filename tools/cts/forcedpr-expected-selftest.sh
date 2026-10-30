#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of forcedpr-expected.py: a 4-layer variant checked by hand, and the
# full 256-layer case 217 with a pinned sha256 of its expected image.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/forcedpr-expected.py"
fails=0

fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
expect() { # name text pattern
	case "$2" in *"$3"*) ;; *) fail "$1: want '$3' in: $2" ;; esac
}
run() { python3 -I "$tool" "$@"; }

# 4 layers, by hand: k=0..3 at (0,0) (173,97) (346,194) (7,291), ranks
# 0 2579 1065 3644, colours (17,101,7) (70,198,200) (123,39,137) (176,136,74).
pix() { run --layers 4 --pixel "$1" "$2"; }
expect "px 0,0 (k0 only)"       "$(pix 0 0)"       "winner 0 word 0xff076511"
expect "px 400,300 (all four)"  "$(pix 400 300)"   "winner 0 word 0xff076511"
expect "px 600,600 (k2 beats k1)" "$(pix 600 600)" "winner 2 word 0xff89277b"
expect "px 10,700 (k3 only)"    "$(pix 10 700)"    "winner 3 word 0xff4a88b0"
expect "px 1000,1000 (clear)"   "$(pix 1000 1000)" "winner None word 0xff000000"

out=$(run --layers 4) || fail "4-layer report exited non-zero"
expect "4 layers sha" "$out" "sha256 df42dda7ca70cb5a054318b652ea9b19b5433ce1761a552df6eccb427e179683"
expect "4 layers hist" "$out" "0:262144    1: 33562    2:209356    3: 98649"
expect "4 layers coverage" "$out" "exact whole-pixel coverage for all 4 layers"

out=$(run) || fail "256-layer report exited non-zero"
expect "256 sha" "$out" "sha256 b33c2dbdfda636dd6df7ecc7b9e5db60ad0780b4575b7cb1c59d2808b86fe0c0"
expect "256 coverage" "$out" "exact whole-pixel coverage for all 256 layers"
expect "256 ranks" "$out" "ranks distinct: yes"
expect "256 depth" "$out" "ordered in 16-bit unorm"

# explain: a FAIL line's pair
e=$(run --layers 4 --explain "FAIL (600,600): got 0x89277b00 want 0xff89277b")
expect "explain want ok" "$e" "expected 0xff89277b"
e=$(run --layers 4 --explain "(600,600) got 0xff000000 want 0xff89277b")
expect "explain clear" "$e" "got is the clear colour"
e=$(run --layers 4 --explain "(600,600) got 0xff076511 want 0xff89277b")
expect "explain layer 0 not covering" "$e" "layer 0 does NOT cover"
e=$(run --layers 4 --explain "(400,300) got 0xff89277b want 0xff076511")
expect "explain behind" "$e" "BEHIND the winner (rank 0)"
e=$(run --layers 4 --explain "(600,600) got 0xffc8c647 want 0xff89277b")
expect "explain layer 1 behind" "$e" "got is layer 1's colour (rank 2579, within 1)"
e=$(run --layers 4 --explain "(600,600) got 0xff89277b want 0xff000000")
expect "explain mismatch" "$e" "EXPECTATION MISMATCH"

if [ "$fails" -eq 0 ]; then
	echo "forcedpr-expected-selftest: ok"
	exit 0
fi
echo "forcedpr-expected-selftest: $fails failure(s)"
exit 1
