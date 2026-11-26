#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/cts/imx95-t6-diff-selftest.sh
#
# Host-only check of tools/cts/imx95-t6-diff.py on a synthetic T6 slice and
# a synthetic iMX95 slice made here under $TMPDIR (no board data): each kind
# of difference (Pass->Fail, Fail->Pass, Pass->hang, Pass->crash,
# Pass->not-run, two refusals of other words, a refusal against none) is
# counted per transition and per cause and listed per case; a refusal cut
# short, a Fail without a refusal on both sides, a NotSupported on both and a
# case missing from a run row is Lost (a difference of its own, counted
# apart); a row the iMX95 slice did not run is listed as not run and its cases
# are no differences (--absent makes them); a case outside the T6 row map is
# extra, no difference; --rows overrides the rows the iMX95 slice ran.
# A slice against itself reports 0 differences, the fixture's and, when the
# tree has it, the T6's 2026-11-24-t6-gles2-S1-8ff01055. Exit 0 when every
# check holds.
set -u
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-.}/imx95-t6-diff.XXXXXX") || exit 2
# SELFTEST_KEEP=1 keeps the fixture (its path goes to stderr) to look at.
if [ -n "${SELFTEST_KEEP:-}" ]; then echo "fixture: $t" >&2; else trap 'rm -rf "$t"' EXIT; fi
fails=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fails=$((fails + 1)); }
has() { # label file fixed-string
	if grep -F -q -- "$3" "$2"; then ok "$1"; else bad "$1 (no '$3')"; fi
}
hasnt() {
	if grep -F -q -- "$3" "$2"; then bad "$1 ('$3' is there)"; else ok "$1"; fi
}
P=dEQP-GLES2.functional.a
X="link error: the vertex shader: online shaderc: the code generator refused this fragment shader: Mali(Valhall: a widget operand of a long form is not lowered yet)"
Xcut="online shaderc: the code generator refused this fragment shader: Mali(Valhall: a widget operand of a lo"
Y="online shaderc: the code generator refused this vertex shader: Mali(Valhall: a gadget is not lowered yet)"
slice() { # dir os
	mkdir -p "$1"
	{
		echo "commit=0000000000000000000000000000000000000000"
		echo "session=S1"
		echo "rows=1-1"
		echo "tree=tools/cts/mali-gles2-tree.txt"
		for k in boot_id_before boot_id_after kernel lib started ended; do echo "$k=-"; done
		[ -n "$2" ] && echo "os=$2"
	} > "$1/meta.txt"
	printf 'row\tlabel\tcases\tstatus\tseconds\tkmsg\tresets\tssh_after\tboot_same\texit_line\n' > "$1/chunks.tsv"
	printf 'case\tstatus\tdetail\n' > "$1/results.tsv"
	printf 'case\trefusal\n' > "$1/refusals.tsv"
}
res() { printf '%s\t%s\t%s\n' "$P.$2" "$3" "$4" >> "$1/results.tsv"; }
ref() { printf '%s\t%s\n' "$P.$2" "$3" >> "$1/refusals.tsv"; }
A=$t/log/2026-11-24-t6-gles2-S1-00000000
B=$t/log/2026-11-25-imx95-gles2-S1a-00000000
slice "$A" ""
slice "$B" zephyr
printf '1\tT6S1R01\t12\tdone\t5\t0\t0\tyes\tyes\t-\n' >> "$A/chunks.tsv"
printf '2\tT6S1R02\t2\tdone\t5\t0\t0\tyes\tyes\t-\n' >> "$A/chunks.tsv"
printf '1\tT6S1R01\t11\thang\t661\t-\t-\t-\t-\t-\n' >> "$B/chunks.tsv"
res "$A" p_f Pass Pass;                   res "$B" p_f Fail -
res "$A" f_p Fail "Image comparison failed"; res "$B" f_p Pass -
res "$A" p_hang Pass Pass;                res "$B" p_hang Lost chunk-hang
res "$A" p_notrun Pass Pass;              res "$B" p_notrun Lost not-run
res "$A" ref_same Fail "Compile failed";  res "$B" ref_same Fail -
ref "$A" ref_same "$X";                   ref "$B" ref_same "$Xcut"
res "$A" ref_other Fail "Compile failed"; res "$B" ref_other Fail -
ref "$A" ref_other "$X";                  ref "$B" ref_other "$Y"
res "$A" fail_both Fail "Image comparison failed: max 3"; res "$B" fail_both Fail -
res "$A" pass_both Pass Pass;             res "$B" pass_both Pass -
res "$A" t6only Pass Pass
res "$A" ns NotSupported "No multisample buffers"; res "$B" ns NotSupported -
res "$A" p_crash Pass Pass;               res "$B" p_crash Lost chunk-crash
res "$A" ref_none Fail "Compile failed";  res "$B" ref_none Fail -
ref "$A" ref_none "$X"
res "$A" r2a Pass Pass; res "$A" r2b Fail "Image comparison failed"
res "$B" x_extra Pass -

