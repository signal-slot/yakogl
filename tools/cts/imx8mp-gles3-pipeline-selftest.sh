#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-pipeline-selftest.sh - synthetic i.MX8MP dEQP-GLES3 slice records through the
# record layout the board scripts write (tools/board.sh imx8mp tree/tally via imx8mp-gles3-board.sh,
# log/README.md naming) into imx8mp-gles3-merge.py --check / --notpass / --totals.
#
#   TMPDIR=<dir> tools/cts/imx8mp-gles3-pipeline-selftest.sh
#
# Host only; no board, no driver. Everything is written under $TMPDIR (never /tmp) and removed at the end.
# Exit 0 when the chain agrees, 1 on any failure. (M4, --notpass naming only Fail and NoResult, is closed:
# the slices carry a cases.tsv and the rows must name NotSupported/QualityWarning/Crash/Timeout.)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
MERGE=$HERE/imx8mp-gles3-merge.py
PI4=$ROOT/docs/es3/pi4-gles3-fails.tsv
: "${TMPDIR:?set TMPDIR to a directory (synthetic records are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
work=$(mktemp -d "$TMPDIR/imx8mp-pipeline-XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT

fails=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; fails=$((fails + 1)); }
expect() { if [ "$2" = 0 ]; then ok "$1"; else bad "$1"; fi; }

C1=0123456789abcdef0123456789abcdef01234567
C2=fedcba9876543210fedcba9876543210fedcba98

