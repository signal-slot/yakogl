#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of check-gles3-failures-doc.py: a passing doc, a wrong count, a
# missing family, a family with no rows, an absent section (exit 2), and
# today's docs/es3-cts-failures.md (exit 2 naming the section).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-gles3-failures-doc.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/check-gles3-failures-doc.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "check-gles3-failures-doc-selftest: FAIL: $*" >&2; exit 1; }

printf 'Fail\tdEQP-GLES3.a.1\tm\tFAM-a\nFail\tdEQP-GLES3.a.2\tm\tFAM-a\nFail\tdEQP-GLES3.b.1\tm\n' >"$tmp/np.tsv"
printf 'FAM-b\tdEQP-GLES3.b.*\n' >"$tmp/fam.txt"

# mk NAME ROWS
mk() {
  { printf '## Other\n\ntext\n\n## AM62P failures by cause family\n\n'
    printf '| Family | Count | Sites | Issue |\n|---|---:|---|---|\n%s\n\n## Next\n' "$2"; } >"$tmp/$1.md"
}
A='| `FAM-a` | 2 | `src/x.rs:1` | #1 |'
B='| FAM-b | 1 | `src/y.rs:2` | #2 |'
mk pass "$A
$B"
mk count "| FAM-a | 3 | s | #1 |
$B"
mk missing "$A"
mk extra "$A
$B
| FAM-c | 1 | s | #3 |"
printf '## Other\n' >"$tmp/absent.md"

# run NAME WANT_EXIT WANT_TEXT
run() {
  rc=0
  python3 -I "$chk" "$tmp/$1.md" "$tmp/np.tsv" "$tmp/fam.txt" >"$tmp/out" 2>&1 || rc=$?
  [ "$rc" = "$2" ] || { cat "$tmp/out" >&2; fail "$1: exit $rc, want $2"; }
  grep -q "$3" "$tmp/out" || { cat "$tmp/out" >&2; fail "$1: output lacks '$3'"; }
}
run pass 0 "ok, 2 families, 3 rows"
run count 1 "table says 3, triage counts 2"
run missing 1 "family 'FAM-b' (1 rows) is missing"
run extra 1 "'FAM-c' names a family with no rows"
run absent 2 "section '## AM62P failures by cause family' is absent"

rc=0
python3 -I "$chk" "$here/../../docs/es3-cts-failures.md" "$here/../../docs/es3/am62p-gles3-notpass.tsv" \
  "$here/am62p-gles3-families.txt" >"$tmp/out" 2>&1 || rc=$?
if [ "$rc" != 2 ] || ! grep -q "AM62P failures by cause family" "$tmp/out"; then
  cat "$tmp/out" >&2; fail "current doc: exit $rc"
fi
echo "check-gles3-failures-doc-selftest: ok"
