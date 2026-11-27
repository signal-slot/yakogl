#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-lookedat-fill.py on copies of the i.MX8MP
# by-eye note under TMPDIR (the note itself is never touched).
#
# The c5 refusal fixtures are assembled from parts, the way accept.sh's c5
# pattern is assembled: the gate's redaction scan reads every tracked file,
# and a written-out private address or .ts.net name carried here would be a
# finding. The fixtures stay refusals of the same runtime values. The tty
# names and 'my password' need no assembling: the scan counts neither.
set -u
[ -n "${TMPDIR:-}" ] || { echo "TMPDIR is not set" >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$HERE")")"
FILL="$HERE/imx8mp-glmark2-lookedat-fill.py"
NOTE="$ROOT/docs/notes/imx8mp-glmark2-pictures-by-eye.md"
[ -f "$FILL" ] || { echo "no $FILL" >&2; exit 2; }
[ -f "$NOTE" ] || { echo "no $NOTE" >&2; exit 2; }
T="$(mktemp -d "$TMPDIR/lookedat.XXXXXX")" || exit 2
trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fail=1; }

cp "$NOTE" "$T/orig.md"
cp "$T/orig.md" "$T/n.md"
S=gm02
CELL="[build] use-vbo=true"
if ! grep -q '^- \*\*\[build\] use-vbo=true\*\*.*<verdict>' "$T/orig.md"; then
  # A note whose entries are already filled: put the placeholders back
  # into this scene's entry, so a fill has something to replace.
  python3 -I - "$T/orig.md" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read().split("\n")
for i, l in enumerate(s):
    if l.startswith("- **[build] use-vbo=true**"):
        s[i] = (l.split("**:")[0]
                + "**: the same horse; verdict <verdict>, route <route>,"
                  " commit <commit>, record <record>, frame crc32 <crc32>,"
                  " picdiff <picdiff>.")
open(p, "w").write("\n".join(s))
PY
  cp "$T/orig.md" "$T/n.md"
fi
f() { python3 -I "$FILL" --note "$T/n.md" "$@"; }

refuse() { # label args...
  local l="$1"; shift
  cp "$T/orig.md" "$T/n.md"
  f "$@" >/dev/null 2>&1; local rc=$?
  if [ $rc -eq 1 ] && cmp -s "$T/orig.md" "$T/n.md"; then ok "refused: $l"
  else bad "refused: $l (exit $rc or file changed)"; fi
}

# The c5 fixtures, assembled so that this file carries no literal of them.
v4="1""0.1.2.3"
v4wide="99""9.999.999.999"
tsnet="box.tail1234"".ts.net"

refuse "unknown token" --scene $S --set bogus=x
refuse "not TOKEN=VALUE" --scene $S --set verdict
refuse "token given twice" --scene $S --set verdict=right --set verdict=right
refuse "empty value" --scene $S --set verdict=
refuse "value with <" --scene $S --set verdict='a<b'
refuse "value with >" --scene $S --set verdict='a>b'
refuse "value with |" --scene $S --set verdict='a|b'
PICLINE='mean |diff| 1.20 0.80 0.90; 3.1% of pixels differ by more than 8 in any channel; bbox 0,0,10,10'
refuse "real picdiff line with |diff|" --scene $S --set picdiff="$PICLINE"
f --scene $S --set picdiff="$PICLINE" 2>&1 >/dev/null | grep -q "abs diff" \
  && ok "refusal names the 'abs diff' wording" || bad "refusal lacks the hint"
f --scene $S --set picdiff="${PICLINE//|diff|/abs diff}" >/dev/null 2>&1 \
  && ok "picdiff line with |diff| spelled 'abs diff' is accepted" || bad "abs diff line refused"
cp "$T/orig.md" "$T/n.md"
refuse "value with newline" --scene $S --set verdict=$'a\nb'
refuse "ipv4" --scene $S --set route="$v4"
refuse "ipv4 outside the private ranges" --scene $S --set commit="$v4wide"
refuse "/dev/tty" --scene $S --set route='/dev/ttyS1'
refuse "ttyUSB" --scene $S --set route='ttyUSB0'
refuse "ttyUSB in another case" --scene $S --set route='TTYUSB0'
refuse "ttyACM" --scene $S --set route='ttyACM0'
refuse "ts.net" --scene $S --set route="$tsnet"
refuse "password" --scene $S --set route='my password'
refuse "unknown scene gm99" --scene gm99 --set verdict=right
refuse "unknown scene 99" --scene 99 --set verdict=right
python3 -I "$FILL" --note "$T/nope.md" --scene $S --set verdict=right >/dev/null 2>&1; rc=$?
[ $rc -eq 1 ] && [ ! -e "$T/nope.md" ] \
  && ok "refused: note file missing, nothing written" \
  || bad "refused: note file missing (exit $rc)"

# a token of the six that this entry does not hold (scratched away first)
cp "$T/orig.md" "$T/miss.md"
python3 -I - "$T/miss.md" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read().split("\n")
for i, l in enumerate(s):
    if l.startswith("- **[build] use-vbo=true**"):
        s[i] = l.replace("<crc32>", "x")
open(p, "w").write("\n".join(s))
PY
cp "$T/miss.md" "$T/n.md"
f --scene $S --set crc32=deadbeef >/dev/null 2>&1; rc=$?
[ $rc -eq 1 ] && cmp -s "$T/miss.md" "$T/n.md" \
  && ok "refused: token not in the entry" \
  || bad "refused: token not in the entry (exit $rc)"

