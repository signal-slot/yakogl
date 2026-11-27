#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/check-common-unchanged-selftest.sh
#
# Self-test of check-common-unchanged.py on inline fixtures written to $TMPDIR
# (no record, no board address). Cases: an unchanged run (exit 0); a new gl_test
# failure, a glmark2 failure, a missing GT verdict, a failed verdict (exit 1,
# each difference named); an unreadable input and a RESULTS.md without the row
# (exit 2); a record directory read through a stub tally.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$HERE/check-common-unchanged.py
command -v python3 > /dev/null || { echo "check-common-unchanged-selftest: python3 missing" >&2; exit 2; }
W=$(mktemp -d "${TMPDIR:-/tmp}/ccu.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fail=0

cat > "$W/RESULTS.md" <<'R'
| Board | glmark2-es2 | gl_test | GPU resets |
|---|---|---|---:|
| AM62P | 489, 33 scenes, 0 failed, 0 GL errors; main `8133663c` | 50 of 50 (GT `x`, 0 failed, 0 timed out) | 0 |
| Pi 4 | 195, 33 scenes, 0 failed | 50 of 50 | 0 |
R
good() { cat <<'E'
common-tally: am62p, 2 rows from tree.txt
GM glmark2 rc=0 verdict=done result=record:GLMARK2 resets 0
    glmark2 score 491; scenes ran 33, failed 0, unsupported 0; status 1 (pass)
GT gl_test rc=0 verdict=done result=record:CASE resets 0
    gl_test passed 50 of 50 run (CASE record, set 256); skipped 0; failing: none
common-tally: rows 2, with a run 2, without 0; verdict done 2, other 0
E
}
good > "$W/good.out"
sed 's/passed 50 of 50 run\(.*\)failing: none/passed 49 of 50 run\1failing: 215/; s/^GT gl_test rc=0 verdict=done/GT gl_test rc=0 verdict=failed/' "$W/good.out" > "$W/gtfail.out"
sed 's/failed 0, unsupported/failed 2, unsupported/' "$W/good.out" > "$W/gmfail.out"
grep -v '^GT ' "$W/good.out" > "$W/nogt.out"
grep -v 'gl_test passed' "$W/good.out" > "$W/nogtline.out"
mkdir "$W/rec"; : > "$W/rec/tree.txt"
printf '#!/bin/sh\ncat "%s"\n' "$W/good.out" > "$W/tally.sh"
printf 'import subprocess,sys\nsys.exit(subprocess.call(["sh","%s"]))\n' "$W/tally.sh" > "$W/tally.py"
grep -v '^| AM62P' "$W/RESULTS.md" > "$W/norow.md"

t() {  # t <name> <want rc> <want grep or -> <args...>
    local n=$1 want=$2 pat=$3 out rc; shift 3
    out=$(python3 "$T" "$@" 2>&1); rc=$?
    if [ "$rc" != "$want" ]; then echo "FAIL $n: rc $rc, want $want"; echo "$out"; fail=1; return; fi
    if [ "$pat" != - ] && ! grep -q -- "$pat" <<<"$out"; then echo "FAIL $n: no '$pat' in output"; echo "$out"; fail=1; return; fi
    echo "ok   $n"
}
t unchanged 0 'ok (am62p' "$W/good.out" "$W/RESULTS.md"
t gtfail 1 'new failing case(s): 215' "$W/gtfail.out" "$W/RESULTS.md"
t gtverdict 1 'GT gl_test verdict=failed' "$W/gtfail.out" "$W/RESULTS.md"
t gmfail 1 'glmark2 failed 2 scenes' "$W/gmfail.out" "$W/RESULTS.md"
t nogt 1 "missing verdict: no 'GT gl_test" "$W/nogt.out" "$W/RESULTS.md"
t nogtline 1 "no 'gl_test passed" "$W/nogtline.out" "$W/RESULTS.md"
t dir-empty 1 'missing verdict' "$W/rec" "$W/RESULTS.md"
CHECK_COMMON_TALLY=$W/tally.py t dir-stub 0 'ok (am62p' "$W/rec" "$W/RESULTS.md"
t unreadable 2 - "$W/none.out" "$W/RESULTS.md"
t norow 2 "no 'AM62P' common row" "$W/good.out" "$W/norow.md"
exit $fail
