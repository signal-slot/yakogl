#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-dtpu-page-selftest.sh - selftest of imx8mp-dtpu-page.py on synthetic words: every outcome
# ((a), (a)+flush, (b), (c)), a check 3 failure (exit 1, "rerun"), the X2 11-11 case (check 4 at
# index 1), a console log, and malformed input (exit 2). Files go under $TMPDIR (never /tmp).
#
#   TMPDIR=<dir> sh tools/cts/imx8mp-dtpu-page-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory (synthetic files are never written to /tmp)}"
[ -d "$TMPDIR" ] || { echo "TMPDIR is not a directory" >&2; exit 2; }
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
T=$(mktemp -d "$TMPDIR/dtpu-page.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
TOOL="$HERE/imx8mp-dtpu-page.py"
fails=0 oks=0

# expect NAME RC PATTERN ARGS...
expect() {
    name=$1 rc=$2 pat=$3; shift 3
    out=$(python3 -I "$TOOL" "$@" 2>&1); got=$?
    if [ "$got" -ne "$rc" ]; then echo "FAIL: $name: rc $got, want $rc: $out" >&2; fails=$((fails + 1)); return; fi
    if ! printf '%s\n' "$out" | grep -q -- "$pat"; then echo "FAIL: $name: no '$pat' in: $out" >&2; fails=$((fails + 1)); return; fi
    echo "ok: $name"; oks=$((oks + 1))
}

# all right: w1 = valid | code 1; w2 = w3 = none/none
expect a            0 'outcome: (a)$'        00001001 10001000 10001000
# code 3: check 4 fails, 5 passes (X2 11-11 shape: check 4 at index 1)
expect x2-11-11     0 'check 4: FAIL.*index 1 (x=1 y=63)' 00003011 10000001 10001000
expect x2-11-11-out 0 'outcome: (a)$'        00003011 10000001 10001000
expect a-fewer      0 'verdict code 3'       0x3011 0x10000001 0x10001000
# check 7 fails with code 2 or on top of code 1
expect flush-code2  0 'outcome: (a)+flush$'  00002081 10001000 00051000
expect flush-on-1   0 'outcome: (a)+flush$'  00001081 10001000 00051000
# (b): checks 4 and 5 fail, 6 passes
expect b            0 'outcome: (b)$'        00004031 00050005 10001000
# (c): 4, 5, 6 fail
expect c            0 'outcome: (c)$'        00005071 00050005 10000007
expect gl-error     0 'GL error 0x0502'      05021101 10001000 10001000
# check 3 fails: not a verdict
expect check3       1 'not a verdict: check 3 failed, rerun' 00000009 10001000 10001000
# malformed words
expect not-hex      2 'not a hex word'       zz 0 0
expect two-words    2 'usage'                00001001 10001000
expect invalid      2 'valid bit'            00001000 10001000 10001000
expect bad-code     2 'not 0..5'             00009001 10001000 10001000
expect bad-index    2 'neither an index'     00001001 10011000 10001000
expect inconsistent 2 'check 3 passed but'   00000001 10001000 10001000
expect inconsistent2 2 'check 3 failed but'  00001009 10001000 10001000

# console logs
G=$T/good.log
{ echo "gc_wdog_arm: armed"; echo "gc_gl_test: case 222 dtpu page 0xb0"
  echo "yk-record: 594b0cb0 00004031 00050005 10001000"; echo "yk-record: 594b0c00 00000000 00000000 00000000"; } > "$G"
expect log-good     0 'outcome: (b)$'        --log "$G"
printf 'gc_gl_test: no dtpu page (case 222 did not reach its arms)\nyk-record: 594b0a01 1 2 3\n' > "$T/none.log"
expect log-nopage   2 'no dtpu page'         --log "$T/none.log"
printf 'yk-record: 594b0cb0 00004031 zzzz\nyk-record: 594b0cb0 1001\n' > "$T/mal.log"
expect log-malformed 2 "no 'yk-record: 594b0cb0"  --log "$T/mal.log"
expect log-missing  2 'No such file'         --log "$T/absent.log"

echo "$oks ok, $fails failed"
[ "$fails" -eq 0 ]
