#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/cts/imx95-gles2-analyze-selftest.sh
#
# Host-only check of tools/cts/imx95-gles2-analyze.sh on two synthetic Verdin
# iMX95 sessions (S1a: rows 1-2, S1b: rows 3-4, eight cases) and one synthetic
# T6 slice made here under $TMPDIR (no board data). Checks the output files of
# the chain and its exit codes: 0 for the whole chain; 1 when a row of a
# session is missing (stage 1, nothing converted); 1 when a console case lies
# outside its row; 2 for a usage error (no sessions, an unknown option, a
# session without console/). Not a gate row. Exit 0 when every check holds.
set -u
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-.}/imx95-gles2-analyze.XXXXXX") || exit 2
# SELFTEST_KEEP=1 keeps the fixture (its path goes to stderr) to look at.
if [ -n "${SELFTEST_KEEP:-}" ]; then echo "fixture: $t" >&2; else trap 'rm -rf "$t"' EXIT; fi
fails=0
ok() { echo "  ok    $1"; }
bad() { echo "  FAIL  $1"; fails=$((fails + 1)); }
has() { # label file fixed-string
	if grep -F -q -- "$3" "$2"; then ok "$1"; else bad "$1 (no '$3' in $(basename "$2"))"; fi
}
hasnt() {
	if grep -F -q -- "$3" "$2"; then bad "$1 ('$3' in $(basename "$2"))"; else ok "$1"; fi
}
code() { # label want got
	if [ "$3" -eq "$2" ]; then ok "$1"; else bad "$1 (exit $3, want $2)"; fi
}
P=dEQP-GLES2.functional
Z=0000000000000000000000000000000000000000
cat > "$t/mustpass.txt" <<EOT
$P.a.one
$P.a.two
$P.b.three
$P.b.four
$P.c.five
$P.c.six
$P.d.seven
$P.d.eight
EOT
{
	n=0
	for g in a b c d; do
		n=$((n + 1))
		echo "# image T6S1R0$n: $P.$g.*"
		echo "$P.$g.*"
	done
} > "$t/tree.txt"

mkdir -p "$t/in"
B="vi95_deqp: build v0.1.0-1-g0000000"
console() { # dir label filter
	mkdir -p "$1/console"
	{
		printf 'U-Boot 2024.04 (board console)\r\n=> go 0xd0000000\r\n*** Booting Zephyr OS build v4.3 ***\r\n%s\r\n' "$B"
		echo "deqp: case filter '$3'"
		cat
		echo "verdin_imx95: board: run over: PSCI SYSTEM_RESET at 5000 ms"
	} > "$1/console/$2.log"
}
rows_head() { printf 'row\tlabel\tcases\tstatus\tseconds\tlifetime_s\tbbm\texit_line\n' > "$1/rows.tsv"; }
A=$t/in/2026-11-25-imx95-gles2-S1a-00000000
Bd=$t/in/2026-11-25-imx95-gles2-S1b-00000000
console "$A" T6S1R01 "$P.a.*" <<EOT
yakogl: glCompileShader(3): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a widget operand is not lowered yet) [fragment stage, 120 bytes]
deqp: Fail $P.a.one
deqp: Pass $P.a.two
deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings, complete=1
deqp: summary: 2 cases begun, 2 with a result, 0 without
EOT
console "$A" T6S1R02 "$P.b.*" <<EOT
deqp: Pass $P.b.three
deqp: Pass $P.b.four
deqp: done: 2 passed, 0 failed, 0 not supported, 0 warnings, complete=1
deqp: summary: 2 cases begun, 2 with a result, 0 without
EOT
console "$Bd" T6S1R03 "$P.c.*" <<EOT
deqp: Pass $P.c.five
deqp: done: 1 passed, 0 failed, 0 not supported, 0 warnings, complete=0
EOT
console "$Bd" T6S1R04 "$P.d.*" <<EOT
deqp: Pass $P.d.seven
deqp: Pass $P.d.eight
deqp: done: 2 passed, 0 failed, 0 not supported, 0 warnings, complete=1
deqp: summary: 2 cases begun, 2 with a result, 0 without
EOT
# row 3 ends in the WDOG3 reset: its second case is Lost
{
	printf 'U-Boot 2024.04 (board console)\r\n=> go 0xd0000000\r\n*** Booting Zephyr OS build v4.3 ***\r\n%s\r\n' "$B"
	echo "deqp: case filter '$P.c.*'"
	echo "deqp: Pass $P.c.five"
	echo "verdin_imx95: WDOG3 feeding stopped at 70000 ms after 69 feeds (no progress): it resets the board within 60000 ms"
} > "$Bd/console/T6S1R03.log"
rows_head "$A"
printf '1\tT6S1R01\t2\tdone\t9\t600\tmatch\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 5000 ms\n' >> "$A/rows.tsv"
printf '2\tT6S1R02\t2\tdone\t8\t600\tmatch\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 5000 ms\n' >> "$A/rows.tsv"
rows_head "$Bd"
printf '3\tT6S1R03\t-\thang\t131\t600\tnone\tverdin_imx95: WDOG3 feeding stopped at 70000 ms after ? feeds (no progress)\n' >> "$Bd/rows.tsv"
printf '4\tT6S1R04\t2\tdone\t8\t600\tmatch\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 5000 ms\n' >> "$Bd/rows.tsv"

