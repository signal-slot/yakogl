#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-family-tree.py, without a board: a synthetic
# slice (refusals.tsv only), mustpass list and families file are written to a
# temporary directory. The table groups the refused cases by family in the
# families file's order and mustpass order, splits a family at --max, lists a
# case once, leaves out a case no line matches and a ARITH_CONST text the real
# families file does not carry, and passes check-gles3-tree.py. --refusal
# filters; every text of the real families file must occur in
# src/shaderc/online/mali/*.rs or carry a "#historical<TAB>text" line; the error exits are checked, and the real families file must not
# name the plan #367 ARITH_CONST text.
#
#   tools/cts/mali-family-tree-selftest.sh   prints "mali-family-tree-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mali-family-tree.py
n=0 fails=0
ok() { n=$((n + 1)); echo "  ok    $1"; }
bad() { n=$((n + 1)); fails=$((fails + 1)); echo "  FAIL  $1"; }
check() { if "${@:2}"; then ok "$1"; else bad "$1"; fi; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-family-tree.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/a" "$tmp/b"
printf 'c.one\nc.two\nc.three\nc.four\nc.five\nc.six\nc.seven\n' > "$tmp/mp.txt"
printf 'fam1\ttext of the first\nfam1\talso the first\nfam2\ttext of the second\n' > "$tmp/fam.txt"
printf 'case\trefusal\nc.six\tMali(x: text of the second is not lowered)\nc.two\tMali(x: also the first)\nc.one\tMali(x: text of the first)\nc.five\tMali(x: an unlisted text)\n' > "$tmp/a/refusals.tsv"
printf 'case\trefusal\nc.one\tMali(x: text of the first)\nc.four\tMali(x: text of the first)\nc.seven\tMali(x: text of the first)\n' > "$tmp/b/refusals.tsv"

python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/fam.txt" --max 2 "$tmp/a" "$tmp/b" > "$tmp/t.txt" 2> "$tmp/err.txt"
check "the table is built (exit 0, no stderr)" test $? -eq 0 -a ! -s "$tmp/err.txt"
check "fam1 fills two rows of at most 2, in mustpass order" \
  test "$(grep -c '^c.one,c.two$\|^c.four,c.seven$' "$tmp/t.txt")" -eq 2
check "fam2 follows in row 3" test "$(grep -c '^c.six$' "$tmp/t.txt")" -eq 1
check "the unlisted case is left out" test "$(grep -c 'c.five' "$tmp/t.txt")" -eq 0
check "a case in two slices is listed once" test "$(grep -o 'c.one' "$tmp/t.txt" | wc -l)" -eq 1
check "the header names the rows of each family" \
  test "$(grep -c '^# fam1: T6RF001-T6RF002, 4 case(s)$\|^# fam2: T6RF003-T6RF003, 1 case(s)$' "$tmp/t.txt")" -eq 2
python3 -I "$HERE/check-gles3-tree.py" --mustpass "$tmp/mp.txt" --max 2 "$tmp/t.txt" > "$tmp/chk.txt" 2>&1
grep -q ' 5 matched once, 2 not matched, 0 matched more than once, 0 empty' "$tmp/chk.txt" \
  && ok "check-gles3-tree.py counts the 5 cases once" || { bad "check-gles3-tree.py counts the 5 cases once"; cat "$tmp/chk.txt"; }

python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/fam.txt" --refusal 'second' "$tmp/a" "$tmp/b" > "$tmp/f.txt"
check "--refusal keeps the cases of that text only" \
  test "$(grep -vc '^#' "$tmp/f.txt")" -eq 1 -a "$(grep -c '^c.six$' "$tmp/f.txt")" -eq 1
python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/fam.txt" --refusal 'second' --refusal 'also' "$tmp/a" "$tmp/b" > "$tmp/g.txt"
check "--refusal repeats: two texts keep both families" test "$(grep -c '^# image ' "$tmp/g.txt")" -eq 2

python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/fam.txt" "$tmp/nonesuch" > /dev/null 2>&1
check "a slice that cannot be read exits 2" test $? -eq 2
python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/fam.txt" > /dev/null 2>&1
check "no slice exits 2" test $? -eq 2
python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/fam.txt" --max 0 "$tmp/a" > /dev/null 2>&1
check "--max 0 exits 2" test $? -eq 2
head -2 "$tmp/mp.txt" > "$tmp/short.txt"
python3 -I "$TOOL" --mustpass "$tmp/short.txt" --families "$tmp/fam.txt" "$tmp/a" > /dev/null 2>&1
check "a refused case outside the mustpass list exits 1" test $? -eq 1
printf 'fam1 no tab\n' > "$tmp/badfam.txt"
python3 -I "$TOOL" --mustpass "$tmp/mp.txt" --families "$tmp/badfam.txt" "$tmp/a" > /dev/null 2>&1
check "a families line without a tab exits 2" test $? -eq 2

check "the real families file names no ARITH_CONST text (plan #367's)" \
  test "$(grep -v '^#' "$HERE/mali-family-refusals.txt" | grep -c 'a constant product in a shape')" -eq 0
check "every real families line is family<TAB>text" \
  test "$(grep -v '^#' "$HERE/mali-family-refusals.txt" | grep -vcP '^[a-z]+\t[^\t]+$')" -eq 0

# The families file against the compiler: no network, no build, grep only.
REPO=$(cd "$HERE/../.." && pwd)
FAMS=$HERE/mali-family-refusals.txt
miss=0 stale=0
while IFS=$'\t' read -r fam text; do
  case $fam in
    '#historical')
      if grep -qF -- "$text" "$REPO"/src/shaderc/online/mali/*.rs; then
        echo "  stale #historical (the compiler has it): $text"; stale=$((stale + 1))
      fi
      continue ;;
    ''|'#'*) continue ;;
  esac
  grep -qF -- "$text" "$REPO"/src/shaderc/online/mali/*.rs && continue
  grep -qxF -- "#historical"$'\t'"$text" "$FAMS" && continue
  echo "  missing: $fam: $text (in neither the compiler nor a #historical line)"
  miss=$((miss + 1))
done < "$FAMS"
check "every families text is in src/shaderc/online/mali/*.rs or marked #historical" test "$miss" -eq 0
check "no #historical line names a text the compiler still has" test "$stale" -eq 0

if [ "$fails" -eq 0 ]; then echo "mali-family-tree-selftest: $n checks passed"; else echo "mali-family-tree-selftest: $fails of $n checks FAILED"; exit 1; fi
