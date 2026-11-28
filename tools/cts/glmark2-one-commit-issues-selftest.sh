#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of glmark2-one-commit-issues.py on synthetic (invented) records built by
# testdata/glmark2-one-commit-issues/build.py. UPDATE=1 rewrites the expected texts.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
td=$here/testdata/glmark2-one-commit-issues
t=$(mktemp -d "${TMPDIR:-/tmp}/g1ci.XXXXXX")
trap 'rm -rf "$t"' EXIT
python3 "$td/build.py" "$t"
fail() { echo "FAIL: $*"; exit 1; }
run() { # case commit -> output in $t/<case>.out, rc echoed
  local c=$1 r=0
  python3 "$here/glmark2-one-commit-issues.py" --am62p "$t/$c/records/am62p/common-"* \
    --pi4 "$t/$c/records/pi4/common-"* --imx8mp "$t/$c/records/imx8mp/common-"* --commit "$2" \
    > "$t/$c.out" 2> "$t/$c.err" || r=$?
  echo "$r"
}
for c in plain x2 mismatch; do
  r=$(run $c 0badf00d)
  [ "$r" = 0 ] || fail "$c exited $r: $(cat "$t/$c.err")"
  sed "s#$t/##g" "$t/$c.out" > "$t/$c.txt"
  if [ "${UPDATE:-}" = 1 ]; then cp "$t/$c.txt" "$td/$c.expected"; fi
  diff -u "$td/$c.expected" "$t/$c.txt" || fail "$c output differs from $c.expected"
done
for k in 1196 1209 15 17 1606; do grep -q "^#$k:" "$t/plain.out" || fail "no #$k section"; done
grep -q 'Closes #1196' "$t/plain.out" || fail "plain does not close #1196"
grep -q 'Armadillo-X2' "$t/x2.out" || fail "x2 #15 text lacks Armadillo-X2"
grep -q 'Armadillo-X2' "$t/plain.out" && fail "plain text names the Armadillo-X2"
grep -q '^NEW ISSUE' "$t/mismatch.out" || fail "no new-issue text for a mismatching picture"
grep -q '^NEW ISSUE' "$t/plain.out" && fail "new-issue text without a mismatch"
# commits that disagree: records among themselves, and against --commit
r=$(run skew 0badf00d); [ "$r" != 0 ] || fail "skewed records exit 0"
r=$(run plain 0badf00e); [ "$r" != 0 ] || fail "--commit mismatch exits 0"
# no host or address in the output or the fixtures
grep -rEq '([0-9]{1,3}\.){3}[0-9]{1,3}|ssh |pi@' "$t"/*.out "$td" && fail "address or host in output/fixtures"
echo "glmark2-one-commit-issues-selftest: ok"