# the T6 slice: a.one Fail as on the iMX95, d.eight Fail where the iMX95 passes,
# c.six Pass where the iMX95 lost it to the hang
T=$t/t6/2026-11-24-t6-gles2-S1-00000000
mkdir -p "$T"
cat > "$T/meta.txt" <<EOT
commit=$Z
session=S1
rows=1-4
tree=tools/cts/mali-gles2-tree.txt
boot_id_before=11111111-2222-3333-4444-555555555555
boot_id_after=11111111-2222-3333-4444-555555555555
kernel=6.1.0-test
lib=libGLESv2.so.2:000000000000
started=2026-11-24T10:00:00Z
ended=2026-11-24T10:10:00Z
EOT
{
	printf 'row\tlabel\tcases\tstatus\tseconds\tkmsg\tresets\tssh_after\tboot_same\texit_line\n'
	for r in 1:a 2:b 3:c 4:d; do printf '%s\t%s\t2\tdone\t10\t0\t0\tyes\tyes\t-\n' "${r%:*}" "T6S1R0${r%:*}"; done
} > "$T/chunks.tsv"
{
	printf 'case\tstatus\tdetail\n'
	printf '%s.a.one\tFail\tImage comparison failed\n' $P
	for c in a.two b.three b.four c.five c.six d.seven; do printf '%s.%s\tPass\tPass\n' $P $c; done
	printf '%s.d.eight\tFail\tImage comparison failed\n' $P
} > "$T/results.tsv"

XOPT=
an() { (cd "$t" && sh "$here/imx95-gles2-analyze.sh" --mustpass mustpass.txt --tree tree.txt --commit $Z \
	--no-git $XOPT --t6 "$T" --expect S1a=T6S1R01-T6S1R02 --expect S1b=T6S1R03-T6S1R04 "$@"); }

O=$t/out
an "$O" "$A" "$Bd" > "$t/out.txt" 2> "$t/err.txt"
code "the chain over two sessions exits 0" 0 $?
for n in "1 rows" "2 slice" "3 tally" "4 triage" "5 family" "6 diff" "7 cause"; do
	has "stage $n reports ok" "$t/out.txt" "stage $n: ok"
done
for f in tally.txt tally.md cover.txt triage.md family.txt diff.txt diff.md cause-input.tsv cause.tsv cause-summary.txt cause-differences.tsv; do
	[ -f "$O/$f" ] && ok "writes $f" || bad "writes $f"
