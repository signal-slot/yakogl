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

# ---- a session built from several split rounds (testdata/pi4-session-tally-rounds)
# mustpass: shaders.g.c1-c4, texture.t.c1-c2. Round 1: S01 (all of shaders.g)
# passes c1, c2 is refused and dEQP stops. Round 2: S01a is c2 alone (refused,
# the run aborts after its one ResourceError), S01b is c3,c4 and stops at c3.
# Round 3: S01b1 is c3 alone (refused), S01b2 is c4. T01 is the texture group.
RD=$HERE/testdata/pi4-session-tally-rounds
export PI4_TREE=$RD/tree.txt PI4_MUSTPASS=$RD/mustpass.txt
G=dEQP-GLES3.functional.shaders.g
X=dEQP-GLES3.functional.texture.t
# console LABEL FILTER [STATUS CASE]...: complete=0 when the last case is a ResourceError
console() {
    local label=$1 filt=$2 n=0 last=Pass st c
    shift 2
    {
        echo "12:00:00 boot"
        echo "deqp: case filter '$filt'"
        while [ $# -gt 0 ]; do
            st=$1; c=$2; shift 2; n=$((n + 1)); last=$st
            echo "deqp: $st $c"
        done
        if [ "$last" = ResourceError ]; then
            echo "deqp: done: 0 passed, $n failed, 0 not supported, 0 warnings, complete=0"
        else
            echo "deqp: done: $n passed, 0 failed, 0 not supported, 0 warnings, complete=1"
        fi
        echo "deqp: summary: $n cases begun, $n with a result, 0 without"
    } > "$S2/$label.console.txt"
}
table() { python3 "$HERE/deqp-uart-tally.py" --table --mustpass "$RD/mustpass.txt" "$S2"/*.console.txt; }
# the none column of the TOTAL row, as `awk '/^TOTAL/{print $NF}'` reads it
none_of() { table | awk '/^TOTAL/{print $NF}'; }
S2=$T/rounds
mkdir "$S2"
# case 1: a one-case image that aborts after its single ResourceError has a result
console S01a "$G.c2" ResourceError $G.c2
out=$(bash "$HERE/pi4-session-tally.sh" "$S2" 2>&1)
case "$out" in *"S01a: aborted"*) ok "one-case image: tally.sh reports it aborted" ;; *) bad "S01a: $out" ;; esac
# (the scope of the table is the case filter, so a lone one-case image is a whole session of one case)
[ "$(none_of)" = 0 ] && ok "one-case ResourceError image alone: none is 0" || bad "one-case image: none=$(none_of)"
grep -q "^ResourceError $G.c2" "$S2/S01a.tally.txt" && ok "its ResourceError is listed in the tally" || bad "S01a tally lacks the ResourceError"
# case 2: the aborted image and all its later split images
console S01 "$G.*" Pass $G.c1 ResourceError $G.c2
[ "$(none_of)" = 2 ] && ok "S01 aborted, S01a ran: none is 2 (c3, c4 never begun)" || bad "S01+S01a: none=$(none_of)"
console S01b "$G.c3,$G.c4" ResourceError $G.c3
console S01b1 "$G.c3" ResourceError $G.c3
[ "$(none_of)" = 1 ] && ok "S01b1 ran, S01b2 not yet: none is 1 (c4)" || bad "S01b1 without S01b2: none=$(none_of)"
console S01b2 "$G.c4" Pass $G.c4
console T01 "$X.*" Pass $X.c1 Pass $X.c2
[ "$(none_of)" = 0 ] && ok "every image of the rounds ran: none is 0" || bad "all rounds: none=$(none_of)"
out=$(bash "$HERE/pi4-session-tally.sh" "$S2" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "tally.sh tallies the round session" || bad "round session rc=$rc: $out"
for l in S01 S01a S01b S01b1; do
    case "$out" in *"$l: aborted"*) ok "$l reported aborted" ;; *) bad "$l not aborted: $out" ;; esac
done
case "$out" in *"S01b2: 1 pass, 0 fail"*) ok "S01b2 complete" ;; *) bad "S01b2: $out" ;; esac
# case 3: a missing image leaves none > 0
mv "$S2/S01b2.console.txt" "$T/S01b2.hold"
[ "$(none_of)" = 1 ] && ok "S01b2 missing: none is 1" || bad "missing image: none=$(none_of)"
# case 4: the TOTAL row is read by awk '/^TOTAL/{print $NF}': the last column is none
tot=$(table | grep '^TOTAL')
[ "$(printf '%s\n' "$tot" | awk '{print $7}')" = 1 ] && ok "TOTAL row: 7th and last field is none" || bad "TOTAL row: $tot"
table | grep -q '^group .* none$' && ok "table header ends in none" || bad "header does not end in none"
mv "$T/S01b2.hold" "$S2/S01b2.console.txt"
[ "$(none_of)" = 0 ] && ok "S01b2 back: TOTAL none is 0" || bad "restored: none=$(none_of)"
unset PI4_TREE PI4_MUSTPASS

if [ $fail -eq 0 ]; then
    echo "pi4-session-tally-selftest: all checks passed"
else
    echo "pi4-session-tally-selftest: $fail failed"
    exit 1
fi
