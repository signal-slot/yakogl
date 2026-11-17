#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-slice.py: the README of the committed 9bc82915 rows17-34 slice is rebuilt from the
# verdict lines taken out of that README (and its results.tsv) and passes merge.py --check --partial and
# redact.py --check, with the verdict lines identical; a one-row slice (rows58, entry 57) passes the same checks;
# a short hash, a missing entry, a duplicate (different) entry, an entry outside the range and a line that
# redact flags are refused. Host only; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-gles3-slice.py
merge=$here/imx8mp-gles3-merge.py
redact=$here/imx8mp-gles3-redact.py
slice=$here/log/2026-11-imx8mp-gles3-9bc82915-rows17-34
full=9bc82915e06367b46e3a62b644dbf2bb659d72ad
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/gles3-slice-selftest.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }
run() { python3 -I "$tool" "$@"; }

# 1. a committed slice, rebuilt (rows 17-34 are entries 16-33)
# (the committed README quotes e26 a second time with another uptime: the first line of each entry is the fenced one)
grep -E '^board\.sh imx8mp e[0-9]+ rc=' "$slice/README.md" | awk '!seen[$3]++' > "$tmp/lines.txt"
# the run's lines and the tally's lines are both in a session's stdout: feed every line twice
cat "$tmp/lines.txt" "$tmp/lines.txt" > "$tmp/stdout.txt"
mkdir -p "$tmp/a"
run --commit $full --rows 17-34 --date 2026-11-14 --lines "$tmp/stdout.txt" --results "$slice/results.tsv" \
	--out "$tmp/a" > "$tmp/a.log" 2>&1 && ok "rows17-34 written" || { bad "rows17-34 refused: $(cat "$tmp/a.log")"; }
cp "$slice/results.tsv" "$slice/rows.md" "$tmp/a/" 2>/dev/null
grep -E '^board\.sh imx8mp e[0-9]+ rc=' "$tmp/a/README.md" > "$tmp/out.txt"
cmp -s "$tmp/lines.txt" "$tmp/out.txt" && ok "verdict lines verbatim, 18 entries" || bad "verdict lines differ from the committed README"
[ "$(wc -l < "$tmp/out.txt")" = 18 ] || bad "not 18 verdict lines"
grep -q "^commit $full\$" "$tmp/a/README.md" && ok "40-hex commit line" || bad "no commit line"
python3 -I "$merge" --check --partial "$tmp/a" > "$tmp/m.txt" 2>&1 && ok "merge --check --partial passes" || bad "merge --check: $(cat "$tmp/m.txt")"
python3 -I "$redact" --check "$tmp/a/README.md" > "$tmp/r.txt" 2>&1 && ok "redact --check passes" || bad "redact --check: $(cat "$tmp/r.txt")"
python3 -I "$merge" --notpass --partial "$tmp/a" > /dev/null 2>&1 || python3 -I "$merge" --notpass "$tmp/a" > /dev/null 2>&1 && ok "merge --notpass reads it" || bad "merge --notpass fails"
run --commit $full --rows 17-34 --date 2026-11-14 --lines "$tmp/lines.txt" --results "$slice/results.tsv" > "$tmp/stdout.md" 2>/dev/null \
	&& cmp -s "$tmp/stdout.md" "$tmp/a/README.md" && ok "stdout form equals the written README" || bad "stdout form"

# 2. a one-row slice, rows58 = entry 57 (synthetic: no committed slice has it)
grep -E '^board\.sh imx8mp e16 ' "$slice/README.md" | sed 's/ e16 / e57 /' > "$tmp/one.txt"
filter=$(grep -v '^\s*#' "$here/imx8mp-gles3-tree.txt" | grep -v '^\s*$' | sed -n 58p)
mkdir -p "$tmp/b"
{ printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names (%s-separated)\n' "';'"
  i=0
  grep -v '^\s*#' "$here/imx8mp-gles3-tree.txt" | grep -v '^\s*$' | while IFS= read -r f; do
	if [ $i = 57 ]; then printf '%s\t%s\t5\t5\t0\t0\t0\t0\t-\n' $i "$f"; else printf '%s\t%s\t-\t-\t-\t-\t-\t-\t-\n' $i "$f"; fi
	i=$((i + 1))
  done; } > "$tmp/b/results.tsv"
run --commit $full --rows 58-58 --date 2026-11-14 --lines "$tmp/one.txt" --results "$tmp/b/results.tsv" --out "$tmp/b" > "$tmp/b.log" 2>&1 \
	&& ok "rows58 written" || bad "rows58 refused: $(cat "$tmp/b.log")"
head -3 "$tmp/b/README.md" | grep -q 'rows 58 (entry 57)' && ok "rows58 header names entry 57" || bad "rows58 header"
python3 -I "$merge" --check --partial "$tmp/b" > "$tmp/m.txt" 2>&1 && ok "rows58: merge --check --partial passes" || bad "rows58 merge --check: $(cat "$tmp/m.txt")"
python3 -I "$redact" --check "$tmp/b/README.md" > "$tmp/r.txt" 2>&1 && ok "rows58: redact --check passes" || bad "rows58 redact --check: $(cat "$tmp/r.txt")"
python3 -I "$here/imx8mp-gles3-times.py" "$tmp/b" 2>/dev/null | grep -q 'e57\|57' && ok "rows58: times.py reads the entry" || bad "rows58 times.py"

# 3. refusals (exit 2), the redact hit (exit 1), and nothing written
refuse() { # name want_rc args...
	local n=$1 want=$2 rc; shift 2
	run "$@" > "$tmp/o.txt" 2> "$tmp/e.txt"; rc=$?
	[ $rc = "$want" ] && [ ! -s "$tmp/o.txt" ] && ok "$n: rc $want, no output" || bad "$n: rc $rc (want $want), $(cat "$tmp/e.txt")"
}
refuse "short hash" 2 --commit 9bc82915 --rows 58-58 --date 2026-11-14 --lines "$tmp/one.txt"
refuse "missing entry" 2 --commit $full --rows 57-58 --date 2026-11-14 --lines "$tmp/one.txt"
sed 's/back_after=[0-9]*s/back_after=1s/' "$tmp/one.txt" | sed 's/^board/board/' > "$tmp/dup.txt"
cat "$tmp/one.txt" "$tmp/dup.txt" > "$tmp/dup2.txt"
refuse "duplicate entry" 2 --commit $full --rows 58-58 --date 2026-11-14 --lines "$tmp/dup2.txt"
refuse "entry outside the range" 2 --commit $full --rows 59-59 --date 2026-11-14 --lines "$tmp/one.txt"
refuse "no record directory" 2 --commit $full --rows 58-58 --date 2026-11-14 --lines "$tmp/one.txt" --records "$tmp/b"
addr="192.168.0"".7"
sed "s/\$/ host=$addr/" "$tmp/one.txt" > "$tmp/leak.txt"
refuse "address in a line" 1 --commit $full --rows 58-58 --date 2026-11-14 --lines "$tmp/leak.txt"

echo "slice-selftest: $nok ok, $nfail failed"
[ "$nfail" = 0 ]