# The generator: slices of the real table and mustpass list, in the layout of board.sh results_tsv.
cat > "$work/gen.py" <<'PY'
import importlib.util, os, sys
root, here, commit, mode = sys.argv[1:5]
spec = importlib.util.spec_from_file_location('merge', os.path.join(here, 'imx8mp-gles3-merge.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
table = m.load_table(os.path.join(here, 'imx8mp-gles3-tree.txt'))
per = m.entry_cases(table, os.path.join(here, 'gles3-mustpass.txt'))
n = len(table)
assert n == 133, n
cut = [(0, 45), (45, 90), (90, n)]
names = ['2026-11-10-imx8mp-gles3-%s-rows%03d-%03d' % (commit[:8], a + 1, b) for a, b in cut]

STATUS = {}

def row(i):
    c = per[i]; k = len(c)
    # verdict mix: Pass everywhere, then per entry a few non-pass statuses
    fail, ns, other, nr, named = 0, 0, 0, 0, []
    st = {}                             # the per-case statuses cases.tsv records
    if i % 7 == 1 and k >= 3:           # Fail (named) + NotSupported
        fail, ns, named = 1, 1, [c[1]]
        st[c[1]], st[c[2]] = 'Fail', 'NotSupported'
    if i % 11 == 2 and k >= 2:          # QualityWarning (counted as other)
        other = 1
        st[c[0]] = 'QualityWarning'
    if i == 5:                          # Crash: dEQP stopped, rest never began
        fail, nr, named = 1, max(0, k // 2), [c[0]]
        st[c[0]] = 'Crash'
    if i == 9:                          # Timeout (other) then stopped
        other, nr = 1, max(0, k // 3)
        st[c[0]] = 'Timeout'
    STATUS[i] = st
    begun = k - nr
    passed = begun - fail - ns - other
    return (i, table[i], begun, passed, fail, ns, other, nr, ';'.join(named) or '-')

def write(d, rows, commit_text):
    os.makedirs(d)
    with open(os.path.join(d, 'README.md'), 'w') as f:
        f.write('# i.MX8MP dEQP-GLES3 slice (synthetic)\n\norigin/main %s\n' % commit_text)
        for r in rows:
            f.write('board.sh imx8mp e%02d rc=0 verdict=done resets=absent\n' % r[0])
    with open(os.path.join(d, 'results.tsv'), 'w') as f:
        f.write("# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names (';'-separated)\n")
        for r in rows:
            f.write('\t'.join(str(x) for x in r) + '\n')
    with open(os.path.join(d, 'cases.tsv'), 'w') as f:
        for r in rows:
            for case, status in STATUS.get(r[0], {}).items():
                f.write('%s\t%s\n' % (case, status))

for (a, b), nm in zip(cut, names):
    d = os.path.join(root, mode, nm)
    rows = []
    for i in range(n):
        if a <= i < b:
            rows.append(row(i))
        elif mode == 'board':           # tally walks tree.txt: rows not run are all '-'
            rows.append((i, table[i], '-', '-', '-', '-', '-', '-', '-'))
    write(d, rows, commit)
PY

py() { python3 -I "$@"; }
for mode in board committed; do
	py "$work/gen.py" "$work" "$HERE" "$C1" "$mode" || { echo "generator failed" >&2; exit 2; }
done
mp() { python3 "$MERGE" "$@"; }

mapfile -t S < <(ls -d "$work"/committed/*/)
mapfile -t B < <(ls -d "$work"/board/*/)
[ ${#S[@]} = 3 ] && [ ${#B[@]} = 3 ] || { echo "generator wrote an unexpected layout" >&2; exit 2; }
echo "slices: $(for d in "${S[@]}"; do basename "$d"; done | paste -sd' ')"

echo "== --check on the three slices (each holding only its own rows)"
mp --check "${S[@]}" > "$work/c1.out" 2>&1; rc=$?
expect "check exits 0 on the full 133-entry set, one commit" "$rc"
grep -q 'with a record: ' "$work/c1.out" && grep -q '133 entries' "$work/c1.out"; expect "check reports 133 entries" $?
mp --check --partial "${S[0]}" "${S[1]}" > "$work/c1s.out" 2>&1; rc=$?
expect "check --partial exits 0 on two slices of three" "$rc"

echo "== --check, the failing inputs"
# mixed commit
py "$work/gen.py" "$work" "$HERE" "$C2" mixed
mapfile -t X < <(ls -d "$work"/mixed/*/)
mp --check --partial "${S[0]}" "${X[1]}" > "$work/c2.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'different commits' "$work/c2.out"; }; expect "check fails on a mixed commit" $?
# short hash of the same commit is fine
sed -i "s/$C1/${C1:0:8}/" "${S[1]}README.md"
mp --check "${S[@]}" > /dev/null 2>&1; expect "check accepts an 8-hex form of the same commit" $?
# duplicate entry
mp --check --partial "${S[0]}" "${S[0]}" > "$work/c3.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'is in both' "$work/c3.out"; }; expect "check fails on an entry in two inputs" $?
# no commit named
nc="$work/nocommit"; mkdir "$nc"; cp "${S[2]}results.tsv" "$nc/"
mp --check --partial "${S[0]}" "$nc" > "$work/c4.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'names no commit' "$work/c4.out"; }; expect "check fails on a slice without a commit" $?
# filter that is not the table's
bf="$work/badfilter"; cp -r "${S[2]}" "$bf"
awk 'BEGIN{FS=OFS="\t"} !/^#/ && !d {$2="dEQP-GLES3.nonsense.*"; d=1} {print}' "${S[2]}results.tsv" > "$bf/results.tsv"
mp --check "${S[0]}" "${S[1]}" "$bf" > "$work/c5.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'filter differs' "$work/c5.out"; }; expect "check fails on a filter that is not the table's" $?

echo "== missing entry (M2)"
mp --check "${S[0]}" "${S[1]}" > "$work/c7.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'no record: 43 of 133 entries (90-132)' "$work/c7.out"; }; expect "check fails on missing entries 90-132, naming the range" $?
mp --totals "${S[0]}" "${S[1]}" > "$work/t2.out" 2>&1
grep -q '^no record: 43 of 133 entries (90-132)' "$work/t2.out"; expect "missing entries are visible in --totals" $?
mp --totals "${S[@]}" > "$work/t3.out" 2>&1
grep -q '^no record: 0 of 133 entries (none), 0 mustpass cases' "$work/t3.out"; expect "totals: the full set has no missing entry" $?

echo "== reset verdict (M3)"
rs="$work/reset"; cp -r "${S[1]}" "$rs"
sed -i 's/e46 rc=0 verdict=done resets=absent/e46 rc=0 verdict=done_with_resets resets=2/' "$rs/README.md"
sed -i "s/^origin\/main .*/origin\/main $C1/" "$rs/README.md"
mp --check "${S[0]}" "$rs" "${S[2]}" > "$work/c6.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'e46: verdict=done_with_resets resets=2' "$work/c6.out"; }; expect "check fails on a reset verdict, naming the entry (M3)" $?
nv="$work/noverdict"; cp -r "${S[2]}" "$nv"; sed -i '/^board\.sh /d' "$nv/README.md"
mp --check "${S[0]}" "${S[1]}" "$nv" > "$work/c8.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'no verdict lines' "$work/c8.out"; }; expect "check fails on a slice without verdict lines" $?

echo "== board-form slices (results.tsv as board.sh tally writes it, dash rows for unrun entries) (M1)"
mp --check "${B[@]}" > "$work/b1.out" 2>&1; expect "board-form slices merge, 133 entries from one commit (M1)" $?
mp --check --partial "${B[0]}" > "$work/b2.out" 2>&1; expect "one board-form slice alone checks with --partial" $?
mp --check "${B[0]}" > "$work/b3.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q 'no record: 88 of 133 entries (45-132)' "$work/b3.out"; }; expect "one board-form slice alone fails --check, naming 45-132" $?

echo "== --notpass shape"
mp --notpass "${S[@]}" > "$work/np.tsv" 2> "$work/np.err"; rc=$?
expect "notpass exits 0" "$rc"
[ ! -s "$work/np.err" ]; expect "notpass has nothing on stderr" $?
# pi4-gles3-fails.tsv: leading '#' header lines, then `<record>:<Status> <case>` rows (record = image there, e<i> here)
shape='^[A-Za-z0-9_.-]+:(Fail|NotSupported|QualityWarning|CompatibilityWarning|ResourceError|InternalError|Crash|Timeout|NoResult) dEQP-GLES3\.[^ ]+$'
# the pi4 records are file paths (build/f-A1.txt), so '/' is allowed there
refshape="${shape/\[A-Za-z0-9_.-\]+:/[A-Za-z0-9_./-]+:}"
grep -v '^#' "$PI4" | grep -Evq "$refshape"
[ $? = 1 ]; expect "the reference pi4-gles3-fails.tsv rows match the shape regex" $?
grep -v '^#' "$work/np.tsv" | grep -Evq "$shape"
[ $? = 1 ]; expect "every notpass row is <record>:<Status> <case>" $?
head -1 "$work/np.tsv" | grep -q '^# i\.MX8MP dEQP-GLES3 at '; expect "notpass header names the commit" $?
grep -q "^# entries run (133 of 133): 0-132" "$work/np.tsv"; expect "notpass header names 133 of 133 entries" $?
grep -q '^e1:Fail dEQP-GLES3\.' "$work/np.tsv"; expect "a named Fail is listed (e1)" $?
grep -q '^# e5: unresolved ' "$work/np.tsv"; expect "the crashed entry (e5) has its unresolved comment" $?
grep -q '^# e9: unresolved ' "$work/np.tsv"; expect "the timed-out entry (e9) has its unresolved comment" $?
grep -q '^e2:QualityWarning dEQP-GLES3\.' "$work/np.tsv" && ! grep -q '^# e2: unresolved ' "$work/np.tsv"; expect "the QualityWarning entry (e2) is named, so nothing is unresolved" $?
grep -q '^e0:' "$work/np.tsv"; [ $? = 1 ]; expect "an all-pass entry (e0) lists nothing" $?
awk -F'[: ]' '!/^#/ {print $1}' "$work/np.tsv" | sort -u | grep -qv '^e[0-9]*$'; [ $? = 1 ]; expect "record column is e<i>" $?
if grep -Evq '^(#|[^:]+:(Fail|NoResult) )' "$work/np.tsv"; then ok "notpass rows go beyond Fail/NoResult (M4)"; else bad "notpass names only Fail/NoResult rows (M4)"; fi
for st in NotSupported QualityWarning Crash Timeout; do
	grep -q "^e[0-9]*:$st dEQP-GLES3\." "$work/np.tsv"; expect "notpass names $st cases (M4)" $?
done
grep -q '^e1:NotSupported dEQP-GLES3\.' "$work/np.tsv" && grep -q '^e1:Fail dEQP-GLES3\.' "$work/np.tsv"; expect "e1 has its Fail and its NotSupported row (M4)" $?
grep -q '^e5:Crash dEQP-GLES3\.' "$work/np.tsv" && ! grep -q '^e5:Fail ' "$work/np.tsv"; expect "e5's named Fail is replaced by the source's Crash (M4)" $?
grep -q '^e9:Timeout dEQP-GLES3\.' "$work/np.tsv"; expect "e9's Timeout is named (M4)" $?
grep -q '^# e1: unresolved ' "$work/np.tsv"; [ $? = 1 ]; expect "e1's unresolved comment falls with the cases the source names (M4)" $?
# a no-record entry becomes NoResult rows
nr="$work/norec"; cp -r "${S[2]}" "$nr"
awk 'BEGIN{FS=OFS="\t"} /^#/ {print; next} !d {print $1,$2,"-","-","-","-","-","-","-"; d=1; next} {print}' "${S[2]}results.tsv" > "$nr/results.tsv"
mp --notpass "${S[0]}" "${S[1]}" "$nr" 2> /dev/null | grep -q '^e90:NoResult dEQP-GLES3\.'; expect "an entry with a dash row gives NoResult cases" $?

if [ "$fails" = 0 ]; then echo "pipeline selftest: OK "; exit 0; fi
echo "pipeline selftest: FAILED ($fails)"; exit 1
