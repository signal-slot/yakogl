#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Reads the verdicts of the Raspberry Pi 5 gl_test image from a console file
# (a tools/board.sh pi5 run's console.txt; row N8 of
# docs/notes/rpi5-gate-rows.md).
#
#   tools/pi5-gltest-verdicts.sh <console file>
#   tools/pi5-gltest-verdicts.sh --selftest
#
# The image runs cases 5, 58, 59, 66, 67 and 102 (V3D_GL_TEST_CASES of
# driver/samples/zephyr/gl_test) and ends on "v3d_gl_test RECORD end"
# (tools/board.sh, END_RE_DEFAULT). The lines read, each after an optional
# "HH:MM:SS " stamp:
#   gl_test: case N <name>: PASS|FAIL|SKIP ...   the run (driver/tools/pvrlab/gl_test.c)
#   yakogl/v3d: draw refused: <why>              the back end's named refusal
#   v3d_gl_test: case N PASS|FAIL|SKIP|...       the record (gl_test/src/main_rpi4.c)
#   v3d_gl_test RECORD end, build <id>           the end line
#
# stdout: six lines, one per case in the order above:
#   pi5 gl_test: case N <name>: PASS
#   pi5 gl_test: case N <name>: REFUSED (<why>)   a FAIL with a refusal named
#                                                 since the previous case's line
#   pi5 gl_test: case N <name>: FAIL (unnamed)    a FAIL with none
#   pi5 gl_test: case N <name>: SKIP ...          as the run printed it
#   pi5 gl_test: case N: MISSING (<what>)
# stderr: one "pi5-gltest-verdicts: ..." line per defect, then a summary.
# Exit 0 when every case has a verdict in the run and in the record, the two
# agree, every FAIL is a named refusal and the end line is there; 1 otherwise;
# 2 on a usage error. A refusal string is logged once per context
# (count_refusal, src/v3d/backend.rs), so a second case refused with the same
# string reads as unnamed.
set -u
CASES="5 58 59 66 67 102"
me=pi5-gltest-verdicts

extract() {
    awk -v cases="$CASES" -v me="$me" '
    function miss(n, what) {
        bad++
        print "pi5 gl_test: case " n ": MISSING (" what ")"
        print me ": case " n ": " what > "/dev/stderr"
    }
    { sub(/\r$/, ""); sub(/^[0-9][0-9]:[0-9][0-9]:[0-9][0-9] /, "") }
    /^yakogl\/v3d: draw refused: / {
        w = $0; sub(/^yakogl\/v3d: draw refused: /, "", w)
        why = (why == "") ? w : why "; " w
        next
    }
    /^gl_test: case [0-9]+ [^ :]+: / {
        n = $3; name = $4; sub(/:$/, "", name)
        v = $0; sub(/^gl_test: case [0-9]+ [^ :]+: /, "", v)
        if (!(n in runv)) { runv[n] = v; rname[n] = name; rwhy[n] = why }
        why = ""
        next
    }
    /^v3d_gl_test: case [0-9]+ / {
        n = $3; v = $0; sub(/^v3d_gl_test: case [0-9]+ /, "", v)
        recv[n] = v
        next
    }
    /^v3d_gl_test RECORD end/ { end = 1; build = $NF }
    END {
        nc = split(cases, c, " ")
        for (i = 1; i <= nc; i++) {
            n = c[i]
            if (!(n in runv)) { miss(n, "no \"gl_test: case " n "\" line"); continue }
            if (!(n in recv)) { miss(n, "no \"v3d_gl_test: case " n "\" line in the record"); continue }
            word = runv[n]; sub(/ .*/, "", word)
            if (recv[n] != "PASS" && recv[n] != "FAIL" && recv[n] != "SKIP") {
                miss(n, "the record says " recv[n]); continue
            }
            if (word != recv[n]) { miss(n, "the run says " word ", the record " recv[n]); continue }
            if (word == "PASS") { np++; print "pi5 gl_test: case " n " " rname[n] ": PASS" }
            else if (word == "SKIP") { ns++; print "pi5 gl_test: case " n " " rname[n] ": " runv[n] }
            else if (rwhy[n] != "") { nr++; print "pi5 gl_test: case " n " " rname[n] ": REFUSED (" rwhy[n] ")" }
            else {
                bad++
                print "pi5 gl_test: case " n " " rname[n] ": FAIL (unnamed)"
                print me ": case " n " failed with no \"yakogl/v3d: draw refused:\" line" > "/dev/stderr"
            }
        }
        if (!end) { bad++; print me ": no \"v3d_gl_test RECORD end\" line" > "/dev/stderr" }
        print me ": build " (end ? build : "?") ": " np + 0 " PASS, " nr + 0 " refused by name, " ns + 0 " SKIP, " bad + 0 " defect(s)" > "/dev/stderr"
        exit bad ? 1 : 0
    }
    ' "$1"
}

