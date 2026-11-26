#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-rows-tsv.py on synthetic console fixtures: a done row, a
# WDOG3 hang, a lifetime-over row and a row with no console after `go`, plus
# the check mode's refusals (a missing row, a non-integer seconds).
# No board data, no addresses.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/imx95-rows-tsv.py"
tree="$here/mali-gles2-tree.txt"
t=$(mktemp -d "${TMPDIR:-.}/imx95-rows-tsv.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
log="$t/log"
bbm="$t/bbm"
mkdir -p "$log" "$bbm"

# --- fixtures: four captures of a session's rows T6S4R01..R04 -----------------

# T6S4R01: a done row, ended by the board layer's run-over reset.
cat > "$log/T6S4R01.log" <<'EOF'
verdin_imx95: WDOG3 armed: CS 0x00000000 -> 0x00003109 (was stopped), TOVAL 0x0000ffff -> 0x0000ea60, timeout 60000 ms, polls 1/2 in 3 us; fed until 600 s of uptime
vi95_deqp: build vi95-selftest
deqp: case filter 'dEQP-GLES2.functional.shaders.builtin_variable.*'
deqp: summary: 119 cases begun, 119 with a result, 0 without
deqp: done: 96 passed, 23 failed, 0 not supported, 0 warnings, complete=1
shaderc: keep high-water 1234567 of 8388608 bytes (0 held now)
deqp: OK (rc=0)
deqp: PASS
yk-record: a0000001 00000077 00000000 00000000
verdin_imx95: board: run over: PSCI SYSTEM_RESET at 42123 ms
EOF
printf 'a0000001 00000077 00000000 00000000\n' > "$bbm/T6S4R01"

# T6S4R02: a case stopped making progress; WDOG3 feeding stopped (no progress).
cat > "$log/T6S4R02.log" <<'EOF'
verdin_imx95: WDOG3 armed: CS 0x00000000 -> 0x00003109 (was stopped), TOVAL 0x0000ffff -> 0x0000ea60, timeout 60000 ms, polls 1/2 in 3 us; fed until 600 s of uptime
vi95_deqp: build vi95-selftest
deqp: case filter 'dEQP-GLES2.functional.shaders.loop.*'
verdin_imx95: WDOG3 feeding stopped at 245000 ms after 245 feeds (no progress): it resets the board within 60000 ms
EOF
printf '00000000 00000000 00000000 00000000\n' > "$bbm/T6S4R02"

# T6S4R03: the lifetime ended (feeding stopped, then the lifetime reset). The
# file name carries no label: --map names it.
cat > "$t/r03-capture.txt" <<'EOF'
verdin_imx95: WDOG3 armed: CS 0x00000000 -> 0x00003109 (was stopped), TOVAL 0x0000ffff -> 0x0000ea60, timeout 60000 ms, polls 1/2 in 3 us; fed until 600 s of uptime
vi95_deqp: build vi95-selftest
deqp: case filter 'dEQP-GLES2.functional.shaders.operator.*'
deqp: summary: 378 cases begun, 361 with a result, 17 without
verdin_imx95: WDOG3 feeding stopped at 600002 ms after 600 feeds (lifetime over): it resets the board within 60000 ms
verdin_imx95: lifetime over: PSCI SYSTEM_RESET at 600003 ms
EOF
printf '00000001 00000002 00000003 00000004\n' > "$bbm/T6S4R03.txt"

# T6S4R04: nothing on the console after `go'. No read-back file: not-read.
: > "$log/T6S4R04.log"

# --- build --------------------------------------------------------------------

python3 -I "$tool" --tree "$tree" --out "$t/rows.tsv" \
    --map "T6S4R03=$t/r03-capture.txt" --seconds T6S4R04=95 --lifetime-s 600 \
    --bbm-dir "$bbm" "$log" > "$t/build.out" || { echo "build failed"; exit 1; }
grep -F -q '4 rows' "$t/build.out" || { echo "build summary"; exit 1; }

row_is() { # <label> <column> <expected>
    got=$(awk -F '\t' -v l="$1" -v c="$2" \
        'BEGIN { split("row label cases status seconds lifetime_s bbm exit_line", h, " ")
                 for (i = 1; i <= 8; i++) if (h[i] == c) ci = i }
         NR > 1 && $2 == l { print $ci }' "$t/rows.tsv")
    [ "$got" = "$3" ] || { echo "$1 $2: got '$got', want '$3'"; exit 1; }
}
head -1 "$t/rows.tsv" | grep -qx \
    'row	label	cases	status	seconds	lifetime_s	bbm	exit_line' || { echo header; exit 1; }
[ "$(awk 'END { print NR }' "$t/rows.tsv")" -eq 5 ] || { echo "row count"; exit 1; }
row_is T6S4R01 row 1
row_is T6S4R02 row 2
row_is T6S4R03 row 3
row_is T6S4R04 row 4
# done row: 42 s (the run-over at 42123 ms), cases from the summary, match
row_is T6S4R01 status done
row_is T6S4R01 cases 119
row_is T6S4R01 seconds 42
row_is T6S4R01 lifetime_s 600
row_is T6S4R01 bbm match
awk -F '\t' '$2 == "T6S4R01" && $8 !~ /run over: PSCI SYSTEM_RESET at 42123 ms/ { exit 1 }' \
    "$t/rows.tsv" || { echo "exit_line R01"; exit 1; }
# WDOG3 hang: seconds from the feeding-stopped time, no summary, all-zero read
row_is T6S4R02 status hang
row_is T6S4R02 cases -
row_is T6S4R02 seconds 245
row_is T6S4R02 lifetime_s 600
row_is T6S4R02 bbm none
awk -F '\t' '$2 == "T6S4R02" && $8 !~ /feeding stopped at 245000 ms/ { exit 1 }' \
    "$t/rows.tsv" || { echo "exit_line R02"; exit 1; }
# lifetime row (label from --map): 600 s, differs (no console record, non-zero read)
row_is T6S4R03 status lifetime
row_is T6S4R03 cases 378
row_is T6S4R03 seconds 600
row_is T6S4R03 lifetime_s 600
row_is T6S4R03 bbm differs
# no console after go: noboot, host-timed seconds, no read-back
row_is T6S4R04 status noboot
row_is T6S4R04 cases -
row_is T6S4R04 seconds 95
row_is T6S4R04 lifetime_s 600
row_is T6S4R04 bbm not-read
awk -F '\t' '$2 == "T6S4R04" && $8 != "-" { exit 1 }' "$t/rows.tsv" || { echo "exit_line R04"; exit 1; }
# the *.tsv.license sidecar, as the T6 slices keep
grep -F -q 'SPDX-License-Identifier: Apache-2.0' "$t/rows.tsv.license" || { echo sidecar; exit 1; }

# no console lifetime and no --lifetime-s: refused, never guessed
if python3 -I "$tool" --tree "$tree" --out "$t/rows2.tsv" \
        --bbm-dir "$bbm" "$log" >/dev/null 2>&1; then
    echo "lifetime invented"; exit 1
fi

# --- check --------------------------------------------------------------------

mkdir -p "$t/slice"
cp "$t/rows.tsv" "$t/slice/rows.tsv"
cp "$t/rows.tsv.license" "$t/slice/rows.tsv.license"
python3 -I "$tool" --tree "$tree" --check "$t/slice" > "$t/check.out" ||
    { echo "check failed a good table"; cat "$t/check.out"; exit 1; }
grep -F -q 'OK:' "$t/check.out" || { echo "check summary"; exit 1; }

# a missing row: the range T6S4R01..R04 no longer covers R03
grep -v T6S4R03 "$t/rows.tsv" > "$t/slice/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1; then
    echo "missing row accepted"; exit 1
fi
# the same row twice
{ cat "$t/rows.tsv"; awk -F '\t' '$2 == "T6S4R04"' "$t/rows.tsv"; } > "$t/slice/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1; then
    echo "duplicated row accepted"; exit 1
fi
# a non-integer seconds
awk -F '\t' 'BEGIN { OFS = "\t" } $2 == "T6S4R01" { $5 = "x" } { print }' \
    "$t/rows.tsv" > "$t/slice/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1; then
    echo "non-integer seconds accepted"; exit 1
fi
# a non-integer lifetime_s
awk -F '\t' 'BEGIN { OFS = "\t" } $2 == "T6S4R01" { $6 = "" } { print }' \
    "$t/rows.tsv" > "$t/slice/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1; then
    echo "empty lifetime_s accepted"; exit 1
fi
# a header that is not the header
tail -n +2 "$t/rows.tsv" > "$t/slice/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1; then
    echo "missing header accepted"; exit 1
fi
# a directory without a rows.tsv
if python3 -I "$tool" --tree "$tree" --check "$bbm" >/dev/null 2>&1; then
    echo "missing rows.tsv accepted"; exit 1
fi
# a label that is no '# image' row
{ head -1 "$t/rows.tsv"; awk -F '\t' 'BEGIN { OFS = "\t" } $2 == "T6S4R01" { $2 = "T6S9R99" } { print }' \
    "$t/rows.tsv" | tail -n +2; } > "$t/slice/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1; then
    echo "unknown label accepted"; exit 1
fi

# --- span guard ---------------------------------------------------------------
# A truncated table is still contiguous; --expect (or the session tag of the
# directory name) ties it to its span.
cp "$t/rows.tsv" "$t/slice/rows.tsv"
python3 -I "$tool" --tree "$tree" --expect T6S4R01-T6S4R04 --check "$t/slice" >/dev/null ||
    { echo "--expect refused a full span"; exit 1; }
sed '$d' "$t/rows.tsv" > "$t/slice/rows.tsv"
python3 -I "$tool" --tree "$tree" --check "$t/slice" >/dev/null 2>&1 ||
    { echo "contiguity alone should pass a short table"; exit 1; }
if python3 -I "$tool" --tree "$tree" --expect T6S4R01-T6S4R04 --check "$t/slice" >/dev/null 2>&1; then
    echo "truncated table accepted by --expect"; exit 1
fi
# split session S1b: T6S1R13-R24 with session-wide row numbers
s1b="$t/2026-11-24-imx95-gles2-S1b-abcdef01"
mkdir -p "$s1b"
head -1 "$t/rows.tsv" > "$s1b/rows.tsv"
grep -E '^# image T6S1R(1[3-9]|2[0-4]):' "$tree" | sed -e 's/^# image //' -e 's/:.*//' |
    awk '{ printf "%d\t%s\t1\tdone\t40\t360\tmatch\t-\n", substr($0, 6) + 0, $0 }' >> "$s1b/rows.tsv"
[ "$(awk 'END { print NR }' "$s1b/rows.tsv")" -eq 13 ] || { echo "S1b fixture"; exit 1; }
cp "$s1b/rows.tsv" "$t/s1b.full"
python3 -I "$tool" --tree "$tree" --check "$s1b" >/dev/null ||
    { echo "correct S1b table refused"; exit 1; }
sed '$d' "$t/s1b.full" > "$s1b/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$s1b" >/dev/null 2>&1; then
    echo "S1b without T6S1R24 accepted"; exit 1
fi
{ head -1 "$t/s1b.full"; tail -n +3 "$t/s1b.full"; } > "$s1b/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$s1b" >/dev/null 2>&1; then
    echo "S1b without T6S1R13 accepted"; exit 1
fi
{ cat "$t/s1b.full"; tail -n 1 "$t/s1b.full"; } > "$s1b/rows.tsv"
if python3 -I "$tool" --tree "$tree" --check "$s1b" >/dev/null 2>&1; then
    echo "S1b with a duplicate row accepted"; exit 1
fi
# --whole: a part of the tree, and the same rows in two directories
cp "$t/s1b.full" "$s1b/rows.tsv"
s1b2="$t/2026-11-24-imx95-gles2-S1b-abcdef02"
mkdir -p "$s1b2"; cp "$t/s1b.full" "$s1b2/rows.tsv"
if python3 -I "$tool" --tree "$tree" --whole --check "$s1b" >/dev/null 2>&1; then
    echo "--whole accepted a part of the tree"; exit 1
fi
if python3 -I "$tool" --tree "$tree" --whole --check "$s1b" "$s1b2" >/dev/null 2>&1; then
    echo "--whole accepted a row in two directories"; exit 1
fi
echo ok
