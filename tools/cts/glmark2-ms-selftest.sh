#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/glmark2-ms-selftest.sh
#
# Standalone self-test of tools/cts/glmark2-ms.py on synthetic glmark2 console
# excerpts (testdata/glmark2-ms/st-*.txt, handwritten from the sample's output
# format in driver/docs/glmark2.md; no board address, UART path or home path).
# Per case the script runs in both modes (table, --cells) and its exit code,
# stdout and stderr are compared byte for byte with st-<case>.expected
# (the format: "rc <n>", "-- stdout", the stdout, "-- stderr", the stderr).
# Cases: a full 33-scene run, a run with a failed and an unsupported scene, a
# run with GL error lines, a truncated run, and the Armadillo-X2 console form
# (header line, bare result line), which stands in for the i.MX8MP.
#
#   --update   rewrite the expected files from the script's output (review the diff)
#
# The x2 case checks that glmark2-ms.py reads an Armadillo-X2 console log: it
# joins a header-only line "[scene] opts:" with the bare " FPS: ..." line that
# follows, and exits 0 (st-x2.table.expected, st-x2.cells.expected: "rc 0"
# and the per-scene table / cells of the i.MX8MP column).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=$HERE/glmark2-ms.py
D=$HERE/testdata/glmark2-ms
update=0; [ "${1:-}" = "--update" ] && update=1
command -v python3 > /dev/null || { echo "glmark2-ms-selftest: python3 missing" >&2; exit 2; }
T=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-ms-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
fail=0

run() {  # run <mode flag or ""> <file> -> $T/out in the expected-file format
    local rc
    # shellcheck disable=SC2086
    python3 -I "$SCRIPT" $1 "$2" > "$T/so" 2> "$T/se"; rc=$?
    { echo "rc $rc"; echo "-- stdout"; cat "$T/so"; echo "-- stderr"; cat "$T/se"; } > "$T/out"
}

for c in full33 failed-scene glerror truncated x2; do
    for mode in table cells; do
        flag=""; [ "$mode" = cells ] && flag="--cells"
        exp=$D/st-$c.$mode.expected
        run "$flag" "$D/st-$c.txt"
        if [ "$update" = 1 ]; then cp "$T/out" "$exp"; echo "  updated $c $mode"; continue; fi
        if cmp -s "$T/out" "$exp"; then
            echo "  ok    $c ($mode): exit and output as expected"
        else
            echo "  FAIL  $c ($mode): differs from st-$c.$mode.expected"
            diff "$exp" "$T/out" | sed 's/^/        | /' | head -20
            fail=$((fail + 1))
        fi
    done
done
# The files carry no address, UART path or home path.
if grep -rEq '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|/home/|/Users/' "$D"/st-*; then
    echo "  FAIL  testdata st-*: an address, UART path or home path"; fail=$((fail + 1))
else
    echo "  ok    testdata st-*: no address, UART path or home path"
fi
[ "$fail" -eq 0 ] || { echo "glmark2-ms-selftest: $fail failed"; exit 1; }
echo "glmark2-ms-selftest: all ok"
