#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pi4-session-tally-selftest.sh
#
# Hermetic check of tools/cts/pi4-session-tally.sh on a scratch copy of the
# testdata consoles: a tally per console, no absolute path in it, an unknown
# label refused with nothing written, no other file changed, and a cut-shaped
# session (an aborted label plus the new label of its moved cases) tallied
# against the tree as it is now.
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

# ---- a cut-shaped session (the recipe section 6 shape): the image FO6 ran the
# whole scissor filter — four synthetic cases — and stopped at its second with
# a ResourceError. The tree was re-split after that run: FO6 keeps the two
# cases that ran (its block now names fewer cases than the console began with)
# and the two that never began moved to the new label FO6a — a digit-ending
# label takes a letter, as the table does. Both consoles sit in one session
# directory, the cut tree beside them.
CT=$T/cut
CS=$CT/session
mkdir -p "$CS"
SC=dEQP-GLES3.functional.fragment_ops.scissor.clear_int_buffer
cat > "$CT/tree.txt" <<EOF
# synthetic tree in the real shape: a count note and one filter line per label
# image FO6: 2 (cut after the abort trigger ${SC}.a2)
${SC}.a1,${SC}.a2

# image FO6a: 2 (the two cases moved out of FO6)
${SC}.a3,${SC}.a4
EOF
cat > "$CT/mustpass.txt" <<EOF
${SC}.a1
${SC}.a2
${SC}.a3
${SC}.a4
EOF
{
    echo "deqp: case filter '${SC}.*'"    # what FO6 ran: the whole filter, 4 cases
    echo "deqp: Pass ${SC}.a1"
    echo "deqp: ResourceError ${SC}.a2"   # the abort trigger, FO6's last begun case
    echo "deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=0"
    echo "deqp: summary: 2 cases begun, 2 with a result, 0 without"
} > "$CS/FO6.console.txt"
{
    echo "deqp: case filter '${SC}.a3,${SC}.a4'"
    echo "deqp: Pass ${SC}.a3"
    echo "deqp: Pass ${SC}.a4"
    echo "deqp: done: 2 passed, 0 failed, 0 not supported, 0 warnings, complete=1"
    echo "deqp: summary: 2 cases begun, 2 with a result, 0 without"
} > "$CS/FO6a.console.txt"
# the cut as the current tree states it: FO6's block is the two cases that ran,
# while its console still carries the pre-cut filter that selects all four
fo6block=$(awk '/^# image FO6:/{getline; print; exit}' "$CT/tree.txt")
[ "$fo6block" = "${SC}.a1,${SC}.a2" ] && ok "FO6's tree block names the two cases it ran" \
    || bad "FO6 block: $fo6block"
case "$fo6block" in *${SC}.a3*|*${SC}.a4*) bad "FO6's block still holds a moved case" ;;
    *) ok "FO6's block lacks the two moved cases" ;; esac
grep -qF "case filter '${SC}.*'" "$CS/FO6.console.txt" && ok "FO6's console ran the pre-cut filter" \
    || bad "FO6 console: $(head -n 1 "$CS/FO6.console.txt")"
# every tally of the session uses the label's current tree block: the tallied
# set is exactly the two labels the tree has now, FO6 stays an aborted cut and
# FO6a is complete; FO6a's own table counts only its filter's two cases (its
# TOTAL is 2 0 0 0), and over the whole session each case of the cut tree has
# a result exactly once — the moved cases come from FO6a, not as NoResult of
# FO6 — so the session TOTAL's none is 0
out=$(PI4_TREE=$CT/tree.txt PI4_MUSTPASS=$CT/mustpass.txt bash "$HERE/pi4-session-tally.sh" "$CS" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "cut session tallied" || bad "cut session rc=$rc: $out"
case "$out" in *"FO6: aborted"*) ok "cut FO6 reported aborted" ;; *) bad "FO6: $out" ;; esac
case "$out" in *"FO6a: 2 pass, 0 fail"*) ok "FO6a complete: the moved cases ran" ;; *) bad "FO6a: $out" ;; esac
[ "$(ls "$CS" | wc -l)" -eq 4 ] && ok "both labels of the current tree got a tally, nothing else" \
    || bad "cut files: $(ls "$CS")"
