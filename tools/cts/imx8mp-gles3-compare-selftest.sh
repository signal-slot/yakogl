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
