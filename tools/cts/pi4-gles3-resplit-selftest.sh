#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pi4-gles3-resplit-selftest.sh
#
# Host-only check of tools/cts/pi4-gles3-resplit.py on the small tree in
# tools/cts/testdata/pi4-gles3-resplit/ (tree.txt over mustpass.txt, 15 cases
# in 4 images A-D: a glob over six cases, a glob over a subtree, a list of
# named cases, a glob left alone). Nothing here is a real run. It checks:
#   - an abort mid-label (A at g1.a.3: A keeps the two that ran, A1 holds the
#     refusing case alone, A2-A3 the three after it, sized by --size 2)
#   - an abort at the first case (B at g1.b.x.1: B has nothing that ran, so it
#     is dropped; B1, B2.. follow)
#   - two aborts in one round, from tally files and from a --refuse-file list
#     (the same tree), a tally that says DONE being left alone
#   - filters as the tree writes them: globs in, "<prefix>.*" out where the
#     whole subtree is in the image (glob.expected.txt, B2 = x.2, y.*, z)
#   - the header image count, and every line outside the split labels kept
#     byte for byte (here C and D, and the header text)
#   - a second round (A2 refused at its first case, B3 at its last): the
#     letter / number alternation of new labels (A2 -> A2a, A2b)
#   - tools/cts/check-gles3-tree.py reporting 0 not matched, 0 matched more than
#     once and 0 empty images on every result
#   - the errors: an unknown label, a case the label does not select, a label
#     refused twice (exit 1, nothing written)
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/pi4-gles3-resplit.py
CHECK=$HERE/check-gles3-tree.py
FIX=$HERE/testdata/pi4-gles3-resplit
P=dEQP-GLES3.functional.
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

WORK=$(mktemp -d "${TMPDIR:-.}/pi4-gles3-resplit.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT

resplit() { python3 -I "$TOOL" --mustpass "$FIX/mustpass.txt" "$@"; }
treecheck() { python3 -I "$CHECK" --mustpass "$FIX/mustpass.txt" "$1" 2>&1 | tail -1; }
zero() {  # 0 not matched, 0 matched more than once, 0 empty images
    case $(treecheck "$1") in *' 0 not matched, 0 matched more than once, 0 empty images') return 0 ;; esac
    return 1
}

echo '-- two aborts in one round, from the tallies'
resplit "$FIX/tree.txt" --size 2 --at abc12345 --tally "$FIX/A.tally.txt" --tally "$FIX/B.tally.txt" \
    --tally "$FIX/D.tally.txt" -o "$WORK/r1.txt"
chk 'tally round equals round1.expected.txt (mid-label abort A, first-case abort B)' cmp -s "$WORK/r1.txt" "$FIX/round1.expected.txt"
chk 'a mid-label abort keeps the cases that ran in A' grep -q '^# image A: 2 ' "$WORK/r1.txt"
chk 'the refusing case is a one-case image' grep -q '^# image A1: 1 ' "$WORK/r1.txt"
chk 'an abort at the first case drops the label' bash -c "! grep -q '^# image B:' '$WORK/r1.txt' && grep -q '^# image B1: 1 ' '$WORK/r1.txt'"
chk 'the header count is 9' grep -q ' in 9 images,' "$WORK/r1.txt"
chk 'check-gles3-tree.py is clean (round 1)' zero "$WORK/r1.txt"
chk 'the images that were not split (C, D) are kept byte for byte' \
    bash -c "diff <(sed -n '/^# image C:/,\$p' '$FIX/tree.txt') <(sed -n '/^# image C:/,\$p' '$WORK/r1.txt') >/dev/null"
chk 'DONE tally D is left alone' bash -c "grep -q '^dEQP-GLES3.functional.g3.\*\$' '$WORK/r1.txt'"

echo '-- the same by an explicit list'
resplit "$FIX/tree.txt" --size 2 --at abc12345 --refuse-file "$FIX/refuse.txt" -o "$WORK/r1b.txt"
chk '--refuse-file gives the same tree' cmp -s "$WORK/r1b.txt" "$WORK/r1.txt"
resplit "$FIX/tree.txt" --size 2 --at abc12345 --refuse "A ${P}g1.a.3" --refuse "B ${P}g1.b.x.1" > "$WORK/r1c.txt"
chk '--refuse to stdout gives the same tree' cmp -s "$WORK/r1c.txt" "$WORK/r1.txt"

echo '-- globs in the tree, globs out'
resplit "$FIX/tree.txt" --at abc12345 --tally "$FIX/B.tally.txt" -o "$WORK/g.txt"
chk 'default size: one rest image, a subtree written as a glob' cmp -s "$WORK/g.txt" "$FIX/glob.expected.txt"
chk 'B2 filter is x.2, y.*, z' grep -qx "${P}g1.b.x.2,${P}g1.b.y.*,${P}g1.b.z" "$WORK/g.txt"
chk 'an unsplit glob row (A) is untouched' grep -qx "${P}g1.a.\*" "$WORK/g.txt"
chk 'check-gles3-tree.py is clean (globs)' zero "$WORK/g.txt"

echo '-- a second round'
resplit "$FIX/round1.expected.txt" --size 2 --at def67890 --refuse "A2 ${P}g1.a.4" --refuse "B3 ${P}g1.b.z" -o "$WORK/r2.txt"
chk 'round 2 equals round2.expected.txt' cmp -s "$WORK/r2.txt" "$FIX/round2.expected.txt"
chk 'a label ending in a number takes a letter (A2 -> A2a, A2b)' grep -q '^# image A2b: 1 ' "$WORK/r2.txt"
chk 'the first-round labels not refused (A, A1, A3, B1, B2) keep their cases' \
    bash -c "for l in A A1 A3 B1 B2; do diff <(grep -A1 \"^# image \$l:\" '$FIX/round1.expected.txt' | tail -1) <(grep -A1 \"^# image \$l:\" '$WORK/r2.txt' | tail -1) >/dev/null || exit 1; done"
chk 'the header count is 11' grep -q ' in 11 images,' "$WORK/r2.txt"
chk 'check-gles3-tree.py is clean (round 2)' zero "$WORK/r2.txt"

echo '-- errors'
for t in "X ${P}g1.a.3" "A ${P}g2.c.1" "A ${P}g1.a.3 extra"; do
    resplit "$FIX/tree.txt" --refuse "$t" -o "$WORK/e.txt" 2>"$WORK/err.txt"; rc=$?
    chk "refused: '$t' (exit 1, nothing written)" bash -c "[ $rc = 1 ] && [ ! -e '$WORK/e.txt' ] && [ -s '$WORK/err.txt' ]"
done
resplit "$FIX/tree.txt" --refuse "A ${P}g1.a.3" --refuse "A ${P}g1.a.4" -o "$WORK/e.txt" 2>/dev/null; rc=$?
chk 'a label refused twice is an error' bash -c "[ $rc = 1 ] && [ ! -e '$WORK/e.txt' ]"

echo "pi4-gles3-resplit self-test: $pass ok, $fail failed"
[ "$fail" = 0 ]
