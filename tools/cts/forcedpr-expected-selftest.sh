#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of forcedpr-expected.py: a 4-layer variant checked by hand, case
# 217 at its default layer count (CONFIG_GL_TEST_FORCEDPR_LAYERS, the tool's
# default) and at the 256 layers of the first board run, each with a pinned
# sha256 of its expected image, and the explain mode past 256 layers (where a
# colour names every 256th layer).
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

# the default: the case's CONFIG_GL_TEST_FORCEDPR_LAYERS (the AM62P crossing,
# tools/cts/log/2026-11-02-pr-217probe), the step the committed case ends at
kdef=$(sed -n '/^config GL_TEST_FORCEDPR_LAYERS/,/^config /{s/^\tdefault \([0-9]*\)$/\1/p}' \
	"$here/../../driver/samples/zephyr/gl_test/Kconfig")
cdef=$(sed -n 's/^#define CONFIG_GL_TEST_FORCEDPR_LAYERS \([0-9]*\)$/\1/p' "$here/../../driver/tools/pvrlab/gl_test.c")
tdef=$(sed -n 's/^DEFAULT_LAYERS = \([0-9]*\)$/\1/p' "$tool")
[ "$kdef" = 1220 ] && [ "$cdef" = 1220 ] && [ "$tdef" = 1220 ] ||
	fail "the default layer count differs: Kconfig '$kdef', gl_test.c '$cdef', forcedpr-expected.py '$tdef' (want 1220)"
out=$(run) || fail "default-layer report exited non-zero"
expect "default layers" "$out" "forcedpr-expected: 1220 layers, 1024x1024"
expect "1220 sha" "$out" "sha256 8a0aee698cbd38f3c83f576b7612d37374c74da63bf67f299efa0bf724794ccf"
expect "1220 coverage" "$out" "exact whole-pixel coverage for all 1220 layers"
expect "1220 ranks" "$out" "ranks distinct: yes"
expect "1220 depth" "$out" "ordered in 16-bit unorm"
expect "1220 winners" "$out" "layers winning no pixel: 1120"

# the first board run's 256 layers (tools/cts/log/2026-10-31-pr-negative)
out=$(run --layers 256) || fail "256-layer report exited non-zero"
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
# past 256 layers a colour names every 256th layer: layer 1's colour at a
# pixel none of 1, 257, 513, 769 covers; layer 2's where all four congruent
# layers cover and lose to the winner
e=$(run --layers 1000 --explain "(10,10) got 0xffc8c647 want 0xff076511")
expect "explain class none covers" "$e" "4 of them do NOT cover the pixel: [1, 257, 513, 769]"
expect "explain class no cover" "$e" "no layer of that colour covers the pixel"
e=$(run --layers 1000 --explain "(600,600) got 0xff89277b want 0xff0ecc44")
expect "explain class winner" "$e" "winner layer 711 (rank 5)"
expect "explain class behind" "$e" "got is layer 770's colour (rank 725); it is BEHIND the winner (rank 5)"

if [ "$fails" -eq 0 ]; then
	echo "forcedpr-expected-selftest: ok"
	exit 0
fi
echo "forcedpr-expected-selftest: $fails failure(s)"
exit 1
