#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pi4-session-readme-selftest.sh
#
# Hermetic check of tools/cts/pi4-session-readme.py on the synthetic two-round
# session of testdata/pi4-session-readme/: S1 (round 1, stopped at a refused
# case), S1a (round 2, the one-case image of that case), S1b and S1c (round 2,
# the rest of S1). Rows are the labels' places in the re-split tree, the
# verdict of a stopped image is named, the counts of each image add up to its
# group rows. A tally of a label that is not in the tree, and group rows that
# do not add up, are refused.
set -u
unset LANGUAGE LC_ALL
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
D=$HERE/testdata/pi4-session-readme
PY=$HERE/pi4-session-readme.py
SHA=0123456789abcdef0123456789abcdef01234567
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

T=$(mktemp -d "${TMPDIR:-/tmp}/pi4-session-readme-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
S=$T/2026-11-pi4-gles3-01234567-aborts2
mkdir "$S"
cp "$D"/session/*.tally.txt "$S/"

out=$(python3 -I "$PY" --commit "$SHA" --tree "$D/tree.txt" --times "$D/times.tsv" -o - "$S" 2>&1)
rc=$?
[ $rc -eq 0 ] && ok "README written" || bad "rc=$rc: $out"
has() { printf '%s\n' "$out" | grep -qF -- "$1" && ok "$2" || bad "$2 (missing: $1)"; }
has "commit: $SHA" "commit line"
has "rows 2-5 of" "row range of the re-split tree"
has "(4 images: S1 S1a S1b S1c)" "images in tree order, A1 and Z9 (no tally) left out"
has "| 2 | S1 | 3 | 0 | 0 | 1 | - | - | - | STOPPED after 4 cases (dEQP ended the session early) |" "round-1 label row"
has "| 3 | S1a | 0 | 0 | 0 | 1 | 0.8 s | 0 | 0 | STOPPED after 1 cases (dEQP ended the session early) |" "one-case aborted image"
has "| 4 | S1b | 5 | 0 | 0 | 0 | 3.0 s | 0 | 0 | DONE, all passed |" "finished image, own verdict"
has "| 5 | S1c | 4 | 0 | 2 | 0 | - | - | - | complete |" "finished image without own verdict line"
printf '%s\n' "$out" | grep -q 'incomplete (no complete' && bad "a stopped image is not 'incomplete'" || ok "no stopped image called incomplete"

# S1 and its re-split parts: 4 + 1 + 5 + 6 results in the table rows
sum=$(printf '%s\n' "$out" | awk -F'|' '/^\| [0-9]+ \|/ { t += $4 + $5 + $6 + $7 } END { print t }')
[ "$sum" = 16 ] && ok "counts of the four images add up to 16 cases" || bad "sum $sum, want 16"

# an unknown label is refused
cp "$S/S1b.tally.txt" "$S/Q7.tally.txt"
python3 -I "$PY" --commit "$SHA" --tree "$D/tree.txt" -o - "$S" >"$T/o" 2>&1 \
    && bad "a tally of a label not in the tree is refused" \
    || { grep -q 'not in the tree: Q7' "$T/o" && ok "unknown label refused by name" || bad "refusal text"; }
rm -f "$S/Q7.tally.txt"

# group rows that do not add up are refused
sed 's/^TOTAL .*/TOTAL 9 0 0 0/' "$D/session/S1b.tally.txt" > "$S/S1b.tally.txt"
python3 -I "$PY" --commit "$SHA" --tree "$D/tree.txt" -o - "$S" >"$T/o" 2>&1 \
    && bad "group rows that do not add up are refused" \
    || { grep -q 'do not add up' "$T/o" && ok "inconsistent tally refused" || bad "refusal text"; }

printf '%s\n' "$out" | grep -Eq '/home/|/Users/|/dev/tty|\.ts\.net|([0-9]{1,3}\.){3}[0-9]{1,3}' \
    && bad "README names a private detail" || ok "README names no home path, address or tty"

echo "pi4-session-readme selftest: $fail failed"
[ $fail -eq 0 ]
