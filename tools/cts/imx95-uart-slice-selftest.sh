#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/cts/imx95-uart-slice-selftest.sh
#
# Host-only check of tools/cts/imx95-uart-slice.py on a synthetic Verdin
# iMX95 session made here under $TMPDIR (no board data): five rows, one that
# ends in PSCI SYSTEM_RESET with a compile refusal and a draw refusal, one
# WDOG3 hang, one lifetime end, one without a console (never ran) and one
# dEQP fatal. The slice it writes must read in mali-deqp-tally.py
# (--check-cover, --tally --per-case), mali-family-tree.py and
# mali-fail-triage.py. rows.tsv is in tools/cts/imx95-rows-tsv.py's columns;
# a status the console contradicts is a warning; a console case of the row's
# filter that the mustpass list lacks goes to extra.tsv. An inconsistent session
# (more cases begun than the row has, a console case outside its row, two
# builds, a wrong header or vocabulary, a session name outside
# mali-deqp-tally.py's S1a|S1b|S2|S3a|S3b|S4) must exit 1 and write nothing. Exit 0 when every check holds.
set -u
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-.}/imx95-uart-slice.XXXXXX") || exit 2
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
P=dEQP-GLES2.functional
cat > "$t/mustpass.txt" <<EOF
$P.a.one
$P.a.two
$P.a.three
$P.b.four
$P.b.five
$P.b.six
$P.c.seven
$P.c.eight
$P.d.nine
$P.d.ten
$P.e.eleven
$P.f.x
$P.f.y
$P.f.z
EOF
{
	for r in a:1 b:2 c:3 d:4 e:5 f:6; do
		g=${r%:*} n=${r#*:}
		echo "# image VS9R0$n: $P.$g.*"
		echo "$P.$g.*"
	done
} > "$t/tree.txt"
S=$t/log/2026-11-25-imx95-gles2-S1a-00000000
mkdir -p "$S/console"
B="vi95_deqp: build v0.1.0-1-g0000000"
boot() {
	printf 'U-Boot 2024.04 (board console)\r\nNet:   eth0: ethernet@4ca0000 192.0.2.10\r\n'
	printf '=> go 0xd0000000\r\n*** Booting Zephyr OS build v4.3 ***\r\n%s\r\n' "$B"
}
{
	boot
	echo "deqp: case filter '$P.a.*'"
	echo "yakogl: glCompileShader(3): online shaderc: the code generator refused this fragment shader: Mali(Valhall: a widget operand is not lowered yet) [fragment stage, 120 bytes]"
	echo "deqp: Fail $P.a.one"
	echo "deqp: Pass $P.a.two"
	echo "yakogl-mali-refusal: -95 mali frame: a blend widget"
	echo "deqp: Fail $P.a.three"
	echo "deqp: done: 1 passed, 2 failed, 0 not supported, 0 warnings, complete=1"
	echo "deqp: summary: 3 cases begun, 3 with a result, 0 without"
	echo "osal(mali): kicks 9, completed 9"
	echo "deqp: FAILED (rc=-1)"
	echo "deqp: FAIL"
	echo "yk-record: v1 deqp rc=-1"
	echo "verdin_imx95: board: run over: PSCI SYSTEM_RESET at 5123 ms"
} > "$S/console/VS9R01.log"
{
	boot
	echo "12:00:01 deqp: Pass $P.b.four"
	echo "12:01:10 verdin_imx95: WDOG3 feeding stopped at 70000 ms after 69 feeds (no progress): it resets the board within 60000 ms"
} > "$S/console/VS9R02.log"
{
	boot
	echo "deqp: Pass $P.c.seven"
	echo "verdin_imx95: lifetime over: PSCI SYSTEM_RESET at 600001 ms"
} > "$S/console/VS9R03.log"
{
	boot
	echo "FATAL ERROR: out of arena"
	echo "deqp: fatal: dEQP ended the run (die()); later cases did not run"
	echo "verdin_imx95: board: run over: PSCI SYSTEM_RESET at 19000 ms"
} > "$S/console/VS9R05.log"
{
	boot
	echo "deqp: Pass $P.f.x"
	echo "deqp: Pass $P.f.z"
	echo "deqp: done: 2 passed, 0 failed, 0 not supported, 0 warnings, complete=1"
	echo "deqp: summary: 3 cases begun, 2 with a result, 1 without"
	echo "verdin_imx95: board: run over: PSCI SYSTEM_RESET at 4000 ms"
} > "$S/console/VS9R06.log"
# rows.tsv in tools/cts/imx95-rows-tsv.py's columns (cases: the summary's
# "cases begun" or -; row: the label's number within its session)
printf 'row\tlabel\tcases\tstatus\tseconds\tlifetime_s\tbbm\texit_line\n' > "$S/rows.tsv"
printf '1\tVS9R01\t3\tdone\t9\t600\tmatch\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 5123 ms\n' >> "$S/rows.tsv"
printf '2\tVS9R02\t-\thang\t131\t600\tnone\tverdin_imx95: WDOG3 feeding stopped at 70000 ms after ? feeds (no progress)\n' >> "$S/rows.tsv"
printf '3\tVS9R03\t-\tlifetime\t661\t600\tdiffers\tverdin_imx95: lifetime over: PSCI SYSTEM_RESET at 600001 ms\n' >> "$S/rows.tsv"
printf '4\tVS9R04\t-\tnoboot\t-\t600\tnot-read\t-\n' >> "$S/rows.tsv"
printf '5\tVS9R05\t-\tdone\t20\t600\tnone\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 19000 ms\n' >> "$S/rows.tsv"
printf '6\tVS9R06\t3\tdone\t8\t600\tnot-read\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 4000 ms\n' >> "$S/rows.tsv"
cp -R "$S" "$t/pristine"

run() { python3 -I "$here/imx95-uart-slice.py" --tree "$t/tree.txt" --mustpass "$t/mustpass.txt" "$@"; }
Z=0000000000000000000000000000000000000000
if run --commit $Z "$S" > "$t/out" 2> "$t/err"; then ok "converts the session"; else bad "converts the session (exit $?)"; cat "$t/err"; fi
has "summary line counts the cases" "$t/out" "6 rows, 14 cases"
R=$S/results.tsv
C=$S/chunks.tsv
has "results.tsv header" "$R" "$(printf 'case\tstatus\tdetail')"
has "a result line's status, detail -" "$R" "$(printf '%s.a.one\tFail\t-' $P)"
has "the case in progress at WDOG3 is Lost chunk-hang" "$R" "$(printf '%s.b.five\tLost\tchunk-hang' $P)"
has "the case after it is Lost not-run" "$R" "$(printf '%s.b.six\tLost\tnot-run' $P)"
has "the case in progress at the lifetime is Lost chunk-hang" "$R" "$(printf '%s.c.eight\tLost\tchunk-hang' $P)"
has "a row without a console is Lost not-run" "$R" "$(printf '%s.d.ten\tLost\tnot-run' $P)"
has "the case in progress at dEQP's fatal is Lost chunk-crash" "$R" "$(printf '%s.e.eleven\tLost\tchunk-crash' $P)"
has "a case the summary counts as begun without a result is Lost chunk-crash" "$R" "$(printf '%s.f.y\tLost\tchunk-crash' $P)"
has "a done row stays done around it" "$C" "$(printf '6\tVS9R06\t3\tdone\t')"
hasnt "no started case is counted as Fail" "$R" "$(printf '%s.b.five\tFail' $P)"
has "a stamped line still reads" "$R" "$(printf '%s.b.four\tPass' $P)"
has "chunks.tsv: the T6's columns" "$C" "$(printf 'row\tlabel\tcases\tstatus\tseconds\tkmsg\tresets\tssh_after\tboot_same\texit_line')"
has "chunks.tsv: a done row with its end line" "$C" "$(printf '1\tVS9R01\t3\tdone\t9\t-\t-\t-\t-\tverdin_imx95: board: run over: PSCI SYSTEM_RESET at 5123 ms')"
has "chunks.tsv: exit_line names the row's first refusal" "$C" "5123 ms first_refused=online shaderc: the code generator refused this fragment shader"
has "chunks.tsv: first_refused=- for a row without one" "$C" "lifetime over: PSCI SYSTEM_RESET at 600001 ms first_refused=-"
has "chunks.tsv: the WDOG3 row is hang" "$C" "$(printf '2\tVS9R02\t3\thang\t131\t')"
has "chunks.tsv: the lifetime row is hang" "$C" "$(printf '3\tVS9R03\t2\thang\t661\t')"
has "chunks.tsv: the row that never ran is lost" "$C" "$(printf '4\tVS9R04\t2\tlost\t0\t-\t-\t-\t-\t-')"
has "chunks.tsv: the fatal row is crash" "$C" "$(printf '5\tVS9R05\t1\tcrash\t20\t')"
has "refusals.tsv: the compile refusal, without the call and the stage tail" "$S/refusals.tsv" \
	"$(printf '%s.a.one\tonline shaderc: the code generator refused this fragment shader: Mali(Valhall: a widget operand is not lowered yet)' $P)"
hasnt "refusals.tsv: no stage tail" "$S/refusals.tsv" "fragment stage, 120 bytes"
has "refusals/VS9R01.txt: a Test case line before the case's lines" "$S/refusals/VS9R01.txt" "Test case '$P.a.three'.."
has "refusals/VS9R02.txt: the case in progress gets the tail" "$S/refusals/VS9R02.txt" "Test case '$P.b.five'.."
hasnt "refusals/*.txt: U-Boot's lines are left out" "$S/refusals/VS9R01.txt" "192.0.2.10"
[ -e "$S/refusals/VS9R04.txt" ] && bad "no refusal file for a row that never ran" || ok "no refusal file for a row that never ran"
has "meta.txt: os=zephyr" "$S/meta.txt" "os=zephyr"
has "meta.txt: the commit" "$S/meta.txt" "commit=$Z"
has "meta.txt: session from the name" "$S/meta.txt" "session=S1a"
has "meta.txt: rows" "$S/meta.txt" "rows=1-6"
has "meta.txt: the build" "$S/meta.txt" "lib=vi95_deqp v0.1.0-1-g0000000"
for f in meta.txt chunks.tsv results.tsv refusals.tsv refusals/VS9R01.txt; do
	[ -f "$S/$f.license" ] || bad "licence beside $f"
done
ok "a licence beside each file"

T=$here/mali-deqp-tally.py
if python3 -I "$T" --check-cover --no-git "$t/mustpass.txt" "$S" 2> "$t/err"; then ok "mali-deqp-tally.py --check-cover accepts the slice"
else bad "mali-deqp-tally.py --check-cover accepts the slice"; cat "$t/err"; fi
python3 -I "$T" --tally --per-case "$S" > "$t/tally" 2>&1 || bad "mali-deqp-tally.py --tally --per-case runs"
has "tally: the refusal is a Fail's cause" "$t/tally" "refused, fragment shader: Valhall: a widget operand is not lowered yet"
has "tally --per-case: the draw refusal is a Fail's cause" "$t/tally" "refused:mali frame: a blend widget"
has "tally: the hang cases" "$t/tally" "chunk-hang"
python3 -I "$T" --tally --markdown "$S" > "$t/md" 2>&1 && ok "mali-deqp-tally.py --tally --markdown runs" || bad "mali-deqp-tally.py --tally --markdown runs"
printf 'widget\tValhall: a widget operand\n' > "$t/families.txt"
if python3 -I "$here/mali-family-tree.py" --mustpass "$t/mustpass.txt" --families "$t/families.txt" "$S" > "$t/family" 2> "$t/err"; then
	ok "mali-family-tree.py runs on the slice"
else bad "mali-family-tree.py runs on the slice"; cat "$t/err"; fi
has "family tree: the refused case is in its family's row" "$t/family" "$P.a.one"
if python3 -I "$here/mali-fail-triage.py" "$S" > "$t/triage" 2> "$t/err"; then ok "mali-fail-triage.py runs on the slice"
else bad "mali-fail-triage.py runs on the slice"; cat "$t/err"; fi
has "triage: the WDOG3 row's case is a hang cause" "$t/triage" "hang: WDOG3 feeding stopped"
has "triage: the lifetime row's case is a hang cause" "$t/triage" "hang: lifetime over"
has "triage: the row's refusal from exit_line's first_refused=" "$t/triage" "Valhall: a widget operand is not lowered yet"
hasnt "triage: no refused case left unexplained" "$t/triage" "unexplained Fail"

# meta.txt present: kept
echo "commit=$Z" > "$t/keep"
cat "$S/meta.txt" > "$t/meta0"
run "$S" > "$t/out2" 2> "$t/err" && has "a second run keeps meta.txt" "$t/out2" "meta.txt kept" || bad "a second run"
cmp -s "$S/meta.txt" "$t/meta0" && ok "meta.txt unchanged" || bad "meta.txt unchanged"

# status mismatch: a warning, rows.tsv wins
E=$t/log/2026-11-25-imx95-gles2-S1b-00000000
cp -R "$t/pristine" "$E"
sed 's/\thang\t/\tlifetime\t/' "$t/pristine/rows.tsv" > "$E/rows.tsv"
run --commit $Z "$E" > /dev/null 2> "$t/err" && has "a status the console contradicts is a warning" "$t/err" "rows.tsv says status lifetime, the console says hang" \
	|| bad "a status the console contradicts still converts"
has "the slice keeps rows.tsv's status (hang)" "$E/chunks.tsv" "$(printf '2\tVS9R02\t3\thang\t131\t')"

# a case of the row's filter that the mustpass list lacks: extra.tsv, not results.tsv
E=$t/log/2026-11-25-imx95-gles2-S3a-00000000
cp -R "$t/pristine" "$E"
grep -v -F "$P.a.two" "$t/mustpass.txt" > "$t/mustpass-short.txt"
python3 -I "$here/imx95-uart-slice.py" --tree "$t/tree.txt" --mustpass "$t/mustpass-short.txt" --commit $Z "$E" > "$t/outx" 2> "$t/err"
rc=$?
if [ "$rc" -eq 0 ]; then ok "a console case outside the mustpass list still converts"; else bad "extra case: exit $rc"; cat "$t/err"; fi
has "the extra case is a warning" "$t/err" "1 case(s) of the row's filter are not in the mustpass list"
has "the summary counts it" "$t/outx" "1 extra case(s) outside the mustpass list in extra.tsv"
has "extra.tsv names it with its status" "$E/extra.tsv" "$(printf 'VS9R01\t%s.a.two\tPass' $P)"
hasnt "results.tsv leaves it out" "$E/results.tsv" "$P.a.two"
has "the row's case count is the mustpass one" "$E/chunks.tsv" "$(printf '1\tVS9R01\t2\tdone\t')"
has "its console lines stay, under an extra-case line" "$E/refusals/VS9R01.txt" "# extra case (not in the mustpass list): $P.a.two"
if [ ! -e "$S/extra.tsv" ]; then ok "no extra.tsv without an extra case"; else bad "no extra.tsv without an extra case"; fi
if python3 -I "$here/mali-deqp-tally.py" --tally --per-case "$E" > "$t/outx" 2> "$t/err"; then ok "tally reads the slice with extra.tsv"; else bad "tally reads the slice with extra.tsv"; cat "$t/err"; fi

# inconsistent sessions: exit 1, nothing written
inconsistent() { # label dir
	run --commit $Z "$2" > /dev/null 2> "$t/err"
	rc=$?
	if [ "$rc" -eq 1 ] && [ ! -e "$2/results.tsv" ] && [ ! -e "$2/meta.txt" ]; then ok "$1: exit 1, nothing written"
	else bad "$1: exit $rc"; cat "$t/err"; fi
}
X=$t/x1/2026-11-25-imx95-gles2-S2-00000000
mkdir -p "$t/x1" && cp -R "$t/pristine" "$X"
sed 's/^1\tVS9R01\t3\t/1\tVS9R01\t4\t/' "$t/pristine/rows.tsv" > "$X/rows.tsv"
inconsistent "more cases begun than the row has" "$X"
X=$t/x2/2026-11-25-imx95-gles2-S2-00000000
mkdir -p "$t/x2" && cp -R "$t/pristine" "$X"
echo "deqp: Pass $P.b.six" >> "$X/console/VS9R03.log"
inconsistent "a console case outside its row" "$X"
X=$t/x3/2026-11-25-imx95-gles2-S2-00000000
mkdir -p "$t/x3" && cp -R "$t/pristine" "$X"
sed 's/g0000000/g1111111/' "$t/pristine/console/VS9R03.log" > "$X/console/VS9R03.log"
inconsistent "two builds in one session" "$X"
X=$t/x4/2026-11-25-imx95-gles2-S2-00000000
mkdir -p "$t/x4" && cp -R "$t/pristine" "$X"
printf 'row\tlabel\tcases\tseconds\tlifetime_s\tend\tbbm\n' > "$X/rows.tsv"
inconsistent "a rows.tsv of the wrong header" "$X"
X=$t/x6/2026-11-25-imx95-gles2-S2-00000000
mkdir -p "$t/x6" && cp -R "$t/pristine" "$X"
sed 's/\tmatch\t/\tyes\t/' "$t/pristine/rows.tsv" > "$X/rows.tsv"
inconsistent "a bbm outside imx95-rows-tsv.py's vocabulary" "$X"
X=$t/x5/2026-11-25-imx95-gles2-S2-12345678
mkdir -p "$t/x5" && cp -R "$t/pristine" "$X"
inconsistent "a commit that is not the session name's" "$X"
X=$t/x7/2026-11-25-imx95-gles2-S9-00000000
mkdir -p "$t/x7" && cp -R "$t/pristine" "$X"
inconsistent "a session outside mali-deqp-tally.py's (S9)" "$X"
run > /dev/null 2>&1; [ $? -eq 2 ] && ok "no session: usage, exit 2" || bad "no session: usage, exit 2"

if [ "$fails" -ne 0 ]; then
	echo "imx95-uart-slice-selftest: $fails check(s) FAILED" >&2
	exit 1
fi
echo "imx95-uart-slice-selftest: all checks passed"
