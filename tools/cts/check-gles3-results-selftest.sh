#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of check-gles3-results.py: one passing section, failing ones, and
# today's RESULTS.md (which must fail naming the 31-image text).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-gles3-results.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/check-gles3-results.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "check-gles3-results-selftest: FAIL: $*" >&2; exit 1; }

# 10 mustpass cases, a 3-image tree, a 4-row TSV.
for i in 1 2 3 4 5 6 7 8 9 10; do echo "dEQP-GLES3.a.b.c$i"; done >"$tmp/mustpass.txt"
printf '# tree\n# image A: x\ndEQP-GLES3.a.*\ndEQP-GLES3.b.*\n\ndEQP-GLES3.c.*\n' >"$tmp/tree.txt"
printf 'Fail\tx.1\nFail\tx.2\nNotSupported\tx.3\nQualityWarning\tx.5\nNoResult\tx.4\n' >"$tmp/np.tsv"

# mk NAME IMAGES COUNTS COMMIT EXTRA
mk() {
  cat >"$tmp/$1.md" <<T
## Other

| Pass | Fail |
|---:|---:|
| 1 | 1 |

## ES 3.0: dEQP-GLES3 (stage 4)

Mustpass list: 10 cases.

**AM62P, whole tree** on $4, 2026-11-20, $2 images:

| Pass | Fail | NotSupported | QualityWarning | No result |
|---:|---:|---:|---:|---:|
| $3 |

- Every case that is not Pass: the TSV. $5

**AM62P, gl_test ES 3.0 cases** on \`cd738b1a\`: 99 images are not counted here.

## Next
T
}
mk ok 3 "5 | 2 | 1 | 1 | 1" '`abcdef12`' ""
mk sum 3 "5 | 2 | 1 | 1 | 2" '`abcdef12`' ""
mk img 46 "5 | 2 | 1 | 1 | 1" '`abcdef12`' ""
mk old 3 "5 | 2 | 1 | 1 | 1" '`abcdef12`' "Table of 31 images."
mk nocommit 3 "5 | 2 | 1 | 1 | 1" 'main' ""
mk tsv 3 "5 | 3 | 1 | 1 | 0" '`abcdef12`' ""

# chk_run NAME [ARGS]: output in $out, exit code in $rc.
chk_run() {
  n=$1; shift
  rc=0
  out=$(python3 -I "$chk" --results "$tmp/$n.md" --tree "$tmp/tree.txt" \
    --mustpass "$tmp/mustpass.txt" "$@" 2>&1) || rc=$?
}
expect() { # NAME PATTERN [ARGS]
  n=$1; pat=$2; shift 2
  chk_run "$n" "$@"
  [ "$rc" = 1 ] || fail "$n: exit $rc, want 1: $out"
  echo "$out" | grep -q -- "$pat" || fail "$n: no message matching '$pat': $out"
}

chk_run ok --tsv "$tmp/np.tsv"
[ "$rc" = 0 ] || fail "passing section did not pass ($rc): $out"
expect sum 'add up to 11, mustpass has 10'
expect img 'says 46 images, am62p-gles3-tree.txt has 3'
expect old 'stale "31 images"'
expect nocommit 'no build commit'
expect tsv 'Fail: table says 3, TSV has 2' --tsv "$tmp/np.tsv"
expect tsv 'No result: table says 0, TSV has 1' --tsv "$tmp/np.tsv"

# --git: a commit that is not in the repository is not an ancestor.
mkdir "$tmp/g" && git -C "$tmp/g" init -q
git -C "$tmp/g" -c user.name=t -c user.email=t@t commit -q --allow-empty -m x
cp "$tmp/ok.md" "$tmp/g/ok.md"
rc=0
out=$(python3 -I "$chk" --results "$tmp/g/ok.md" --tree "$tmp/tree.txt" \
  --mustpass "$tmp/mustpass.txt" --git --ref HEAD 2>&1) || rc=$?
[ "$rc" = 1 ] && echo "$out" | grep -q 'not an ancestor' || fail "--git: unknown commit accepted: $out"

# today's RESULTS.md: while it holds the old record it must fail with both messages.
if grep -q '31-image table' "$here/RESULTS.md"; then
  rc=0
  out=$(python3 -I "$chk" --results "$here/RESULTS.md" 2>&1) || rc=$?
  [ "$rc" = 1 ] || fail "RESULTS.md: exit $rc, want 1"
  echo "$out" | grep -q 'stale "31 images"' || fail "RESULTS.md: stale text not named"
  echo "$out" | grep -q 'images, am62p-gles3-tree.txt has' || fail "RESULTS.md: image count not named"
fi
echo "check-gles3-results-selftest: ok"
