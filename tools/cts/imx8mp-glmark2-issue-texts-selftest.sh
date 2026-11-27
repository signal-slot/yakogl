#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-issue-texts.py on synthetic notes in $TMPDIR and
# on the filled-note fixtures in tools/cts/testdata/imx8mp-glmark2-issue-texts/
# (the board wording cases: an X2 note, a Verdin note, a note naming no board).
# Needs only python3 and bash. Exit 2 if TMPDIR is unset.
set -u
if [ -z "${TMPDIR:-}" ]; then
  echo "TMPDIR is unset" >&2
  exit 2
fi
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-glmark2-issue-texts.py
skel=$here/../../docs/notes/imx8mp-glmark2-pictures-by-eye.md
d=$(mktemp -d "$TMPDIR/issue-texts.XXXXXX") || exit 2
trap 'rm -rf "$d"' EXIT
fail=0
bad() { echo "FAIL: $*" >&2; fail=1; }

# mk OUT "ROW NUMBERS THAT ARE WRONG": the skeleton with every row filled in
mk() {
  python3 - "$skel" "$1" "$2" <<'PY'
import sys
skel, out, wrong = sys.argv[1], sys.argv[2], set(sys.argv[3].split())
n = 0
lines = []
with open(skel, encoding="utf-8") as f:
    src = f.read().split("\n")
for l in src:
    if l.startswith("| [") and "not yet looked at" in l:
        n += 1
        c = [x.strip() for x in l.strip("|").split("|")]
        v = "wrong" if str(n) in wrong else "right"
        l = "| %s | 0123abcd | records/imx8mp/20261125-%04d-gm%02d-x-0123abcd/ | dump | %s |" % (
            c[0], 900 + n, n, v)
    lines.append(l)
t = "\n".join(lines)
if wrong:
    t = t.replace(
        "None yet. A scene judged",
        "- **Title**: [bump] bump-render=normals: flat on the i.MX8MP (GC7000UL)\n"
        "- **Title**: [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5: "
        "black on the i.MX8MP (GC7000UL)\n\nA scene judged")
with open(out, "w", encoding="utf-8") as f:
    f.write(t)
PY
}

mk "$d/right.md" ""
mk "$d/wrong.md" "11 33"

out=$(python3 "$tool" --note "$d/right.md" 2>"$d/e1") || bad "all-right note refused: $(cat "$d/e1")"
grep -q "33 right, 0 wrong, 0 seen without a value, of 33" <<<"$out" || bad "all-right counts"
[ "$(grep -c '^== [1-6]\. ' <<<"$out")" = 6 ] || bad "six blocks"
grep -q "no wrong scene" <<<"$out" || bad "all-right: no wrong scene"

out2=$(python3 "$tool" --note "$d/wrong.md" 2>"$d/e2") || bad "wrong note refused: $(cat "$d/e2")"
grep -q "31 right, 2 wrong, 0 seen without a value, of 33" <<<"$out2" || bad "wrong counts"
grep -q "\[bump\] bump-render=normals" <<<"$out2" || bad "wrong scene 1 named"
grep -q "\[loop\] fragment-steps=5:fragment-uniform=true" <<<"$out2" || bad "wrong scene 2 named"
grep -q "flat on the i.MX8MP" <<<"$out2" || bad "issue title 1"
grep -q "black on the i.MX8MP" <<<"$out2" || bad "issue title 2"
grep -q "間違いのシーン" <<<"$out2" || bad "Japanese wrong line"

# one photograph+probes row, judged by content only
awk -v n=0 '/^\| \[/ && /\| dump \| right \|$/ { n++; if (n == 5) sub(/\| dump \| right \|$/, "| photograph+probes | seen without a value |") } { print }' "$d/right.md" >"$d/pp.md"
grep -q 'photograph+probes' "$d/pp.md" || bad "pp fixture not built"
out4=$(python3 "$tool" --note "$d/pp.md" 2>"$d/e4") || bad "photograph+probes note refused: $(cat "$d/e4")"
grep -q "32 right, 0 wrong, 1 seen without a value, of 33" <<<"$out4" || bad "photograph+probes counts"
grep -q "route(s): dump, photograph+probes" <<<"$out4" || bad "photograph+probes named in routes"
sed 's/| photograph+probes |/| bogus-route |/' "$d/pp.md" >"$d/bad.md"
if python3 "$tool" --note "$d/bad.md" >/dev/null 2>&1; then bad "bogus route accepted"; fi

if python3 "$tool" --note "$skel" >"$d/o3" 2>"$d/e3"; then bad "skeleton accepted"; fi
[ -s "$d/o3" ] && bad "skeleton refusal printed to stdout"
grep -q "not yet looked at" "$d/e3" || bad "skeleton refusal message"

