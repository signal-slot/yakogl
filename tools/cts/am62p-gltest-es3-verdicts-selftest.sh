#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of tools/cts/am62p-gltest-es3-verdicts.py on synthetic, redacted AM62P serial-log
# snippets of the gl_test ES 3.0 image (cases 196-205), written under a mktemp dir in $TMPDIR:
# a clean run, a GPU reset inside case 203, a watchdog restart inside case 203, case 201
# missing, a failing srgb reading, a PASS line over a wrong occl reading, a skip by name, a
# rerun log after a reset, and an unreadable log. No board, no address, no device path.
# usage: sh tools/cts/am62p-gltest-es3-verdicts-selftest.sh
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/am62p-gltest-es3-verdicts.py"
d=$(mktemp -d "${TMPDIR:-/tmp}/am62p-gltest-es3-verdicts.XXXXXX") || exit 2
trap 'rm -rf "$d"' EXIT
fail=0
T='[00:00:41.120,000] <inf> yakogl: '

# chk N WHAT: N check lines of one case, all ok, numbered from 1
chk() {
    i=1
    while [ "$i" -le "$1" ]; do
        printf 'gl_test:   %-28s (%2d,%2d) = 0xff00ff00  want 0xff00ff00  ok  [check %d]\n' "$2 $i" "$i" "$i" "$i"
        i=$((i + 1))
    done
}

{
    printf '\033[0m*** Booting Zephyr OS build v4.2.0 ***\r\n'
    echo "${T}pvr: firmware booted, render service up"
    echo 'gl_test: panel: EGL 1.4, 1024x600 XRGB8888 window, 0 samples'
    for q in 'q0 hidden:0' 'q1 visible:1' 'q2 hidden conservative:0' 'q3 visible conservative:1' 'q4 white:1'; do
        echo "gl_test: occl: ${q%:*}: available 1 result ${q##*:} (want 1 ${q##*:})"
    done
    chk 2 'occl quad'
    echo 'gl_test: case 196 occl: PASS'
    for k in 0 1 2 3; do echo "gl_test: vtxgrid lod $k (level $k, 16x16): 0 of 4096 pixels wrong"; done
    echo 'gl_test: case 197 vtxgrid: PASS'
    echo 'gl_test: a2cconst picture 1: 0 of 4096 pixels hold green'
    echo 'gl_test: a2cconst picture 2: 0 of 4096 pixels hold green'
    echo 'gl_test: a2cconst: 0 of 4096 pixels differ between the two pictures'
    chk 2 'a2cconst cell'
    echo 'gl_test: case 198 a2cconst: PASS'
    chk 2 'trunc and shifts'; echo 'gl_test: case 199 es3trunc: PASS'
    chk 16 'rgba8ui'; echo 'gl_test: case 200 intrt: PASS'
    chk 8 'usampler2D'; echo 'gl_test: case 201 usamp: PASS'
    chk 12 '565 blit'; echo 'gl_test: case 202 blitconv: PASS'
    chk 8 'tfcap'
    echo 'gl_test:   tfcap C positions             12 floats  ok  [check 9]'
    echo 'gl_test: case 203 tfcap: PASS'
    chk 8 'srgb'; echo 'gl_test: case 204 srgb: PASS'
    chk 15 'mappbo'; echo 'gl_test: case 205 mappbo: PASS'
    echo 'gl_test: all: 0 of 10 cases failed (10 passed; 0 timed out; first failure case none)'
    echo "${T}render freelists: 0 grow requests, 0 served, 0 unserved, 0 KiB taken on demand; GPU recoveries 0 (HWR counter 0)"
} > "$d/clean.log"

# a GPU recovery inside case 203, which then still prints PASS
awk -v t="$T" '/case 202 blitconv: PASS/ { print; print t "render: HWR during fence 7 (counter 0 -> 1)";
    print t "FWCCB: CONTEXT_RESET ctxid=3 reason=1 dm=3 jobref=1 flags=0x0 PC=0x0 MMU=0x0"; next }
    { sub(/GPU recoveries 0 \(HWR counter 0\)/, "GPU recoveries 1 (HWR counter 1)"); print }' \
    "$d/clean.log" > "$d/reset.log"
# the watchdog restarts the image during case 203; the restarted image is not run again
awk '/\[check 3\]/ && seen { print "*** Booting Zephyr OS build v4.2.0 ***"; exit }
    /case 202 blitconv: PASS/ { seen = 1 } { print }' "$d/clean.log" > "$d/restart.log"
# case 201 never printed anything
awk '/usampler2D/ || /case 201 usamp/ { next } { print }' "$d/clean.log" > "$d/missing.log"
# srgb's last check reads wrong; the case FAILs
awk '/srgb 8 / { sub(/  ok  /, "  MISMATCH  ") } { sub(/case 204 srgb: PASS/, "case 204 srgb: FAIL"); print }' \
    "$d/clean.log" > "$d/bad.log"
# occl q1 answers 0 but the case line says PASS: the reading decides
sed 's/q1 visible: available 1 result 1/q1 visible: available 1 result 0/' "$d/clean.log" > "$d/wrongpass.log"
# an ES 2.0 context: case 205 skips by name, case 204 by the runner's walk
sed -e 's/case 205 mappbo: PASS/case 205 mappbo: SKIP (an ES 2.0 library has no glMapBufferRange nor pixel buffers)/' \
    -e 's/gl_test: case 204 srgb: PASS/gl_test: case 204 skipped: ES 3.00 source, outside the ES 2.0 compiler/' \
    "$d/clean.log" > "$d/skip.log"
