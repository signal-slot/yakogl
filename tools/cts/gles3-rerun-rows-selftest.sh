#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-rerun-rows.py against a synthetic AM62P-shaped tree,
# mustpass list, not-pass TSV and family rule file (all under $TMPDIR):
# globs, a family (rules, message regex, family column), range merging,
# stderr row counts on stdout only the spec, and the error cases
# (unknown family, a family with no row, a glob matching nothing,
# a case no row holds, usage).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/gles3-rerun-rows.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gles3-rerun-rows.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "gles3-rerun-rows-selftest: FAIL: $*" >&2; exit 1; }
tree=$tmp/tree.txt
must=$tmp/mustpass.txt
tsv=$tmp/notpass.tsv
rules=$tmp/rules.txt
out=$tmp/out.txt
err=$tmp/err.txt

cat >"$tree" <<'T'
# a synthetic AM62P-shaped tree: '#' and blank lines skipped, rows from 0
dEQP-GLES3.functional.a.*

dEQP-GLES3.functional.b.x.*,dEQP-GLES3.functional.c.*
dEQP-GLES3.functional.b.y.*
dEQP-GLES3.functional.deep.a.b.*
dEQP-GLES3.functional.deep.a.cx
# image ZZ: holds z
dEQP-GLES3.functional.z.*
T

cat >"$must" <<'T'
dEQP-GLES3.functional.a.1
dEQP-GLES3.functional.a.2
dEQP-GLES3.functional.b.x.1
dEQP-GLES3.functional.b.x.2
dEQP-GLES3.functional.b.y.1
dEQP-GLES3.functional.c.1
dEQP-GLES3.functional.deep.a.b.1
dEQP-GLES3.functional.deep.a.b.2
dEQP-GLES3.functional.deep.a.cx
dEQP-GLES3.functional.z.1
T

# 3-column rows and one 4-column (family) row; a NoResult row with an empty message
{
  printf '# synthetic not-pass list\n'
  printf 'Fail\tdEQP-GLES3.functional.b.x.1\tFail\n'
  printf 'Fail\tdEQP-GLES3.functional.b.y.1\tsome message\n'
  printf 'Fail\tdEQP-GLES3.functional.deep.a.b.1\tFail\n'
  printf 'NotSupported\tdEQP-GLES3.functional.deep.a.cx\tFormat not supported\n'
  printf 'NoResult\tdEQP-GLES3.functional.z.1\t\n'
  printf 'Fail\tdEQP-GLES3.functional.a.1\tFail\n'
  printf 'Fail\tdEQP-GLES3.functional.a.2\tFail\texplicit-fam\n'
} >"$tsv"

{
  printf '# synthetic rule file (gles3-triage.py shape)\n'
  printf 'RCb\tdEQP-GLES3.functional.b.x.*,dEQP-GLES3.functional.c.*\n'
  printf 'RCmsg\tdEQP-GLES3.functional.b.y.*\tsome message\n'
  printf 'RCdeep\tdEQP-GLES3.functional.deep.a.*\n'
  printf 'RCnever\tdEQP-GLES3.functional.z.*\tnever-ran-here\n'
  printf '#9-wd\tdEQP-GLES3.functional.a.*\n'
} >"$rules"

# expect <label> <want spec> <args...>: exit 0, stdout only the spec
expect() {
  label=$1 want=$2; shift 2
  rc=0
  outv=$(python3 -I "$tool" --tree "$tree" --mustpass "$must" "$@" 2>"$err") || rc=$?
  [ "$rc" = 0 ] || fail "$label: exit $rc, want 0 ($(sed 's/^/  /' "$err"))"
  [ "$outv" = "$want" ] || fail "$label: stdout '$outv', want '$want'"
}
rowline() { grep -q "^row $1  $2  $3 case[s]*$" "$err" || fail "$label: no 'row $1  $2  $3 case(s)' on stderr"; }
expect_rc() { # <label> <want rc> <stderr regex> <args...>
  label=$1 rcwant=$2 pat=$3; shift 3
  rc=0
  python3 -I "$tool" --tree "$tree" --mustpass "$must" "$@" >"$out" 2>"$err" || rc=$?
  [ "$rc" = "$rcwant" ] || fail "$label: exit $rc, want $rcwant"
  grep -q "$pat" "$err" || fail "$label: stderr lacks /$pat/"
  if [ -s "$out" ]; then fail "$label: error text reached stdout ($(cat "$out"))"; fi
  # an 'if' on purpose: a function ending in a false 'cmd && ...' test returns 1 and dash's errexit would kill the caller
}

