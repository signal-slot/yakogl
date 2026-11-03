#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of gles3-triage.py: the real am62p not-pass list and a synthetic one.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
tri="$here/gles3-triage.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/gles3-triage.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail() { echo "gles3-triage-selftest: FAIL: $*" >&2; exit 1; }

sum_cases() { awk '/^- cases: /{s+=$3}END{print s+0}'; }
nsec() { grep -c '^## ' || true; }

# 1. real list: group counts sum to the non-header rows, at several depths.
np="$root/docs/es3/am62p-gles3-notpass.tsv"
rows=$(grep -vc -e '^#' -e '^[[:space:]]*$' "$np")
for d in 2 4 6; do
  python3 -I "$tri" "$np" --host "$root/docs/es3/host-gles3-tip.tsv" \
    --table "$root/tools/cts/imx8mp-gles3-tree.txt" --depth "$d" >"$tmp/real$d.md"
  [ "$(sum_cases <"$tmp/real$d.md")" = "$rows" ] || fail "depth $d: sum != $rows"
done
[ "$(nsec <"$tmp/real2.md")" -le "$(nsec <"$tmp/real4.md")" ] || fail "depth 2 has more groups than depth 4"
grep -q '^- host-also-fails: ' "$tmp/real4.md" || fail "no host columns"
grep -q '^- table entries: e' "$tmp/real4.md" || fail "no table entries"

# 2. synthetic list with a synthetic host file and table.
cat >"$tmp/np.tsv" <<'T'
# header
Fail	dEQP-GLES3.functional.a.b.c.one	msg x
Fail	dEQP-GLES3.functional.a.b.c.two	msg x
NotSupported	dEQP-GLES3.functional.a.b.c.three
Fail	dEQP-GLES3.functional.a.b.d.four	Fail
Crash	dEQP-GLES3.functional.z.y.x.five	boom
NoResult	dEQP-GLES3.functional.z.y.w.six
Fail	dEQP-GLES3.functional.a.b.c.seven	msg x
Fail	dEQP-GLES3.functional.a.b.c.eight	msg x
Fail	dEQP-GLES3.functional.a.b.c.nine	msg x
Fail	dEQP-GLES3.functional.a.b.c.ten	msg x
T
printf 'gpu\tgroup\tpass\tfail\tnotsupported\tcrash\tother\nvivante\tfunctional.a\t5\t0\t0\t0\t0\nvivante\tfunctional.z\t0\t4\t0\t0\t0\n' >"$tmp/host.tsv"
printf '# t\ndEQP-GLES3.functional.a.*\ndEQP-GLES3.functional.z.*\n' >"$tmp/tree.txt"
python3 -I "$tri" "$tmp/np.tsv" --host "$tmp/host.tsv" --table "$tmp/tree.txt" >"$tmp/syn.md"
[ "$(nsec <"$tmp/syn.md")" = 4 ] || fail "synthetic: expected 4 groups, got $(nsec <"$tmp/syn.md")"
[ "$(sum_cases <"$tmp/syn.md")" = 10 ] || fail "synthetic: expected 10 cases"
sec=$(awk '/^## functional.a.b.c$/{f=1;next}/^## /{f=0}f' "$tmp/syn.md")
grep -q '^- cases: 7$' <<<"$sec" || fail "a.b.c count"
grep -q '^- board-only: 7$' <<<"$sec" || fail "a.b.c board-only"
grep -q 'e00 (7)' <<<"$sec" || fail "a.b.c entry"
[ "$(grep -c '^  - `' <<<"$sec")" = 5 ] || fail "a.b.c should list 5 examples"
sec=$(awk '/^## functional.z.y.x$/{f=1;next}/^## /{f=0}f' "$tmp/syn.md")
grep -q '^- host-also-fails: 1$' <<<"$sec" || fail "z.y.x host-also-fails"
grep -q 'e01 (1)' <<<"$sec" || fail "z.y.x entry"
python3 -I "$tri" "$tmp/np.tsv" --depth 2 >"$tmp/syn3.md"
[ "$(nsec <"$tmp/syn3.md")" = 2 ] || fail "synthetic depth 2: expected 2 groups"

# 3. pi4 row shape
printf '# h\nbuild/f-A1.txt:Fail dEQP-GLES3.functional.q.r.s.t\nNoResult dEQP-GLES3.functional.q.r.s.u\n' >"$tmp/pi4.txt"
python3 -I "$tri" "$tmp/pi4.txt" >"$tmp/pi4.md"
[ "$(sum_cases <"$tmp/pi4.md")" = 2 ] || fail "pi4 shape"

echo "gles3-triage-selftest: ok"
