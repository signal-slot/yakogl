#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-gles3-compare.py: synthetic lists, the cause map, the entry map, the real table.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cmp="$here/imx8mp-gles3-compare.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/imx8mp-gles3-compare.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "imx8mp-gles3-compare-selftest: FAIL: $*" >&2; exit 1; }
P=dEQP-GLES3.functional
t=$(printf '\t')

# board in the merge shape, host in the tsv shape
cat >"$tmp/board.txt" <<T
# header
e1:Fail $P.transform_feedback.a
e1:Fail $P.state_query.b
e2:NotSupported $P.fbo.blit.c
e2:Crash $P.texture.shadow.d
e3:Fail $P.weird.e
T
{
  printf '# header\n'
  printf 'Fail\t%s\tmsg\n' "$P.transform_feedback.a"
  printf 'NotSupported\t%s\n' "$P.state_query.b"
  printf 'NotSupported\t%s\n' "$P.fbo.blit.c"
  printf 'Timeout\t%s\n' "$P.texture.shadow.d"
  printf 'Fail\t%s\n' "$P.transform_feedback.hostonly"
  printf 'Fail\t%s\n' "$P.nocause.hostonly"
} >"$tmp/host.tsv"
{
  printf '# header\n'
  printf 'tf\t%s\tTF\n' "$P.transform_feedback.*"
  printf 'sq\t%s\tSQ\n' "$P.state_query.*"
  printf 'bl\t%s\tBL\n' "$P.fbo.blit.*"
  printf 'sh\t%s\tSH\n' "$P.texture.shadow.*"
} >"$tmp/causes.tsv"
cat >"$tmp/tree.txt" <<T
# header
$P.transform_feedback.*
$P.state_query.*,$P.fbo.*
$P.texture.*
T

python3 -I "$cmp" "$tmp/board.txt" "$tmp/host.tsv" --causes "$tmp/causes.tsv" --table "$tmp/tree.txt" --tsv >"$tmp/o.tsv"
grep -qx "host-predicted${t}tf${t}e0${t}Fail${t}$P.transform_feedback.a" "$tmp/o.tsv" || fail "host-predicted"
grep -qx "host-other${t}sq${t}e1${t}Fail${t}$P.state_query.b" "$tmp/o.tsv" || fail "host-other (Fail vs NotSupported)"
grep -qx "host-predicted${t}bl${t}e1${t}NotSupported${t}$P.fbo.blit.c" "$tmp/o.tsv" || fail "same outcome is predicted"
grep -qx "host-predicted${t}sh${t}e2${t}Crash${t}$P.texture.shadow.d" "$tmp/o.tsv" || fail "crash vs timeout is predicted"
grep -qx "board-only${t}unclassified${t}-${t}Fail${t}$P.weird.e" "$tmp/o.tsv" || fail "board-only, unclassified, no entry"
[ "$(wc -l <"$tmp/o.tsv")" = 5 ] || fail "tsv line count"

python3 -I "$cmp" "$tmp/board.txt" "$tmp/host.tsv" --causes "$tmp/causes.tsv" --table "$tmp/tree.txt" --examples 1 >"$tmp/o.md"
grep -q '^| tf | 1 | 0 | 0 | 1 | 1 |$' "$tmp/o.md" || fail "summary row tf (with host-only)"
grep -q '^| unclassified | 0 | 0 | 1 | 1 | 1 |$' "$tmp/o.md" || fail "summary row unclassified"
grep -q '^| \*\*total\*\* | 3 | 1 | 1 | 2 | 5 |$' "$tmp/o.md" || fail "summary total"
grep -qF "$P.weird.e (Fail, -)" "$tmp/o.md" || fail "example line"

# the case-first layout of docs/es3/host-gc-gles3-notpass.tsv (gpu, case, status, ...) reads like the outcome-first one
{
  printf '# header\n'
  printf 'gpu\tcase\tstatus\tkind\tmessage\n'
  printf 'vivante\t%s\tFail\tgl-error\tmsg\n' "$P.transform_feedback.a"
  printf 'vivante\t%s\tNotSupported\trefusal\n' "$P.state_query.b"
  printf '%s\tNotSupported\n' "$P.fbo.blit.c"
  printf '%s\tTimeout\n' "$P.texture.shadow.d"
  printf '%s\tFail\n' "$P.transform_feedback.hostonly"
  printf '%s\tFail\n' "$P.nocause.hostonly"
} >"$tmp/hostcf.tsv"
python3 -I "$cmp" "$tmp/board.txt" "$tmp/hostcf.tsv" --causes "$tmp/causes.tsv" --table "$tmp/tree.txt" --tsv >"$tmp/ocf.tsv"
cmp -s "$tmp/o.tsv" "$tmp/ocf.tsv" || fail "case-first host list differs from the outcome-first one"
python3 -I "$cmp" "$tmp/board.txt" "$tmp/hostcf.tsv" --causes "$tmp/causes.tsv" --table "$tmp/tree.txt" --examples 1 >"$tmp/ocf.md"
cmp -s "$tmp/o.md" "$tmp/ocf.md" || fail "case-first host list: markdown differs"
# a mixed file (both layouts) is fine
{ printf 'Fail\t%s\n' "$P.transform_feedback.a"; printf '%s\tNotSupported\n' "$P.state_query.b"; } >"$tmp/mixed.tsv"
python3 -I "$cmp" "$tmp/board.txt" "$tmp/mixed.tsv" --tsv >"$tmp/om.tsv"
grep -qx "host-predicted${t}unclassified${t}-${t}Fail${t}$P.transform_feedback.a" "$tmp/om.tsv" || fail "mixed: outcome-first row"
grep -qx "host-other${t}unclassified${t}-${t}Fail${t}$P.state_query.b" "$tmp/om.tsv" || fail "mixed: case-first row"
# a line that fits neither layout is refused with a message
printf 'Fail\tnot-a-case\n' >"$tmp/bad1.tsv"
printf '%s\tMaybe\n' "$P.x" >"$tmp/bad2.tsv"
for b in bad1 bad2; do
  if python3 -I "$cmp" "$tmp/board.txt" "$tmp/$b.tsv" >"$tmp/bad.out" 2>"$tmp/bad.err"; then fail "$b: accepted"; fi
  grep -q "$b.tsv:1:" "$tmp/bad.err" || fail "$b: no file:line in the error"
done

# the record-prefixed shape on both sides
python3 -I "$cmp" "$tmp/board.txt" "$tmp/board.txt" --tsv >"$tmp/self.tsv"
[ "$(cut -f1 "$tmp/self.tsv" | sort -u)" = host-predicted ] || fail "self compare"

# real files: a list against itself
am="$root/docs/es3/am62p-gles3-notpass.tsv"
n=$(grep -vc -e '^#' -e '^[[:space:]]*$' "$am")
python3 -I "$cmp" "$am" "$am" --causes "$here/imx8mp-gles3-causes.tsv" \
  --table "$here/imx8mp-gles3-tree.txt" --tsv >"$tmp/real.tsv"
[ "$(wc -l <"$tmp/real.tsv")" = "$n" ] || fail "real: line count"
[ "$(cut -f1 "$tmp/real.tsv" | sort -u)" = host-predicted ] || fail "real: not all host-predicted"
cut -f2 "$tmp/real.tsv" | grep -qvx unclassified || fail "real: no case is classified"
cut -f3 "$tmp/real.tsv" | grep -q '^e[0-9]' || fail "real: no entry mapped"
echo "imx8mp-gles3-compare-selftest: OK"
