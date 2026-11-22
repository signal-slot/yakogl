#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of am62p-gles3-merge.py's NoResult causes, end to end: three synthetic
# images (clean, watchdog mid-way, GPU reset) -> merge -> gles3-notpass.py ->
# gles3-triage.py --families --check. Fixtures are made here; no real board
# address, host name or UART path is in them.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/am62p-merge.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "am62p-gles3-merge-selftest: FAIL: $*" >&2; exit 1; }
sha=abcdef12
P=dEQP-GLES3.functional.shaders.builtin_functions.precision.abs.highp_fragment
for i in 1 2 3 4 5 6 7 8; do echo "$P.s$i"; done >"$tmp/mustpass.txt"

# slice ROWS: make a slice directory, print its path; cases.tsv from stdin
slice() {
  local d="$tmp/2026-11-22-am62p-gles3-$sha-rows$1"
  mkdir -p "$d"
  printf 'commit: %s\n' "$sha" >"$d/README.md"
  cat >"$d/cases.tsv"
  echo "$d"
}
begun() { printf '#beginTestCaseResult %s.%s\n' "$P" "$1"; }
whole() { begun "$1"; echo '#endTestCaseResult'; }

# image 1: clean (s1 Pass, s2 Fail)
a=$(slice 00-00 < <(printf '%s.s1\tPass\n%s.s2\tFail\twrong\n' "$P" "$P"))
(echo 'rti_arm: armed'; whole s1; whole s2
 echo 'deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1') >"$a/console.txt"
# image 2: the 600 s watchdog reset the board while s4 ran; s5 never began
b=$(slice 01-01 < <(printf '%s.s3\tPass\n' "$P"))
(echo 'rti_arm: armed'; whole s3; begun s4; echo 'rti_arm: armed') >"$b/console.txt"
# image 3: GPU recoveries, the image ended in the middle of s6; s7 never began
c=$(slice 02-02 < <(printf '%s.s8\tPass\n' "$P"))
(echo 'rti_arm: armed'; begun s6; echo 'render: HWR during fence 12 (counter 0 -> 1)') >"$c/console.txt"

m() { python3 -I "$here/am62p-gles3-merge.py" --mustpass "$tmp/mustpass.txt" "$@"; }
m "$a" "$b" "$c" >"$tmp/merged.tsv" 2>"$tmp/merged.err" || { cat "$tmp/merged.err" >&2; fail "merge"; }
row() { grep -F "$P.$1"$'\t' "$tmp/merged.tsv" || true; }
row s4 | grep -q $'^NoResult\t[^\t]*\twatchdog: ' || fail "s4 not named watchdog: $(row s4)"
row s6 | grep -q $'^NoResult\t[^\t]*\tgpu-reset: ' || fail "s6 not named gpu-reset: $(row s6)"
for s in s5 s7; do
  row $s | grep -q $'^NoResult\t[^\t]*\tnot-reached: ' || fail "$s not named not-reached: $(row $s)"
done
grep -q $'^Fail\t[^\t]*\\.s2\twrong$' "$tmp/merged.tsv" || fail "the clean image's Fail row changed"
for s in s1 s3 s8; do [ -z "$(row $s)" ] || fail "$s is a Pass, no row expected"; done
[ "$(grep -c '^NoResult' "$tmp/merged.tsv")" = 4 ] || fail "expected 4 NoResult rows"
if grep -qP '^NoResult\t[^\t]+\t$' "$tmp/merged.tsv"; then fail "a NoResult row has no cause"; fi

# gles3-notpass.py on the same slices: the same outcome and case rows
python3 -I "$here/gles3-notpass.py" --mustpass "$tmp/mustpass.txt" -o "$tmp/notpass.tsv" "$a" "$b" "$c" 2>/dev/null \
  || fail "gles3-notpass.py"
diff <(grep -v '^#' "$tmp/merged.tsv" | cut -f1,2) <(grep -v '^#' "$tmp/notpass.tsv" | cut -f1,2) \
  || fail "notpass and merge disagree on the rows"

# triage: every row of the merged list gets a family
python3 -I "$here/gles3-triage.py" "$tmp/merged.tsv" --families "$here/am62p-gles3-families.txt" \
  --apply "$tmp/fam.tsv" || fail "triage --apply"
python3 -I "$here/gles3-triage.py" "$tmp/fam.tsv" --check >/dev/null || fail "triage --check"

# records with no watchdog or reset: byte-identical to the merge before the change
first=$(git -C "$root" log --format=%H --diff-filter=A -- tools/cts/am62p-gles3-merge.py | tail -1)
git -C "$root" show "$first:tools/cts/am62p-gles3-merge.py" >"$tmp/old-merge.py" \
  || fail "cannot read the first version of am62p-gles3-merge.py from git"
rm "$a/console.txt"
python3 -I "$tmp/old-merge.py" --mustpass "$tmp/mustpass.txt" "$a" >"$tmp/old.tsv" 2>"$tmp/old.err"
m "$a" >"$tmp/new.tsv" 2>"$tmp/new.err"
cmp "$tmp/old.tsv" "$tmp/new.tsv" || fail "clean image: output differs from the old merge"
cmp "$tmp/old.err" "$tmp/new.err" || fail "clean image: totals differ"
# with its console (the image reached its end) the output is the same
(echo 'rti_arm: armed'; echo 'deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1') >"$a/console.txt"
m "$a" >"$tmp/new2.tsv" 2>/dev/null
cmp "$tmp/old.tsv" "$tmp/new2.tsv" || fail "a clean console changed the output"
echo "am62p-gles3-merge-selftest: ok"
