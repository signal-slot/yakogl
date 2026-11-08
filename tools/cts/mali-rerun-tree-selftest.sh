#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/cts/mali-rerun-tree.py, without a board: two small
# synthetic slices (results.tsv only) in tools/cts/testdata/mali-rerun-tree/
# give a table that must equal expected.txt byte for byte, with --max 3 (rows
# of board-lost and not-run cases fill up, a crash and a hang stand alone, one
# case Lost in both slices is listed once); the table must pass
# check-gles3-tree.py against the synthetic mustpass (exactly the Lost cases)
# and mali-deqp-run.sh --dry-run --tree must print its plan. --reason filters
# and the error exits are checked too.
#
#   tools/cts/mali-rerun-tree-selftest.sh   prints "mali-rerun-tree-selftest: N checks passed"
#
# Host only, a few seconds.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
D=$HERE/testdata/mali-rerun-tree
TOOL=$HERE/mali-rerun-tree.py
MP=$D/mustpass.txt
n=0 fails=0
ok() { n=$((n + 1)); echo "  ok    $1"; }
bad() { n=$((n + 1)); fails=$((fails + 1)); echo "  FAIL  $1"; }
check() { if "${@:2}"; then ok "$1"; else bad "$1"; fi; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/mali-rerun-tree.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
SL=("$D"/a/*/ "$D"/b/*/)

python3 -I "$TOOL" --mustpass "$MP" --max 3 "${SL[@]}" > "$tmp/table.txt" 2> "$tmp/err.txt"
check "the table is built (exit 0, no stderr)" test $? -eq 0 -a ! -s "$tmp/err.txt"
check "the table equals expected.txt byte for byte" cmp -s "$tmp/table.txt" "$D/expected.txt"
python3 -I "$HERE/check-gles3-tree.py" --mustpass "$MP" --max 3 "$tmp/table.txt" > "$tmp/chk.txt" 2>&1
check "the table passes check-gles3-tree.py against the synthetic mustpass" test $? -eq 0
grep -q ' 9 matched once, 0 not matched, 0 matched more than once, 0 empty' "$tmp/chk.txt" \
  && ok "the check counts all 9 Lost cases once" || { bad "the check counts all 9 Lost cases once"; cat "$tmp/chk.txt"; }
rows=$(grep -c '^# image ' "$tmp/table.txt")
MALI_DEQP_MUSTPASS=$MP "$HERE/mali-deqp-run.sh" --dry-run --rows "1-$rows" --tree "$tmp/table.txt" > "$tmp/plan.txt" 2>&1
check "mali-deqp-run.sh --dry-run --tree exits 0" test $? -eq 0
grep -q "$rows row(s), 9 case(s)" "$tmp/plan.txt" \
  && ok "the plan has $rows rows and 9 cases" || { bad "the plan has $rows rows and 9 cases"; cat "$tmp/plan.txt"; }

python3 -I "$TOOL" --mustpass "$MP" --reason chunk-hang "${SL[@]}" > "$tmp/hang.txt"
check "--reason chunk-hang keeps one row of one case" \
  test "$(grep -vc '^#' "$tmp/hang.txt")" -eq 1 -a "$(grep -c 'c.hang1$' "$tmp/hang.txt")" -eq 1
python3 -I "$TOOL" --mustpass "$MP" --reason board-lost --reason not-run "${SL[@]}" > "$tmp/lost.txt"
check "two --reason options keep board-lost and not-run only" \
  test "$(grep -c 'crash1\|hang1' "$tmp/lost.txt")" -eq 0
check "the default --max is 400 (one grouped row between the isolated cases)" \
  test "$(python3 -I "$TOOL" --mustpass "$MP" "${SL[@]}" | grep -c '^# image ')" -eq 5

python3 -I "$TOOL" --mustpass "$MP" "$tmp/nonesuch" > /dev/null 2>&1
check "a slice that cannot be read exits 2" test $? -eq 2
python3 -I "$TOOL" --mustpass "$MP" --reason bogus "${SL[@]}" > /dev/null 2>&1
check "an unknown --reason exits 2" test $? -eq 2
head -4 "$MP" > "$tmp/short.txt"
python3 -I "$TOOL" --mustpass "$tmp/short.txt" "${SL[@]}" > /dev/null 2>&1
check "a Lost case outside the mustpass list exits 1" test $? -eq 1

if [ "$fails" -eq 0 ]; then echo "mali-rerun-tree-selftest: $n checks passed"; else echo "mali-rerun-tree-selftest: $fails of $n checks FAILED"; exit 1; fi
