#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-report-selftest.sh - synthetic i.MX8MP dEQP-GLES3 slice records (built as in
# imx8mp-gles3-pipeline-selftest.sh) through imx8mp-gles3-report.py: a complete run and an incomplete one.
#
#   TMPDIR=<dir> tools/cts/imx8mp-gles3-report-selftest.sh
#
# Host only. Everything is written under $TMPDIR (never /tmp) and removed at the end.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
REPORT=$HERE/imx8mp-gles3-report.py
: "${TMPDIR:?set TMPDIR to a directory (synthetic records are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
work=$(mktemp -d "$TMPDIR/imx8mp-report-XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT

fails=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; fails=$((fails + 1)); }
expect() { if [ "$2" = 0 ]; then ok "$1"; else bad "$1"; fi; }

C1=0123456789abcdef0123456789abcdef01234567

cat > "$work/gen.py" <<'PY'
import importlib.util, os, sys
root, here, commit = sys.argv[1:4]
spec = importlib.util.spec_from_file_location('merge', os.path.join(here, 'imx8mp-gles3-merge.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
table = m.load_table(os.path.join(here, 'imx8mp-gles3-tree.txt'))
per = m.entry_cases(table, os.path.join(here, 'gles3-mustpass.txt'))
n = len(table)
cut = [(0, 45), (45, 90), (90, n)]
names = ['2026-11-10-imx8mp-gles3-%s-rows%03d-%03d' % (commit[:8], a + 1, b) for a, b in cut]

def row(i):
    c = per[i]; k = len(c)
    fail, ns, other, named = 0, 0, 0, []
    if i % 7 == 1 and k >= 3:
        fail, ns, named = 1, 1, [c[1]]
    if i % 11 == 2 and k >= 2:
        other = 1
    return (i, table[i], k, k - fail - ns - other, fail, ns, other, 0, ';'.join(named) or '-')

for (a, b), nm in zip(cut, names):
    d = os.path.join(root, nm)
    os.makedirs(d)
    rows = [row(i) for i in range(a, b)]
    with open(os.path.join(d, 'README.md'), 'w') as f:
        f.write('# i.MX8MP dEQP-GLES3 slice (synthetic)\n\norigin/main %s\n' % commit)
        for r in rows:
            f.write('board.sh imx8mp e%02d rc=0 verdict=done resets=absent\n' % r[0])
    with open(os.path.join(d, 'results.tsv'), 'w') as f:
        f.write("# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names (';'-separated)\n")
        for r in rows:
            f.write('\t'.join(str(x) for x in r) + '\n')
PY
python3 -I "$work/gen.py" "$work" "$HERE" "$C1" || { echo "generator failed" >&2; exit 2; }
mapfile -t S < <(ls -d "$work"/*-imx8mp-gles3-*/)
[ ${#S[@]} = 3 ] || { echo "generator wrote an unexpected layout" >&2; exit 2; }

echo "== complete run"
python3 "$REPORT" --date 2026-11-10 "${S[@]}" > "$work/full.out" 2> "$work/full.err"; rc=$?
expect "complete run exits 0" "$rc"
grep -q "$C1" "$work/full.out"; expect "full commit is present" $?
grep -q '`01234567`' "$work/full.out"; expect "short sha is present" $?
grep -q PARTIAL "$work/full.out"; [ $? = 1 ]; expect "no PARTIAL in a complete run" $?
grep -q 'all 133 entries' "$work/full.out"; expect "133 entries named" $?
want=$(python3 "$HERE/imx8mp-gles3-merge.py" --totals "${S[@]}" | python3 -c '
import sys
for l in sys.stdin:
    f = l.split()
    if f and f[0] == "total":
        print("| " + " | ".join("{:,}".format(int(x)) for x in f[3:8]) + " |")')
[ -n "$want" ] && grep -qxF "$want" "$work/full.out"; expect "totals row equals merge --totals ($want)" $?
for h in 'RESULTS.md' 'STATUS.md' 'ROADMAP.md' 'issue #2'; do
	grep -q "^--- .*$h" "$work/full.out"; expect "block headed $h" $?
done
grep -q 'リセット0' "$work/full.out"; expect "STATUS line is Japanese with 0 resets" $?
grep -q '(#1262)' "$work/full.out"; expect "ROADMAP line cites #1262" $?

echo "== incomplete run (entries 90-132 missing)"
python3 "$REPORT" --date 2026-11-10 "${S[0]}" "${S[1]}" > "$work/part.out" 2> "$work/part.err"; rc=$?
expect "incomplete run exits 1" "$([ "$rc" = 1 ] && echo 0 || echo 1)"
grep -q 'PARTIAL' "$work/part.out"; expect "marked PARTIAL" $?
grep -q '90-132' "$work/part.out"; expect "the missing entries 90-132 are named" $?
grep -q "$C1" "$work/part.out"; expect "drafts are still printed (commit present)" $?
grep -q '^--- GitHub issue #2' "$work/part.out"; expect "issue comment still printed" $?

echo "== a reset verdict"
rs="$work/reset"; cp -r "${S[1]}" "$rs"
sed -i 's/e46 rc=0 verdict=done resets=absent/e46 rc=0 verdict=done_with_resets resets=2/' "$rs/README.md"
python3 "$REPORT" "${S[0]}" "$rs" "${S[2]}" > "$work/rs.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q PARTIAL "$work/rs.out" && grep -q 'e46' "$work/rs.out"; }; expect "reset verdict: exit 1, PARTIAL, e46 named" $?

python3 "$REPORT" --help > /dev/null; expect "--help works" $?

if [ "$fails" = 0 ]; then echo "report selftest: OK"; exit 0; fi
echo "report selftest: FAILED ($fails)"; exit 1