done
[ -f "$O/slices/2026-11-25-imx95-gles2-S1a-00000000/results.tsv" ] && ok "converts a copy of each session" || bad "converts a copy of each session"
[ ! -f "$A/results.tsv" ] && ok "leaves the session folder as it was" || bad "leaves the session folder as it was"
[ -f "$O/slices/2026-11-25-imx95-gles2-S1b-00000000/README.md" ] && ok "writes the slice README" || bad "writes the slice README"
has "tally: 8 cases" "$O/tally.txt" "TOTAL"
has "tally: 5 Pass of the iMX95" "$O/tally.txt" "Pass"
has "tally: the hang case is Lost" "$O/tally.txt" "Lost"
has "triage: a compile refusal cause" "$O/triage.md" "refused"
has "triage: the hang is a known cause" "$O/triage.md" "hang"
has "diff: Fail->Pass (d.eight)" "$O/diff.txt" "Fail->Pass"
has "diff: Pass->hang (c.six)" "$O/diff.txt" "Pass->hang"
has "cause: header gets a cause column" "$O/cause.tsv" "$(printf 'case\tt6_result\timx95_result\tcause')"
has "cause: the same Fail on both is a compiler refusal" "$O/cause.tsv" "$(printf '%s.a.one\tFail\tFail\tcompiler-refusal' $P)"
has "cause: Lost is passed as Missing and tagged hang" "$O/cause.tsv" "$(printf '%s.c.six\tPass\tMissing\thang' $P)"
has "cause: Fail on the T6 only is unexplained" "$O/cause.tsv" "$(printf '%s.d.eight\tFail\tPass\tunexplained' $P)"
has "cause: summary has the total" "$O/cause-summary.txt" "$(printf '# total\t8')"
hasnt "cause-differences leaves out no-difference" "$O/cause-differences.tsv" "no-difference"
has "cause-differences keeps the header" "$O/cause-differences.tsv" "cause"

# a row missing from a session: stage 1 fails, nothing is converted
M=$t/miss/2026-11-25-imx95-gles2-S1a-00000000
mkdir -p "$t/miss"
cp -R "$A" "$M"
sed '/T6S1R02/d' "$A/rows.tsv" > "$M/rows.tsv"
an "$t/out-miss" "$M" "$Bd" > "$t/out.txt" 2> "$t/err.txt"
code "a missing row exits 1" 1 $?
has "a missing row is named at stage 1" "$t/err.txt" "stage 1 rows: FAIL"
has "the tool says which row" "$t/err.txt" "T6S1R02"
[ ! -d "$t/out-miss/slices/2026-11-25-imx95-gles2-S1a-00000000" ] && ok "nothing is converted after a stage 1 failure" || bad "nothing is converted after a stage 1 failure"
# --whole: a session set that lacks a row of the tree
# (with a tree of its own, whole's span of the tag S1a is not in the tree, so
# it fails at stage 1 either way; the whole-tree run is the default tree's)
XOPT=--whole
an "$t/out-whole" "$A" > "$t/out.txt" 2> "$t/err.txt"
rc=$?
XOPT=
code "--whole: a session set short of the tree exits 1" 1 $rc
has "--whole fails at stage 1" "$t/err.txt" "stage 1 rows: FAIL"
# a case outside its row's filter
W=$t/wrong/2026-11-25-imx95-gles2-S1a-00000000
mkdir -p "$t/wrong"
cp -R "$A" "$W"
printf 'deqp: Pass %s.d.seven\n' "$P" > "$t/x"
sed "s/^deqp: Pass $P.b.three/deqp: Pass $P.d.seven/" "$A/console/T6S1R02.log" > "$W/console/T6S1R02.log"
an "$t/out-wrong" "$W" "$Bd" > "$t/out.txt" 2> "$t/err.txt"
code "a console case outside its row exits 1" 1 $?
has "that is a stage 2 failure" "$t/err.txt" "stage 2 slice: FAIL"
# usage
sh "$here/imx95-gles2-analyze.sh" > /dev/null 2>&1
code "no arguments exit 2" 2 $?
sh "$here/imx95-gles2-analyze.sh" --bogus "$t/o" "$A" > /dev/null 2>&1
code "an unknown option exits 2" 2 $?
sh "$here/imx95-gles2-analyze.sh" "$t/o2" "$t/nosuch" > /dev/null 2>&1
code "a session folder without console/ exits 2" 2 $?
if [ "$fails" -eq 0 ]; then echo "all checks passed"; else echo "$fails check(s) FAILED"; exit 1; fi