D=$here/imx95-t6-diff.py
python3 -I "$D" --imx95 "$B" --t6 "$A" > "$t/out" 2> "$t/err" && ok "runs" || { bad "runs"; cat "$t/err"; }
has "counts the cases on both sides, Lost and extra" "$t/out" "cases on both sides: 11; Lost (T6 case of a run row, missing from the iMX95 results): 1; extra (iMX95 case not in the T6 map, no difference): 1"
has "the row not run is listed" "$t/out" "rows not run (1, 2 T6 cases): T6S1R02"
has "the rows compared are listed" "$t/out" "rows compared (1): T6S1R01"
has "the extra case is named" "$t/out" "extra cases: $P.x_extra"
hasnt "an extra case is no difference" "$t/out" "$P.x_extra  T6"
hasnt "a row not run gives no per-case difference" "$t/out" "$P.r2b"
has "eight differences (seven and the Lost case)" "$t/out" "differences: 8"
has "the Lost case is a difference" "$t/out" "$P.t6only  T6: Pass (-)  iMX95: Lost (missing)"
grep -E -q '^  Pass->Lost +1$' "$t/out" && ok "transition Pass->Lost" || bad "transition Pass->Lost"
for tr in Pass-\>Fail Fail-\>Pass Pass-\>hang Pass-\>crash Pass-\>not-run; do
	if grep -E -q "^  $tr +1$" "$t/out"; then ok "transition $tr counted once"; else bad "transition $tr counted once"; fi
