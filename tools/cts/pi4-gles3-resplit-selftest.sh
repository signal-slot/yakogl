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
#   - a third round (the refused case already the first case of its image,
#     here a one-case refusal image refused again at a new commit):
#     A1 -> A1a, A3 -> A3a
#   - the image headers as tools/cts/check-pi4-gles3-log.py reads them: the
#     refusing case's image reads '# image <LABEL>: 1' in every round, the
#     rest images their own case count, and a synthetic session per round
#     (one tally per image) passes the log checker with 0 missing, 0 aborted
#     and every refusal image read as a refused one-case image
#   - tools/cts/check-gles3-tree.py clean on every result: exit 0 (which also
#     pins every '# image <LABEL>: <count>' to the cases the filter selects)
#     and 0 not matched, 0 matched more than once, 0 empty images
#   - the errors: an unknown label, a case the label does not select, a label
#     refused twice (exit 1, nothing written)
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/pi4-gles3-resplit.py
CHECK=$HERE/check-gles3-tree.py
LOGCHECK=$HERE/check-pi4-gles3-log.py
FIX=$HERE/testdata/pi4-gles3-resplit
P=dEQP-GLES3.functional.
SHA40=$(printf 'a%.0s' $(seq 40))
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
line() { grep -qxF "$1" "$2"; }  # an exact line of tree $2 (the header pins)

WORK=$(mktemp -d "${TMPDIR:-.}/pi4-gles3-resplit.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT

resplit() { python3 -I "$TOOL" --mustpass "$FIX/mustpass.txt" "$@"; }
zero() {  # check-gles3-tree.py clean on $1: exit 0 — every '# image <LABEL>:
          # <count>' header matches the cases its filter selects — and
          # 0 not matched, 0 matched more than once, 0 empty images
    python3 -I "$CHECK" --mustpass "$FIX/mustpass.txt" "$1" > "$WORK/tc.log" 2>&1 || return 1
    case $(tail -1 "$WORK/tc.log") in
        *' 0 not matched, 0 matched more than once, 0 empty images') return 0 ;;
    esac
    return 1
}
logcheck() {  # $1 tree, $2 tag: a synthetic session for the tree (one tally
              # per image: the refusal images — whose '# image' note names
              # "alone: dEQP stopped" — get the 1 begun / 1 with a result
              # complete=0 summary of a refused case, every other image a
              # complete summary with its real case count, as printed by
              # check-gles3-tree.py), and tools/cts/check-pi4-gles3-log.py
              # on it. Passes when the checker exits 0 and names every
              # refusal image 'refused (one-case image)': the checker reads
              # the case count of a label from its '# image <LABEL>: <N>'
              # header and accepts the refused summary only for a label its
              # header says 1 — so a refusal image whose header did not read
              # '# image <LABEL>: 1' fails here as 'aborted'.
    local d=$WORK/log-$2 out=$WORK/log-$2.out lab missed=0
    rm -rf "$d"; mkdir -p "$d"
    printf 'commit: %s\n' "$SHA40" > "$d/README.md"
    python3 -I "$CHECK" --mustpass "$FIX/mustpass.txt" "$1" |
    while read -r lab n; do
        case $lab in ''|*[!A-Za-z0-9]*) continue ;; esac
        case $n in ''|*[!0-9]*) continue ;; esac
        if grep -q "^# image $lab: .*alone: dEQP stopped" "$1"; then
            printf 'deqp: complete=0\ndeqp: summary: 1 cases begun, 1 with a result, 0 without\n'
        else
            printf 'deqp: complete=1\ndeqp: summary: %s cases begun, %s with a result, 0 without\n' "$n" "$n"
        fi > "$d/$lab.tally.txt"
    done
    python3 -I "$LOGCHECK" --tree "$1" --mustpass "$FIX/mustpass.txt" \
        --commit "$SHA40" "$d" > "$out" 2>&1 || return 1
    while read -r lab; do
        grep -qx "info: $lab: refused (one-case image)" "$out" || missed=1
    done < <(sed -n 's/^# image \([A-Za-z0-9]*\): .*alone: dEQP stopped.*/\1/p' "$1")
    return $missed
}

echo '-- two aborts in one round, from the tallies'
resplit "$FIX/tree.txt" --size 2 --at abc12345 --tally "$FIX/A.tally.txt" --tally "$FIX/B.tally.txt" \
    --tally "$FIX/D.tally.txt" -o "$WORK/r1.txt"
chk 'tally round equals round1.expected.txt (mid-label abort A, first-case abort B)' cmp -s "$WORK/r1.txt" "$FIX/round1.expected.txt"
chk 'a mid-label abort keeps the cases that ran in A' grep -q '^# image A: 2 ' "$WORK/r1.txt"
chk 'the refusing case is a one-case image' grep -q '^# image A1: 1 ' "$WORK/r1.txt"
chk 'round 1: the refusal image header reads exactly "# image A1: 1 ..."' \
    line '# image A1: 1 (g1.a.3 alone: dEQP stopped at it in A at abc12345)' "$WORK/r1.txt"
