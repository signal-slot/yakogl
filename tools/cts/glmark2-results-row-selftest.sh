#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of glmark2-results-row.py, standalone (bash, python3, no board).
# Fixtures in tools/cts/testdata/glmark2-results-row/ (made-up numbers):
#   verdicts-clean.txt   common-tally.py's GM (and GT) lines of a clean Pi 4 run
#   verdicts-imx8mp.txt  a console-less i.MX8MP GM row (glmark2-pages.txt form)
#   verdicts-pic.txt     an amber cell the pictures made, terrain and shadow
#                        mismatch lines named
#   ms-clean.md          glmark2-ms.py's table of the clean run, 33 timed scenes
#   cells-clean.txt      its --cells line for the same run
#   cells-imx8mp.txt     its --cells line for the i.MX8MP run
#   clean.expected, imx8mp.expected   the cells the tool prints for them
#   README.fixture.md, RESULTS.md.in  a minimal README/RESULTS pair; the
#                        generated Pi 4 cell replaces @PI4_CELL@
# Checks: the clean cell; the --cells input form; the console-less shapes with
# --allow-no-console (green (record and pages), green (record only)); a
# picture-amber cell the tool prints with terrain's and refract's states; the
# --pictures override; refusals (exit 2, message on stderr, nothing on stdout)
# for a failed scene, a missing scene time, a missing table scene, a GL error,
# an out-of-memory scene, an out-of-memory line outside any scene, another
# amber known-bad line, a verdict that is not done, two GM runs in one block,
# a score disagreement, a black picture, a picture summary that names too few
# scenes, a record not at the commit, a record of another board, a bad board
# or commit, --prior holding a '|', a missing picture summary and a missing
# input file; and that the clean cell put into the fixture RESULTS.md passes
# readme-glmark2-check.py --scenes terrain,shadow,refract (fixture paths,
# --ref none), while dropping its record directory makes the checker fail.
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

common=(--board pi4 --commit 1234abcd --record 20261201-1015-GM-1234abcd)
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

# 1. the clean run's cell (table input, --prior)
run "$fx/verdicts-clean.txt" "$fx/ms-clean.md" --prior 'terrain 150.00 ms at abcdef12'
if [ "$rc" -eq 0 ] && cmp -s "$tmp/out" "$fx/clean.expected"; then
    ok "clean run: the expected cell"
else
    bad "clean run: exit $rc; diff against clean.expected:"
    diff "$fx/clean.expected" "$tmp/out"
    cat "$tmp/err"
fi
[ "$(wc -l < "$tmp/out")" -eq 1 ] && ok "the cell is one line" || bad "the cell is not one line"
cp "$tmp/out" "$tmp/cell"

# 2. the --cells form of glmark2-ms.py gives the same scene figures
run "$fx/verdicts-clean.txt" "$fx/cells-clean.txt" --prior 'terrain 150.00 ms at abcdef12'
if [ "$rc" -eq 0 ] && cmp -s "$tmp/out" "$fx/clean.expected"; then
    ok "--cells input: the same cell"
else
    bad "--cells input: exit $rc, '$(cat "$tmp/out")' $(cat "$tmp/err")"
fi

# 3. the i.MX8MP shapes: common-tally.py's glmark2-pages.txt cell line, and
#    the record-only line, both console-less
common=(--board imx8mp --commit 1234abcd --record 20261201-1100-GM-1234abcd)
refuses "no console without --allow-no-console" "--allow-no-console" "$fx/verdicts-imx8mp.txt" "$fx/cells-imx8mp.txt"
run "$fx/verdicts-imx8mp.txt" "$fx/cells-imx8mp.txt" --allow-no-console
if [ "$rc" -eq 0 ] && cmp -s "$tmp/out" "$fx/imx8mp.expected"; then
    ok "green (record and pages) with --allow-no-console: the expected cell"
else
    bad "green (record and pages): exit $rc; diff against imx8mp.expected:"
    diff "$fx/imx8mp.expected" "$tmp/out"
    cat "$tmp/err"
fi
sed 's/glmark2 cell green (record and pages); scenes in glmark2-pages.txt 33/glmark2 cell green (record only); per-scene lines absent (no console)/' \
    "$fx/verdicts-imx8mp.txt" >"$tmp/v-only.txt"
run "$tmp/v-only.txt" "$fx/cells-imx8mp.txt" --allow-no-console
if [ "$rc" -eq 0 ] && grep -qF 'not read (no console; tally cell green (record only))' "$tmp/out"; then
    ok "green (record only) with --allow-no-console: accepted, the cell names the tally cell"
