#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of check-pi4-gles3-results.py on synthetic inputs
# (tools/cts/testdata/check-pi4-gles3-results/): one consistent set that
# passes, and sets with a wrong sum, a wrong row count, a commit that differs
# between documents and a TSV that differs from the counts, each of which
# must fail naming its problem. Hermetic: every input is a path.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-pi4-gles3-results.py"
td="$here/testdata/check-pi4-gles3-results"
fail() { echo "check-pi4-gles3-results-selftest: FAIL: $*" >&2; exit 1; }

# run SET: the files of SET/ replace those of ok/; output in $out, status in $rc.
run() {
  set_=$1; shift
  f() { if [ -f "$td/$set_/$1" ]; then echo "$td/$set_/$1"; else echo "$td/ok/$1"; fi; }
  rc=0
  out=$(python3 -I "$chk" --results "$(f RESULTS.md)" --status "$(f STATUS.md)" \
    --roadmap "$(f ROADMAP.md)" --tree "$(f tree.txt)" --mustpass "$(f mustpass.txt)" \
    --tsv "$(f fails.tsv)" "$@" 2>&1) || rc=$?
}
expect() { # SET PATTERN
  run "$1"
  [ "$rc" = 1 ] || fail "$1: exit $rc, want 1: $out"
  echo "$out" | grep -q -- "$2" || fail "$1: no message matching '$2': $out"
}

run ok
[ "$rc" = 0 ] || fail "the consistent set did not pass ($rc): $out"
expect sum 'counts add up to 11, mustpass has 10'
expect rows 'says 46 rows, pi4-gles3-tree.txt has 3'
expect commit 'STATUS.md Pi 4 entry names commit 9999aaaa, RESULTS.md names abcdef12'
expect tsv 'fail: RESULTS.md says 2, TSV has 1 Fail rows'

# --git: a commit that is not in the repository is not an ancestor.
g=$(mktemp -d "${TMPDIR:-/tmp}/check-pi4-gles3-results.XXXXXX")
trap 'rm -rf "$g"' EXIT
git -C "$g" init -q
git -C "$g" -c user.name=t -c user.email=t@t commit -q --allow-empty -m x
cp "$td/ok/RESULTS.md" "$g/RESULTS.md"
rc=0
out=$(python3 -I "$chk" --results "$g/RESULTS.md" --status "$td/ok/STATUS.md" \
  --roadmap "$td/ok/ROADMAP.md" --tree "$td/ok/tree.txt" --mustpass "$td/ok/mustpass.txt" \
  --tsv "$td/ok/fails.tsv" --git --ref HEAD 2>&1) || rc=$?
[ "$rc" = 1 ] && echo "$out" | grep -q 'not an ancestor' || fail "--git: unknown commit accepted: $out"
echo "check-pi4-gles3-results-selftest: ok"