ok() { echo "selftest: ok $1"; }
no() { echo "selftest: FAIL $1"; fails=$((fails + 1)); }

# mutant <name> <want in stdout> <want in stderr>: must exit 1 and say why
mutant() {
    out=$(extract "$tmp/$1.txt" 2>"$tmp/err"); rc=$?
    if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q "$2" && grep -q "$3" "$tmp/err"; then
        ok "mutant rejected: $1"
    else
        no "mutant not rejected: $1 (exit $rc)"
    fi
}

selftest() {
    fix=$(dirname "$0")/testdata/pi5-gltest-synthetic.console.txt
    [ -r "$fix" ] || { echo "$me: selftest: no fixture $fix" >&2; exit 2; }
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/pi5gt.XXXXXX") || exit 2
    trap 'rm -rf "$tmp"' EXIT
    fails=0
    lost='an earlier job wrote, and the target has no depth/stencil buffer to keep it in'

    out=$(extract "$fix" 2>"$tmp/err"); rc=$?
    if [ "$rc" -eq 0 ] && [ "$(printf '%s\n' "$out" | grep -c '^pi5 gl_test: case ')" -eq 6 ] &&
        [ "$(printf '%s\n' "$out" | grep -c ': PASS$')" -eq 4 ] &&
        printf '%s\n' "$out" | grep -q "^pi5 gl_test: case 59 clearmask: REFUSED (the draw reads stencil $lost)\$" &&
        printf '%s\n' "$out" | grep -q "^pi5 gl_test: case 67 fbods: REFUSED (the draw reads depth $lost)\$"; then
        ok "the fixture: six verdicts, 4 PASS, 59 and 67 refused by name, exit 0"
    else
        no "the fixture (exit $rc)"
    fi

    grep -v 'gl_test: case 66 ' "$fix" > "$tmp/dropped-case.txt"
    mutant dropped-case '^pi5 gl_test: case 66: MISSING' 'case 66: no '
    grep -v 'draw refused: the draw reads depth' "$fix" > "$tmp/unnamed-refusal.txt"
    mutant unnamed-refusal '^pi5 gl_test: case 67 fbods: FAIL (unnamed)$' 'case 67 failed with no '
    grep -v 'v3d_gl_test RECORD end' "$fix" > "$tmp/missing-end.txt"
    mutant missing-end '^pi5 gl_test: case 102 maskclear: PASS$' 'no "v3d_gl_test RECORD end" line'

    if [ "$fails" -eq 0 ]; then
        echo "selftest: fixture accepted, 3 mutants rejected (dropped case, unnamed refusal, missing end)"
        exit 0
    fi
    echo "selftest: $fails check(s) failed"
    exit 1
}

usage() { echo "usage: $0 <console file> | --selftest" >&2; exit 2; }
[ $# -eq 1 ] || usage
case $1 in
--selftest) selftest ;;
-*) usage ;;
esac
[ -r "$1" ] || { echo "$me: cannot read $1" >&2; exit 2; }
extract "$1"