head -n 100 "$d/right.md" | grep -v '^| \[loop\]' >"$d/short.md"
if python3 "$tool" --note "$d/short.md" >/dev/null 2>&1; then bad "short note accepted"; fi

if printf '%s\n%s\n' "$out" "$out2" | grep -En '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password'; then
  bad "forbidden text in output"
fi
# every quoted 'replace' anchor of blocks 3, 4 and 6 exists verbatim in the real file
for pair in "3:RESULTS.md" "4:STATUS.md" "6:../../ROADMAP.md"; do
  n=${pair%%:*}; f=$here/${pair#*:}
  hdr=$(grep "^== $n\. " <<<"$out")
  [ -n "$hdr" ] || { bad "block $n header missing"; continue; }
  anchors=$(grep -o "'[^']*'" <<<"$hdr" | sed "s/^'//; s/'\$//")
  [ -n "$anchors" ] || bad "block $n names no anchor"
  while IFS= read -r a; do
    [ -n "$a" ] && { grep -qF -- "$a" "$f" || bad "block $n anchor not in ${pair#*:}: $a"; }
  done <<<"$anchors"
done

# --- board wording (the owner's rule of 2026-11-27: the i.MX8MP pictures are
# measured on the Armadillo-X2 while the Verdin is away) ---
td=$here/testdata/imx8mp-glmark2-issue-texts
[ -f "$td/note-x2.md" ] || bad "fixture $td/note-x2.md missing"

# perblock OUT NEEDLE LABEL: each of the six blocks' printed text names NEEDLE
perblock() {
  local o=$1 needle=$2 label=$3 b
  for b in 1 2 3 4 5 6; do
    grep -A2 -- "^== $b\. " <<<"$o" | grep -qF -- "$needle" \
      || bad "$label: block $b does not name $needle"
  done
}

outx=$(python3 "$tool" --note "$td/note-x2.md" 2>"$d/ex") || bad "x2 fixture refused: $(cat "$d/ex")"
grep -q "33 right, 0 wrong, 0 seen without a value, of 33" <<<"$outx" || bad "x2 note counts"
perblock "$outx" "Armadillo-X2" "x2 note, board derived from the note"
grep -q "Armadillo-X2 で測定、" <<<"$outx" || bad "x2 note: STATUS lines not marked as X2 readings"
grep -q "board: Armadillo-X2" <<<"$outx" || bad "x2 note: 'where' names no board"

outf=$(python3 "$tool" --note "$td/note-x2.md" --board "Armadillo-X2 (bench)" 2>"$d/ef") || bad "--board free text refused: $(cat "$d/ef")"
perblock "$outf" "Armadillo-X2 (bench)" "--board free text"

outv=$(python3 "$tool" --note "$td/note-x2.md" --board verdin 2>"$d/ev") || bad "--board verdin refused: $(cat "$d/ev")"
grep -q "Verdin iMX8M Plus" <<<"$outv" || bad "--board verdin names no board"
if grep -q "Armadillo-X2" <<<"$outv"; then bad "--board verdin printed Armadillo-X2"; fi

outn=$(python3 "$tool" --note "$td/note-verdin.md" 2>"$d/en") || bad "verdin fixture refused: $(cat "$d/en")"
grep -q "Verdin iMX8M Plus, GC7000UL" <<<"$outn" || bad "verdin note keeps the pre-rule wording"
grep -q "board: Verdin iMX8M Plus" <<<"$outn" || bad "verdin note: 'where' names no board"
if grep -q "Armadillo-X2" <<<"$outn"; then bad "verdin note printed Armadillo-X2"; fi

outb=$(python3 "$tool" --note "$td/note-no-board-mention.md" 2>"$d/eb") || bad "no-board fixture refused: $(cat "$d/eb")"
perblock "$outb" "Armadillo-X2" "note naming no board (owner's-rule default)"

# 192.0.2.0/24 is reserved (RFC 5737): a fixture address, never a lab one.
if python3 "$tool" --note "$td/note-x2.md" --board "192.0.2.99" >/dev/null 2>"$d/e10"; then bad "--board with an address accepted"; fi
grep -q "redaction" "$d/e10" || bad "--board address refusal names no redaction"
if python3 "$tool" --note "$td/note-x2.md" --board "/dev/ttyUSB3" >/dev/null 2>&1; then bad "--board with a tty path accepted"; fi
if python3 "$tool" --note "$td/note-x2.md" --board "" >/dev/null 2>&1; then bad "empty --board accepted"; fi

if printf '%s\n' "$outx" "$outf" "$outv" "$outn" "$outb" | grep -En '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password'; then
  bad "forbidden text in board-wording output"
fi

if [ "$fail" = 0 ]; then echo "imx8mp-glmark2-issue-texts-selftest: ok"; fi
exit "$fail"
