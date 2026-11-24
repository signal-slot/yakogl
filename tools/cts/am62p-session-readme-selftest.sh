#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/am62p-session-readme-selftest.sh
#
# Hermetic check of tools/cts/am62p-session-readme.py on a fake AM62P record
# built under mktemp -d in $TMPDIR: board.sh's tree.txt, three run
# directories (row 2 run twice, the last one done; row 3 done_with_resets) and
# a gl_test ES 3.0 run with cases 196-205. The README has the commit line, the
# table and its sha256, the bounds, the native-3D arm, env names as set/unset
# only, per-row verdicts and resets, check-tree-log.py's row lines and the
# gl_test cases. A record line with an IPv4-like string, a UART path or a home
# path, and such an argument, are refused (exit 1, nothing on stdout).
set -u
unset LANGUAGE LC_ALL
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
PY=$HERE/am62p-session-readme.py
SHA=0123456789abcdef0123456789abcdef01234567
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

T=$(mktemp -d "${TMPDIR:-/tmp}/am62p-session-readme-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
R=$T/20261124-1200-gles3-rows0-3-01234567
mkdir -p "$R"
TABLE=$T/table.txt
printf '# image g01:\ndEQP-GLES3.info.*\n# image g02:\ndEQP-GLES3.a.*\n# image g03:\ndEQP-GLES3.b.*\n# image g04:\ndEQP-GLES3.c.*\n' > "$TABLE"
printf 'g01\timg/g01/zephyr/zephyr.bin\t-\tdEQP-GLES3.info.*\ng02\timg/g02/zephyr/zephyr.bin\t-\tdEQP-GLES3.a.*\ng03\timg/g03/zephyr/zephyr.bin\t-\tdEQP-GLES3.b.*\ng04\timg/g04/zephyr/zephyr.bin\t-\tdEQP-GLES3.c.*\n' > "$R/tree.txt"
run() { # run DIR NAME VERDICT RESETS
    mkdir -p "$R/$1"
    printf '# tools/board.sh run record\nboard: am62p\nname: %s\nverdict: board.sh am62p %s rc=0 verdict=%s boot=uboot row boot_verdict=done result=record:DEQP resets=%s record=1,2,3,4 back_after=40s\nrecords:\n' \
        "$2" "$2" "$3" "$4" > "$R/$1/INDEX.txt"
    printf 'dEQP Core vulkan-cts-<3.2.0-0>-555-gabcdef0 starting..\n' > "$R/$1/console.txt"
}
run 20261124-1201-g01-aaaaaaaa g01 done 0
run 20261124-1210-g02-bbbbbbbb g02 failed absent
run 20261124-1230-g02-bbbbbbbb g02 done 0
run 20261124-1240-g03-cccccccc g03 done_with_resets 2
run 20261124-1300-gltest-dddddddd gltest done 0
for n in 196 197 198 199 200 201 202 203 204; do
    r=PASS; [ $n = 199 ] && r=FAIL
    printf 'gl_test: case %d es3_case%d: %s\n' $n $n $r >> "$R/20261124-1300-gltest-dddddddd/console.txt"
done

out=$(env -u BOARD_UART BOARD_HOST=x python3 -I "$PY" --commit "$SHA" --table "$TABLE" \
    --tree-rows 0-2 --run-bound 900 --return-bound 240 --native-3d 1 "$R" 2>&1)
rc=$?
[ $rc -eq 0 ] && ok "README written" || bad "rc=$rc: $out"
has() { printf '%s\n' "$out" | grep -qF -- "$1" && ok "$2" || bad "$2 (missing: $1)"; }
printf '%s\n' "$out" | grep -qx "commit: $SHA" && ok "commit line" || bad "commit line"
has "sha256 $(sha256sum "$TABLE" | cut -d' ' -f1)" "table sha256"
has "- TREE_ROWS: 0-2 " "TREE_ROWS"
has "- RUN_BOUND: 900" "RUN_BOUND"
has "- RETURN_BOUND: 240" "RETURN_BOUND"
has "- DEQP_PVR_NATIVE_3D: 1" "native-3D arm"
has "BOARD_HOST set" "a set env name"
has "BOARD_UART unset" "an unset env name"
printf '%s\n' "$out" | grep -q '=x\b\|BOARD_HOST x' && bad "an env value is printed" || ok "no env value"
has "| 01 | g01 | done | 0 | 1 | \`20261124-1201-g01-aaaaaaaa\` |" "row 1"
has "| 02 | g02 | done | 0 | 2 | \`20261124-1230-g02-bbbbbbbb\` |" "row 2: the last of two runs"
has "| 03 | g03 | done_with_resets | 2 | 1 |" "row 3 with resets"
printf '%s\n' "$out" | grep -q '| 04 | g04' && bad "a row not run is listed" || ok "a row not run is left out"
has "row 03: board.sh am62p g03 rc=0 verdict=done_with_resets" "check-tree-log row line"
has "| 199 | es3_case199 | FAIL |" "gl_test case FAIL"
has "| 205 | - | no result | - |" "gl_test case without a result"

# check-tree-log.py reads the same row lines (it needs its own dir name and all rows)
S=$T/2026-11-24-am62p-gles3-01234567-rows01-03
mkdir "$S"
printf '%s\n' "$out" | sed "s/^commit: .*/commit: ${SHA:0:8}/" > "$S/README.md"
: > "$S/cases.tsv"
python3 -I "$HERE/check-tree-log.py" --rows 3 "$S" > "$T/o" 2>&1
grep -q 'row 3 .*resets is 2, not 0' "$T/o" && ! grep -q 'no verdict line' "$T/o" \
    && ok "check-tree-log.py reads the row lines" || bad "check-tree-log.py: $(cat "$T/o")"

refused() { # refused WHAT ARGS...: exit 1, nothing on stdout, a reason on stderr
    local what=$1; shift
    python3 -I "$PY" --commit "$SHA" --table "$TABLE" "$@" "$R" > "$T/out" 2> "$T/err"
    local r=$?
    if [ $r -eq 1 ] && [ ! -s "$T/out" ] && grep -q 'refused' "$T/err"; then ok "$what"; else bad "$what (rc=$r: $(cat "$T/err"))"; fi
}
# an IP-like string in a record line
IDX=$R/20261124-1201-g01-aaaaaaaa/INDEX.txt
cp "$IDX" "$T/idx"
printf 'board host: %s\n' "192.0.2.$((10 + 7))" >> "$IDX"
refused "an IP-like string in INDEX.txt is refused"
grep -q 'INDEX.txt:[0-9]*: looks like an IPv4' "$T/err" && ! grep -q '192\.0\.2' "$T/err" \
    && ok "the refusal names the line, not the address" || bad "refusal text: $(cat "$T/err")"
cp "$T/idx" "$IDX"
# a UART path in the gl_test console
GC=$R/20261124-1300-gltest-dddddddd/console.txt
cp "$GC" "$T/gc"
printf 'console on %s\n' "/dev/""ttyUSB0" >> "$GC"
refused "a UART path in a gl_test console is refused"
cp "$T/gc" "$GC"
# a home path in tree.txt
cp "$R/tree.txt" "$T/tree"
printf 'g05\t%s\t-\tx\n' "/home""/someone/img.bin" >> "$R/tree.txt"
refused "a home path in tree.txt is refused"
cp "$T/tree" "$R/tree.txt"
# a private argument
refused "an IP-like note is refused" --note "at 10.1.2.$((2 + 1))"
# back to clean
python3 -I "$PY" --commit "$SHA" --table "$TABLE" "$R" > /dev/null 2>&1 && ok "clean again" || bad "clean again"
python3 -I "$PY" --commit abc --table "$TABLE" "$R" > /dev/null 2>&1 && bad "a short sha is refused" || ok "a short sha is refused"

printf '%s\n' "$out" | grep -Eq '/hom[e]/|/dev/tt[y]|([0-9]{1,3}\.){3}[0-9]{1,3}' \
    && bad "README names a private detail" || ok "README names no home path, address or tty"

echo "am62p-session-readme selftest: $fail failed"
[ $fail -eq 0 ]