else
    bad "green (record only): exit $rc, '$(cat "$tmp/out")' $(cat "$tmp/err")"
fi
common=(--board pi4 --commit 1234abcd --record 20261201-1015-GM-1234abcd)

# 4. the pictures: an amber cell the pictures made, terrain and shadow named;
#    refract is not named, so it is a match (every not-match scene has a line)
run "$fx/verdicts-pic.txt" "$fx/ms-clean.md"
if [ "$rc" -eq 0 ] && grep -qF 'match 31, mismatch 2; terrain picture mismatch (probe c off 12), refract picture match' "$tmp/out"; then
    ok "picture-amber run: accepted, terrain mismatch named, refract derived as a match"
else
    bad "picture-amber run: exit $rc, '$(cat "$tmp/out")' $(cat "$tmp/err")"
fi
# the same summary counts but only one scene line named: the tool refuses
sed '/picture mismatch (mean/d' "$fx/verdicts-pic.txt" >"$tmp/v-pic-unnamed.txt"
refuses "un-named picture mismatch" "name every scene the summary does not count" "$tmp/v-pic-unnamed.txt" "$fx/ms-clean.md"
# a black picture scene refuses
sed 's/match 31, mismatch 2/match 32, black 1/' "$fx/verdicts-pic.txt" >"$tmp/v-pic-black.txt"
refuses "black picture" "black scene" "$tmp/v-pic-black.txt" "$fx/ms-clean.md"
# --pictures as an override, for a board judged outside common-tally.py
grep -v 'glmark2 pictures against\|picture mismatch' "$fx/verdicts-pic.txt" >"$tmp/v-nopics.txt"
run "$tmp/v-nopics.txt" "$fx/ms-clean.md" --pictures 'glmark2 pictures against column pi4: 33 scenes, match 33'
if [ "$rc" -eq 0 ] && grep -qF 'match 33; terrain picture match, refract picture match' "$tmp/out"; then
    ok "--pictures override: the cell cites it"
else
    bad "--pictures override: exit $rc, '$(cat "$tmp/out")' $(cat "$tmp/err")"
fi

# 5. refusals: the run's own counts and lines
sed 's/scenes ran 33, failed 0/scenes ran 32, failed 1/' "$fx/verdicts-clean.txt" >"$tmp/v-failed.txt"
refuses "failed scene" "want 33 ran, 0 failed" "$tmp/v-failed.txt" "$fx/ms-clean.md"
sed 's/scenes ran 33, failed 0, unsupported 0/scenes ran 33, failed 0, unsupported 1/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-unsup.txt"
refuses "unsupported scene" "unsupported 1; want 33 ran" "$tmp/v-unsup.txt" "$fx/ms-clean.md"
sed 's/^| \[refract\] <default> | 60.10 | 16 |$/| [refract] <default> | set up failed | - |/' \
    "$fx/ms-clean.md" >"$tmp/ms-norefract.md"
grep -q 'refract\] <default> | set up failed' "$tmp/ms-norefract.md" || bad "fixture edit: refract line not replaced"
refuses "missing scene time (table)" "[refract] <default> has no time" "$fx/verdicts-clean.txt" "$tmp/ms-norefract.md"
sed 's/terrain 120.50 ms (8 FPS), shadow 40.25 ms (24 FPS), refract/terrain 120.50 ms (8 FPS), refract/' \
    "$fx/cells-clean.txt" >"$tmp/cells-noshadow.txt"
refuses "missing scene time (cells)" "no ms a frame for shadow" "$fx/verdicts-clean.txt" "$tmp/cells-noshadow.txt"
sed '/^| \[loop\] fragment-steps=5:fragment-uniform=true:vertex-steps=5/d' "$fx/ms-clean.md" >"$tmp/ms-32.md"
refuses "table of 32 scenes" "32 timed scenes" "$fx/verdicts-clean.txt" "$tmp/ms-32.md"
sed 's/with known-bad lines 0$/with known-bad lines 1\n    scene [terrain] <default>: amber, 17 lines (gl-error 16, incomplete-texture 1)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-glerr.txt"
refuses "GL error" "16 GL error lines" "$tmp/v-glerr.txt" "$fx/ms-clean.md"
sed 's/with known-bad lines 0$/with known-bad lines 1\n    scene [refract] <default>: amber, 4 lines (out-of-memory 1, tiled-copy 3)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-oom.txt"
refuses "out-of-memory scene" "out-of-GPU-memory line (scene [refract] <default>)" "$tmp/v-oom.txt" "$fx/ms-clean.md"
sed 's/with known-bad lines 0$/with known-bad lines 1\n    outside any scene: 1 lines (out-of-memory 1)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-oom-out.txt"
refuses "out-of-memory line outside any scene" "outside any scene" "$tmp/v-oom-out.txt" "$fx/ms-clean.md"
sed 's/with known-bad lines 0$/with known-bad lines 1\n    scene [terrain] <default>: amber, 1 lines (incomplete-texture 1)/' \
    "$fx/verdicts-clean.txt" >"$tmp/v-amber.txt"
