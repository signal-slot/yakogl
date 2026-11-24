#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/am62p-gles3-resplit-selftest.sh
#
# Host-only check of tools/cts/am62p-gles3-resplit.py on the real tree and
# mustpass list; its fixtures (a tally, a console log) are made under
# mktemp -d in $TMPDIR. Nothing here is a board run. It checks:
#   - an abort (--refuse, and a STOPPED tally, of row 22): the split of the row
#     into cases with a result / the refusing case / never begun, a SPLIT diff
#     of the generator, TREE_ROWS from the changed generator in memory
#   - an overrun at a *_linear_mipmap_linear case: an isolated one needs no
#     change; one in a row of globs is not isolated by any entry
#   - a precision overrun: a WEIGHT value and the OWN alternative
#   - the console log's per-case spans
#   - a tally that is not STOPPED being left alone
#   - the errors: unknown row, a case the row does not select, a row named
#     twice, no input (exit 2), and that the generator and the tree are
#     never written
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/am62p-gles3-resplit.py
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

WORK=$(mktemp -d "${TMPDIR:-.}/am62p-gles3-resplit.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT
T() { python3 -I "$TOOL" "$@"; }
sums() { sha256sum "$HERE/gen-am62p-gles3-tree.py" "$HERE/am62p-gles3-tree.txt"; }
sums > "$WORK/before"

F=dEQP-GLES3.functional.
C2D=${F}texture.filtering.2d.combinations.linear_linear_clamp_
FIRST22=${C2D}clamp
ISO=${F}texture.filtering.3d.formats.rgb10_a2_linear_mipmap_linear
NONISO=${F}texture.filtering.3d.formats.r11f_g11f_b10f_nearest_mipmap_linear
NOSUCH=${F}buffer.nothing
# the 100th case of row 7 (builtin_functions.precision.add .. floor)
PREC=$(python3 -I - "$HERE" <<'PY'
import fnmatch, sys
h = sys.argv[1]
rows = [l.rstrip('\n') for l in open(h + '/am62p-gles3-tree.txt') if l.strip() and not l.startswith('#')]
m = [l.strip() for l in open(h + '/gles3-mustpass.txt') if l.strip()]
c = [x for x in m if any(fnmatch.fnmatchcase(x, p) for p in rows[6].split(','))]
print(c[99])
PY
)

echo '-- an abort'
T --refuse "22 $FIRST22" > "$WORK/a.txt"
chk 'a proposal for row 22, first case' test -s "$WORK/a.txt"
chk 'nothing had a result, the refusing case is named' bash -c "grep -q 'with a result (0)' '$WORK/a.txt' && grep -q 'refusing case: $FIRST22' '$WORK/a.txt'"
chk 'the never-begun cases are counted' grep -q 'never begun (318)' "$WORK/a.txt"
chk 'a unified diff of the generator adds a SPLIT entry' bash -c "grep -q '^+    .functional.texture.filtering.2d.,' '$WORK/a.txt' && grep -q '^--- a/tools/cts/gen-am62p-gles3-tree.py' '$WORK/a.txt'"
chk 'TREE_ROWS is given from the changed generator' grep -q '^TREE_ROWS=20-21$' "$WORK/a.txt"
{
    echo '# AM62P dEQP-GLES3 image img22-x: STOPPED: dEQP ended the session early (complete=0)'
    echo "Pass ${C2D}clamp"
    echo "Fail ${C2D}mirror"
} > "$WORK/img22-x.tally.txt"
T --tally "$WORK/img22-x.tally.txt" > "$WORK/a2.txt"
chk 'a STOPPED tally: the last case refuses, one case has a result' bash -c "grep -q 'with a result (1)' '$WORK/a2.txt' && grep -q 'refusing case: ${C2D}mirror' '$WORK/a2.txt'"
sed 's/STOPPED.*/complete=1/' "$WORK/img22-x.tally.txt" > "$WORK/img22-y.tally.txt"
T --tally "$WORK/img22-y.tally.txt" > "$WORK/a3.txt"; rc=$?
chk 'a tally that is not STOPPED is left alone (exit 0)' bash -c "[ $rc = 0 ] && grep -q 'not stopped, left alone' '$WORK/a3.txt'"

echo '-- an overrun at a *_linear_mipmap_linear case'
T --overrun "25 $ISO" > "$WORK/o1.txt"
chk 'an isolated case: already an image of its own, no diff' bash -c "grep -q 'already an image of its own' '$WORK/o1.txt' && grep -q 'none: the generator already' '$WORK/o1.txt' && ! grep -q '^+++ ' '$WORK/o1.txt'"
chk 'only that row reruns (0-based 24)' grep -q '^TREE_ROWS=24 ' "$WORK/o1.txt"
T --overrun "23 $NONISO" > "$WORK/o2.txt"
chk 'a case in the row of globs: no entry isolates it, 56 never begun' bash -c "grep -q 'no entry of the' '$WORK/o2.txt' && grep -q 'never begun (56)' '$WORK/o2.txt'"

echo '-- a precision overrun'
T --overrun "7 $PREC" > "$WORK/p.txt"
chk 'the cases that ran are counted' grep -q 'with a result (99)' "$WORK/p.txt"
chk 'WEIGHT is raised in the diff' bash -c "grep -q '^-    .functional.shaders.builtin_functions.precision.: 6,' '$WORK/p.txt' && grep -Eq '^\+    .functional.shaders.builtin_functions.precision.: [0-9]+,' '$WORK/p.txt'"
chk 'the split by function is named' grep -q 'split by function' "$WORK/p.txt"
chk 'the rerun rows are a TREE_ROWS list' grep -Eq '^TREE_ROWS=[0-9][0-9,-]* ?' "$WORK/p.txt"

echo '-- the console log'
{
    echo "10:00:00 Test case '${C2D}clamp'.."
    echo "10:00:07 Test case '${C2D}mirror'.."
    echo "10:00:09 Test case '${C2D}repeat'.."
} > "$WORK/console.log"
T --refuse "22 ${C2D}repeat" --log "$WORK/console.log" > "$WORK/l.txt"
chk 'spans: 2 timed cases, 9 s' grep -q 'log: 2 timed cases, 9 s in all' "$WORK/l.txt"
chk 'the slowest case is listed with its 7 s' grep -q 'slowest    7 s' "$WORK/l.txt"

echo '-- errors'
for t in "99 $FIRST22" "x $FIRST22" "22 $NOSUCH"; do
    T --refuse "$t" >/dev/null 2>"$WORK/err.txt"; rc=$?
    chk "refused: '$t' (exit 2, a message)" bash -c "[ $rc = 2 ] && [ -s '$WORK/err.txt' ]"
done
T --refuse "22 $FIRST22" --overrun "img22-z $FIRST22" >/dev/null 2>&1; rc=$?
chk 'a row named twice is exit 2' [ $rc = 2 ]
T >/dev/null 2>&1; rc=$?
chk 'no input is exit 2' [ $rc = 2 ]

echo '-- nothing was written'
sums > "$WORK/after"
chk 'the generator and the tree are unchanged' cmp -s "$WORK/before" "$WORK/after"

echo "am62p-gles3-resplit self-test: $pass ok, $fail failed"
[ "$fail" = 0 ]