# 1. a glob picks its rows, adjacent rows merged into one range, stderr per-row counts
label='glob b.*'
expect "$label" 1-2 --case 'dEQP-GLES3.functional.b.*'
[ "$(grep -c '^row ' "$err")" = 2 ] || fail "$label: want 2 row lines"
rowline 1 img2 2
rowline 2 img3 1 case

# 2. '*' spans dots (deep.a.* holds deep.a.b.* and deep.a.cx), merged again
label='glob deep.a.*'
expect "$label" 3-4 --case 'dEQP-GLES3.functional.deep.a.*'
rowline 3 img4 2
rowline 4 img5 1 case

# 3. discontiguous rows: exact case + glob -> 0,5; the labelled row shows its label
label='discontiguous'
expect "$label" 0,5 --case 'dEQP-GLES3.functional.a.1' --case 'dEQP-GLES3.functional.z.*'
rowline 0 img1 1 case
rowline 5 ZZ 1 case

# 4. family through the rule file (first matching rule line wins over RCb for deep)
label='family RCdeep'
expect "$label" 3-4 --family RCdeep --tsv "$tsv" --rules "$rules"
label='family RCb'
expect "$label" 1 --family RCb --tsv "$tsv" --rules "$rules"
label='family RCmsg'
expect "$label" 2 --family RCmsg --tsv "$tsv" --rules "$rules"

# 5. a filled family column wins over the rules ('#9-wd' would match a.2 too);
#    a family id may start with '#<digit>' (not a comment line)
label='family #9-wd'
expect "$label" 0 --family '#9-wd' --tsv "$tsv" --rules "$rules"
rowline 0 img1 1 case
label='family column'
expect "$label" 0 --family explicit-fam --tsv "$tsv" --rules "$rules"

# 6. a case named by --case and by --family is counted once per row
label='dedup'
expect "$label" 1 --case 'dEQP-GLES3.functional.b.x.*' --family RCb --tsv "$tsv" --rules "$rules"
rowline 1 img2 2

# 7. two families merge into one spec with a range and a comma
label='two families'
expect "$label" 1,3-4 --family RCb --family RCdeep --tsv "$tsv" --rules "$rules"

# 8. errors: each exits 1 (1 = nothing selected), nothing on stdout
label='unknown family'
expect_rc "$label" 1 "unknown\|--family 'nosuch-family'" --family nosuch-family --tsv "$tsv" --rules "$rules"
grep -q "no rule of $rules names it" "$err" || fail "$label: does not name the rule file"
label='family with no row'
expect_rc "$label" 1 "RCnever" --family RCnever --tsv "$tsv" --rules "$rules"
if grep -q 'no rule of' "$err"; then fail "$label: named the rule file, though RCnever is one"; fi
label='glob matching nothing'
expect_rc "$label" 1 "nope" --case 'dEQP-GLES3.functional.nope.*'
label='case no row holds'
expect_rc "$label" 1 "no tree row holds dEQP-GLES3.functional.deep.a.b" --case 'dEQP-GLES3.functional.deep.a.b'
label='no spec'
rc=0
python3 -I "$tool" --tree "$tree" >"$out" 2>"$err" || rc=$?
[ "$rc" = 2 ] || fail "no spec: exit $rc, want 2 (usage)"
grep -q 'usage' "$err" || fail "no spec: no usage line"

echo "gles3-rerun-rows-selftest: ok"
