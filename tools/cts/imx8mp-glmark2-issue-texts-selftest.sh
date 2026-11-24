#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of imx8mp-glmark2-issue-texts.py on synthetic notes in $TMPDIR.
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

if python3 "$tool" --note "$skel" >"$d/o3" 2>"$d/e3"; then bad "skeleton accepted"; fi
[ -s "$d/o3" ] && bad "skeleton refusal printed to stdout"
grep -q "not yet looked at" "$d/e3" || bad "skeleton refusal message"

head -n 100 "$d/right.md" | grep -v '^| \[loop\]' >"$d/short.md"
if python3 "$tool" --note "$d/short.md" >/dev/null 2>&1; then bad "short note accepted"; fi

if printf '%s\n%s\n' "$out" "$out2" | grep -En '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|ttyUSB|ttyACM|ts\.net|password'; then
  bad "forbidden text in output"
fi
if [ "$fail" = 0 ]; then echo "imx8mp-glmark2-issue-texts-selftest: ok"; fi
exit "$fail"
