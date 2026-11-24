#!/bin/sh
# Selftest of tools/cts/am62p-gltest-es3-verdicts.py: four synthetic redacted logs (one clean
# run, one with a reset mid-case, one with a case missing, one failing reading) written under
# a mktemp dir in $TMPDIR, the script's lines and exit codes checked. No board needed.
# usage: sh tools/cts/am62p-gltest-es3-verdicts-selftest.sh
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/am62p-gltest-es3-verdicts.py"
d=$(mktemp -d "${TMPDIR:-/tmp}/gltest-es3-verdicts.XXXXXX") || exit 2
trap 'rm -rf "$d"' EXIT
fail=0

for k in clean reset missing bad; do
    python3 "$tool" --fixture "$k" > "$d/$k.log" || { echo "FAIL: no fixture $k"; exit 1; }
done

# run NAME WANT_RC PATTERN...: the tool on NAME.log must exit WANT_RC and print each pattern
run() {
    name=$1 want=$2
    shift 2
    python3 "$tool" "$d/$name.log" > "$d/$name.out"
    rc=$?
    if [ "$rc" != "$want" ]; then
        echo "FAIL: $name exited $rc, want $want"; cat "$d/$name.out"; fail=1
    fi
    for p in "$@"; do
        grep -q -- "$p" "$d/$name.out" || { echo "FAIL: $name lacks '$p'"; cat "$d/$name.out"; fail=1; }
    done
}

run clean 0 'case 196 occl PASS verdict=done resets=0' 'case 202 blitconv PASS' '12 of 12' \
    'case 205 mappbo PASS verdict=done resets=0' '15 of 15'
n=$(grep -c '^case ' "$d/clean.out")
[ "$n" = 10 ] || { echo "FAIL: clean prints $n case lines, want 10"; fail=1; }
run reset 1 'case 203 tfcap FAIL verdict=reset resets=1'
run missing 1 'case 201 usamp MISSING verdict=none'
run bad 1 'case 204 srgb FAIL verdict=done' '7 of 8'

python3 "$tool" "$d/does-not-exist.log" > /dev/null 2>&1
[ $? = 2 ] || { echo "FAIL: an unreadable log must exit 2"; fail=1; }
python3 "$tool" --selftest > /dev/null || { echo "FAIL: --selftest"; fail=1; }

if [ "$fail" = 0 ]; then echo "am62p-gltest-es3-verdicts-selftest: ok"; else exit 1; fi
