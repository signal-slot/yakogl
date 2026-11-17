#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-gles3-slice.py. The README of the committed 9bc82915 rows17-34 slice is rebuilt from the
# verdict lines taken out of that README (fed twice, as a session's run and tally both print them, with page
# lines and noise between) and its results.tsv; the result has the same verdict lines and passes
# imx8mp-gles3-merge.py --check --partial and imx8mp-gles3-redact.py --check. A one-row slice (rows58, entry
# 57) passes the same checks and times.py reads it. Refused: a short hash, a missing entry, two different lines
# for one entry, an entry outside the range, a missing record directory, a results.tsv without the entry
# (exit 2), and a line the redact check flags (exit 1); none of them prints or writes a README.
# Host only, no argument; scratch under $TMPDIR (else the current directory), never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-gles3-slice.py
merge=$here/imx8mp-gles3-merge.py
redact=$here/imx8mp-gles3-redact.py
slice=$here/log/2026-11-imx8mp-gles3-9bc82915-rows17-34
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

# 1. the committed rows17-34 slice (entries 16-33), rebuilt. Its README quotes e26 a second time (the rerun, with
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

echo "slice-selftest: $nok ok, $nfail failed"
[ "$nfail" = 0 ]