chk 'an abort at the first case drops the label' bash -c "! grep -q '^# image B:' '$WORK/r1.txt' && grep -q '^# image B1: 1 ' '$WORK/r1.txt'"
chk 'round 1: the first-case refusal image header reads exactly "# image B1: 1 ..."' \
    line '# image B1: 1 (g1.b.x.1 alone: dEQP stopped at it in B at abc12345)' "$WORK/r1.txt"
chk 'round 1: the rest images carry their own case counts in the headers' bash -c \
    "grep -qxF '# image A2: 2 (the rest of A after g1.a.3, never begun at abc12345)' '$WORK/r1.txt' && grep -qxF '# image A3: 1 (the rest of A after g1.a.3, never begun at abc12345)' '$WORK/r1.txt' && grep -qxF '# image B2: 2 (the rest of B after g1.b.x.1, never begun at abc12345)' '$WORK/r1.txt' && grep -qxF '# image B3: 2 (the rest of B after g1.b.x.1, never begun at abc12345)' '$WORK/r1.txt'"
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
chk 'round 2: the refusing case was A2 first case: A2 goes, A2a reads "# image A2a: 1"' bash -c \
    "! grep -q '^# image A2:' '$WORK/r2.txt' && grep -qxF '# image A2a: 1 (g1.a.4 alone: dEQP stopped at it in A2 at def67890)' '$WORK/r2.txt' && grep -qxF '# image A2b: 1 (the rest of A2 after g1.a.4, never begun at def67890)' '$WORK/r2.txt'"
chk 'round 2: a last-case refusal keeps B3 with what ran and B3a reads "# image B3a: 1"' bash -c \
    "grep -qxF '# image B3: 1 (the cases of B3 that ran before g1.b.z at def67890; g1.b.z alone in B3a)' '$WORK/r2.txt' && grep -qxF '# image B3a: 1 (g1.b.z alone: dEQP stopped at it in B3 at def67890)' '$WORK/r2.txt'"
chk 'the first-round labels not refused (A, A1, A3, B1, B2) keep their cases' \
    bash -c "for l in A A1 A3 B1 B2; do diff <(grep -A1 \"^# image \$l:\" '$FIX/round1.expected.txt' | tail -1) <(grep -A1 \"^# image \$l:\" '$WORK/r2.txt' | tail -1) >/dev/null || exit 1; done"
chk 'the header count is 11' grep -q ' in 11 images,' "$WORK/r2.txt"
chk 'check-gles3-tree.py is clean (round 2)' zero "$WORK/r2.txt"

echo '-- a third round: the refusing case is already the first case of its image'
resplit "$FIX/round2.expected.txt" --size 2 --at fed98765 \
    --refuse "A1 ${P}g1.a.3" --refuse "A3 ${P}g1.a.6" -o "$WORK/r3.txt"
chk 'round 3 equals round3.expected.txt' cmp -s "$WORK/r3.txt" "$FIX/round3.expected.txt"
chk 'the re-refused case gets a new label and the same one-case header (A1 -> A1a)' \
    line '# image A1a: 1 (g1.a.3 alone: dEQP stopped at it in A1 at fed98765)' "$WORK/r3.txt"
chk 'round 3: A1 is gone from the tree' bash -c "! grep -q '^# image A1:' '$WORK/r3.txt'"
chk 'round 3: the rest one-case image re-refused too (A3 -> A3a, still ": 1")' \
    line '# image A3a: 1 (g1.a.6 alone: dEQP stopped at it in A3 at fed98765)' "$WORK/r3.txt"
chk 'the header count is 11' grep -q ' in 11 images,' "$WORK/r3.txt"
chk 'check-gles3-tree.py is clean (round 3)' zero "$WORK/r3.txt"

echo '-- the trees as tools/cts/check-pi4-gles3-log.py reads them'
chk 'round 1: the checker exits 0 and reads A1 and B1 as refused one-case images' \
    logcheck "$WORK/r1.txt" r1
chk 'round 2: the same, the second-round refusals (A2a, B3a) included' \
    logcheck "$WORK/r2.txt" r2
chk 'round 3: the same, the re-refused one-case images (A1a, A3a) included' \
    logcheck "$WORK/r3.txt" r3

echo '-- errors'
for t in "X ${P}g1.a.3" "A ${P}g2.c.1" "A ${P}g1.a.3 extra"; do
    resplit "$FIX/tree.txt" --refuse "$t" -o "$WORK/e.txt" 2>"$WORK/err.txt"; rc=$?
    chk "refused: '$t' (exit 1, nothing written)" bash -c "[ $rc = 1 ] && [ ! -e '$WORK/e.txt' ] && [ -s '$WORK/err.txt' ]"
done
resplit "$FIX/tree.txt" --refuse "A ${P}g1.a.3" --refuse "A ${P}g1.a.4" -o "$WORK/e.txt" 2>/dev/null; rc=$?
chk 'a label refused twice is an error' bash -c "[ $rc = 1 ] && [ ! -e '$WORK/e.txt' ]"

echo "pi4-gles3-resplit self-test: $pass ok, $fail failed"
[ "$fail" = 0 ]
