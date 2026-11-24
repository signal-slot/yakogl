#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-note-fill.py on a copy of the by-eye note.
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$HERE")")"
FILL="$HERE/imx8mp-glmark2-note-fill.py"
NOTE="$ROOT/docs/notes/imx8mp-glmark2-pictures-by-eye.md"
T="$(mktemp -d "${TMPDIR:-$ROOT}/notefill.XXXXXX")" || exit 2
trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fail=1; }

C=0123456789abcdef0123456789abcdef01234567
R=records/imx8mp/20261124-1200-dumps-a-01234567/
# a skeleton copy: every row reset, so the test also works on a finished note
python3 - "$NOTE" "$T/orig.md" <<'PY'
import re, sys
s = open(sys.argv[1]).read()
s = re.sub(r"^(\| \[[a-z0-9]+\] [^|]*)\|.*$",
           r"\1| - | - | - | not yet looked at |", s, flags=re.M)
open(sys.argv[2], "w").write(s)
PY
cp "$T/orig.md" "$T/n.md"
fill() { python3 "$FILL" --note "$T/n.md" --commit "$C" --record "$R" "$@"; }

# one fill of each kind: the diff is one line per fill
check_one() { # label scene route verdict
  cp "$T/n.md" "$T/before.md"
  if ! fill --scene "$2" --route "$3" --verdict "$4" >/dev/null; then
    bad "$1: refused"; return
  fi
  n=$(diff "$T/before.md" "$T/n.md" | grep -c '^>')
  m=$(diff "$T/before.md" "$T/n.md" | grep -c '^<')
  if [ "$n" = 1 ] && [ "$m" = 1 ]; then ok "$1: one line changed"
  else bad "$1: diff $m/$n lines"; fi
}
check_one "label dump right" gm01 dump right
check_one "cell both right" "[build] use-vbo=true" both right
check_one "photograph" gm03 photograph "seen without a value"
check_one "wrong" gm13 dump wrong
check_one "effect2d cell" "[effect2d] kernel=0,1,0;1,-4,1;0,1,0;" dump right
if grep -q "^| \[build\] use-vbo=false | 01234567 | $R | dump | right |$" "$T/n.md"; then
  ok "row text"; else bad "row text"; fi

# refusals: each must exit nonzero with a message, and leave the file alone
cp "$T/n.md" "$T/before.md"
G=(--scene gm02 --commit "$C" --record "$R" --route dump --verdict right)
refuse() { # label option value   (G with that option replaced)
  local l="$1" o=("${G[@]}") i
  for ((i = 0; i < ${#o[@]}; i++)); do
    if [ "${o[i]}" = "$2" ]; then o[i+1]="$3"; fi
  done
  if python3 "$FILL" --note "$T/n.md" "${o[@]}" >/dev/null 2>"$T/err"; then
    bad "refusal $l: accepted"
  elif [ -s "$T/err" ]; then ok "refusal $l"
  else bad "refusal $l: no message"; fi
}
refuse "unknown scene" --scene "[nope] x=1"
refuse "unknown label" --scene gm99
refuse "commit non-hex" --commit 0123zz789abc
refuse "commit too short" --commit 012345
refuse "commit too long" --commit "${C}0"
refuse "record bad name" --record records/imx8mp/2026-1200-x-01234567/
refuse "record bad dir" --record records/pi4/20261124-1200-x-01234567/
refuse "record short sha" --record records/imx8mp/20261124-1200-x-0123456/
refuse "record sha8 not prefix" --record records/imx8mp/20261124-1200-x-deadbeef/
refuse "bad route" --route camera
refuse "bad verdict" --verdict fine
refuse "verdict todo" --verdict "not yet looked at"
refuse "dump with seen" --verdict "seen without a value"
refuse "photograph with right" --route photograph
# --check-main: a commit that is not on origin/main
if git -C "$ROOT" rev-parse -q --verify origin/main >/dev/null; then
  if python3 "$FILL" --note "$T/n.md" "${G[@]}" --check-main >/dev/null 2>&1; then
    bad "refusal check-main: accepted"; else ok "refusal check-main"; fi
else
  echo "  note  no origin/main here: --check-main not exercised"
fi
if cmp -s "$T/n.md" "$T/before.md"; then ok "refusals left the file alone"
else bad "refusals changed the file"; fi

# all 33 rows, then the completeness check
cp "$T/orig.md" "$T/n.md"
n=0
while read -r label _; do
  n=$((n + 1))
  fill --scene "$label" --route dump --verdict right >/dev/null || bad "fill $label"
done < <(bash "$HERE/imx8mp-glmark2-dumps.sh" scenes)
if [ "$n" = 33 ]; then ok "33 scenes listed"; else bad "$n scenes listed"; fi
d=$(diff "$T/orig.md" "$T/n.md" | grep -c '^>')
if [ "$d" = 33 ]; then ok "33 lines changed"; else bad "$d lines changed"; fi
if python3 "$HERE/imx8mp-glmark2-verdicts.py" "$T/n.md" --complete; then
  ok "verdicts.py --complete on the filled copy"
else bad "verdicts.py --complete"; fi

if [ "$fail" = 0 ]; then echo "== note-fill self-test: all passed"
else echo "== note-fill self-test: FAILED"; fi
exit "$fail"