done
grep -E -q '^  Fail->Fail +2$' "$t/out" && ok "transition Fail->Fail (two refusal changes)" || bad "transition Fail->Fail (two refusal changes)"
has "per cause: two refusals of other words" "$t/out" "T6: refused, fragment shader: Valhall: a widget operand of a long form is not lowered yet | iMX95: refused, vertex shader: Valhall: a gadget is not lowered yet"
has "per cause: a refusal against none" "$t/out" "| iMX95: -"
has "per case: states and causes" "$t/out" "$P.f_p  T6: Fail (Image comparison failed)  iMX95: Pass (-)"
has "per case: the hang" "$t/out" "$P.p_hang  T6: Pass (-)  iMX95: hang (chunk-hang)"
hasnt "a refusal cut short is the same refusal" "$t/out" "$P.ref_same "
hasnt "a Fail without a refusal on both sides is no difference" "$t/out" "$P.fail_both "
hasnt "NotSupported on both sides is no difference" "$t/out" "$P.ns "
python3 -I "$D" --absent --imx95 "$B" --t6 "$A" > "$t/abs" 2>&1
has "--absent: the cases of the row not run are differences" "$t/abs" "differences: 10"
grep -E -q '^  Pass->absent +1$' "$t/abs" && ok "--absent: transition Pass->absent" || bad "--absent: transition Pass->absent"
grep -E -q '^  Fail->absent +1$' "$t/abs" && ok "--absent: transition Fail->absent" || bad "--absent: transition Fail->absent"
python3 -I "$D" --rows T6S1R01,T6S1R02 --imx95 "$B" --t6 "$A" > "$t/r12" 2>&1
has "--rows adds a row the slice did not run: its cases are Lost" "$t/r12" "Lost (T6 case of a run row, missing from the iMX95 results): 3"
has "--rows: no row left not run" "$t/r12" "rows not run (0, 0 T6 cases): none"
has "--rows: differences" "$t/r12" "differences: 10"
python3 -I "$D" --rows T6S1R01-R02 --imx95 "$B" --t6 "$A" > "$t/rng" 2>&1
has "--rows: a range" "$t/rng" "rows compared (2): T6S1R01, T6S1R02"
python3 -I "$D" --rows T6S1R02 --imx95 "$B" --t6 "$A" > "$t/r2" 2>&1
has "--rows narrower than the slice: its cases are left out" "$t/r2" "iMX95 cases of rows outside the set, left out: 11"
has "--rows narrower: the row's own cases are Lost" "$t/r2" "Lost (T6 case of a run row, missing from the iMX95 results): 2"
has "--rows narrower: row 1 is not run" "$t/r2" "rows not run (1, 12 T6 cases): T6S1R01"
python3 -I "$D" --rows T6S1R09 --imx95 "$B" --t6 "$A" > /dev/null 2>&1
[ $? -eq 2 ] && ok "--rows with an unknown label exits 2" || bad "--rows with an unknown label exits 2"
python3 -I "$D" --limit 2 --imx95 "$B" --t6 "$A" > "$t/lim" 2>&1
has "--limit: the rest counted" "$t/lim" "... and 6 more"
has "--limit: the counts stay whole" "$t/lim" "differences: 8"
python3 -I "$D" --markdown --imx95 "$B" --t6 "$A" > "$t/md" 2>&1
has "--markdown: transition table" "$t/md" "| T6 -> iMX95 | Cases |"
has "--markdown: cause table" "$t/md" "| T6 -> iMX95 | T6 cause | iMX95 cause | Cases |"
has "--markdown: case table row" "$t/md" "| \`$P.p_f\` | Pass | - | Fail | - |"
has "--markdown: slices named" "$t/md" "iMX95 slices: \`2026-11-25-imx95-gles2-S1a-00000000\`."
python3 -I "$D" --imx95 "$A" --t6 "$A" > "$t/self" 2>&1
has "a slice against itself: 0 differences" "$t/self" "differences: 0"
R=$here/log/2026-11-24-t6-gles2-S1-8ff01055
if [ -d "$R" ]; then
	python3 -I "$D" --imx95 "$R" --t6 "$R" > "$t/real" 2>&1
	has "the T6's 8ff01055 S1 slice against itself: 0 differences" "$t/real" "differences: 0"
	has "the T6's slice against itself: nothing Lost, nothing extra" "$t/real" "Lost (T6 case of a run row, missing from the iMX95 results): 0; extra (iMX95 case not in the T6 map, no difference): 0"
else
	echo "  skip  the T6's 8ff01055 S1 slice is not in this tree"
fi
python3 -I "$D" --imx95 "$t/none" --t6 "$A" > /dev/null 2>&1
[ $? -eq 2 ] && ok "an unreadable slice exits 2" || bad "an unreadable slice exits 2"
python3 -I "$D" --t6 "$A" > /dev/null 2>&1
[ $? -eq 2 ] && ok "no iMX95 slice: usage, exit 2" || bad "no iMX95 slice: usage, exit 2"

if [ "$fails" -ne 0 ]; then
	echo "imx95-t6-diff-selftest: $fails check(s) FAILED" >&2
	exit 1
fi
echo "imx95-t6-diff-selftest: all checks passed"
