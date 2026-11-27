#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of am62p-notpass-causes.py: a small synthetic TSV against an
# expected output, plus the --strict exit codes.
set -u
here=$(cd "$(dirname "$0")" && pwd)
d=$here/testdata/am62p-notpass-causes
out=$(mktemp "${TMPDIR:-.}/am62p-causes.XXXXXX") || exit 1
trap 'rm -f "$out"' EXIT
fail=0
python3 -I "$here/am62p-notpass-causes.py" "$d/in.tsv" >"$out" || { echo "FAIL: exit status of a plain run"; fail=1; }
if ! diff -u "$d/expected.txt" "$out"; then echo "FAIL: output differs from expected.txt"; fail=1; fi
python3 -I "$here/am62p-notpass-causes.py" --strict "$d/in.tsv" >/dev/null
[ $? -eq 1 ] || { echo "FAIL: --strict must exit 1 with unattributed lines"; fail=1; }
python3 -I "$here/am62p-notpass-causes.py" --strict "$d/clean.tsv" >/dev/null
[ $? -eq 0 ] || { echo "FAIL: --strict must exit 0 when all lines are attributed"; fail=1; }
[ $fail -eq 0 ] && echo "am62p-notpass-causes-selftest: ok"
exit $fail
