#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of glmark2-results-row.py, standalone (bash, python3, no board).
# Fixtures in tools/cts/testdata/glmark2-results-row/ (made-up numbers):
#   verdicts-clean.txt   common-tally.py's GM (and GT) lines of a clean run
#   ms-clean.md          glmark2-ms.py's table of that run, 33 timed scenes
#   clean.expected       the cell the tool prints for them (with --prior)
#   README.fixture.md, RESULTS.md.in   a minimal README/RESULTS pair; the
#                        generated cell replaces @PI4_CELL@
# Checks: the clean cell; the --cells form; refusals (exit 2) for a failed
# scene, a missing scene time, a GL error, a record not at the commit and a
# missing input; the clean cell accepted by readme-glmark2-check.py
# --scenes terrain,shadow,refract (fixture paths, --ref none).
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/glmark2-results-row.py"
chk="$here/readme-glmark2-check.py"
fx="$here/testdata/glmark2-results-row"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/glmark2-results-row.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
nfail=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; nfail=$((nfail + 1)); }

common=(--board pi4 --commit 1234abcd --record 20261201-1015-glmark2-1234abcd)
run() { # <verdicts> <ms> [more args]: the tool's stdout in $tmp/out, stderr in $tmp/err, rc in $rc
    local v=$1 m=$2
    shift 2
    python3 -I "$tool" "${common[@]}" --verdicts "$v" --ms "$m" "$@" >"$tmp/out" 2>"$tmp/err"
    rc=$?
}
refuses() { # <what> <expected stderr text> <verdicts> <ms> [more args]
    local what=$1 want=$2
    shift 2
    run "$@"
    if [ "$rc" -eq 2 ] && [ ! -s "$tmp/out" ] && grep -qF -- "$want" "$tmp/err"; then
        ok "$what: exit 2 ($(head -n1 "$tmp/err"))"
    else
        bad "$what: exit $rc, stdout '$(cat "$tmp/out")', stderr '$(cat "$tmp/err")', want exit 2 with '$want'"
    fi
}

# 1. the clean run's cell
run "$fx/verdicts-clean.txt" "$fx/ms-clean.md" --prior 'terrain 150.00 ms at abcdef12'
if [ "$rc" -eq 0 ] && cmp -s "$tmp/out" "$fx/clean.expected"; then
    ok "clean run: the expected cell"
else
    bad "clean run: exit $rc; diff against clean.expected:"
    diff "$fx/clean.expected" "$tmp/out"
    cat "$tmp/err"
fi
cp "$tmp/out" "$tmp/cell"

# 2. the --cells form of glmark2-ms.py gives the same scene figures
printf 'terrain 120.50 ms (8 FPS), shadow 40.25 ms (24 FPS), refract 60.10 ms (16 FPS)\n' >"$tmp/cells.txt"
run "$fx/verdicts-clean.txt" "$tmp/cells.txt" --prior 'terrain 150.00 ms at abcdef12'
if [ "$rc" -eq 0 ] && cmp -s "$tmp/out" "$fx/clean.expected"; then
    ok "--cells input: the same cell"
else
    bad "--cells input: exit $rc, '$(cat "$tmp/out")' $(cat "$tmp/err")"
fi

# 3. refusals
sed 's/scenes ran 33, failed 0/scenes ran 32, failed 1/' "$fx/verdicts-clean.txt" >"$tmp/v-failed.txt"
refuses "failed scene" "failed 1" "$tmp/v-failed.txt" "$fx/ms-clean.md"
sed 's/^| \[refract\] <default> | 60.10 | 16 |$/| [refract] <default> | set up failed | - |/' \
    "$fx/ms-clean.md" >"$tmp/ms-norefract.md"
