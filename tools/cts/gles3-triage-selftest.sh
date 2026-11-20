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

# 4. notpass --pi4 row shape: '<LABEL>.console.txt:<status> <case>' under its header.
cat >"$tmp/pi4n.txt" <<'T'
# Pi 4 dEQP-GLES3 at abcdef12: every case that is not Pass, per image of tools/cts/pi4-gles3-tree.txt
# rows: <record>:<status> <case>, sorted by case name   (<image>.console.txt = the UART console of the image, the records of d1; NoResult = the case never began)
# images run (2 of 91): A1 FO6* (*: cases of the image never began)
A1.console.txt:Fail dEQP-GLES3.functional.a.b.c.one
A1.console.txt:QualityWarning dEQP-GLES3.functional.a.b.c.two
A1.console.txt:NotSupported dEQP-GLES3.functional.a.b.c.three
FO6.console.txt:NoResult dEQP-GLES3.functional.z.y.x.four
FO6.console.txt:NoResult dEQP-GLES3.functional.z.y.x.five
FO6.console.txt:Fail dEQP-GLES3.functional.z.y.w.six
T
printf 'gpu\tgroup\tpass\tfail\tnotsupported\tcrash\tother\nv3d\tfunctional.a\t5\t0\t0\t0\t0\nv3d\tfunctional.z\t0\t4\t0\t0\t0\n' >"$tmp/hostv.tsv"
for extra in "" "--gpu v3d" "--gpu v3d --host $tmp/hostv.tsv"; do
  # shellcheck disable=SC2086
  python3 -I "$tri" "$tmp/pi4n.txt" $extra >"$tmp/pi4n.md"
  [ "$(sum_cases <"$tmp/pi4n.md")" = 6 ] || fail "pi4 console rows ($extra): expected 6 cases"
  sec=$(awk '/^## functional.z.y.x$/{f=1;next}/^## /{f=0}f' "$tmp/pi4n.md")
  grep -q '^- cases: 2$' <<<"$sec" || fail "pi4 console rows ($extra): NoResult rows of z.y.x"
done
sec=$(awk '/^## functional.z.y.w$/{f=1;next}/^## /{f=0}f' "$tmp/pi4n.md")
grep -q '^- host-also-fails: 1$' <<<"$sec" || fail "pi4 console rows: --gpu v3d host verdict"

# 5. cause families: 3- and 4-column rows, --apply, --check, family grouping.
printf '# rules\nRCx-a\tdEQP-GLES3.functional.a.*\nRCy-b\tdEQP-GLES3.functional.b.*,dEQP-GLES3.functional.c.*\tboom\n#9-wd\tdEQP-GLES3.functional.d.*\n' >"$tmp/rules.txt"
printf '# h\nFail\tdEQP-GLES3.functional.a.one\tm\nFail\tdEQP-GLES3.functional.b.two\tboom here\nNoResult\tdEQP-GLES3.functional.d.three\nFail\tdEQP-GLES3.functional.a.four\tm\tKEEP-me\n' >"$tmp/f3.tsv"
python3 -I "$tri" "$tmp/f3.tsv" --depth 2 >"$tmp/f3.md"            # 3/4-column rows parse
[ "$(sum_cases <"$tmp/f3.md")" = 4 ] || fail "family: mixed 3/4-column parse"
python3 -I "$tri" "$tmp/f3.tsv" --families "$tmp/rules.txt" --apply "$tmp/f4.tsv"
[ "$(grep -vc '^#' "$tmp/f4.tsv")" = 4 ] || fail "apply: row count"
[ "$(grep -c '^#' "$tmp/f4.tsv")" = 1 ] || fail "apply: header kept"
awk -F'\t' '!/^#/ && NF != 4 {exit 1}' "$tmp/f4.tsv" || fail "apply: not 4 columns"
grep -q "$(printf 'one\tm\tRCx-a$')" "$tmp/f4.tsv" || fail "apply: RCx-a"
grep -q "$(printf 'two\tboom here\tRCy-b$')" "$tmp/f4.tsv" || fail "apply: regex rule"
grep -q "$(printf 'three\t\t#9-wd$')" "$tmp/f4.tsv" || fail "apply: '#' family id"
grep -q "$(printf 'four\tm\tKEEP-me$')" "$tmp/f4.tsv" || fail "apply: existing family kept"
python3 -I "$tri" "$tmp/f4.tsv" >"$tmp/fam.md"                      # groups by family
[ "$(nsec <"$tmp/fam.md")" = 4 ] || fail "family grouping: expected 4 groups"
grep -q '^## RCx-a$' "$tmp/fam.md" || fail "family grouping: RCx-a heading"
grep -q '^- family: RCx-a (1)$' "$tmp/fam.md" || fail "family grouping: count"
python3 -I "$tri" "$tmp/f4.tsv" --depth 2 >"$tmp/famd.md"
grep -q '^## functional.a$' "$tmp/famd.md" || fail "--depth keeps path grouping"
python3 -I "$tri" "$tmp/f4.tsv" --check >"$tmp/chk.txt" || fail "check should pass"
[ "$(tail -n1 "$tmp/chk.txt")" = "$(printf 'total\t4')" ] || fail "check total line"
grep -qx "$(printf 'RCx-a\t1')" "$tmp/chk.txt" || fail "check first family"
# a row no rule matches is 'unassigned', and --check lists it and fails
printf 'Fail\tdEQP-GLES3.functional.z.nope\tm\n' >>"$tmp/f3.tsv"
python3 -I "$tri" "$tmp/f3.tsv" --families "$tmp/rules.txt" --apply "$tmp/f5.tsv"
grep -q "$(printf 'nope\tm\tunassigned$')" "$tmp/f5.tsv" || fail "apply: unassigned"
if python3 -I "$tri" "$tmp/f5.tsv" --check >"$tmp/o.txt" 2>"$tmp/e.txt"; then fail "check passed with unassigned"; fi
grep -q 'functional.z.nope' "$tmp/e.txt" || fail "check does not list the unassigned row"
# an empty family cell (3-column row) fails --check
if python3 -I "$tri" "$tmp/f3.tsv" --check >"$tmp/o.txt" 2>"$tmp/e.txt"; then fail "check passed with empty family"; fi
grep -q 'empty family' "$tmp/e.txt" || fail "check does not name the empty family"
# an empty outcome cell fails --check
printf '\tdEQP-GLES3.functional.a.x\tm\tRCx-a\n' >"$tmp/eo.tsv"
if python3 -I "$tri" "$tmp/eo.tsv" --check >"$tmp/o.txt" 2>"$tmp/e.txt"; then fail "check passed with empty outcome"; fi
grep -q 'empty outcome' "$tmp/e.txt" || fail "check does not name the empty outcome"
# the real list through the seeded rules: --apply gives 4 columns, --check sums or lists
python3 -I "$tri" "$np" --families "$here/am62p-gles3-families.txt" --apply "$tmp/real4.tsv"
awk -F'\t' '!/^#/ && NF != 4 {exit 1}' "$tmp/real4.tsv" || fail "real apply: not 4 columns"
if python3 -I "$tri" "$tmp/real4.tsv" --check >"$tmp/rc.txt" 2>/dev/null; then
  [ "$(tail -n1 "$tmp/rc.txt")" = "$(printf 'total\t%s' "$rows")" ] || fail "real check total"
fi

echo "gles3-triage-selftest: ok"
