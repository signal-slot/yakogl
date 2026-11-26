#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-lookedat-fill.py on copies of the by-eye note.
set -u
[ -n "${TMPDIR:-}" ] || { echo "TMPDIR is not set" >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$HERE")")"
FILL="$HERE/imx8mp-glmark2-lookedat-fill.py"
NOTE="$ROOT/docs/notes/imx8mp-glmark2-pictures-by-eye.md"
T="$(mktemp -d "$TMPDIR/lookedat.XXXXXX")" || exit 2
trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fail=1; }

cp "$NOTE" "$T/orig.md"
cp "$NOTE" "$T/n.md"
S=gm02
CELL="[build] use-vbo=true"
if ! grep -q '^- \*\*\['"build"'\] use-vbo=true\*\*.*<verdict>' "$T/orig.md"; then
  # a finished note: put the placeholders back into the scene's entry
  python3 - "$T/orig.md" <<'PY'
import re, sys
p = sys.argv[1]; s = open(p).read().split("\n")
for i, l in enumerate(s):
    if l.startswith("- **[build] use-vbo=true**"):
        s[i] = l.split("**:")[0] + "**: the same horse; verdict <verdict>, route <route>, commit <commit>, record <record>, frame crc32 <crc32>, picdiff <picdiff>."
open(p, "w").write("\n".join(s))
PY
  cp "$T/orig.md" "$T/n.md"
fi
f() { python3 "$FILL" --note "$T/n.md" "$@"; }

refuse() { # label args...
  local l="$1"; shift
  cp "$T/orig.md" "$T/n.md"
  f "$@" >/dev/null 2>&1; local rc=$?
  if [ $rc -eq 1 ] && cmp -s "$T/orig.md" "$T/n.md"; then ok "refused: $l"
  else bad "refused: $l (exit $rc or file changed)"; fi
}
refuse "unknown token" --scene $S --set bogus=x
refuse "token given twice" --scene $S --set verdict=right --set verdict=right
refuse "value with <" --scene $S --set verdict='a<b'
refuse "value with >" --scene $S --set verdict='a>b'
refuse "value with |" --scene $S --set verdict='a|b'
refuse "value with newline" --scene $S --set verdict=$'a\nb'
refuse "ipv4" --scene $S --set route='10.1.2.3'
refuse "/dev/tty" --scene $S --set route='/dev/ttyS1'
refuse "ttyUSB" --scene $S --set route='ttyUSB0'
refuse "ttyACM" --scene $S --set route='ttyACM0'
refuse "ts.net" --scene $S --set route='box.tail1234.ts.net'
refuse "password" --scene $S --set route='my password'
refuse "unknown scene" --scene gm99 --set verdict=right
# token present in the file but not in this entry: fill it, then ask again
cp "$T/orig.md" "$T/n.md"
f --scene $S --set verdict=right >/dev/null
f --scene $S --set verdict=wrong >/dev/null 2>&1; rc=$?
[ $rc -eq 1 ] && ok "refused: token already filled" || bad "token already filled (exit $rc)"
f --scene $S >/dev/null 2>&1; rc=$?
[ $rc -eq 2 ] && ok "usage error exits 2" || bad "usage error (exit $rc)"

# one fill changes only that entry line
cp "$T/orig.md" "$T/n.md"
f --scene "$CELL" --set verdict=right --set route=dump >"$T/out" 2>&1; rc=$?
n=$(diff "$T/orig.md" "$T/n.md" | grep -c '^>')
[ $rc -eq 0 ] && [ "$n" -eq 1 ] && ok "one fill changes one line" || bad "one fill changes $n lines (exit $rc)"
grep -q 'unfilled: commit, record, crc32, picdiff' "$T/out" && ok "unfilled tokens printed" || bad "unfilled tokens not printed"
diff "$T/orig.md" "$T/n.md" | grep '^>' | grep -q -F -- "- **$CELL** ($S-build)" \
  && ok "scene text byte for byte" || bad "scene text changed"
# the lead line
lead_o=$(grep -n '^`\|`<verdict>`' "$T/orig.md" | head -3); lead_n=$(grep -n '^`\|`<verdict>`' "$T/n.md" | head -3)
[ -n "$lead_o" ] && [ "$lead_o" = "$lead_n" ] && ok "lead line untouched" || bad "lead line changed"
# dry run writes nothing, prints one line
cp "$T/orig.md" "$T/n.md"
[ "$(f --scene $S --set crc32=deadbeef --dry-run 2>/dev/null | wc -l)" -eq 1 ] \
  && cmp -s "$T/orig.md" "$T/n.md" && ok "--dry-run prints one line, writes nothing" || bad "--dry-run"

# all six tokens: the verdict checker names no placeholder for the entry
cp "$T/orig.md" "$T/n.md"
f --scene $S --set verdict=right --set route=dump --set commit=0123456789abcdef \
  --set record=records/imx8mp/20261124-1200-gm02-01234567/ --set crc32=deadbeef \
  --set picdiff=0.1% >"$T/out" 2>&1
grep -q 'unfilled: none' "$T/out" && ok "all six filled, none unfilled" || bad "unfilled not none"
n=$(grep -c '<\(verdict\|route\|commit\|record\|crc32\|picdiff\)>' "$T/orig.md")
m=$(grep -c '<\(verdict\|route\|commit\|record\|crc32\|picdiff\)>' "$T/n.md")
a=$(grep -n '^- \*\*\[build\] use-vbo=true\*\*' "$T/n.md" | cut -d: -f1)
python3 -I "$HERE/imx8mp-glmark2-verdicts.py" "$T/n.md" --complete >"$T/v" 2>&1
if ! grep -E "use-vbo=true|line $a:" "$T/v" | grep -qi 'placeholder\|<'; then
  ok "verdicts.py: no placeholder problem for the entry"; else bad "verdicts.py names a placeholder"; fi
[ "$m" -lt "$n" ] && ok "entry's placeholders gone" || bad "placeholders remain"

[ $fail -eq 0 ] && echo "lookedat-fill selftest: ok" || { echo "lookedat-fill selftest: FAILED"; exit 1; }
