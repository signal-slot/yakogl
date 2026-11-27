#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/common-tally-selftest.sh [--update]
#
# Standalone self-test of tools/cts/common-tally.py on small run directories
# (testdata/common-tally/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/, handwritten,
# redacted: no board address, UART path or home path). The cases:
#   x2     an Armadillo-X2 record: three record words (snvs.txt, the devmem
#          record; the describe line has no "(pass)" after the status number),
#          pictures judged against the imx8mp column of glmark2-pictures.txt
#   pi4    the Raspberry Pi 4 (tryboot) and am62p (kexec-dev): four words
# For each case the exit code and output (the "build" line, which names the
# checkout, left out) are compared with testdata/common-tally/expected/<case>.out.
# Needs yakogl-tools (YAKOGL_TOOLS, else cargo builds it), as common-tally.py does.
#
#   --update   rewrite the expected files from the output (review the diff)
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=$HERE/common-tally.py
D=$HERE/testdata/common-tally
update=0; [ "${1:-}" = "--update" ] && update=1
command -v python3 > /dev/null || { echo "common-tally-selftest: python3 missing" >&2; exit 2; }
# the run directory of a board's fixture, relative to testdata/ so the output is the same anywhere
run_of() { local d; d=$(cd "$D" && ls -d "$1"/2*-*-*) || return 1; echo "testdata/common-tally/$d"; }
fail=0

check() {  # check <case> <args...>: run common-tally.py from tools/cts, compare with expected/<case>.out
    local c=$1 rc out exp=$D/expected/$1.out
    shift
    out=$(cd "$HERE" && python3 -I "$SCRIPT" "$@" 2>&1); rc=$?
    out=$(printf 'rc %s\n%s\n' "$rc" "$out" | grep -v '^common-tally: build ')
    if [ "$update" = 1 ]; then mkdir -p "$D/expected"; printf '%s\n' "$out" > "$exp"; echo "  updated $c"; return; fi
    if [ "$out" = "$(cat "$exp" 2>/dev/null)" ]; then
        echo "  ok    $c"
    else
        echo "  FAIL  $c: differs from expected/$c.out"
        diff "$exp" <(printf '%s\n' "$out") | sed 's/^/        | /' | head -20
        fail=$((fail + 1))
    fi
}

X2=$(run_of x2) PI4=$(run_of pi4) AM=$(run_of am62p) || { echo "common-tally-selftest: a fixture is missing" >&2; exit 2; }
check x2-tally x2 --row "GM=$X2"
check x2-pictures --gm-pictures "$X2" imx8mp
check x2-pictures-as-x2 --gm-pictures "$X2" x2
check x2-picture-lines --gm-picture-lines "$X2" imx8mp
check pi4-tally pi4 --row "GM=$PI4"
check am62p-tally am62p --row "GM=$AM"

# the verdict for the imx8mp column, said in so many words
if [ "$update" != 1 ]; then
    if grep -q 'pictures against column imx8mp' "$D/expected/x2-tally.out" && grep -q 'verdict=done' "$D/expected/x2-tally.out"; then
        echo "  ok    x2 tally prints a verdict for the imx8mp column"
    else
        echo "  FAIL  x2 tally: no verdict for the imx8mp column"; fail=$((fail + 1))
    fi
fi
if grep -rEq '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|/home/|/Users/' "$D"; then
    echo "  FAIL  testdata/common-tally: an address, UART path or home path"; fail=$((fail + 1))
else
    echo "  ok    testdata/common-tally: no address, UART path or home path"
fi
[ "$fail" = 0 ] && echo "common-tally-selftest: ok" || echo "common-tally-selftest: $fail failed"
[ "$fail" = 0 ]
