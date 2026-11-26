#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-wrongscene.py on synthetic notes in $TMPDIR
# (made from the by-eye note, which is only read). Exit 2 without TMPDIR.
set -u
[ -n "${TMPDIR:-}" ] || { echo "imx8mp-glmark2-wrongscene-selftest: TMPDIR is unset" >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$(dirname "$HERE")")"
WS="$HERE/imx8mp-glmark2-wrongscene.py"
VERDICTS="$HERE/imx8mp-glmark2-verdicts.py"
NOTE="$ROOT/docs/notes/imx8mp-glmark2-pictures-by-eye.md"
T="$(mktemp -d "$TMPDIR/wrongscene.XXXXXX")" || exit 2
trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fail=1; }

ROW=16
REC=20261124-1216-gm16-desktop-0123abcd
PIC='picdiff: mean 41.20 max 255 differing 38.5% (ref glmark2-ref/gm16-desktop.png)'
WHAT='the four windows are drawn without the blur; their edges are sharp'

# skeleton copy (every row reset) and filled copy: every row right, row ROW wrong
python3 - "$NOTE" "$T/skel.md" "$T/filled.md" "$ROW" <<'PY'
import re, sys
src, skel, filled, row = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
s = open(src, encoding="utf-8").read()
pat = re.compile(r"^(\| \[[a-z0-9]+\] [^|]*)\|.*$", re.M)
open(skel, "w", encoding="utf-8").write(
    pat.sub(r"\1| - | - | - | not yet looked at |", s))
n = 0
def fill(m):
    global n
    n += 1
    v = "wrong" if n == row else "right"
    return (f"{m.group(1)}| 0123abcd | records/imx8mp/20261124-12{n:02d}-s{n}-0123abcd/"
            f" | dump | {v} |")
f = pat.sub(fill, s)
# a filled note as verdicts.py --complete wants it: one 'What was looked at'
# bullet per scene and none of the skeleton's placeholder texts. The real
# note may already hold a bullet for a scene (its draft entries, d535563e):
# only the scenes without one get a bullet here, so none appears twice.
scenes = [m.group(1).strip("| ") for m in pat.finditer(s)]
have = set(re.findall(r"^- \*\*(\[[^*]+)\*\*", s, re.M))
f = f.replace("## What was looked at\n",
              "## What was looked at\n\n"
              + "".join(f"- **{c}**: looked at it.\n" for c in scenes
                        if c not in have), 1)
for p in ("Not yet established", "To fill in", "Skeleton: no scene"):
    f = f.replace(p, "Filled")
open(filled, "w", encoding="utf-8").write(f)
PY

scene_of() { # note row -> scene cell
  python3 - "$1" "$2" <<'PY'
import sys
rows, sect = [], None
for line in open(sys.argv[1], encoding="utf-8"):
    if line.startswith("## "):
        sect = line[3:].strip()
    elif sect == "Verdicts" and line.startswith("| ["):
        rows.append(line.strip().strip("|").split("|")[0].strip())
print(rows[int(sys.argv[2]) - 1])
PY
}
gen() { # note [extra args...]
  local n="$1"; shift
  python3 "$WS" --note "$n" --row "$ROW" --record "$REC" --picdiff "$PIC" --what "$WHAT" "$@"
}

# 1. heading equals the scene cell, on the skeleton and the filled note
for n in skel filled; do
  want="### $(scene_of "$T/$n.md" "$ROW")"
  if gen "$T/$n.md" > "$T/$n.out" 2> "$T/$n.err"; then
    got=$(head -n 1 "$T/$n.out")
    [ "$got" = "$want" ] && ok "$n: heading is row $ROW's scene cell" \
                         || bad "$n: heading '$got', want '$want'"
  else
    bad "$n: refused: $(cat "$T/$n.err")"
  fi
done
# the skeleton keeps the real note's scene cells: row ROW's is the same text
# (the real note itself is not run: once filled, its commit is not 0123abcd)
[ "$(scene_of "$NOTE" "$ROW")" = "$(scene_of "$T/skel.md" "$ROW")" ] \
  && ok "skeleton: row $ROW's scene cell is the real note's" \
  || bad "skeleton: row $ROW's scene cell differs from the real note's"