# a token present in the note but already filled in this entry
cp "$T/orig.md" "$T/n.md"
f --scene $S --set verdict=right >/dev/null
f --scene $S --set verdict=wrong >/dev/null 2>&1; rc=$?
[ $rc -eq 1 ] && ok "refused: token already filled" || bad "token already filled (exit $rc)"

f --scene $S >/dev/null 2>&1; rc=$?
[ $rc -eq 2 ] && ok "usage: no --set exits 2" || bad "usage: no --set (exit $rc)"
f >/dev/null 2>&1; rc=$?
[ $rc -eq 2 ] && ok "usage: no --scene exits 2" || bad "usage: no --scene (exit $rc)"
f --scene $S --set verdict=x --bogus >/dev/null 2>&1; rc=$?
[ $rc -eq 2 ] && ok "usage: unknown option exits 2" || bad "usage: unknown option (exit $rc)"

# the scene forms fill the same entry, alike: gmNN, NN and the exact cell
cp "$T/orig.md" "$T/gm02.md"
python3 -I "$FILL" --note "$T/gm02.md" --scene gm02 --set verdict=right >/dev/null 2>&1 \
  && ok "resolve: gm02 fills the entry" || bad "resolve: gm02"
for s in 2 02 gm2 "$CELL"; do
  cp "$T/orig.md" "$T/r.md"
  python3 -I "$FILL" --note "$T/r.md" --scene "$s" --set verdict=right >/dev/null 2>&1 \
    && cmp -s "$T/r.md" "$T/gm02.md" && ok "resolve: $s fills the same entry" \
    || bad "resolve: $s"
done

# one fill changes only that entry line
cp "$T/orig.md" "$T/n.md"
f --scene "$CELL" --set verdict=right --set route=dump >"$T/out" 2>&1; rc=$?
n=$(diff "$T/orig.md" "$T/n.md" | grep -c '^>')
[ $rc -eq 0 ] && [ "$n" -eq 1 ] && ok "one fill changes one line" \
  || bad "one fill changes $n lines (exit $rc)"
grep -q 'unfilled: commit, record, crc32, picdiff' "$T/out" \
  && ok "unfilled tokens printed" || bad "unfilled tokens not printed"
diff "$T/orig.md" "$T/n.md" | grep '^>' | grep -q -F -- "- **$CELL** ($S-build)" \
  && ok "scene text byte for byte" || bad "scene text changed"
lead_o=$(grep -nF -- '`<verdict>`' "$T/orig.md"); lead_n=$(grep -nF -- '`<verdict>`' "$T/n.md")
[ -n "$lead_o" ] && [ "$lead_o" = "$lead_n" ] && ok "lead line untouched" \
  || bad "lead line changed"

# --dry-run writes nothing and prints the new line, the one a real fill writes
cp "$T/orig.md" "$T/n.md"
f --scene $S --set crc32=deadbeef --dry-run 2>/dev/null >"$T/dry.out"
dry_wrote=nothing; cmp -s "$T/orig.md" "$T/n.md" || dry_wrote=changed
cp "$T/orig.md" "$T/n.md"
f --scene $S --set crc32=deadbeef >/dev/null 2>&1
diff "$T/orig.md" "$T/n.md" | grep '^> ' | sed 's/^> //' >"$T/real.line"
if [ "$dry_wrote" = nothing ] && [ "$(wc -l < "$T/dry.out")" -eq 1 ] \
   && cmp -s "$T/dry.out" "$T/real.line"; then
  ok "--dry-run prints the new line only, writes nothing"
else bad "--dry-run"; fi

# all six tokens: the entry's placeholders are gone and the verdict checker
# names no placeholder for it
cp "$T/orig.md" "$T/n.md"
f --scene $S --set verdict=right --set route=dump --set commit=0123456789abcdef \
  --set record=records/imx8mp/20261124-1200-gm02-01234567/ --set crc32=deadbeef \
  --set picdiff=0.1% >"$T/out" 2>&1; rc=$?
[ $rc -eq 0 ] && grep -q 'unfilled: none' "$T/out" \
  && ok "all six filled, unfilled: none" || bad "all six filled (exit $rc)"
n=$(grep -c '<\(verdict\|route\|commit\|record\|crc32\|picdiff\)>' "$T/orig.md")
m=$(grep -c '<\(verdict\|route\|commit\|record\|crc32\|picdiff\)>' "$T/n.md")
[ "$m" -lt "$n" ] && ok "the entry's placeholders are gone" || bad "placeholders remain"
a=$(grep -n '^- \*\*\[build\] use-vbo=true\*\*' "$T/n.md" | cut -d: -f1)
python3 -I "$HERE/imx8mp-glmark2-verdicts.py" "$T/n.md" --complete >"$T/v" 2>&1
if ! grep -E "use-vbo=true|line $a:" "$T/v" | grep -qi 'placeholder\|<'; then
  ok "verdicts.py --complete names no placeholder for the entry"
else bad "verdicts.py names a placeholder for the entry"; fi

[ $fail -eq 0 ] && echo "lookedat-fill selftest: ok" \
  || { echo "lookedat-fill selftest: FAILED"; exit 1; }
