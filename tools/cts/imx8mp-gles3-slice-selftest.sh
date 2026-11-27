#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-gles3-slice.py. The README of the 9bc82915 rows17-34 slice is rebuilt from the fixture's
# README (tools/cts/testdata/imx8mp-gles3-slice/: a byte-for-byte copy of that slice's README.md, results.tsv
# and rows.md, so retiring the campaign's slice directories does not break this test), from the verdict lines
# taken out of that README (fed twice, as a session's run and tally both print them, with page
# lines and noise between) and its results.tsv; the result has the same verdict lines and passes
# imx8mp-gles3-merge.py --check --partial and imx8mp-gles3-redact.py --check. A one-row slice (rows58, entry
# 57) passes the same checks and times.py reads it. Refused: a short hash, a missing entry, two different lines
# for one entry, an entry outside the range, a missing record directory, a results.tsv without the entry
# (exit 2), and a line the redact check flags (exit 1); none of them prints or writes a README.
# An Armadillo-X2 slice (rows53-54 from the fixture's x2-lines.txt and x2-results.tsv: board.sh x2 lines, '-' in
# begun) says it was measured on the Armadillo-X2, names records/x2 and the X2 commands, passes merge --check
# --partial alone and beside the Verdin slice, takes --run-date for the Run lines, and lines of both boards in one
# slice are refused.
# Reset and hang rows (--scan, x2-faults-*.txt): marker-led README lines, counted by report.py's fault_events.
# Host only, no argument; scratch under $TMPDIR (else the current directory), never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-gles3-slice.py
merge=$here/imx8mp-gles3-merge.py
redact=$here/imx8mp-gles3-redact.py
slice=$here/testdata/imx8mp-gles3-slice
full=9bc82915e06367b46e3a62b644dbf2bb659d72ad
base=${TMPDIR:-$PWD}
case "$(cd "$base" && pwd -P)" in /tmp | /tmp/* | /var/tmp | /var/tmp/*) echo "slice-selftest: refusing scratch under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "$base/gles3-slice-selftest.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
nok=0 nfail=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }
run() { python3 -I "$tool" "$@"; }
vlines() { grep -E '^board\.sh imx8mp e[0-9]+ rc=' "$1"; }

# 1. the fixture rows17-34 slice (entries 16-33), rebuilt. Its README quotes e26 a second time (the rerun, with
# another uptime): the first line of each entry is the one in its fenced verdict block.
vlines "$slice/README.md" | awk '!seen[$3]++' > "$tmp/lines.txt"
{
	cat "$tmp/lines.txt"
	echo "imx8mp: e18 page 0 of 12"
	sed -n 's/^\(board\.sh imx8mp e18\) \(rc=.*\)$/\1-p0 \2/p' "$tmp/lines.txt"
	sed -n 's/^\(board\.sh imx8mp e18\) \(rc=.*\)$/\1-p1 \2/p' "$tmp/lines.txt"
	sed 's/^/  /' "$tmp/lines.txt"
} > "$tmp/stdout.txt"
mkdir -p "$tmp/a/runs/20261115-1010-e16-0123abcd"
for e in $(seq 17 33); do mkdir -p "$tmp/a/runs/20261115-1010-e$e-0123abcd"; done
if run --commit $full --rows 17-34 --date 2026-11-15 --lines "$tmp/stdout.txt" --results "$slice/results.tsv" \
	--records "$tmp/a/runs" --out "$tmp/a/slice" > "$tmp/a.log" 2>&1; then
	ok "rows17-34 written"
else
	bad "rows17-34 refused: $(cat "$tmp/a.log")"
fi
cp "$slice/results.tsv" "$slice/rows.md" "$tmp/a/slice/"
vlines "$tmp/a/slice/README.md" > "$tmp/out.txt"
cmp -s "$tmp/lines.txt" "$tmp/out.txt" && [ "$(wc -l < "$tmp/out.txt")" = 18 ] \
	&& ok "18 verdict lines, verbatim and in entry order" || bad "verdict lines differ from the committed README's"
grep -q "^commit $full\$" "$tmp/a/slice/README.md" && ok "40-hex commit line" || bad "no 40-hex commit line"
grep -q -- '-p[0-9]' "$tmp/out.txt" && bad "a page line was copied" || ok "page lines not copied"
grep -q '^| e18 | 0 | done | absent | 54s | 25s | 2 | ' "$tmp/a/slice/README.md" \
	&& ok "per-entry row: back_after, uptime, 2 pages" || bad "per-entry row of e18"
grep -q '^Records (gitignored): `records/imx8mp/runs`\.$' "$tmp/a/slice/README.md" \
	&& ok "records named by their last component" || bad "records line"
python3 -I "$merge" --check --partial "$tmp/a/slice" > "$tmp/m.txt" 2>&1 \
	&& ok "merge --check --partial passes" || bad "merge --check: $(cat "$tmp/m.txt")"
python3 -I "$merge" --notpass --partial "$tmp/a/slice" > "$tmp/n1.txt" 2>/dev/null
python3 -I "$merge" --notpass --partial "$slice" > "$tmp/n2.txt" 2>/dev/null
[ -s "$tmp/n1.txt" ] && cmp -s "$tmp/n1.txt" "$tmp/n2.txt" \
	&& ok "merge --notpass equals the committed slice's" || bad "merge --notpass differs from the committed slice's"
python3 -I "$redact" --check "$tmp/a/slice/README.md" > "$tmp/r.txt" 2>&1 \
	&& ok "redact --check passes" || bad "redact --check: $(cat "$tmp/r.txt")"
ls -A "$tmp/a/slice" | grep -q '^\.\|\.part$' && bad "scratch left beside the README" || ok "no scratch left"
run --commit $full --rows 17-34 --date 2026-11-15 --lines - --results "$slice/results.tsv" \
	--records "$tmp/a/runs" < "$tmp/stdout.txt" > "$tmp/stdout.md" 2> "$tmp/s.log" \
	&& cmp -s "$tmp/stdout.md" "$tmp/a/slice/README.md" && ok "stdout form (lines on stdin) equals the written README" \
	|| bad "stdout form: $(cat "$tmp/s.log")"

# 2. a one-row slice, rows58 = entry 57 (no committed slice has it: e16's line renamed, a results.tsv with
# only row 57 filled and every other row '-', as board.sh tally writes it)
sed -n 's/^\(board\.sh imx8mp\) e16 /\1 e57 /p' "$tmp/lines.txt" > "$tmp/one.txt"
mkdir -p "$tmp/b"
awk -F'\t' 'BEGIN { OFS = "\t" } /^#/ { print; next }
	{ if ($1 == 57) print $1, $2, 5, 4, 1, 0, 0, 0, "dEQP-GLES3.x.y"; else print $1, $2, "-", "-", "-", "-", "-", "-", "-" }' \
	"$slice/results.tsv" > "$tmp/b/results.tsv"
run --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --results "$tmp/b/results.tsv" --out "$tmp/b" \
	> "$tmp/b.log" 2>&1 && ok "rows58 written" || bad "rows58 refused: $(cat "$tmp/b.log")"
grep -q '^# i\.MX8MP dEQP-GLES3 table image es3-9bc82915, rows 58 (entry 57)$' "$tmp/b/README.md" \
	&& ok "rows58: the title names entry 57" || bad "rows58 title"
grep -q '^- e57: 1 failing: dEQP-GLES3\.x\.y$' "$tmp/b/README.md" && ok "rows58: not-pass list" || bad "rows58 not-pass list"
python3 -I "$merge" --check --partial "$tmp/b" > "$tmp/m.txt" 2>&1 \
	&& ok "rows58: merge --check --partial passes" || bad "rows58 merge --check: $(cat "$tmp/m.txt")"
python3 -I "$redact" --check "$tmp/b/README.md" > "$tmp/r.txt" 2>&1 \
	&& ok "rows58: redact --check passes" || bad "rows58 redact --check: $(cat "$tmp/r.txt")"
python3 -I "$here/imx8mp-gles3-times.py" --tsv "$tmp/b" 2>/dev/null | grep -q "	e57	68	" \
	&& ok "rows58: times.py reads 93s - 25s" || bad "rows58: times.py"

# 3. refusals: exit 2 (exit 1 for a redact hit), nothing printed and no README written
refuse() { # name want_rc args...
	local n=$1 want=$2 rc
	shift 2
	rm -rf "$tmp/c"
	run "$@" --out "$tmp/c" > "$tmp/o.txt" 2> "$tmp/e.txt"
	rc=$?
	if [ "$rc" = "$want" ] && [ ! -s "$tmp/o.txt" ] && [ ! -e "$tmp/c/README.md" ] && [ -s "$tmp/e.txt" ]; then
		ok "$n refused: rc $want, nothing written"
	else
		bad "$n: rc $rc (want $want), $(cat "$tmp/e.txt")"
	fi
}
refuse "short hash" 2 --commit 9bc82915 --rows 58 --date 2026-11-15 --lines "$tmp/one.txt"
refuse "upper-case hash" 2 --commit "$(tr a-f A-F <<< $full)" --rows 58 --date 2026-11-15 --lines "$tmp/one.txt"
refuse "missing entry" 2 --commit $full --rows 57-58 --date 2026-11-15 --lines "$tmp/one.txt"
sed 's/back_after=93s/back_after=94s/' "$tmp/one.txt" > "$tmp/rerun.txt"
refuse "duplicate entry" 2 --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --lines "$tmp/rerun.txt"
refuse "entry outside the range" 2 --commit $full --rows 59 --date 2026-11-15 --lines "$tmp/one.txt"
refuse "bad rows" 2 --commit $full --rows 58-57 --date 2026-11-15 --lines "$tmp/one.txt"
refuse "no record directory" 2 --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --records "$tmp/a/runs"
grep -v '^57	' "$tmp/b/results.tsv" > "$tmp/no57.tsv"
refuse "results without the entry" 2 --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" \
	--results "$tmp/no57.tsv"
addr="192.0.2"".7"
sed "s/\$/ host=$addr/" "$tmp/one.txt" > "$tmp/leak.txt"
refuse "an address in a line" 1 --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/leak.txt"

# 4. the optional inputs: the default output is the same file as before (section 1's README, without them), and a
# draft with --artefacts, --preflight and --window has the What ran and Console sections, no path, and passes the
# redact check and merge --check --partial
pre=$tmp/ws"/out"
{
	printf '16443032 %064d %s/es3-9bc82915/zephyr/zephyr.bin\n' 1 "$pre"
	printf '38164 %064d %s/es3-9bc82915.table.conf\n' 2 "$pre"
	printf '132944 %064d %s/deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a\n' 3 "$pre"
	echo '# CONFIG_DEQP_REPORT_ES2 is not set'
	echo 'Memory region         Used Size  Region Size  %age Used'
	echo '             RAM:     222568 KB       256 MB     84.90%'
} > "$tmp/art.txt"
{ for i in 1 2 3; do echo "  ok   check $i ($pre/es3-9bc82915/zephyr/zephyr.bin)"; done; echo "preflight: all ok"; } > "$tmp/pre.txt"
run --commit $full --rows 17-34 --date 2026-11-15 --lines "$tmp/stdout.txt" --results "$slice/results.tsv" \
	--records "$tmp/a/runs" > "$tmp/plain.md" 2>&1
# the X2 campaign's slice name 2026-11-imx8mp-gles3-x2-<hash8>-rowsAA-BB is taken from --out; another --out is not
h8=${full:0:8}
for nm in 2026-11-imx8mp-gles3-x2-$h8-rows53-54 2026-11-28-imx8mp-gles3-$h8-rows53-54; do
	run --commit $full --rows 53-54 --date 2026-11-28 --lines "$slice/x2-lines.txt" --results "$slice/x2-results.tsv" \
		--out "$tmp/n/$nm" > "$tmp/n.log" 2>&1 || bad "x2 name $nm: refused: $(cat "$tmp/n.log")"
	grep -qF "\`$nm\`, rows 53-54 of" "$tmp/n/$nm/README.md" && grep -q '^commit '$full'$' "$tmp/n/$nm/README.md" \
		&& ok "x2: --out $nm names the slice and the README keeps the commit" || bad "x2: slice name $nm"
done
for nm in 2026-11-imx8mp-gles3-x2-$h8-rows53-55 2026-11-imx8mp-gles3-x2-0123abcd-rows53-54 scratch; do
	run --commit $full --rows 53-54 --date 2026-11-28 --lines "$slice/x2-lines.txt" --results "$slice/x2-results.tsv" \
		--out "$tmp/n/$nm" > "$tmp/n.log" 2>&1
	grep -qF '`2026-11-28-imx8mp-gles3-'$h8'-rows53-54`' "$tmp/n/$nm/README.md" \
		&& ok "x2: --out $nm (other rows, commit or form) is not taken as the name" || bad "x2: slice name for --out $nm"
done
run --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --out "$tmp/n/2026-11-imx8mp-gles3-x2-$h8-rows58-58" > "$tmp/n.log" 2>&1
grep -qF '`2026-11-15-imx8mp-gles3-'$h8'-rows58-58`' "$tmp/n/2026-11-imx8mp-gles3-x2-$h8-rows58-58/README.md" \
	&& ok "Verdin: the slice name never comes from --out" || bad "Verdin: slice name taken from --out"
cmp -s "$tmp/plain.md" "$tmp/a/slice/README.md" && ok "default output unchanged without the new options" \
	|| bad "default output differs"
grep -q '^## What ran\|^## Console' "$tmp/plain.md" && bad "default output has the new sections" || ok "default has no new sections"
mkdir -p "$tmp/d"
run --commit $full --rows 17-34 --date 2026-11-15 --lines "$tmp/stdout.txt" --results "$slice/results.tsv" \
	--artefacts "$tmp/art.txt" --preflight "$tmp/pre.txt" --window 20:46/21:33 --out "$tmp/d" > "$tmp/d.log" 2>&1 \
	&& ok "draft with the new options written" || bad "draft with the new options refused: $(cat "$tmp/d.log")"
cp "$slice/results.tsv" "$slice/rows.md" "$tmp/d/"
d=$tmp/d/README.md
grep -q '^## What ran$' "$d" && grep -q '^## Console$' "$d" && ok "What ran and Console sections" || bad "sections missing"
grep -q 'not captured, recoveries not observed' "$d" && ok "console: not captured" || bad "console text"
grep -q '^Run 2026-11-15, 20:46-21:33 UTC' "$d" && ok "window written" || bad "window"
grep -q '^16443032 0*1 es3-9bc82915/zephyr/zephyr.bin$' "$d" && grep -q '^132944 0*3 deqp-stage1-gles3/build/' "$d" \
	&& ok "artefact lines cut below IMX8MP_OUT" || bad "artefact lines"
grep -q 'RAM:     222568 KB' "$d" && ok "memory line kept" || bad "memory line"
grep -q '3 `ok` lines, `preflight: all ok`' "$d" && ok "preflight result" || bad "preflight result"
grep -q 'TREE_ROWS=16-33 TREE_IMAGE=\$IMX8MP_OUT/es3-9bc82915/' "$d" && ok "commands with knob names" || bad "commands"
grep -qF "$pre" "$d" && bad "a path was copied" || ok "no path copied"
python3 -I "$redact" --check "$d" > "$tmp/r.txt" 2>&1 && ok "new-options draft passes redact --check" || bad "redact: $(cat "$tmp/r.txt")"
python3 -I "$merge" --check --partial "$tmp/d" > "$tmp/m.txt" 2>&1 && ok "new-options draft: merge --check passes" \
	|| bad "new-options merge --check: $(cat "$tmp/m.txt")"
refuse "bad window" 2 --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --window 25:00/26:00
refuse "empty artefacts" 2 --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --artefacts "$tmp/pre.txt"

# 5. an Armadillo-X2 slice, rows53-54 (entries 52-53): the fixture's x2-lines.txt (board.sh x2 verdict lines, a page
# line, the run's "===" lines) and x2-results.tsv (three-word records: '-' in begun and noresult)
mkdir -p "$tmp/x/runs/20261128-0102-e52-89abcdef" "$tmp/x/runs/20261128-0109-e53-89abcdef"
run --commit $full --rows 53-54 --date 2026-11-16 --lines "$slice/x2-lines.txt" --results "$slice/x2-results.tsv" \
	--records "$tmp/x/runs" --out "$tmp/x/slice" > "$tmp/x.log" 2>&1 && ok "x2 rows53-54 written" \
	|| bad "x2 rows53-54 refused: $(cat "$tmp/x.log")"
cp "$slice/x2-results.tsv" "$tmp/x/slice/results.tsv"
x=$tmp/x/slice/README.md
grep -qF 'measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2 (crash-route kexec)' "$x" \
	&& ok "x2: the README says measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)" || bad "x2: no measured-on line"
grep -q "owner's decision of 2026-11-27" "$x" && ok "x2: the owner's decision named" || bad "x2: no owner's decision"
grep -q 'Toradex Verdin iMX8M Plus (GC7000UL): the slice' "$x" && bad "x2: the Verdin sentence was written" \
	|| ok "x2: no Verdin run sentence"
[ "$(grep -c '^board\.sh x2 e5[23] rc=0 verdict=done boot=kexec-dev DEV ' "$x")" = 2 ] \
	&& ok "x2: two verdict lines, verbatim" || bad "x2: verdict lines"
grep -q 'e53-p0' "$x" && bad "x2: a page line was copied" || ok "x2: page lines not copied"
grep -q '^| e53 | 0 | done | absent | 38s | - | 1 | - | 132 | 2 | 0 | 0 | - |$' "$x" \
	&& ok "x2: per-entry row (no uptime, one page, begun and none '-')" || bad "x2: per-entry row of e53"
grep -q '^Records (gitignored): `records/x2/runs`\.$' "$x" && ok "x2: records under records/x2" || bad "x2: records line"
python3 -I "$merge" --check --partial "$tmp/x/slice" > "$tmp/m.txt" 2>&1 \
	&& ok "x2: merge --check --partial passes" || bad "x2: merge --check: $(cat "$tmp/m.txt")"
python3 -I "$merge" --check --partial "$tmp/a/slice" "$tmp/x/slice" > "$tmp/m.txt" 2>&1 \
	&& ok "x2: a Verdin slice and an X2 slice of one commit merge" || bad "x2: Verdin+X2 merge --check: $(cat "$tmp/m.txt")"
python3 -I "$redact" --check "$x" > "$tmp/r.txt" 2>&1 && ok "x2: redact --check passes" || bad "x2: redact: $(cat "$tmp/r.txt")"
python3 -I "$here/imx8mp-gles3-times.py" --tsv "$tmp/x/slice" 2>/dev/null | grep -q "	e53	" \
	&& ok "x2: times.py reads the X2 lines" || bad "x2: times.py"
pre2=$tmp/ws2"/out"
{
	printf '16443032 %064d %s/x2/es3-9bc82915/zephyr/zephyr.bin\n' 1 "$pre2"
	printf '132944 %064d %s/deqp-stage1-gles3/build/modules/gles3/libdeqp-gles3-package.a\n' 3 "$pre2"
} > "$tmp/art2.txt"
mkdir -p "$tmp/y"
run --commit $full --rows 53-54 --date 2026-11-16 --lines "$slice/x2-lines.txt" --results "$slice/x2-results.tsv" \
	--artefacts "$tmp/art2.txt" --preflight "$tmp/pre.txt" --window 01:00/01:15 --out "$tmp/y" > "$tmp/y.log" 2>&1 \
	&& ok "x2 draft with the new options written" || bad "x2 draft refused: $(cat "$tmp/y.log")"
y=$tmp/y/README.md
grep -q '^16443032 0*1 x2/es3-9bc82915/zephyr/zephyr.bin$' "$y" && ok "x2: artefact path cut below IMX8MP_OUT, x2/ kept" \
	|| bad "x2: artefact line"
grep -q 'imx8mp-gles3-artefacts.sh x2/es3-9bc82915' "$y" && grep -q 'imx8mp-gles3-preflight.sh --board x2 --commit' "$y" \
	&& ok "x2: artefacts and preflight commands name the X2" || bad "x2: artefacts/preflight command lines"
grep -q 'TREE_ROWS=52-53 TREE_IMAGE=\$IMX8MP_OUT/x2/es3-9bc82915/zephyr/zephyr.bin IMX8MP_BOARD=x2' "$y" \
	&& grep -q 'imx8mp-gles3-board.sh tree \$REC$' "$y" && grep -q 'records/x2/<YYYYMMDD-HHMM>-es3-9bc82915-rows53-54-<image sha8>' "$y" \
	&& ! grep -q 'BOARD_BOOT=uboot-mmc' "$y" \
	&& ok "x2: the run commands are the X2's" || bad "x2: run commands"
grep -qF "$pre2" "$y" && bad "x2: a path was copied" || ok "x2: no path copied"
{ cat "$tmp/one.txt"; cat "$slice/x2-lines.txt"; } > "$tmp/mixed.txt"
grep -q '^Run 2026-11-16, 01:00-01:15 UTC' "$y" && ok "x2: without --run-date the Run line takes --date" || bad "x2: Run line"
run --commit $full --rows 53-54 --date 2026-11-16 --run-date 2026-11-28 --lines "$slice/x2-lines.txt" --window 01:00/01:15 \
	--results "$slice/x2-results.tsv" > "$tmp/z.md" 2> "$tmp/z.log"
grep -q '^Run 2026-11-28, measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL)' "$tmp/z.md" && grep -q '^Run 2026-11-28, 01:00-01:15 UTC' "$tmp/z.md" \
	&& grep -q '`2026-11-16-imx8mp-gles3-9bc82915-rows53-54`' "$tmp/z.md" \
	&& ok "x2: --run-date in the Run lines, --date in the slice name" || bad "x2: --run-date: $(cat "$tmp/z.log")"
refuse "bad run date" 2 --commit $full --rows 58 --date 2026-11-15 --run-date 28-11-2026 --lines "$tmp/one.txt"
refuse "lines of both boards" 2 --commit $full --rows 53-58 --date 2026-11-16 --lines "$tmp/mixed.txt"
# the X2 campaign's slice name 2026-11-imx8mp-gles3-x2-<hash8>-rowsAA-BB is taken from --out; another --out is not
h8=${full:0:8}
for nm in 2026-11-imx8mp-gles3-x2-$h8-rows53-54 2026-11-28-imx8mp-gles3-$h8-rows53-54; do
	run --commit $full --rows 53-54 --date 2026-11-28 --lines "$slice/x2-lines.txt" --results "$slice/x2-results.tsv" \
		--out "$tmp/n/$nm" > "$tmp/n.log" 2>&1 || bad "x2 name $nm: refused: $(cat "$tmp/n.log")"
	grep -qF "\`$nm\`, rows 53-54 of" "$tmp/n/$nm/README.md" && grep -q '^commit '$full'$' "$tmp/n/$nm/README.md" \
		&& ok "x2: --out $nm names the slice and the README keeps the commit" || bad "x2: slice name $nm"
done
for nm in 2026-11-imx8mp-gles3-x2-$h8-rows53-55 2026-11-imx8mp-gles3-x2-0123abcd-rows53-54 scratch; do
	run --commit $full --rows 53-54 --date 2026-11-28 --lines "$slice/x2-lines.txt" --results "$slice/x2-results.tsv" \
		--out "$tmp/n/$nm" > "$tmp/n.log" 2>&1
	grep -qF '`2026-11-28-imx8mp-gles3-'$h8'-rows53-54`' "$tmp/n/$nm/README.md" \
		&& ok "x2: --out $nm (other rows, commit or form) is not taken as the name" || bad "x2: slice name for --out $nm"
done
run --commit $full --rows 58 --date 2026-11-15 --lines "$tmp/one.txt" --out "$tmp/n/2026-11-imx8mp-gles3-x2-$h8-rows58-58" > "$tmp/n.log" 2>&1
grep -qF '`2026-11-15-imx8mp-gles3-'$h8'-rows58-58`' "$tmp/n/2026-11-imx8mp-gles3-x2-$h8-rows58-58/README.md" \
	&& ok "Verdin: the slice name never comes from --out" || bad "Verdin: slice name taken from --out"
cmp -s "$tmp/plain.md" "$tmp/a/slice/README.md" && ok "Verdin README unchanged by the X2 support" || bad "Verdin README changed"

# 6. one reset row and one hang row (x2-faults-lines.txt: e52 ends on a WDOG1 window (back_after 612s, the scan's
# NO FINISH kind "watchdog end", with its last case), e53 is verdict=no_record): the README carries a marker-led line
# for each, `WDOG1 e52 ...` and `Hang: e53 ...`, and imx8mp-gles3-report.py counts 1 reset and 1 hang, none unknown.
# Without the scan, a verdict done_with_resets row gives the same WDOG1 line (no last case).
f=$tmp/f
run --commit $full --rows 53-54 --date 2026-11-16 --lines "$slice/x2-faults-lines.txt" --results "$slice/x2-results.tsv" \
	--scan "$slice/x2-faults-scan.txt" --out "$f" > "$tmp/f.log" 2>&1 && ok "fault slice written" || bad "fault slice refused: $(cat "$tmp/f.log")"
cp "$slice/x2-results.tsv" "$f/results.tsv"
grep -q '^- WDOG1 e52: .*back_after=612s.*; last case dEQP-GLES3\.functional\.state_query\.integers\.max_texture_size$' "$f/README.md" \
	&& ok "fault: WDOG1 e52 line with the last case" || bad "fault: WDOG1 e52 line"
grep -q '^- Hang: e53: .*verdict=no_record' "$f/README.md" && ok "fault: Hang e53 line" || bad "fault: Hang e53 line"
grep -qF 'measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL) with tools/board.sh x2 (crash-route kexec)' "$f/README.md" \
	&& ok "fault: still says measured on the Armadillo-X2" || bad "fault: no measured-on line"
cat > "$tmp/count.py" <<'PYEOF'
import importlib.util, sys
sp = importlib.util.spec_from_file_location('rep', sys.argv[1])
m = importlib.util.module_from_spec(sp)
sp.loader.exec_module(m)
resets, hangs, short, unnamed = m.fault_events([sys.argv[2]], [])
print(len(resets), len(hangs), len(unnamed))
PYEOF
[ "$(python3 -I "$tmp/count.py" "$here/imx8mp-gles3-report.py" "$f")" = "1 1 0" ] \
	&& ok "report.py counts 1 reset and 1 hang, none unknown" || bad "report.py count: $(python3 -I "$tmp/count.py" "$here/imx8mp-gles3-report.py" "$f" 2>&1)"
python3 -I "$redact" --check "$f/README.md" > "$tmp/r.txt" 2>&1 && ok "fault: redact --check passes" || bad "fault: redact: $(cat "$tmp/r.txt")"
python3 -I "$merge" --check --partial "$f" > "$tmp/m.txt" 2>&1 && bad "fault: merge --check passed a no_record row" \
	|| ok "fault: merge --check still refuses the not-clean row (as it must)"
sed 's/e52 rc=0 verdict=done /e52 rc=0 verdict=done_with_resets /' "$slice/x2-faults-lines.txt" > "$tmp/f2.txt"
run --commit $full --rows 53-54 --date 2026-11-16 --lines "$tmp/f2.txt" > "$tmp/f2.md" 2>&1
grep -q '^- WDOG1 e52: ' "$tmp/f2.md" && ! grep -q 'last case' "$tmp/f2.md" && ok "fault: verdict alone gives the WDOG1 line" || bad "fault: verdict-only line"
grep -q '^## Resets and hangs' "$tmp/x/slice/README.md" && bad "a clean slice has the fault section" || ok "a clean slice has no fault section"

echo "slice-selftest: $nok ok, $nfail failed"
[ "$nfail" = 0 ]
