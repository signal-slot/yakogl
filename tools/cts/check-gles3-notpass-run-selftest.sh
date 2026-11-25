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

BODY=$good
tsv "$tmp/nr.tsv" "# rows 0-2: build $a, run 2026-11-22"
run "$tmp/nr.tsv" --no-noresult; expect "--no-noresult refuses a NoResult row" 1 "c2: line 4: NoResult: dEQP-GLES3.c.1"
run "$tmp/nr.tsv"; expect "NoResult is accepted without --no-noresult" 0 "^ok:"

# Four row sessions of one build, one date each: header ranges of one sha8 pass (c1).
BODY='Fail\tdEQP-GLES3.a.1\tmsg\tf\n'
tsv "$tmp/four.tsv" "# rows 0-0: build $m, run 2026-11-25" "# rows 1-1: build $m, run 2026-11-26" \
  "# rows 2-2: build $m, run 2026-11-27"
run "$tmp/four.tsv" --no-noresult; expect "several ranges of one build" 0 "^ok:"

BODY='Fail\tdEQP-GLES3.a.1\tmsg\nNoResult\tdEQP-GLES3.c.1\n'
tsv "$tmp/nofam.tsv" "# rows 0-2: build $s, run 2026-11-22"
run "$tmp/nofam.tsv"; expect "no family column" 2 "no cause-family column yet"
grep -q "not an ancestor" <<<"$out" || fail "no-family run still names c1: $out"

rc=0; python3 -I "$chk" "$tmp/missing.tsv" "$tmp/tree.txt" 2>/dev/null || rc=$?
[ "$rc" = 2 ] || fail "an unreadable TSV exits $rc, want 2"

# ---- Re-split rows of one build (testdata/gles3-notpass-resplit): tree3 has
# the base rows A, B, C; tree4 has B split in Ba, Bb (base row 1 as 1a-1b).
rs="$here/testdata/gles3-notpass-resplit"
np="$here/gles3-notpass.py"
oldtsv() {  # oldtsv FILE HEADER-LINES...: OLD's body under the given header
  local f=$1; shift
  { echo "# SPDX-License-Identifier: Apache-2.0"; printf '%s\n' "$@"; cat "$rs/old.body"; } >"$f"
}
splice() {  # splice OLD OUT ARGS...  (tree4, its mustpass, build $a)
  local o=$1 out=$2; shift 2
  python3 -I "$np" --splice "$o" --tree "$rs/tree4.txt" --mustpass "$rs/mustpass.txt" \
    --commit "$a" -o "$out" "$@"
}
rrun() {  # rrun TSV TREE [args]
  rc=0; out=$(python3 -I "$chk" "$1" "$2" --mustpass "$rs/mustpass.txt" --repo "$repo" --ref main \
    "${@:3}") || rc=$?
}

# The first splice: rows 1-2 of tree4 (Ba, Bb) run on 2026-11-25 from two
# sources, the later one newest; row B had been run 2026-11-22 as one image.
oldtsv "$tmp/rs-old.tsv" "# rows 0-2: build $a, run 2026-11-22"
splice "$tmp/rs-old.tsv" "$tmp/rs-new.tsv" --rows 1-2 --resplit 1=2 --date 2026-11-25 \
  "$rs/s1.tsv" "$rs/s2.tsv" 2>/dev/null || fail "re-split splice refused"
grep -qxF "# rows 0-2 (1 re-split as 1a-1b): build $a, run 2026-11-22/25" "$tmp/rs-new.tsv" \
  || fail "re-split header: $(grep '^# rows' "$tmp/rs-new.tsv")"
[ "$(grep -c '^# rows' "$tmp/rs-new.tsv")" = 1 ] || fail "one build is one header line"
body=$(grep -v '^#' "$tmp/rs-new.tsv")
want=$(printf 'Fail\tdEQP-GLES3.a.0\tmsg a0\tfamA\nFail\tdEQP-GLES3.b.0\tnew b0\t\nFail\tdEQP-GLES3.b.1\told b1\tfamB\nFail\tdEQP-GLES3.c.0\tmsg c0\tfamC')
[ "$body" = "$want" ] || fail "re-split rows: newest source wins, an unreported case keeps OLD: $body"

# A second splice of the new TSV (the header now names the re-split row, no
# --resplit needed): row 0 rerun on 2026-11-27, row Bb untouched.
printf 'dEQP-GLES3.a.0\tPass\n' >"$tmp/s3.tsv"
splice "$tmp/rs-new.tsv" "$tmp/rs-new2.tsv" --rows 0-0 --date 2026-11-27 "$tmp/s3.tsv" 2>/dev/null \
  || fail "second splice of a re-split table refused"
grep -qxF "# rows 0-2 (1 re-split as 1a-1b): build $a, run 2026-11-22/25/27" "$tmp/rs-new2.tsv" \
  || fail "second splice header: $(grep '^# rows' "$tmp/rs-new2.tsv")"

# The same build, a mixed second build on a part: refused (one build per row).
rc=0; python3 -I "$np" --splice "$tmp/rs-new.tsv" --tree "$rs/tree4.txt" --mustpass "$rs/mustpass.txt" \
  --commit "$m" --date 2026-11-28 --rows 2-2 "$rs/s2.tsv" >/dev/null 2>"$tmp/err" || rc=$?
