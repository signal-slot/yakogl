#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/gles3-notpass-diff-selftest.sh
#
# Host-only checks of tools/cts/gles3-notpass-diff.py on small synthetic TSVs
# written to $TMPDIR (default: a mktemp directory): the output compared
# exactly (totals, family table, --cases lists), --rows with --tree, bad
# input (exit 2), and the git:<rev>:<path> form and the totals of the current
# docs/es3/am62p-gles3-notpass.tsv against grep -c per outcome.
set -u

HERE=$(dirname "$0")
TOOL=$HERE/gles3-notpass-diff.py
REAL=$HERE/../../docs/es3/am62p-gles3-notpass.tsv
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/notpass-diff.XXXXXX") || exit 2
trap 'rm -rf "$WORK"' EXIT
P=dEQP-GLES3.functional

printf '%s\n' \
  "$P.fbo.color.blend.a" "$P.fbo.color.blend.b" "$P.fbo.color.tex2d.c" \
  "$P.texture.filtering.2d.d" "$P.texture.filtering.2d.e" \
  "$P.shaders.builtin_functions.common.f" "$P.shaders.builtin_functions.common.g" \
  "$P.texture.filtering.2d.h" > "$WORK/must.txt"
printf '%s\n' "$P.fbo.*" "$P.texture.*" "$P.shaders.*" > "$WORK/tree.txt"
{
  printf '# header line\n'
  printf 'Fail\t%s\tx\n' "$P.fbo.color.blend.a"
  printf 'NotSupported\t%s\tno ext\n' "$P.fbo.color.blend.b"
  printf 'Fail\t%s\tx\n' "$P.fbo.color.tex2d.c"
  printf 'QualityWarning\t%s\t\n' "$P.texture.filtering.2d.e"
  printf 'NoResult\t%s\t\n' "$P.shaders.builtin_functions.common.f"
  printf 'Fail\t%s\told message\n' "$P.shaders.builtin_functions.common.g"
} > "$WORK/old.tsv"
{
  printf '# header line\n'
  printf 'Fail\t%s\tx\n' "$P.fbo.color.blend.b"
  printf 'Fail\t%s\tx\n' "$P.fbo.color.tex2d.c"
  printf 'Fail\t%s\tx\n' "$P.texture.filtering.2d.d"
  printf 'Fail\t%s\tx\n' "$P.shaders.builtin_functions.common.f"
  printf 'Fail\t%s\tnew message\n' "$P.shaders.builtin_functions.common.g"
  printf 'QualityWarning\t%s\t\n' "$P.texture.filtering.2d.h"
} > "$WORK/new.tsv"

# check <label> <want rc> <want output file> <args...>: stdout must equal the file.
check() {
  local label=$1 rc=$2 want=$3
  shift 3
  local got
  got=$("$@" 2>"$WORK/err")
  local grc=$?
  if [ "$grc" = "$rc" ] && [ "$got" = "$(cat "$want")" ]; then ok "$label"
  else bad "$label (rc $grc, want $rc)"; printf '%s\n' "$got" | diff "$want" - | head -20; fi
}

cat > "$WORK/want-all.txt" <<'EOF'
cases: 8
outcome              old      new    delta
Pass                   2        2       +0
Fail                   3        5       +2
NotSupported           1        0       -1
QualityWarning         1        1       +0
NoResult               1        0       -1

family                                              fixed regressed  changed
fbo.color.blend                                         1         0        1
shaders.builtin_functions.common                        0         0        1
texture.filtering.2d                                    1         2        0
total                                                   2         2        2
EOF
check "totals and families" 0 "$WORK/want-all.txt" \
  python3 "$TOOL" "$WORK/old.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt"

cat > "$WORK/want-cases.txt" <<EOF
$(cat "$WORK/want-all.txt")

fixed (2)
$P.fbo.color.blend.a	Fail -> Pass
$P.texture.filtering.2d.e	QualityWarning -> Pass

regressed (2)
$P.texture.filtering.2d.d	Pass -> Fail
$P.texture.filtering.2d.h	Pass -> QualityWarning

changed (2)
$P.fbo.color.blend.b	NotSupported -> Fail
$P.shaders.builtin_functions.common.f	NoResult -> Fail
EOF
check "--cases lists" 0 "$WORK/want-cases.txt" \
  python3 "$TOOL" "$WORK/old.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" --cases