grep -q "^ResourceError ${SC}.a2" "$CS/FO6.tally.txt" && ok "FO6's tally keeps its abort trigger" \
    || bad "FO6 tally lacks ${SC}.a2"
grep -q STOPPED "$CS/FO6.tally.txt" && ok "FO6's tally says STOPPED (complete=0)" \
    || bad "FO6 tally has no STOPPED"
grep -Eq '^TOTAL[[:space:]]+2[[:space:]]+0[[:space:]]+0[[:space:]]+0$' "$CS/FO6a.tally.txt" \
    && ok "FO6a's tally totals only its own two moved cases" \
    || bad "FO6a TOTAL: $(grep TOTAL "$CS/FO6a.tally.txt")"
cutnone=$(python3 "$HERE/deqp-uart-tally.py" --table --mustpass "$CT/mustpass.txt" "$CS"/*.console.txt \
    | awk '/^TOTAL/{print $NF}')
[ "$cutnone" = 0 ] && ok "every case of the cut tree is tallied once: session none is 0" \
    || bad "cut session: none=$cutnone"
# no absolute path in a cut tally or in the tally output: the worker's HOME,
# this checkout, the scratch directory and TMPDIR (a pattern shorter than four
# characters is skipped: a HOME of '/' would match every line)
leak=0
for pat in "${HOME:-/nonexistent-home}" "$(cd "$HERE/../.." && pwd)" "$T" "${TMPDIR:-/nonexistent-tmp}"; do
    [ "${#pat}" -ge 4 ] && { grep -qF -- "$pat" "$CS"/*.tally.txt && leak=1; printf '%s\n' "$out" | grep -qF -- "$pat" && leak=1; }
done
[ $leak -eq 0 ] && ok "no HOME, checkout or TMPDIR path in the cut tallies or their output" \
    || bad "an absolute path leaked into a cut tally"
# the same directory against the tree as it was before the re-split: FO6 alone
# with the whole filter, no FO6a. A label the tree lacks is an error that names
# it, before anything is written — never a silent skip of its console
printf '# image FO6: 4 (pre-split)\n%s.*\n' "$SC" > "$CT/tree-pre.txt"
rm -f "$CS"/*.tally.txt
out=$(PI4_TREE=$CT/tree-pre.txt PI4_MUSTPASS=$CT/mustpass.txt bash "$HERE/pi4-session-tally.sh" "$CS" 2>&1); rc=$?
[ $rc -ne 0 ] && ok "a label missing from the tree is an error, not a skip" \
    || bad "FO6a accepted though the pre-split tree lacks it"
case "$out" in *FO6a*) ok "the error names FO6a" ;; *) bad "no FO6a in: $out" ;; esac
[ -z "$(ls "$CS" | grep tally)" ] && ok "the refused session wrote nothing" || bad "files after refusal: $(ls "$CS")"
# and the reverse cut: a tree holding only the moved label FO6a refuses the
# same directory, naming the aborted label FO6 the tree lacks
printf '# image FO6a: 2\n%s.a3,%s.a4\n' "$SC" "$SC" > "$CT/tree-moved.txt"
out=$(PI4_TREE=$CT/tree-moved.txt PI4_MUSTPASS=$CT/mustpass.txt bash "$HERE/pi4-session-tally.sh" "$CS" 2>&1); rc=$?
[ $rc -ne 0 ] && ok "a tree without the cut label is an error too" || bad "FO6 accepted though the tree lacks it"
case "$out" in *"label 'FO6'"*) ok "the error names FO6" ;; *) bad "no FO6 in: $out" ;; esac
[ -z "$(ls "$CS" | grep tally)" ] && ok "still nothing written" || bad "files: $(ls "$CS")"

if [ $fail -eq 0 ]; then
    echo "pi4-session-tally-selftest: all checks passed"
else
    echo "pi4-session-tally-selftest: $fail failed"
    exit 1
fi