# 2. the body: title, commit, record path, route, picdiff verbatim, what, label
out="$T/filled.out"
scene=$(scene_of "$T/filled.md" "$ROW")
for want in "**Title**: $scene: renders wrong on the i.MX8MP (GC7000UL)" \
            "Vivante GC7000UL" "Commit: 0123abcd" \
            "Record: \`records/imx8mp/$REC/\`" "Route: dump" "$PIC" \
            "What differs from llvmpipe: $WHAT" \
            "tools/cts/imx8mp-glmark2-dumps.sh build <img-dir> gm16" \
            "tools/cts/imx8mp-glmark2-dumps.sh run <img-dir> <records-dir> gm16" \
            "gm16-desktop.png"; do
  grep -qF -- "$want" "$out" && ok "body has: $want" || bad "body lacks: $want"
done
grep -qxF -- "$PIC" "$out" && ok "picdiff line verbatim on its own line" \
                           || bad "picdiff line not verbatim"
[ "$(grep -c '^## ' "$out")" = 0 ] && ok "entry opens no '## ' section" \
                                   || bad "entry has a '## ' line"

# 3. pasted under '## Wrong scenes' of the filled note: verdicts.py --complete
#    accepts it; without it, it does not
if python3 "$VERDICTS" "$T/filled.md" --complete > "$T/v0" 2>&1; then
  bad "verdicts.py --complete accepts a wrong row without an entry"
else
  grep -q "no entry under '## Wrong scenes'" "$T/v0" \
    && ok "no entry: verdicts.py --complete refuses" \
    || bad "no entry: unexpected refusal: $(cat "$T/v0")"
fi
paste() { # note entry out
  python3 - "$1" "$2" "$3" <<'PY'
import sys
note, entry = (open(p, encoding="utf-8").read() for p in sys.argv[1:3])
dst = sys.argv[3]
note = note.replace("## Wrong scenes\n", "## Wrong scenes\n\n" + entry + "\n", 1)
open(dst, "w", encoding="utf-8").write(note)
PY
}
paste "$T/filled.md" "$out" "$T/pasted.md"
if python3 "$VERDICTS" "$T/pasted.md" --complete > "$T/v1" 2>&1; then
  ok "pasted: verdicts.py --complete: $(cat "$T/v1")"
else
  bad "pasted: verdicts.py --complete refuses: $(cat "$T/v1")"
fi

# 3a. issue-texts.py prints the title of the pasted entry (the Title line
#     starts with the scene cell it looks for)
ISSUE="$HERE/imx8mp-glmark2-issue-texts.py"
python3 "$ISSUE" --note "$T/pasted.md" > "$T/it.out" 2> "$T/it.err"
if grep -qF -- "$scene: renders wrong on the i.MX8MP (GC7000UL)" "$T/it.out"; then
  ok "issue-texts.py prints the entry's title"
else
  bad "issue-texts.py lacks the title: $(head -n 3 "$T/it.err") $(grep -c 'issue title not in the note' "$T/it.out") untitled"
fi
grep -qF "(issue title not in the note)" "$T/it.out" \
  && bad "issue-texts.py: '(issue title not in the note)'" \
  || ok "issue-texts.py: no '(issue title not in the note)'"

