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
grep -q '(#24)' "$work/full.out"; expect "ROADMAP line cites #24" $?

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

echo "== anchors and --apply (scratch copy of the three docs)"
# A fixture tree, not the repository's own docs: the anchors are the lines the first i.MX8MP run
# replaces, and once a run has landed the live docs no longer hold them.
ROOT=$HERE/testdata/imx8mp-gles3-report
mkroot() { rm -rf "$1"; mkdir -p "$1/tools/cts"; cp "$ROOT/ROADMAP.md" "$1/"; cp "$ROOT/tools/cts/RESULTS.md" "$ROOT/tools/cts/STATUS.md" "$1/tools/cts/"; }
mkroot "$work/r1"; mkroot "$work/r1.orig"
grep -q '^--- anchors' "$work/full.out"; expect "anchors block printed" $?
python3 "$REPORT" --date 2026-11-10 --root "$work/r1" "${S[@]}" > "$work/anc.out" 2>&1
grep -q 'RESULTS.md line [0-9]*, replaces' "$work/anc.out" && grep -q 'ROADMAP.md line [0-9]*, replaces' "$work/anc.out" \
	&& grep -q 'STATUS.md line [0-9]*, replaces' "$work/anc.out"; expect "anchors name the lines of all three docs" $?
cmp -s "$work/r1/tools/cts/RESULTS.md" "$ROOT/tools/cts/RESULTS.md" && cmp -s "$work/r1/ROADMAP.md" "$ROOT/ROADMAP.md"; expect "without --apply nothing is written" $?
python3 "$REPORT" --date 2026-11-10 --apply "$work/r1" "${S[@]}" > "$work/app.out" 2>&1; rc=$?
expect "--apply exits 0" "$rc"
grep -n 'i.MX8MP\*\*: not run on the board' "$work/r1/tools/cts/RESULTS.md" > /dev/null; [ $? = 1 ]; expect "c3 grep RESULTS.md has no match" $?
grep -n 'i.MX8MP: not run on the board yet' "$work/r1/ROADMAP.md" > /dev/null; [ $? = 1 ]; expect "c3 grep ROADMAP.md has no match" $?
grep -q 'Pi 4 と i.MX8MP の GLES3 全ツリー' "$work/r1/tools/cts/STATUS.md"; [ $? = 1 ]; expect "STATUS.md sentence replaced" $?
grep -q 'i.MX8MP の dEQP-GLES3 は' "$work/r1/tools/cts/STATUS.md" && grep -q 'i.MX8MP, whole tree' "$work/r1/tools/cts/RESULTS.md" \
	&& grep -q '^- i.MX8MP: dEQP-GLES3 whole tree' "$work/r1/ROADMAP.md"; expect "the drafts are in the three docs" $?
for f in ROADMAP.md tools/cts/RESULTS.md tools/cts/STATUS.md; do
	diff "$work/r1.orig/$f" "$work/r1/$f" > "$work/d.diff"
	[ "$(grep -c '^[<>]' "$work/d.diff")" -gt 0 ] || { bad "$f changed by --apply"; continue; }
	[ "$(grep -c '^[0-9]' "$work/d.diff")" = 1 ]; expect "$f: exactly one changed hunk" $?
done
cmp -s "$work/r1.orig/ROADMAP.md" "$ROOT/ROADMAP.md"; expect "the scratch original is the fixture's ROADMAP.md" $?

mkroot "$work/r2"; sed -i '/^\*\*i\.MX8MP\*\*: not run on the board/d' "$work/r2/tools/cts/RESULTS.md"
mkroot "$work/r2.orig"; cp "$work/r2/tools/cts/RESULTS.md" "$work/r2.orig/tools/cts/RESULTS.md"
python3 "$REPORT" --apply "$work/r2" "${S[@]}" > "$work/miss.out" 2>&1; rc=$?
{ [ "$rc" != 0 ] && grep -q 'missing' "$work/miss.out" && grep -q 'RESULTS.md' "$work/miss.out"; }; expect "missing anchor: --apply fails and names it" $?
cmp -s "$work/r2/ROADMAP.md" "$ROOT/ROADMAP.md" && cmp -s "$work/r2/tools/cts/STATUS.md" "$ROOT/tools/cts/STATUS.md" \
	&& cmp -s "$work/r2/tools/cts/RESULTS.md" "$work/r2.orig/tools/cts/RESULTS.md"; expect "missing anchor: nothing written" $?

mkroot "$work/r3"; printf '\n- i.MX8MP: not run on the board yet (#24)\n' >> "$work/r3/ROADMAP.md"
mkroot "$work/r3.orig"; cp "$work/r3/ROADMAP.md" "$work/r3.orig/"
python3 "$REPORT" --apply "$work/r3" "${S[@]}" > "$work/dup.out" 2>&1; rc=$?
{ [ "$rc" != 0 ] && grep -q 'matches 2 times' "$work/dup.out"; }; expect "duplicate anchor: --apply fails and names it" $?
cmp -s "$work/r3/ROADMAP.md" "$work/r3.orig/ROADMAP.md" && cmp -s "$work/r3/tools/cts/RESULTS.md" "$ROOT/tools/cts/RESULTS.md" \
	&& cmp -s "$work/r3/tools/cts/STATUS.md" "$ROOT/tools/cts/STATUS.md"; expect "duplicate anchor: nothing written" $?

python3 "$REPORT" --help > /dev/null; expect "--help works" $?

if [ "$fails" = 0 ]; then echo "report selftest: OK"; exit 0; fi
echo "report selftest: FAILED ($fails)"; exit 1