grep -q 'refract\] <default> | set up failed' "$tmp/ms-norefract.md" || bad "fixture edit: refract line not replaced"
refuses "missing scene time (table)" "[refract] <default> has no time" "$fx/verdicts-clean.txt" "$tmp/ms-norefract.md"
printf 'terrain 120.50 ms (8 FPS), refract 60.10 ms (16 FPS)\n' >"$tmp/cells-noshadow.txt"
refuses "missing scene time (cells)" "no ms a frame for shadow" "$fx/verdicts-clean.txt" "$tmp/cells-noshadow.txt"
sed '/^| \[loop\] fragment-steps=5:fragment-uniform=true/d' "$fx/ms-clean.md" >"$tmp/ms-32.md"
refuses "table of 32 scenes" "32 timed scenes" "$fx/verdicts-clean.txt" "$tmp/ms-32.md"
sed 's/with known-bad lines 0$/with known-bad lines 1\n    scene [terrain] <default>: amber, 17 lines (gl-error 16, incomplete-texture 1)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-glerr.txt"
refuses "GL error" "16 GL error lines" "$tmp/v-glerr.txt" "$fx/ms-clean.md"
sed 's/with known-bad lines 0$/with known-bad lines 1\n    scene [refract] <default>: amber, 4 lines (out-of-memory 1, tiled-copy 3)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-oom.txt"
refuses "out-of-memory scene" "1 scenes ran out of GPU memory" "$tmp/v-oom.txt" "$fx/ms-clean.md"
sed 's/scenes in the console 33, clean 33, with known-bad lines 0$/per-scene lines absent (no console)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-nocons.txt"
refuses "no console" "--allow-no-console" "$tmp/v-nocons.txt" "$fx/ms-clean.md"
run "$tmp/v-nocons.txt" "$fx/ms-clean.md" --allow-no-console
if [ "$rc" -eq 0 ] && grep -qF '0 unsupported, GL errors and out-of-memory scenes not read (no console);' "$tmp/out"; then
    ok "no console with --allow-no-console: the cell says so"
else
    bad "no console with --allow-no-console: exit $rc, '$(cat "$tmp/out")' $(cat "$tmp/err")"
fi
sed 's/verdict=done/verdict=failed/' "$fx/verdicts-clean.txt" >"$tmp/v-verdict.txt"
refuses "verdict failed" "verdict=failed" "$tmp/v-verdict.txt" "$fx/ms-clean.md"
refuses "missing --ms file" "--ms $tmp/none.md" "$fx/verdicts-clean.txt" "$tmp/none.md"
common=(--board pi4 --commit 1234abcd --record 20261201-1015-glmark2-abcdef12)
refuses "record not at the commit" "is not at 1234abcd" "$fx/verdicts-clean.txt" "$fx/ms-clean.md"
common=(--board pi4 --commit 1234abcd --record 20261201-1015-glmark2-1234abcd)
grep -v 'glmark2 pictures against' "$fx/verdicts-clean.txt" >"$tmp/v-nopics.txt"
refuses "no picture summary" "no picture summary" "$tmp/v-nopics.txt" "$fx/ms-clean.md"
python3 -I "$tool" --board pi4 --commit 1234abcd --verdicts "$fx/verdicts-clean.txt" \
    --ms "$fx/ms-clean.md" >/dev/null 2>&1
rc=$?
[ "$rc" -eq 2 ] && ok "missing --record: exit 2" || bad "missing --record: exit $rc, want 2"

# 4. the clean cell, put into the RESULTS fixture, passes readme-glmark2-check.py --scenes
python3 -I - "$fx/RESULTS.md.in" "$tmp/cell" "$tmp/RESULTS.md" <<'EOF'
import sys
tpl, cell, out = sys.argv[1:4]
text = open(tpl, encoding="utf-8").read()
cell = open(cell, encoding="utf-8").read().strip()
open(out, "w", encoding="utf-8").write(text.replace("| @PI4_CELL@ |", "| %s |" % cell))
EOF
grep -q '^| Pi 4 | 123, 33 scenes' "$tmp/RESULTS.md" || bad "the cell was not put into the RESULTS fixture"
if out=$(python3 -I "$chk" --readme "$fx/README.fixture.md" --results "$tmp/RESULTS.md" \
        --ref none --scenes terrain,shadow,refract 2>&1); then
    ok "readme-glmark2-check.py --scenes terrain,shadow,refract accepts the cell ($out)"
else
    bad "readme-glmark2-check.py rejected the cell: $out"
fi
# and the checker does read the Pi 4 cell: without its record directory it fails
sed 's#records/pi4/20261201-1015-glmark2-1234abcd/#records/pi4/#' "$tmp/RESULTS.md" >"$tmp/RESULTS-norec.md"
if python3 -I "$chk" --readme "$fx/README.fixture.md" --results "$tmp/RESULTS-norec.md" \
        --ref none --scenes terrain,shadow,refract >/dev/null 2>&1; then
    bad "readme-glmark2-check.py accepted a Pi 4 cell without its record directory"
else
    ok "readme-glmark2-check.py reads the Pi 4 cell (fails without its record directory)"
fi

if [ "$nfail" -ne 0 ]; then
    echo "glmark2-results-row-selftest: $nfail failed"
    exit 1
fi
echo "glmark2-results-row-selftest: all passed"