# 3b. the row's route cell is used: photograph+probes is a route of a wrong
#     row, photograph alone ('seen without a value') is not
route_note() { # route out
  python3 - "$T/filled.md" "$2" "$ROW" "$1" <<'PY'
import sys
src, dst, row, route = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
out, n, sect = [], 0, None
for line in open(src, encoding="utf-8").read().split("\n"):
    if line.startswith("## "):
        sect = line[3:].strip()
    elif sect == "Verdicts" and line.startswith("| ["):
        n += 1
        if n == row:
            c = line.strip().strip("|").split("|")
            c[3] = f" {route} "
            line = "|" + "|".join(c) + "|"
    out.append(line)
open(dst, "w", encoding="utf-8").write("\n".join(out))
PY
}
route_note photograph+probes "$T/probes.md"
if gen "$T/probes.md" > "$T/probes.out" 2> "$T/probes.err"; then
  grep -qxF -- "- Route: photograph+probes" "$T/probes.out" \
    && ok "photograph+probes row: Route: photograph+probes" \
    || bad "photograph+probes row: route line missing"
  paste "$T/probes.md" "$T/probes.out" "$T/probes-pasted.md"
  python3 "$VERDICTS" "$T/probes-pasted.md" --complete > "$T/v2" 2>&1 \
    && ok "photograph+probes pasted: verdicts.py --complete: $(cat "$T/v2")" \
    || bad "photograph+probes pasted: verdicts.py --complete refuses: $(cat "$T/v2")"
else
  bad "photograph+probes row refused: $(cat "$T/probes.err")"
fi
route_note photograph "$T/photo.md"

# 4. the note is never written
sk=$(sha256sum < "$T/filled.md")
gen "$T/filled.md" > /dev/null 2>&1
[ "$(sha256sum < "$T/filled.md")" = "$sk" ] && ok "note unchanged" || bad "note was written"

# 5. refusals
refuse() { # label args...
  local l="$1"; shift
  if python3 "$WS" "$@" > "$T/r.out" 2> "$T/r.err"; then
    bad "$l: accepted"
  elif [ -s "$T/r.out" ]; then
    bad "$l: refused but printed an entry"
  else
    ok "$l: refused ($(head -n 1 "$T/r.err"))"
  fi
}
base=(--note "$T/filled.md" --row "$ROW" --picdiff "$PIC" --what "$WHAT")
refuse "record without time" "${base[@]}" --record 20261124-gm16-0123abcd
refuse "record with short sha" "${base[@]}" --record 20261124-1216-gm16-0123abc
refuse "record with other sha" "${base[@]}" --record 20261124-1216-gm16-89abcdef
# The redaction-pattern values are built at run time, so this file holds none
# literally (tools/release/redaction-scan.py scans the tree).
ipv4="$((190 + 2)).$((160 + 8)).1.20"
tty="/dev/tty""USB0"
tailnet="board.example.ts"".net"
credword="pass""word.md"
refuse "IP-like --what" --note "$T/filled.md" --row "$ROW" --record "$REC" \
  --picdiff "$PIC" --what "seen on $ipv4"
refuse "tty in --picdiff" --note "$T/filled.md" --row "$ROW" --record "$REC" \
  --picdiff "capture of $tty" --what "$WHAT"
refuse "tailnet name in --what" --note "$T/filled.md" --row "$ROW" --record "$REC" \
  --picdiff "$PIC" --what "seen on $tailnet"
refuse "credential in --note" --note "$T/$credword" --row "$ROW" --record "$REC" \
  --picdiff "$PIC" --what "$WHAT"
refuse "photograph route (seen without a value)" --note "$T/photo.md" --row "$ROW" \
  --record "$REC" --picdiff "$PIC" --what "$WHAT"
refuse "--route photograph" --note "$T/skel.md" --row "$ROW" --record "$REC" \
  --picdiff "$PIC" --what "$WHAT" --route photograph
refuse "row out of range" --note "$T/filled.md" --row 34 --record "$REC" \
  --picdiff "$PIC" --what "$WHAT"
# a full records/imx8mp/<dir>/ path is accepted as the name
gen "$T/filled.md" > /dev/null 2>&1 \
  && python3 "$WS" --note "$T/filled.md" --row "$ROW" --record "records/imx8mp/$REC/" \
       --picdiff "$PIC" --what "$WHAT" | cmp -s - "$T/filled.out" \
  && ok "records/imx8mp/<dir>/ form gives the same entry" \
  || bad "records/imx8mp/<dir>/ form differs"

if [ "$fail" != 0 ]; then
  echo "== imx8mp-glmark2-wrongscene self-test FAILED" >&2
  exit 1
fi
echo "== imx8mp-glmark2-wrongscene self-test: all cases passed"
