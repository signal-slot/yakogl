#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of check-gles3-notpass-run.py: fixtures and a throwaway git
# repository in $TMPDIR (an ancestor commit and a commit that is not one).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-gles3-notpass-run.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/check-notpass-run.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "check-gles3-notpass-run-selftest: FAIL: $*" >&2; exit 1; }

# A tree of 3 rows.
cat >"$tmp/tree.txt" <<'T'
# image A:
dEQP-GLES3.a.*
# image B:
dEQP-GLES3.b.*
# image C:
dEQP-GLES3.c.*
T

# Throwaway repository: main = c1 <- c2; side = c1 <- s1 (not an ancestor of main).
repo="$tmp/repo"
git init -q -b main "$repo"
g() { git -C "$repo" -c user.name=t -c user.email=t@example.invalid "$@"; }
echo 1 >"$repo/f"; g add f; g commit -q -m c1
a=$(g rev-parse --short=8 HEAD)
g checkout -q -b side; echo s >"$repo/s"; g add s; g commit -q -m s1
s=$(g rev-parse --short=8 HEAD)
g checkout -q main; echo 2 >>"$repo/f"; g commit -q -am c2
m=$(g rev-parse --short=8 HEAD)

tsv() {  # tsv FILE HEADER-LINES... ; body from $BODY
  local f=$1; shift
  { echo "# SPDX-License-Identifier: Apache-2.0"; printf '%s\n' "$@"; printf '%b' "$BODY"; } >"$f"
}
good='Fail\tdEQP-GLES3.a.1\tmsg\tfamA\nNoResult\tdEQP-GLES3.c.1\t\t#1285-watchdog\n'

run() {  # run TSV [args] -> sets rc, out
  rc=0; out=$(python3 -I "$chk" "$1" "$tmp/tree.txt" --repo "$repo" --ref main "${@:2}") || rc=$?
}
expect() {  # expect NAME RC WORD
  [ "$rc" = "$2" ] || fail "$1: exit $rc, want $2: $out"
  [ -z "${3:-}" ] || grep -q -- "$3" <<<"$out" || fail "$1: no '$3' in: $out"
}
not() { ! grep -q -- "$1" <<<"$out" || fail "$2: unexpected '$1' in: $out"; }

BODY=$good
tsv "$tmp/ok.tsv" "# rows 0-2: build $a, run 2026-11-22"
run "$tmp/ok.tsv"; expect "ancestor, whole tree" 0 "^ok:"

tsv "$tmp/ok2.tsv" "# rows 0-1: build $m, run 2026-11-22" "# rows 2-2: build $a, run 2026-11-23"
run "$tmp/ok2.tsv"; expect "a single row on its own commit" 0 "^ok:"

tsv "$tmp/legacy.tsv" "# dEQP-GLES3 on the X, build $a, run 2026-10-13/14: every mustpass case"
run "$tmp/legacy.tsv"; expect "one-line header covers the tree" 0 "^ok:"

tsv "$tmp/notanc.tsv" "# rows 0-2: build $s, run 2026-11-22"
run "$tmp/notanc.tsv"; expect "a commit that is not an ancestor" 1 "c1: build $s is not an ancestor of main"

tsv "$tmp/ref.tsv" "# rows 0-2: build $m, run 2026-11-22"
run "$tmp/ref.tsv" --ref "$s"; expect "--ref names the branch to test against" 1 "not an ancestor of $s"

run "$tmp/ok.tsv" --ref nosuchref; expect "an unknown ref" 1 "nosuchref is not a commit"

tsv "$tmp/unk.tsv" "# rows 0-2: build 0123abcd, run 2026-11-22"
run "$tmp/unk.tsv"; expect "a build the repository lacks" 1 "build 0123abcd is not a commit"

tsv "$tmp/gap.tsv" "# rows 0-1: build $a, run 2026-11-22"
run "$tmp/gap.tsv"; expect "a row with no build" 1 "no build for 1 of the tree's 3 rows, first: row 2"

tsv "$tmp/dup.tsv" "# rows 0-1: build $a, run 2026-11-22" "# rows 1-2: build $a, run 2026-11-22"
run "$tmp/dup.tsv"; expect "a row in two ranges" 1 "row 1 is in two header ranges"

tsv "$tmp/two.tsv" "# rows 0-1: build $m, run 2026-11-22" "# rows 2-2: build $a, run 2026-11-23" \
  "# rows 3-3: build $a, run 2026-11-23"
run "$tmp/two.tsv"; expect "a row beyond the tree" 1 "names row 3"

printf '# image A:\ndEQP-GLES3.a.*\n# image B:\ndEQP-GLES3.b.*\n# image C:\ndEQP-GLES3.c.*\n# image D:\ndEQP-GLES3.d.*\n# image E:\ndEQP-GLES3.e.*\n' >"$tmp/tree5.txt"
tsv "$tmp/mix.tsv" "# rows 0-0: build $a, run 2026-11-22" "# rows 1-2: build $m, run 2026-11-22" \
  "# rows 3-4: build $a, run 2026-11-22"
rc=0; out=$(python3 -I "$chk" "$tmp/mix.tsv" "$tmp/tree5.txt" --repo "$repo" --ref main) || rc=$?
expect "a multi-row range of another build" 1 "rows 1-2 come from build $m"
not "rows 0-0 come" "a single row of the minority build"

tsv "$tmp/nohdr.tsv" "# nothing here"
run "$tmp/nohdr.tsv"; expect "no build in the header" 1 "names no build"

BODY='Fail\tdEQP-GLES3.a.1\tmsg\tfamA\nFail\tdEQP-GLES3.a.2\tmsg\t\n\tdEQP-GLES3.b.1\tmsg\tfamB\nBogus\tdEQP-GLES3.b.2\tm\tfamB\n'
tsv "$tmp/c2.tsv" "# rows 0-2: build $a, run 2026-11-22"
run "$tmp/c2.tsv"; expect "c2" 1 "line 4: empty cause family: dEQP-GLES3.a.2"
grep -q "line 5: empty outcome" <<<"$out" || fail "empty outcome not named: $out"
grep -q "line 6: bad outcome" <<<"$out" || fail "bad outcome not named: $out"
[ "$(grep -c '^c2:' <<<"$out")" = 3 ] || fail "want one line per failed row: $out"

BODY='Fail\tdEQP-GLES3.a.1\tmsg\nNoResult\tdEQP-GLES3.c.1\n'
tsv "$tmp/nofam.tsv" "# rows 0-2: build $s, run 2026-11-22"
run "$tmp/nofam.tsv"; expect "no family column" 2 "no cause-family column yet"
grep -q "not an ancestor" <<<"$out" || fail "no-family run still names c1: $out"

rc=0; python3 -I "$chk" "$tmp/missing.tsv" "$tmp/tree.txt" 2>/dev/null || rc=$?
[ "$rc" = 2 ] || fail "an unreadable TSV exits $rc, want 2"

echo "check-gles3-notpass-run-selftest: ok"
