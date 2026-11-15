#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/route_choice_selftest.sh
#
# Hermetic self-test of route_choice.sh on fixed logs (written to $TMPDIR):
# each route, the verdicts.sh output as input, stdin, missing lines,
# duplicate and contradictory lines, usage errors.
set -u
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
R=$HERE/route_choice.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/route_choice_selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
n=0
fail=0
chk() { # <name> <want line> <want rc> <got line> <got rc>
    n=$((n + 1))
    if [ "$4" = "$2" ] && [ "$5" = "$3" ]; then
        printf '  ok    %s\n' "$1"
    else
        printf '  FAIL  %s: want [%s] rc %s, got [%s] rc %s\n' "$1" "$2" "$3" "$4" "$5"
        fail=$((fail + 1))
    fi
}
run() { # <name> <file> <want line> <want rc>
    local out rc
    out=$(bash "$R" "$2" 2>&1); rc=$?
    chk "$1" "$3" "$4" "$out" "$rc"
}
mk() { # <file> <scanout> <adopt> <mailbox>  ('-' omits the line)
    {
        echo "--- Zephyr rpi5_display ---"
        [ "$2" = - ] || echo "u3 scanout: $2"
        echo "u3 plane: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080"
        [ "$3" = - ] || echo "u3 adopt-readback: $3 (x)"
        [ "$4" = - ] || echo "u3 mailbox-fb: $4 (x)"
        echo "u3 split-screen: drawn (text on top, pattern below)"
        echo "rpi5_display: PASS (x)"
    } > "$1"
}

R1="u3 route: 1 firmware scanout running, adopt-readback PASS"
R3="u3 route: 3 adopt-readback FAIL, mailbox-fb FAIL"
R2="u3 route: 2 adopt-readback FAIL, mailbox-fb PASS"

if bash -n "$R"; then chk "route_choice.sh parses" ok 0 ok 0; else chk "route_choice.sh parses" ok 0 bad 1; fi

mk "$T/r1" running PASS PASS;   run "route 1" "$T/r1" "$R1" 0
mk "$T/r1b" running PASS -;     run "route 1, mailbox line absent" "$T/r1b" "$R1" 0
mk "$T/r2" running FAIL PASS;   run "route 2, adopt FAIL" "$T/r2" "$R2" 0
mk "$T/r2b" stopped FAIL PASS;  run "route 2, scanout stopped" "$T/r2b" "u3 route: 2 scanout stopped, mailbox-fb PASS" 0
mk "$T/r2c" stopped - PASS;     run "route 2, no adopt line while stopped" "$T/r2c" "u3 route: 2 scanout stopped, mailbox-fb PASS" 0
mk "$T/r3" running FAIL FAIL;   run "route 3, both FAIL" "$T/r3" "$R3" 0
mk "$T/r3b" stopped FAIL FAIL;  run "route 3, scanout stopped" "$T/r3b" "u3 route: 3 scanout stopped, mailbox-fb FAIL" 0

# stdin and a CRLF log
out=$(bash "$R" < "$T/r1"); chk "stdin" "$R1" 0 "$out" $?
sed 's/$/\r/' "$T/r3" > "$T/crlf"; run "CRLF log" "$T/crlf" "$R3" 0

# verdicts.sh output as input
bash "$HERE/verdicts.sh" "$T/r2" > "$T/v2.out" 2>&1
run "verdicts.sh output" "$T/v2.out" "$R2" 0

# missing lines
mk "$T/m1" - PASS PASS;     run "missing scanout" "$T/m1" "u3 route: undecided missing: u3 scanout" 2
mk "$T/m2" running - PASS;  run "missing adopt-readback" "$T/m2" "u3 route: undecided missing: u3 adopt-readback" 2
mk "$T/m3" running FAIL -;  run "missing mailbox-fb" "$T/m3" "u3 route: undecided missing: u3 mailbox-fb" 2
mk "$T/m4" stopped - -;     run "missing mailbox-fb, stopped" "$T/m4" "u3 route: undecided missing: u3 mailbox-fb" 2
: > "$T/empty";             run "empty log" "$T/empty" "u3 route: undecided missing: u3 scanout" 2

# duplicates: identical is fine, differing is contradictory
{ cat "$T/r1"; echo "u3 adopt-readback: PASS (again)"; } > "$T/d1"
run "identical duplicate" "$T/d1" "$R1" 0
{ cat "$T/r1"; echo "u3 adopt-readback: FAIL (late)"; } > "$T/d2"
run "adopt-readback PASS and FAIL" "$T/d2" "u3 route: undecided contradictory lines: adopt-readback:PASS+FAIL" 2
{ cat "$T/r3"; echo "u3 scanout: stopped"; } > "$T/d3"
run "scanout running and stopped" "$T/d3" "u3 route: undecided contradictory lines: scanout:running+stopped" 2
{ cat "$T/r3"; echo "u3 mailbox-fb: PASS (late)"; } > "$T/d4"
run "mailbox-fb FAIL and PASS" "$T/d4" "u3 route: undecided contradictory lines: mailbox-fb:PASS+FAIL" 2
mk "$T/c1" stopped PASS PASS
run "adopt PASS with scanout stopped" "$T/c1" "u3 route: undecided contradictory lines: scanout:stopped adopt-readback:PASS" 2

# usage: no route line, exit 2
out=$(bash "$R" "$T/none" 2>/dev/null); chk "unreadable file prints no route" "" 2 "$out" $?
out=$(bash "$R" a b 2>/dev/null); chk "two arguments" "" 2 "$out" $?
if [ -x "$R" ]; then chk "route_choice.sh executable" ok 0 ok 0; else chk "route_choice.sh executable" ok 0 no 1; fi

echo "route_choice_selftest: $((n - fail)) of $n checks, $fail failed"
[ "$fail" -eq 0 ]