[ "$rc" = 1 ] && grep -q "re-split and has parts of build $a" "$tmp/err" \
  || fail "one part of a re-split row on another build: rc=$rc $(cat "$tmp/err")"

sed 's/\tnew b0\t$/\tnew b0\tfamB/' "$tmp/rs-new.tsv" >"$tmp/rs-ok.tsv"   # the triage fills the family
rrun "$tmp/rs-ok.tsv" "$rs/tree4.txt" --no-noresult; expect "re-split header, one build, whole tree" 0 "^ok:"
rrun "$tmp/rs-ok.tsv" "$rs/tree4.txt"; expect "re-split header without --no-noresult" 0 "^ok:"

# The checker still refuses: a gap, two builds, a NoResult left in a re-split row.
oldtsv "$tmp/rs-gap.tsv" "# rows 0-1 (1 re-split as 1a-1b): build $a, run 2026-11-25"
rrun "$tmp/rs-gap.tsv" "$rs/tree4.txt"; expect "a gap in the base rows" 1 "no build for 1 of the tree's 3 rows, first: row 2"
{ echo "# SPDX-License-Identifier: Apache-2.0"
  echo "# rows 0-1 (1 re-split as 1a-1b): build $m, run 2026-11-25"; echo "# rows 2-2: build $a, run 2026-11-25"
  grep -v '^#' "$tmp/rs-ok.tsv"; } >"$tmp/rs-two.tsv"
rrun "$tmp/rs-two.tsv" "$rs/tree4.txt"; expect "two builds, a single row of the other is allowed" 0 "^ok:"
oldtsv "$tmp/rs-two2.tsv" "# rows 0-0: build $m, run 2026-11-25" "# rows 1-2 (1 re-split as 1a-1b): build $a, run 2026-11-25" \
  "# rows 3-3: build $m, run 2026-11-25"
rrun "$tmp/rs-two2.tsv" "$rs/tree4.txt"; expect "a row beyond the base rows" 1 "names row 3"
oldtsv "$tmp/rs-nr.tsv" "# rows 0-2 (1 re-split as 1a-1b): build $a, run 2026-11-25"
rrun "$tmp/rs-nr.tsv" "$rs/tree4.txt"
expect "a NoResult left in a re-split row" 1 "c2: line 6: NoResult in re-split row 1 (1a-1b): dEQP-GLES3.b.2"
grep -q "line 7: NoResult in re-split row 1" <<<"$out" || fail "second NoResult not named: $out"
[ "$(grep -c '^c2: .*NoResult' <<<"$out")" = 2 ] || fail "want the two NoResult rows of B only: $out"
# The same NoResult in an ordinary row is allowed (no --no-noresult).
BODY='Fail\tdEQP-GLES3.a.0\tm\tf\nNoResult\tdEQP-GLES3.c.1\t\t#x\n'
tsv "$tmp/rs-plain.tsv" "# rows 0-2 (1 re-split as 1a-1b): build $a, run 2026-11-25"
rrun "$tmp/rs-plain.tsv" "$rs/tree4.txt"; expect "NoResult in a row that is not re-split" 0 "^ok:"
# A header that names a re-split row the tree lacks, and a malformed one.
oldtsv "$tmp/rs-few.tsv" "# rows 0-2 (1 re-split as 1a-1b): build $a, run 2026-11-25"
rrun "$tmp/rs-few.tsv" "$rs/tree3.txt"; expect "a re-split row the tree does not have: the header names a base row too many" 1 "names row 2, but the tree has rows 0-1"
oldtsv "$tmp/rs-bad.tsv" "# rows 0-2 (1 re-split as 2a-2b): build $a, run 2026-11-25"
rrun "$tmp/rs-bad.tsv" "$rs/tree4.txt"; expect "a malformed re-split item" 1 "c1: header"
# Rows beyond 45 without a re-split row: the header just covers the longer tree.
oldtsv "$tmp/rs-ext.tsv" "# rows 0-3: build $a, run 2026-11-25"
rrun "$tmp/rs-ext.tsv" "$rs/tree4.txt"; expect "rows beyond the old table, no re-split" 0 "^ok:"

# Without a re-split row the header is the old one, byte for byte.
printf 'dEQP-GLES3.b.0\tFail\tn\n' >"$tmp/s4.tsv"
oldtsv "$tmp/pl-old.tsv" "# rows 0-2: build $a, run 2026-11-22"
python3 -I "$np" --splice "$tmp/pl-old.tsv" --tree "$rs/tree3.txt" --mustpass "$rs/mustpass.txt" \
  --commit "$m" --date 2026-11-25 --rows 1-1 -o "$tmp/pl-new.tsv" "$tmp/s4.tsv" 2>/dev/null \
  || fail "plain splice refused"
want=$(printf '# rows 0-0: build %s, run 2026-11-22\n# rows 1-1: build %s, run 2026-11-25\n# rows 2-2: build %s, run 2026-11-22' "$a" "$m" "$a")
[ "$(grep '^# rows' "$tmp/pl-new.tsv")" = "$want" ] || fail "plain header changed: $(grep '^# rows' "$tmp/pl-new.tsv")"
grep -q "re-split" "$tmp/pl-new.tsv" && fail "plain header mentions re-split"

echo "check-gles3-notpass-run-selftest: ok"