# a second log reruns case 203 alone and passes: the earlier reset still counts
awk '/case 202 blitconv: PASS/ { on = 1; next } on && /tfcap|case 203|render freelists/ { print }' \
    "$d/clean.log" > "$d/rerun203.log"

# run NAME WANT_RC LOG... -- PATTERN...: the tool must exit WANT_RC and print each pattern
run() {
    name=$1 want=$2
    shift 2
    logs=
    while [ "$1" != -- ]; do logs="$logs $d/$1"; shift; done
    shift
    # shellcheck disable=SC2086
    python3 "$tool" $logs > "$d/$name.out" 2> "$d/$name.err"
    rc=$?
    if [ "$rc" != "$want" ]; then
        echo "FAIL: $name exited $rc, want $want"; cat "$d/$name.out" "$d/$name.err"; fail=1
    fi
    for p in "$@"; do
        grep -qF -- "$p" "$d/$name.out" || { echo "FAIL: $name lacks '$p'"; cat "$d/$name.out"; fail=1; }
    done
}

run clean 0 clean.log -- \
    'case 196 occl PASS verdict=done resets=0 occl q0-q4 available 1,1,1,1,1 results 0,1,0,1,1; checks 2/2 ok' \
    'case 197 vtxgrid PASS verdict=done resets=0 4 of 4 lods with 0 pixels wrong' \
    'case 198 a2cconst PASS verdict=done resets=0' \
    'case 199 es3trunc PASS verdict=done resets=0 checks 2/2 ok' \
    'case 200 intrt PASS verdict=done resets=0 checks 16/16 ok' \
    'case 201 usamp PASS verdict=done resets=0 checks 8/8 ok' \
    'case 202 blitconv PASS verdict=done resets=0 checks 12/12 ok' \
    'case 203 tfcap PASS verdict=done resets=0 checks 9/9 ok' \
    'case 204 srgb PASS verdict=done resets=0 checks 8/8 ok' \
    'case 205 mappbo PASS verdict=done resets=0 checks 15/15 ok' \
    '# run: log 1 of 1: GPU recoveries 0; gl_test: all: 0 of 10 cases failed'
n=$(grep -c '^case ' "$d/clean.out")
[ "$n" = 10 ] || { echo "FAIL: clean prints $n case lines, want 10"; fail=1; }
run reset 1 reset.log -- 'case 203 tfcap FAIL verdict=reset resets=1 verdict line PASS' \
    'case 202 blitconv PASS verdict=done resets=0' 'case 204 srgb PASS verdict=done resets=0' \
    'GPU recoveries 1'
run restart 1 restart.log -- 'case 202 blitconv PASS verdict=done resets=0' \
    'case 203 tfcap FAIL verdict=reset resets=1 no verdict line, the image restarted inside the case' \
    'case 204 srgb MISSING verdict=none resets=0' 'case 205 mappbo MISSING verdict=none'
run missing 1 missing.log -- 'case 201 usamp MISSING verdict=none resets=0' 'case 202 blitconv PASS'
run bad 1 bad.log -- "case 204 srgb FAIL verdict=done resets=0 verdict line FAIL; checks 7/8 ok, first MISMATCH check 8 'srgb 8 ( 8, 8) = 0xff00ff00 want 0xff00ff00'"
run wrongpass 1 wrongpass.log -- 'case 196 occl FAIL verdict=done resets=0 verdict line PASS but the reading is not the expected one: occl q0-q4 available 1,1,1,1,1 results 0,0,0,1,1'
run skip 1 skip.log -- 'case 205 mappbo SKIP verdict=done resets=0' 'case 204 srgb SKIP verdict=done' \
    'case 203 tfcap PASS'
run rerun 1 reset.log rerun203.log -- 'case 203 tfcap FAIL verdict=reset resets=1 verdict line PASS; checks 9/9 ok; 1 reset(s) in an earlier log' \
    'case 196 occl PASS verdict=done resets=0' '# run: log 2 of 2'
python3 "$tool" --cases 202,203 "$d/bad.log" > "$d/subset.out"
rc=$?
[ "$rc" = 0 ] && [ "$(grep -c '^case ' "$d/subset.out")" = 2 ] ||
    { echo "FAIL: --cases 202,203 on bad.log exited $rc"; cat "$d/subset.out"; fail=1; }

python3 "$tool" "$d/does-not-exist.log" > /dev/null 2>&1
[ $? = 2 ] || { echo "FAIL: an unreadable log must exit 2"; fail=1; }
python3 "$tool" "$d" > /dev/null 2>&1
[ $? = 2 ] || { echo "FAIL: a directory must exit 2"; fail=1; }
python3 "$tool" > /dev/null 2>&1
[ $? = 2 ] || { echo "FAIL: no log must exit 2"; fail=1; }
python3 "$tool" --cases x "$d/clean.log" > /dev/null 2>&1
[ $? = 2 ] || { echo "FAIL: a bad --cases must exit 2"; fail=1; }

if [ "$fail" = 0 ]; then echo "am62p-gltest-es3-verdicts-selftest: ok"; else exit 1; fi
