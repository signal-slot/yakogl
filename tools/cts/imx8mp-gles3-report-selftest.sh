#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-report-selftest.sh - synthetic i.MX8MP dEQP-GLES3 slice records (built as in
# imx8mp-gles3-pipeline-selftest.sh) through imx8mp-gles3-report.py: a complete run and an incomplete one,
# a complete run whose records begin cases outside the mustpass list (not PARTIAL, the row sums to the
# mustpass count), the three real 9bc82915 slices alone (PARTIAL, exit 1) and completed with synthetic
# slices (exit 0), the record order by first row, and the drafts checked against the layout of
# tools/cts/RESULTS.md, STATUS.md and ROADMAP.md.
#
#   TMPDIR=<dir> sh tools/cts/imx8mp-gles3-report-selftest.sh
#
# Host only. Everything is written under $TMPDIR (never /tmp) and removed at the end.
[ -n "${BASH_VERSION:-}" ] || exec bash "$0" "$@"
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
# gen.py ROOT HERE COMMIT [EXTRA [CUTS]]
#   EXTRA: 0 none; 1 out-of-list cases beyond the mustpass list (extra passes on every 5th entry, a named
#          out-of-list Fail on every 9th); 2 as 1, plus 2 extra NotSupported on entry 7
#   CUTS:  'a-b:name,...' (0-based entry spans, end exclusive), default three slices dated 2026-11-10
import importlib.util, os, sys
root, here, commit = sys.argv[1:4]
extra = int(sys.argv[4]) if len(sys.argv) > 4 else 0
spec = importlib.util.spec_from_file_location('merge', os.path.join(here, 'imx8mp-gles3-merge.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
table = m.load_table(os.path.join(here, 'imx8mp-gles3-tree.txt'))
per = m.entry_cases(table, os.path.join(here, 'gles3-mustpass.txt'))
n = len(table)
if len(sys.argv) > 5:
    cut, names = [], []
    for part in sys.argv[5].split(','):
        span, nm = part.split(':')
        a, b = span.split('-')
        cut.append((int(a), int(b))); names.append(nm)
else:
    cut = [(0, 45), (45, 90), (90, n)]
    names = ['2026-11-10-imx8mp-gles3-%s-rows%03d-%03d' % (commit[:8], a + 1, b) for a, b in cut]

def row(i):
    c = per[i]; k = len(c)
    fail, ns, other, named = 0, 0, 0, []
    if i % 7 == 1 and k >= 3:
        fail, ns, named = 1, 1, [c[1]]
    if i % 11 == 2 and k >= 2:
        other = 1
    begun, passed = k, k - fail - ns - other
    if extra and i % 5 == 0:          # a wildcard that also selects 3 passing cases outside the list
        begun, passed = begun + 3, passed + 3
    if extra and i % 9 == 4:          # and one failing case outside the list, named
        begun, fail = begun + 1, fail + 1
        named.append('dEQP-GLES3.functional.zz_outside_mustpass.case_%d' % i)
    if extra == 2 and i == 7:
        begun, ns = begun + 2, ns + 2
    return (i, table[i], begun, passed, fail, ns, other, 0, ';'.join(named) or '-')

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
grep -q '133 of 133 entries run' "$work/full.out"; expect "133 entries named" $?
want=$(python3 "$HERE/imx8mp-gles3-merge.py" --totals "${S[@]}" | python3 -c '
import sys
for l in sys.stdin:
    f = l.split()
    if f and f[0] == "total":
        print("| " + " | ".join("{:,}".format(int(x)) for x in f[3:8]) + " |")')
[ -n "$want" ] && grep -qxF "$want" "$work/full.out"; expect "totals row equals merge --totals ($want)" $?
for h in 'RESULTS.md' 'STATUS.md' 'ROADMAP.md' 'issue #24'; do
	grep -q "^--- .*$h" "$work/full.out"; expect "block headed $h" $?
done
grep -q 'リセット 0' "$work/full.out"; expect "STATUS line is Japanese with 0 resets" $?
grep -q '(#24)' "$work/full.out"; expect "ROADMAP line cites #24" $?

echo "== record names: numeric slice order, no absolute path"
python3 "$REPORT" --date 2026-11-10 "${S[2]}" "${S[1]}" "${S[0]}" > "$work/ord.out" 2>&1; rc=$?
expect "slices given in reverse order: exit 0" "$rc"
grep -qF "$work" "$work/ord.out"; [ $? = 1 ]; expect "no absolute path of the scratch directory in the drafts" $?
python3 - "$work/ord.out" <<'PY'
import re, sys
t = open(sys.argv[1]).read()
line = [l for l in t.splitlines() if l.startswith('- Records:')][-1]   # the issue comment's (the RESULTS bullet wraps)
rows = [int(x) for x in re.findall(r'-rows(\d+)-', line)]
sys.exit(0 if len(rows) == 3 and rows == sorted(rows) and 'tools/cts/log/' in line else 1)
PY
expect "Records lists the slices by their first row (rows001 before rows0xx), as tools/cts/log/<name>" $?

echo "== incomplete run (entries 90-132 missing)"
python3 "$REPORT" --date 2026-11-10 "${S[0]}" "${S[1]}" > "$work/part.out" 2> "$work/part.err"; rc=$?
expect "incomplete run exits 1" "$([ "$rc" = 1 ] && echo 0 || echo 1)"
grep -q 'PARTIAL' "$work/part.out"; expect "marked PARTIAL" $?
grep -q '90-132' "$work/part.out"; expect "the missing entries 90-132 are named" $?
grep -q "$C1" "$work/part.out"; expect "drafts are still printed (commit present)" $?
grep -q '^--- GitHub issue #24 comment' "$work/part.out"; expect "issue comment still printed" $?

echo "== a reset verdict"
rs="$work/reset"; cp -r "${S[1]}" "$rs"
sed -i 's/e46 rc=0 verdict=done resets=absent/e46 rc=0 verdict=done_with_resets resets=2/' "$rs/README.md"
python3 "$REPORT" "${S[0]}" "$rs" "${S[2]}" > "$work/rs.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q PARTIAL "$work/rs.out" && grep -q 'e46' "$work/rs.out"; }; expect "reset verdict: exit 1, PARTIAL, e46 named" $?

echo "== anchors and --apply (scratch copy of the three docs)"
ROOT=$(cd "$HERE/../.." && pwd)
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
grep -q 'i.MX8MP は全ツリー' "$work/r1/tools/cts/STATUS.md" && grep -q 'i.MX8MP, whole tree' "$work/r1/tools/cts/RESULTS.md" \
	&& grep -q '^- i.MX8MP: the whole tree' "$work/r1/ROADMAP.md"; expect "the drafts are in the three docs" $?
grep -q 'i.MX8MP はまだボードで走らせていない' "$work/r1/tools/cts/STATUS.md"; [ $? = 1 ]; expect "STATUS.md 'not run yet' sentence replaced" $?
grep -q 'Pi 4 の GLES3 全ツリー。' "$work/r1/tools/cts/STATUS.md"; expect "STATUS.md next steps keep the Pi 4 whole tree only" $?
for f in ROADMAP.md:1 tools/cts/RESULTS.md:1 tools/cts/STATUS.md:2; do
	h=${f##*:}; f=${f%:*}
	diff "$work/r1.orig/$f" "$work/r1/$f" > "$work/d.diff"
	[ "$(grep -c '^[<>]' "$work/d.diff")" -gt 0 ] || { bad "$f changed by --apply"; continue; }
	[ "$(grep -c '^[0-9]' "$work/d.diff")" = "$h" ]; expect "$f: exactly $h changed hunk(s)" $?
done
cmp -s "$work/r1.orig/ROADMAP.md" "$ROOT/ROADMAP.md"; expect "the scratch original is the repository's ROADMAP.md" $?

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

cat > "$work/gen1.py" <<'PY'
# one record rerunning entry <i>: begun and passed k+<add> times (k = its mustpass cases), same commit
import importlib.util, os, sys
d, here, commit, i, add = sys.argv[1:6]
i, add = int(i), int(add)
spec = importlib.util.spec_from_file_location('merge', os.path.join(here, 'imx8mp-gles3-merge.py'))
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
table = m.load_table(os.path.join(here, 'imx8mp-gles3-tree.txt'))
per = m.entry_cases(table, os.path.join(here, 'gles3-mustpass.txt'))
k = len(per[i])
os.makedirs(d)
with open(os.path.join(d, 'README.md'), 'w') as f:
    f.write('# rerun of entry %d (synthetic)\n\norigin/main %s\n' % (i, commit))
    f.write('board.sh imx8mp e%02d rc=0 verdict=done resets=absent\n' % i)
with open(os.path.join(d, 'results.tsv'), 'w') as f:
    f.write('# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names\n')
    f.write('\t'.join(str(x) for x in (i, table[i], k + add, k + add, 0, 0, 0, 0, '-')) + '\n')
PY

echo "== an entry in two slices, begun counted once"
python3 -I "$work/gen1.py" "$work/dup1" "$HERE" "$C1" 46 500 || { echo "rerun generator failed" >&2; exit 2; }
python3 "$REPORT" --date 2026-11-10 "${S[@]}" "$work/dup1" > "$work/dup.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q PARTIAL "$work/dup.out" && grep -q 'entry 46 is in both' "$work/dup.out"; }
expect "entry in two slices: exit 1, PARTIAL, entry 46 named" $?
grep -Fq 'Totals over the 42,810 mustpass cases (42,810 begun)' "$work/dup.out"
expect "the rerun is not summed into begun (42,810, not 43,310 of 42,810)" $?

echo "== a constructed overcount is named, never printed silently"
python3 -I "$work/gen1.py" "$work/dup2" "$HERE" "$C1" 46 5000 || { echo "rerun generator failed" >&2; exit 2; }
python3 "$REPORT" --date 2026-11-10 "$work/dup2" "${S[@]}" > "$work/over.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q PARTIAL "$work/over.out" && grep -q 'entry 46 is in both' "$work/over.out" \
	&& grep -qF '5,000 results the records give beyond' "$work/over.out" && grep -qF 'records begin 47,810 cases' "$work/over.out"; }
expect "overcount: exit 1, PARTIAL for the entry in two slices, the 5,000 excess and the raw 47,810 named" $?
tr '\n' ' ' < "$work/over.out" | grep -Fq '42,810 mustpass cases (each counted once, 42,810 begun)'
expect "RESULTS.md draft: begun never above its mustpass count" $?
grep -Fq 'Totals over the 42,810 mustpass cases (42,810 begun)' "$work/over.out"
expect "issue comment: each mustpass case counted once (the 5,000-case excess not printed as begun)" $?
grep -q '^- Mustpass cases only: 5,000 results' "$work/over.out"
expect "the mustpass-only rule is stated in the RESULTS draft" $?

cat > "$work/row.py" <<'PY'
# row.py OUT: the RESULTS.md table row of a report output as "pass fail ns other none sum"
import re, sys
t = open(sys.argv[1]).read()
r = [l for l in t.splitlines() if re.fullmatch(r'\| [0-9,]+ \| [0-9,]+ \| [0-9,]+ \| [0-9,]+ \| [0-9,]+ \|', l)]
if len(r) != 1:
    sys.exit('no single five-number row')
v = [int(x.replace(',', '')) for x in re.findall(r'[0-9,]+', r[0])]
print(' '.join(map(str, v + [sum(v)])))
PY
MP=42810
sums_to() { set -- $(python3 -I "$work/row.py" "$1" 2>/dev/null); [ "${6:-}" = "$MP" ]; }

echo "== complete run whose records begin cases outside the mustpass list"
mkdir "$work/x1" "$work/x2"
python3 -I "$work/gen.py" "$work/x1" "$HERE" "$C1" 1 || { echo "generator failed" >&2; exit 2; }
python3 -I "$work/gen.py" "$work/x2" "$HERE" "$C1" 2 || { echo "generator failed" >&2; exit 2; }
mapfile -t X1 < <(ls -d "$work"/x1/*-imx8mp-gles3-*/)
mapfile -t X2 < <(ls -d "$work"/x2/*-imx8mp-gles3-*/)
python3 "$HERE/imx8mp-gles3-merge.py" --totals "${X1[@]}" 2>/dev/null | awk '$1=="total"{exit !($3>$2)}'
expect "the synthetic records begin more cases than the mustpass list has (merge --totals begun > mustpass)" $?
python3 "$REPORT" --date 2026-11-10 "${X1[@]}" > "$work/x1.out" 2>&1; rc=$?
expect "out-of-list cases: exit 0" "$rc"
grep -q PARTIAL "$work/x1.out"; [ $? = 1 ]; expect "out-of-list cases: no PARTIAL marker" $?
sums_to "$work/x1.out"; expect "out-of-list cases: the five-number row sums to the $MP mustpass cases" $?
[ "$(python3 -I "$work/row.py" "$work/x1.out")" = "$(python3 -I "$work/row.py" "$work/full.out")" ]
expect "out-of-list passes and named out-of-list fails are left out: the row equals the run without them" $?
grep -q '^note: mustpass cases only: [0-9,]* results' "$work/x1.out" && grep -q '^- Mustpass cases only:' "$work/x1.out"
expect "the left-out cases are named in a note and in the RESULTS draft" $?
grep -Fq 'Totals over the 42,810 mustpass cases (42,810 begun)' "$work/x1.out"
expect "out-of-list cases: begun is the mustpass count" $?
python3 "$REPORT" --date 2026-11-10 "${X2[@]}" > "$work/x2.out" 2>&1; rc=$?
{ [ "$rc" = 0 ] && ! grep -q PARTIAL "$work/x2.out" && sums_to "$work/x2.out"; }
expect "extra NotSupported cases too: exit 0, no PARTIAL, the row sums to $MP" $?

echo "== the three real 9bc82915 slices"
# the three Verdin slices only (later X2 slices of the campaign sit beside them in tools/cts/log)
mapfile -t REAL < <(ls -d "$ROOT"/tools/cts/log/*-imx8mp-gles3-9bc82915-rows01-13/ "$ROOT"/tools/cts/log/*-imx8mp-gles3-9bc82915-rows17-34/ "$ROOT"/tools/cts/log/*-imx8mp-gles3-9bc82915-rows35-51/ 2>/dev/null)
if [ ${#REAL[@]} = 3 ]; then
	python3 "$REPORT" --date 2026-11-10 "${REAL[@]}" > "$work/real.out" 2>&1; rc=$?
	{ [ "$rc" = 1 ] && grep -q '^\*\*\* PARTIAL' "$work/real.out" && grep -q 'no record: 85 of 133 entries (13-15, 51-132)' "$work/real.out"; }
	expect "real slices: exit 1, PARTIAL, entries 13-15, 51-132 named" $?
	sums_to "$work/real.out"; expect "real slices: the row sums to the $MP mustpass cases" $?
	# Complete the campaign with synthetic records of the missing entries at the same commit; the real
	# records begin 29,594 cases for 28,459 mustpass cases. Names out of lexical order and old mtimes.
	C9=$(python3 -I - "${REAL[0]}" <<'PY'
import re, sys, os
for n in sorted(os.listdir(sys.argv[1])):
    if n.lower().startswith('readme'):
        m = re.search(r'\b[0-9a-f]{40}\b', open(os.path.join(sys.argv[1], n), errors='replace').read())
        if m: print(m.group(0)); break
PY
)
	mkdir "$work/fill"
	python3 -I "$work/gen.py" "$work/fill" "$HERE" "${C9:-9bc82915}" 0 \
		"13-16:2026-12-02-imx8mp-gles3-9bc82915-rows14-16,51-107:2026-12-01-imx8mp-gles3-9bc82915-rows52-107,107-133:2026-11-30-imx8mp-gles3-9bc82915-rows108-133" \
		|| { echo "generator failed" >&2; exit 2; }
	touch -d 2026-11-01 "$work"/fill/*-rows14-16
	mapfile -t FILL < <(ls -d "$work"/fill/*/)
	python3 "$REPORT" --date 2026-11-10 "${FILL[@]}" "${REAL[@]}" > "$work/real133.out" 2>&1; rc=$?
	expect "real slices completed: exit 0" "$rc"
	grep -q PARTIAL "$work/real133.out"; [ $? = 1 ]; expect "real slices completed: no PARTIAL although they begin cases outside the list" $?
	sums_to "$work/real133.out"; expect "real slices completed: the row sums to the $MP mustpass cases" $?
	python3 - "$work/real133.out" <<'PY'
import re, sys
t = open(sys.argv[1]).read()
first = lambda s: [int(x) for x in re.findall(r'-rows(\d+)-', s)]
rec = ' '.join(l.strip() for l in t.split('--- RESULTS.md')[1].split('--- STATUS.md')[0].splitlines())
rec = rec.split('Records:')[1].split(';')[0]
issue = [l for l in t.splitlines() if l.startswith('- Records:')][-1]   # the issue comment's (the RESULTS bullet wraps)
status = [l for l in t.splitlines() if l.startswith('i.MX8MP は')][0]
want = [1, 14, 17, 35, 52, 108]
sys.exit(0 if first(rec) == first(issue) == first(status) == want else 1)
PY
	expect "records in the RESULTS, STATUS and issue drafts by first row: rows01, rows14, rows17, rows35, rows52, rows108" $?
	REPORT_FMT=$work/real133.out
	MIXED=1
else
	bad "real slices: expected 3 tools/cts/log/*-imx8mp-gles3-9bc82915-rows* directories, found ${#REAL[@]}"
	REPORT_FMT=$work/full.out
fi

# The campaign measured on two boards: the three real slices on the Verdin, the missing entries (synthetic, fake
# data) as X2 slices whose README says so. Which board measured which entries is read from the records.
echo "== a mixed campaign: Verdin and Armadillo-X2 slices"
blk() { sed -n '/^--- GitHub issue #24/,/^--- anchors/p' "$1"; }
sec() { sed -n "/^--- $2/,/^--- /p" "$1" | sed '$d' | tr '\n' ' ' | tr -s ' '; }
if [ -n "${MIXED:-}" ]; then
	mkdir "$work/mixfill"
	for d in "${FILL[@]}"; do
		cp -r "$d" "$work/mixfill/$(basename "$d")"
		r="$work/mixfill/$(basename "$d")/README.md"
		sed -i 's/board\.sh imx8mp/board.sh x2/' "$r"
		printf '\nMeasured on the Armadillo-X2.\n' >> "$r"
	done
	mapfile -t MFILL < <(ls -d "$work"/mixfill/*/)
	python3 "$REPORT" --date 2026-11-10 "${MFILL[@]}" "${REAL[@]}" > "$work/mix.out" 2>&1; rc=$?
	expect "mixed complete: exit 0" "$rc"
	grep -q PARTIAL "$work/mix.out"; [ $? = 1 ]; expect "mixed complete: no PARTIAL (X2 slices are full records)" $?
	grep -qi 'unknown' "$work/mix.out"; [ $? = 1 ]; expect "mixed complete: no unknown board" $?
	sums_to "$work/mix.out"; expect "mixed complete: the row sums to the $MP mustpass cases" $?
	XM='entries 0-12 and 16-50 on the Verdin iMX8M Plus, 13-15 and 51-132 on the Armadillo-X2'
	sec "$work/mix.out" 'RESULTS.md' | grep -qF "measured on $XM, 2026-11-10:"; expect "mixed complete: RESULTS.md names both boards with their entries" $?
	sec "$work/mix.out" 'ROADMAP.md' | grep -qF "measured on $XM at"; expect "mixed complete: the ROADMAP.md bullet names both boards with their entries" $?
	sec "$work/mix.out" 'STATUS.md' | grep -qF 'i.MX8MP は Verdin iMX8M Plus（エントリ 0-12、16-50）と Armadillo-X2（エントリ 13-15、51-132）で全ツリー'
	expect "mixed complete: the STATUS.md sentence names both boards with their entries" $?
	blk "$work/mix.out" | grep -qF "measured on $XM"; [ $? = 0 ] || blk "$work/mix.out" | grep -q 'Boards:'; expect "mixed complete: the #24 block names the boards" $?
	grep -qF "$work" "$work/mix.out"; [ $? = 1 ]; expect "mixed complete: no scratch path in the drafts" $?
	# the ranges come from the records: take the X2 README line away from the slice of entries 107-132 and that slice is Verdin's
	mkdir "$work/mixv"; cp -r "$work"/mixfill/* "$work/mixv/"
	for d in "$work"/mixv/*-rows108-133; do sed 's/board\.sh x2/board.sh imx8mp/' "$d/README.md" | grep -v 'Armadillo-X2' > "$d/R"; mv "$d/R" "$d/README.md"; done
	mapfile -t MV < <(ls -d "$work"/mixv/*/)
	python3 "$REPORT" --date 2026-11-10 "${MV[@]}" "${REAL[@]}" > "$work/mixv.out" 2>&1; rc=$?
	sec "$work/mixv.out" 'ROADMAP.md' | grep -qF 'entries 0-12, 16-50 and 107-132 on the Verdin iMX8M Plus, 13-15 and 51-106 on the Armadillo-X2 at'
	expect "mixed: the ranges follow each slice's README (entries 107-132 moved to the Verdin)" $?
	# a mixed campaign with a real gap: the X2 slice of entries 51-106 is missing
	python3 "$REPORT" --date 2026-11-10 "$work"/mixfill/*-rows14-16 "$work"/mixfill/*-rows108-133 "${REAL[@]}" > "$work/mixp.out" 2>&1; rc=$?
	expect "mixed partial: exit 1" "$([ "$rc" = 1 ] && echo 0 || echo 1)"
	grep -q '^\*\*\* PARTIAL' "$work/mixp.out"; expect "mixed partial: marked PARTIAL" $?
	grep -q 'no record: .*(51-106)' "$work/mixp.out"; expect "mixed partial: the missing entries 51-106 are named" $?
	sec "$work/mixp.out" 'ROADMAP.md' | grep -qF 'measured on entries 0-12 and 16-50 on the Verdin iMX8M Plus, 13-15 and 107-132 on the Armadillo-X2 at'
	expect "mixed partial: the boards still name the entries each measured" $?
	else
	bad "mixed campaign: needs the three real slices"
fi

echo "== drafts in the layout of their target files"
python3 - "$REPORT_FMT" "$ROOT" "$work/part.out" <<'PY'
import re, sys
out, root, part = sys.argv[1:4]
t = open(out).read()
def block(t, head, nxt):
    return t.split('--- ' + head)[1].split('\n', 1)[1].split('--- ' + nxt)[0].strip('\n')
res, sta, road = block(t, 'RESULTS.md', 'STATUS.md'), block(t, 'STATUS.md', 'ROADMAP.md'), block(t, 'ROADMAP.md', 'GitHub')
fails = []
def check(what, cond):
    print(('ok   ' if cond else 'FAIL ') + what)
    if not cond:
        fails.append(what)
R = open(root + '/tools/cts/RESULTS.md', encoding='utf-8').read()
es3 = R.split('## ES 3.0')[1].split('\n## ')[0]
heads = [l for l in es3.splitlines() if l.startswith('| Pass |')]
cols = lambda l: len(l.strip().strip('|').split('|'))
dh = [l for l in res.splitlines() if l.startswith('| Pass |')]
check('RESULTS: the table has the column count of the ES 3.0 tables (%d)' % (cols(heads[0]) if heads else -1),
      heads and len(dh) == 1 and cols(dh[0]) == cols(heads[0]))
check('RESULTS: header columns 1-3 and 5 as the neighbours, the alignment row as theirs',
      heads and dh and dh[0].split('|')[1:4] == heads[0].split('|')[1:4] and dh[0].split('|')[5] == heads[0].split('|')[5]
      and '|---:|---:|---:|---:|---:|' in res and '|---:|---:|---:|---:|---:|' in es3)
nb = re.search(r'^\*\*Pi 4, whole tree\*\* on `([0-9a-f]{8})` \(`\1[0-9a-f]{32}`;', es3, re.M)
check('RESULTS: the neighbour Pi 4 paragraph opens "**<board>, whole tree** on `<sha8>` (`<sha40>`;"', bool(nb))
flat = lambda x: ' '.join(x.split())
check('RESULTS: the draft opens the same way', bool(re.match(r'\*\*i\.MX8MP, whole tree\*\* on `([0-9a-f]{8})` \(`\1[0-9a-f]{32}`;', flat(res))))
check('RESULTS: the date follows the recipe, then ": ", as the AM62P and Pi 4 paragraphs',
      bool(re.search(r'\), \d{4}-\d{2}-\d{2}: ', ' '.join(res.splitlines()))))
check('RESULTS: lines at most 80 columns, bullets "- " with 2-space continuation',
      all(len(l) <= 80 for l in res.splitlines() if not l.startswith('|'))
      and all(l.startswith(('- ', '  ', '|', '**')) or not l or l[0].isalnum() or l[0] in '`(' for l in res.splitlines()))
nums = lambda s: re.findall(r'(?<![\w.,/-])\d{4,}(?![\w,-])', re.sub(r'\d{4}-\d{2}-\d{2}', '', re.sub(r'`[^`]*`', '', s)))
check('RESULTS, STATUS, ROADMAP: every count of four digits or more has a thousands comma (42,810)',
      not nums(res) and not nums(sta) and not nums(road))
S = open(root + '/tools/cts/STATUS.md', encoding='utf-8').read()
five_ja = r'必須 [0-9,]+ 件中 合格 [0-9,]+ / 失敗 [0-9,]+ / 未対応 [0-9,]+ / その他 [0-9,]+ / 結果なし [0-9,]+、'
check('STATUS: the Pi 4 sentence of STATUS.md has the five-number form', bool(re.search(five_ja + r'.*リセット 0', S)))
line = sta.splitlines()[0]
check('STATUS: the draft has the same form ("必須 N 件中 合格 ... / 結果なし N、... リセット 0")',
      bool(re.search(five_ja + r'(?:リセット 0|リセットまたはハング \d+ 件)', line)))
check('STATUS: the commit is written as in STATUS.md (sha8 without backticks, date in full-width parentheses)',
      '`' not in line and bool(re.search(r' [0-9a-f]{8}（\d{4}-\d{2}-\d{2}）で実行し', line))
      and bool(re.search(r' [0-9a-f]{8}（\d{4}-\d{2}-\d{2}）で実行し', S)))
check('STATUS: the draft replaces the ES 3.0 bullet\'s i.MX8MP sentence, which STATUS.md has once',
      S.count('i.MX8MP はまだボードで走らせていない。') == 1 and 'ES 3.0 bullet' in t)
check('STATUS: one Japanese sentence ending in 。', line.endswith('。') and line.count('\n') == 0)
D = open(root + '/ROADMAP.md', encoding='utf-8').read()
st4 = D.split('## 4.')[1].split('\n## ')[0]
five_en = r'[0-9,]+ cases,\s+[0-9,]+ pass /\s+[0-9,]+ fail /\s+[0-9,]+ NotSupported /\s+[0-9,]+ other /\s+[0-9,]+\s+without\s+a\s+result'
check('ROADMAP: the stage-4 Pi 4 bullet has the form "N cases, a pass / b fail / c NotSupported / d other / e without a result"',
      bool(re.search(r'^- Raspberry Pi 4: the whole tree.*?' + five_en, st4, re.M | re.S)))
check('ROADMAP: the draft has the same form', bool(re.match(r'- i\.MX8MP: the whole tree .*?at `[0-9a-f]{8}` \(\d{4}-\d{2}-\d{2}\): ' + five_en + r'.*\(#24\)\.$', flat(road))))
check('ROADMAP: the draft wraps at 78 columns with 2-space continuation lines, as its neighbours',
      all(len(l) <= 78 for l in road.splitlines()) and all(l.startswith('  ') for l in road.splitlines()[1:]))
p = open(part).read()
proad = block(p, 'ROADMAP.md', 'GitHub')
check('ROADMAP, incomplete run: the draft says PARTIAL and names the entries without a record',
      proad.startswith('- i.MX8MP: PARTIAL: ') and 'no record: 90-132' in ' '.join(proad.split()))
sys.exit(1 if fails else 0)
PY
expect "the drafts match RESULTS.md, STATUS.md and ROADMAP.md" $?

echo "== board named from the records"
mkx() {   # mkx DEST SRC [readme]: copy of a slice as an X2 record (three-word rows, '-' in begun and noresult)
	cp -r "$2" "$1"
	python3 - "$1" "${3:-}" <<'PY'
import os, sys
d, mode = sys.argv[1:3]
if mode != 'readme':
    t = os.path.join(d, 'README.md')
    s = open(t).read().replace('board.sh imx8mp', 'board.sh x2')
    open(t, 'w').write(s)
else:
    with open(os.path.join(d, 'README.md'), 'a') as f:
        f.write('\nMeasured on the Armadillo-X2.\n')
p = os.path.join(d, 'results.tsv')
out = []
for l in open(p):
    f = l.rstrip('\n').split('\t')
    if not l.startswith('#') and len(f) >= 8:
        f[2] = f[7] = '-'
    out.append('\t'.join(f))
open(p, 'w').write('\n'.join(out) + '\n')
PY
}
blk() { sed -n '/^--- GitHub issue #24/,/^--- anchors/p' "$1"; }
mkdir "$work/bx2" "$work/bmx"
X=()
for i in 0 1 2; do mkx "$work/bx2/$(basename "${S[$i]}")" "${S[$i]}"; X+=("$work/bx2/$(basename "${S[$i]}")"); done
python3 "$REPORT" --date 2026-11-10 "${X[@]}" > "$work/bx2.out" 2>&1; rc=$?
expect "all-X2: exit 0" "$rc"
blk "$work/bx2.out" | grep -q 'measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)'; expect "all-X2: the #24 block says measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)" $?
blk "$work/bx2.out" | grep -q 'Boards:'; [ $? = 1 ]; expect "all-X2: no per-slice board list" $?
mkx "$work/bmx/$(basename "${S[0]}")" "${S[0]}" readme
mkdir "$work/brd"; cp -r "${S[1]}" "${S[2]}" "$work/brd/"
python3 "$REPORT" --date 2026-11-10 "$work/bmx/$(basename "${S[0]}")" "$work/brd/$(basename "${S[1]}")" "$work/brd/$(basename "${S[2]}")" > "$work/brd.out" 2>&1
blk "$work/brd.out" | grep -q 'Boards:.*rows001-045.*Armadillo-X2.*rows046-090.*Verdin'; expect "mixed: the #24 block names each slice's board" $?
blk "$work/brd.out" | grep -q 'measured on the Armadillo-X2'; [ $? = 1 ]; expect "mixed: no blanket X2 claim" $?
mkdir "$work/bro"; ro="$work/bro/$(basename "${S[0]}")"; cp -r "${S[0]}" "$ro"
grep -v 'board\.sh' "$ro/README.md" > "$ro/R" && mv "$ro/R" "$ro/README.md"; echo 'Board: Armadillo-X2' >> "$ro/README.md"
python3 "$REPORT" --date 2026-11-10 "$ro" 2>&1 | sed -n '/^--- GitHub issue #24/,/^--- anchors/p' | grep -q 'Armadillo-X2'; expect "a README line 'Armadillo-X2' alone names the X2" $?
blk "$work/full.out" | grep -qE 'Armadillo|Board'; [ $? = 1 ]; expect "all-Verdin: the #24 block names no board" $?
# the RESULTS.md, STATUS.md and ROADMAP.md drafts name the board too (lines joined: the drafts are wrapped)
sec() { sed -n "/^--- $2/,/^--- /p" "$1" | sed '$d' | tr '\n' ' ' | tr -s ' '; }
XL='measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)'
sec "$work/bx2.out" 'RESULTS.md' | grep -qF "recipe \`docs/es3/board-recipe-imx8mp.md\`), $XL, 2026-11-10:"
expect "all-X2: the RESULTS.md paragraph says $XL" $?
sec "$work/bx2.out" 'STATUS.md' | grep -qF 'i.MX8MP は Armadillo-X2（i.MX 8M Plus、GC7000UL）で全ツリー'
expect "all-X2: the STATUS.md sentence says Armadillo-X2（i.MX 8M Plus、GC7000UL）で" $?
sec "$work/bx2.out" 'ROADMAP.md' | grep -qF "tools/cts/imx8mp-gles3-tree.txt\`), $XL at \`"
expect "all-X2: the ROADMAP.md bullet says $XL" $?
sec "$work/brd.out" 'RESULTS.md' | grep -qF 'measured on entries 0-44 on the Armadillo-X2, 45-132 on the Verdin iMX8M Plus, 2026-11-10'
expect "mixed: the RESULTS.md paragraph names both boards" $?
sec "$work/brd.out" 'RESULTS.md' | grep -q -- '- Boards: `[^`]*rows001-045/`: Armadillo-X2 (i.MX 8M Plus, GC7000UL); `[^`]*rows046-090/`: Verdin'
expect "mixed: the RESULTS.md draft lists each slice's board" $?
sec "$work/brd.out" 'STATUS.md' | grep -qF 'i.MX8MP は Armadillo-X2（エントリ 0-44）と Verdin iMX8M Plus（エントリ 45-132）で'
expect "mixed: the STATUS.md sentence names both boards" $?
sec "$work/brd.out" 'ROADMAP.md' | grep -qF 'measured on entries 0-44 on the Armadillo-X2, 45-132 on the Verdin iMX8M Plus at'
expect "mixed: the ROADMAP.md bullet names both boards" $?
for f in RESULTS.md STATUS.md ROADMAP.md; do
	sec "$work/full.out" "$f" | grep -qE 'Armadillo|Verdin|measured on'; [ $? = 1 ]; expect "all-Verdin: the $f draft names no board (as before)" $?
done

echo "== a WDOG1 end and a hang that read verdict=done are counted from the records"
wd="$work/wdog"; mkdir "$wd"
for i in 0 1 2; do cp -r "${S[$i]}" "$wd/"; done
w0="$wd/$(basename "${S[0]}")"; w1="$wd/$(basename "${S[1]}")"
python3 - "$w0" "$w1" <<'PY'
import sys
# slice 0: entry 3 ends short (shortfall) and its README names the WDOG1 reset; slice 1: a README line names a hang
# at entry 50 whose row is complete; a line that denies a reset must not count.
d0, d1 = sys.argv[1:3]
rows = open(d0 + '/results.tsv').read().split('\n')
for k, l in enumerate(rows):
    f = l.split('\t')
    if f[0] == '3':
        f[2] = str(max(1, int(f[2]) - 1)); f[3] = str(max(0, int(f[3]) - 1)); rows[k] = '\t'.join(f)
open(d0 + '/results.tsv', 'w').write('\n'.join(rows))
open(d0 + '/README.md', 'a').write('\nWDOG1 e03 ended on the watchdog reset after 600 s, the run returned to Linux\nThe watchdog did not reset the other entries.\n')
open(d1 + '/README.md', 'a').write('\nHang: e50 on its last case; the board was power-cycled\n')
PY
python3 "$REPORT" --date 2026-11-10 "$w0" "$w1" "$wd/$(basename "${S[2]}")" > "$wd.out" 2>&1
grep -q '2 resets or hangs counted from the records (1 reset line, 1 hang line; entries begun short: 3)' "$wd.out"
expect "a WDOG1 end and a hang counted (1 reset line, 1 hang line, shortfall entry 3)" $?
grep -q '0 resets' "$wd.out"; [ $? = 1 ]; expect "no '0 resets' when a WDOG1 end is recorded" $?
grep -q 'リセットまたはハング 2 件' "$wd.out"; expect "the STATUS line counts them in Japanese" $?
grep -q '2 resets or hangs at\|2 resets or hangs, ' "$wd.out" || grep -q '2 resets or hangs (#24)' "$wd.out"; expect "the ROADMAP line counts them" $?
# the shortfall alone (no README line) is one hang or reset of unknown kind
python3 - "$w0" <<'PY'
import sys
t = open(sys.argv[1] + '/README.md').read().replace('WDOG1 e03', 'e03')
open(sys.argv[1] + '/README.md', 'w').write(t)
PY
python3 "$REPORT" --date 2026-11-10 "$w0" "$w1" "$wd/$(basename "${S[2]}")" 2>&1 | grep -q '1 shortfall entry no README line names (3)'
expect "a shortfall no README line names still counts" $?

echo "== a refused X2 jump and a ping-less return (made-up data)"
rf="$work/refused"; mkdir "$rf"
for i in 0 1 2; do cp -r "${S[$i]}" "$rf/"; done
r1="$rf/$(basename "${S[1]}")"; r2="$rf/$(basename "${S[2]}")"
python3 - "$r1" "$r2" <<'PY'
import sys
d1, d2 = sys.argv[1:3]
# slice 1: entry 46 was refused (no record, no t_reboot/back/rc_cycle); its meta.txt carries the cause
t = open(d1 + '/README.md').read().replace('board.sh imx8mp e46 rc=0 verdict=done resets=absent', 'board.sh x2 e46 rc=3 verdict=refused')
open(d1 + '/README.md', 'w').write(t)
rows = [l for l in open(d1 + '/results.tsv').read().split('\n') if l.split('\t')[0] != '46']
open(d1 + '/results.tsv', 'w').write('\n'.join(rows))
open(d1 + '/meta.txt', 'w').write('jump=refused (exit 3)\nrefused_rc=3\nrefused=kexec_zephyr refused (exit 3): made-up loader line\n')
# slice 2: a return without ping: lost=, back=1 and a timeline with 'ping n/a'
open(d2 + '/README.md', 'a').write('\nlost=120 back=1\ntimeline: t0 reboot, ping n/a, ssh up at 95 s\n')
PY
python3 "$REPORT" --date 2026-11-10 "$r1" "$(dirname "$r1")/$(basename "${S[0]}")" "$r2" > "$rf.out" 2>&1; rc=$?
{ [ "$rc" = 1 ] && grep -q PARTIAL "$rf.out"; }; expect "refused row: exit 1, PARTIAL (no record for the entry)" $?
grep -q 'refused, not a reset or hang and not counted: e46 (kexec_zephyr refused (exit 3): made-up loader line)' "$rf.out"
expect "refused row: e46 named with its cause" $?
grep -q 'reset verdicts NOT CLEAN\|reset verdicts not clean' "$rf.out"; [ $? = 1 ]; expect "refused row: not a reset verdict problem" $?
grep -q 'resets or hangs counted' "$rf.out"; [ $? = 1 ]; expect "refused row: not counted as a reset or hang" $?
grep -q 'refused kexec jump, with its cause for the close: e46' "$rf.out"; expect "refused row: listed under Remains" $?
mkdir "$rf/ok"; for i in 0 1 2; do cp -r "${S[$i]}" "$rf/ok/"; done
python3 - "$rf/ok/$(basename "${S[2]}")" <<'PY'
import sys
open(sys.argv[1] + '/README.md', 'a').write('\nlost=120 back=1\ntimeline: t0 reboot, ping n/a, ssh up at 95 s\n')
PY
python3 "$REPORT" --date 2026-11-10 "$rf"/ok/*-imx8mp-gles3-* > "$rf/ok.out" 2>&1; rc=$?
expect "ping-less return: complete run exits 0" "$rc"
python3 "$REPORT" --date 2026-11-10 "${S[@]}" 2>&1 | sed -n '1,/^--- anchors/p' > "$rf/base.out"
sed -n '1,/^--- anchors/p' "$rf/ok.out" | sed "s#$rf/ok#$work#g" > "$rf/ok2.out"
cmp -s "$rf/base.out" "$rf/ok2.out"; expect "ping-less return: an ordinary return, the drafts are unchanged" $?

python3 "$REPORT" --help > /dev/null; expect "--help works" $?

if [ "$fails" = 0 ]; then echo "report selftest: OK"; exit 0; fi
echo "report selftest: FAILED ($fails)"; exit 1