cat > "$WORK/want-rows.txt" <<'EOF'
cases: 3 (rows 0)
outcome              old      new    delta
Pass                   0        1       +1
Fail                   2        2       +0
NotSupported           1        0       -1
QualityWarning         0        0       +0
NoResult               0        0       +0

family                                              fixed regressed  changed
fbo.color.blend                                         1         0        1
total                                                   1         0        1
EOF
check "--rows 0 with --tree" 0 "$WORK/want-rows.txt" \
  python3 "$TOOL" "$WORK/old.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" \
  --rows 0 --tree "$WORK/tree.txt"

: > "$WORK/empty.txt"
python3 "$TOOL" "$WORK/old.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" --rows 5 \
  --tree "$WORK/tree.txt" >/dev/null 2>&1
[ $? = 2 ] && ok "--rows outside the tree: exit 2" || bad "--rows outside the tree: exit 2"
python3 "$TOOL" "$WORK/old.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" --rows 0 \
  >/dev/null 2>&1
[ $? = 2 ] && ok "--rows without --tree: refused" || bad "--rows without --tree: refused"
printf 'Fail\t%s\tx\nFail\tdEQP-GLES3.functional.nope.x.y\tx\n' "$P.fbo.color.blend.a" > "$WORK/stray.tsv"
python3 "$TOOL" "$WORK/stray.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" >/dev/null 2>"$WORK/err"
{ [ $? = 2 ] && grep -q 'not in the mustpass list' "$WORK/err"; } \
  && ok "case outside the mustpass list: exit 2" || bad "case outside the mustpass list: exit 2"
printf 'Pass\t%s\tx\n' "$P.fbo.color.blend.a" > "$WORK/badline.tsv"
python3 "$TOOL" "$WORK/badline.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" >/dev/null 2>&1
[ $? = 2 ] && ok "a Pass line in a TSV: exit 2" || bad "a Pass line in a TSV: exit 2"
python3 "$TOOL" "$WORK/missing.tsv" "$WORK/new.tsv" --mustpass "$WORK/must.txt" >/dev/null 2>&1
[ $? = 2 ] && ok "missing file: exit 2" || bad "missing file: exit 2"
python3 "$TOOL" git:HEAD "$WORK/new.tsv" --mustpass "$WORK/must.txt" >/dev/null 2>&1
[ $? = 2 ] && ok "malformed git: source: exit 2" || bad "malformed git: source: exit 2"

# The current TSV: totals against grep -c, and the git: form (HEAD's copy of
# the same file against the working copy: nothing differs when it is clean).
out=$(python3 "$TOOL" "$REAL" "$REAL")
rc=$?
must=$(grep -c . "$HERE/gles3-mustpass.txt")
np=0
okall=1
for o in Fail NotSupported QualityWarning NoResult; do
  n=$(grep -c "^$o	" "$REAL")
  np=$((np + n))
  line=$(printf '%s\n' "$out" | grep "^$o ")
  want=$(printf '%-15s %8d %8d %+8d' "$o" "$n" "$n" 0)
  [ "$line" = "$want" ] || { okall=0; bad "totals $o: '$line' want '$want'"; }
done
line=$(printf '%s\n' "$out" | grep '^Pass ')
want=$(printf '%-15s %8d %8d %+8d' Pass $((must - np)) $((must - np)) 0)
[ "$line" = "$want" ] || { okall=0; bad "totals Pass: '$line' want '$want'"; }
[ "$rc" = 0 ] && [ "$okall" = 1 ] && ok "current TSV totals match grep -c"

if git -C "$HERE" cat-file -e HEAD:docs/es3/am62p-gles3-notpass.tsv 2>/dev/null \
   && git -C "$HERE" diff --quiet HEAD -- ../../docs/es3/am62p-gles3-notpass.tsv; then
  git_out=$(python3 "$TOOL" git:HEAD:docs/es3/am62p-gles3-notpass.tsv "$REAL")
  [ "$git_out" = "$out" ] && ok "git:HEAD:<path> reads the same file" || bad "git:HEAD:<path> differs"
else
  ok "git:<rev>:<path> check skipped (TSV not in a clean HEAD)"
fi

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
