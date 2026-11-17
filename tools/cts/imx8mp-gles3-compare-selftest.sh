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

# the same host list case-first, as docs/es3/host-gc-gles3-5454be1b/host-gc-gles3-notpass.tsv
# (column row, gpu column) and without the gpu column: the same output
{
  printf '# header\n'
  printf 'gpu\tcase\tstatus\tkind\tmessage\trc\towner\n'
  printf 'vivante\t%s\tFail\tgl-error\tmsg\t\t\n' "$P.transform_feedback.a"
  printf 'vivante\t%s\tNotSupported\trefusal\tmsg\t\t\n' "$P.state_query.b"
  printf 'vivante\t%s\tNotSupported\trefusal\t\t\t\n' "$P.fbo.blit.c"
  printf 'vivante\t%s\tTimeout\t\t\t\t\n' "$P.texture.shadow.d"
  printf 'vivante\t%s\tFail\tcompile\t\t\t\n' "$P.transform_feedback.hostonly"
  printf 'vivante\t%s\tFail\tcompile\t\t\t\n' "$P.nocause.hostonly"
} >"$tmp/host-cf.tsv"
grep -v -e '^gpu' "$tmp/host-cf.tsv" | sed "s/^vivante$t//" >"$tmp/host-cf2.tsv"
for h in host-cf host-cf2; do
  python3 -I "$cmp" "$tmp/board.txt" "$tmp/$h.tsv" --causes "$tmp/causes.tsv" --table "$tmp/tree.txt" --tsv >"$tmp/$h.o.tsv"
  cmp -s "$tmp/o.tsv" "$tmp/$h.o.tsv" || fail "$h: case-first host list reads differently (tsv)"
  python3 -I "$cmp" "$tmp/board.txt" "$tmp/$h.tsv" --causes "$tmp/causes.tsv" --table "$tmp/tree.txt" --examples 1 >"$tmp/$h.o.md"
  cmp -s "$tmp/o.md" "$tmp/$h.o.md" || fail "$h: case-first host list reads differently (markdown)"
done

# a host list mixing both layouts line by line reads each line by its own layout
{
  printf 'Fail\t%s\n' "$P.transform_feedback.a"
  printf 'vivante\t%s\tNotSupported\trefusal\n' "$P.state_query.b"
  printf '%s\tNotSupported\n' "$P.fbo.blit.c"
} >"$tmp/mixed.tsv"
python3 -I "$cmp" "$tmp/board.txt" "$tmp/mixed.tsv" --tsv >"$tmp/mixed.o.tsv"
grep -qx "host-predicted${t}unclassified${t}-${t}Fail${t}$P.transform_feedback.a" "$tmp/mixed.o.tsv" || fail "mixed: outcome-first line"
grep -qx "host-other${t}unclassified${t}-${t}Fail${t}$P.state_query.b" "$tmp/mixed.o.tsv" || fail "mixed: case-first line with gpu"
grep -qx "host-predicted${t}unclassified${t}-${t}NotSupported${t}$P.fbo.blit.c" "$tmp/mixed.o.tsv" || fail "mixed: case-first line"
grep -q '^Board not-pass cases: 5; host not-pass cases: 3$' \
  <(python3 -I "$cmp" "$tmp/board.txt" "$tmp/mixed.tsv") || fail "mixed: host count"

# a line that fits neither layout is refused, naming the file and line
printf 'Fail\t%s\n' "$P.transform_feedback.a" >"$tmp/bad0.tsv"
printf 'Fail\tnot-a-case\n' >>"$tmp/bad0.tsv"                       # no dEQP-GLES3. case
printf '%s\tMaybe\n' "$P.x" >"$tmp/bad1.tsv"                        # no status word
printf 'vivante\t%s\n' "$P.x" >"$tmp/bad2.tsv"                       # no outcome at all
printf 'gpu\tcase\tkind\n' >"$tmp/bad3.tsv"                         # not the column row
printf 'Fail\tdEQP-GLES2.functional.x\n' >"$tmp/bad4.tsv"             # another module
for b in bad0:2 bad1:1 bad2:1 bad3:1 bad4:1; do
  f=${b%:*}
  if python3 -I "$cmp" "$tmp/board.txt" "$tmp/$f.tsv" >"$tmp/bad.out" 2>"$tmp/bad.err"; then fail "$f: accepted"; fi
  grep -q "$f.tsv:${b#*:}: neither" "$tmp/bad.err" || fail "$f: no file:line error ($(cat "$tmp/bad.err"))"
  [ ! -s "$tmp/bad.out" ] || fail "$f: output on a refused list"
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