refuses "amber known-bad line" "known-bad lines that are neither GL errors" "$tmp/v-amber.txt" "$fx/ms-clean.md"
sed 's/verdict=done/verdict=failed/' "$fx/verdicts-clean.txt" >"$tmp/v-verdict.txt"
refuses "verdict failed" "verdict=failed, want rc=0 verdict=done" "$tmp/v-verdict.txt" "$fx/ms-clean.md"
# two GM runs in one block (a tally tree re-ran the row): which block is the cell about?
awk 'NR==1{print; print "GM#1 glmark2 rc=0 verdict=done result=record:GLMARK2 record=11111111,22222222,33333333,44444444; resets (record) 0; image=aaaaaaaaaaaaaaaa build=YAKOGL_SAMPLE_TREE=12345678"; next} {print}' \
    "$fx/verdicts-clean.txt" >"$tmp/v-two.txt"
refuses "two GM runs in --verdicts" "one run's block" "$tmp/v-two.txt" "$fx/ms-clean.md"
sed 's/glmark2 Score: 123/glmark2 Score: 124/' "$fx/ms-clean.md" >"$tmp/ms-score.md"
refuses "score disagreement" "the GM record's 123" "$fx/verdicts-clean.txt" "$tmp/ms-score.md"
grep -v 'glmark2 pictures against' "$fx/verdicts-clean.txt" >"$tmp/v-nopic.txt"
refuses "no picture summary" "no picture summary" "$tmp/v-nopic.txt" "$fx/ms-clean.md"

# 6. refusals: the arguments
refuses "missing --ms file" "--ms $tmp/none.md" "$fx/verdicts-clean.txt" "$tmp/none.md"
common=(--board pi4 --commit 1234abcd --record 20261201-1015-GM-abcdef12)
refuses "record not at the commit" "is not at 1234abcd" "$fx/verdicts-clean.txt" "$fx/ms-clean.md"
common=(--board pi4 --commit 1234abcd --record records/imx8mp/20261201-1100-GM-1234abcd)
refuses "record of another board" "not under records/pi4/" "$fx/verdicts-clean.txt" "$fx/ms-clean.md"
common=(--board pi5 --commit 1234abcd --record 20261201-1015-GM-1234abcd)
refuses "unknown board" "want one of am62p, pi4, imx8mp" "$fx/verdicts-clean.txt" "$fx/ms-clean.md"
common=(--board pi4 --commit 00eeee --record 20261201-1015-GM-1234abcd)
refuses "commit not a sha8" "not a sha8" "$fx/verdicts-clean.txt" "$fx/ms-clean.md"
common=(--board pi4 --commit 1234abcd --record 20261201-1015-GM-1234abcd)
refuses "--prior holds a table pipe" "split the table cell" "$fx/verdicts-clean.txt" "$fx/ms-clean.md" --prior '461 at `445c5653` | 42'
python3 -I "$tool" --board pi4 --commit 1234abcd --verdicts "$fx/verdicts-clean.txt" \
    --ms "$fx/ms-clean.md" >/dev/null 2>&1
[ "$?" -eq 2 ] && ok "missing --record: exit 2" || bad "missing --record: want exit 2"
python3 -I "$tool" --board pi4 --commit 1234abcd --record 20261201-1015-GM-1234abcd \
    --verdicts "$fx/verdicts-clean.txt" --prior 'x' >/dev/null 2>&1
[ "$?" -eq 2 ] && ok "missing --ms: exit 2" || bad "missing --ms: want exit 2"

# 7. the clean cell, put into the RESULTS fixture, passes readme-glmark2-check.py
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
sed 's#records/pi4/20261201-1015-GM-1234abcd/#records/pi4/#' "$tmp/RESULTS.md" >"$tmp/RESULTS-norec.md"
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
