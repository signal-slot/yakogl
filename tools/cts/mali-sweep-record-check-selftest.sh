#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test for tools/cts/mali-sweep-record-check.py (host only, no board).
#
# Builds a good synthetic record and one bad record per failure mode under
# mktemp -d (TMPDIR), plus runs the checker on the real
# tools/cts/log/2026-11-t6-sweep-1a22bd38 record. A good record exits 0 with
# every check ok; every bad record exits 1 with exactly one FAIL line, naming
# exactly the check it targets (so a fixture never trips a second check).
# The real 1a22bd38 record needs no --reference flag: its README names its own
# commit 1a22bd38e, which satisfies the default --reference 1a22bd38 rule
# incidentally (the record is the reference, it does not cite it as one); the
# --reference '' form drops the rule entirely and is shown on a fixture.
#
# Usage: sh tools/cts/mali-sweep-record-check-selftest.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/mali-sweep-record-check.py"
real_record="$here/log/2026-11-t6-sweep-1a22bd38"
T=$(mktemp -d 2> /dev/null) || { echo "FAIL selftest: mktemp -d failed" >&2; exit 2; }
trap 'rm -rf "$T"' EXIT
fail=0
# split markers (D a dot, C a colon, M empty) so that this file itself never
# holds a literal dotted quad, MAC address, home path or tty path that a record
# scan could match: /ho${M}me/ is /ho<empty>me/, /dev/t${M}ty is /dev/t<empty>ty
D=. C=: M=

ok() { echo "ok   $1"; }
bad() { echo "FAIL selftest: $1"; fail=1; }

# mkgood DIR: a synthetic record that passes every check
mkgood() {
    d=$1
    mkdir -p "$d" || { echo "FAIL selftest: cannot mkdir $d" >&2; exit 2; }
    cat > "$d/README.md" << 'EOF'
# T6 sweep at main abcdef1234: panthor-gltest --sweep x2

Board: NanoPC-T6 (RK3588, Mali-G610 MC4, panthor, kernel
6.18.55-current-rockchip64), the board's own Linux, not Zephyr.

boot_id unchanged over both runs (`5e2a981d-…`). 2026-11-25, about 03:52 UTC.

Code: origin/main `abcdef1234`, nothing on top. The binary carried
`yakogl-build=v0.1.0-1005-gabcdef1234`.

## 1. Sweep, two runs

`sweep-1.verdicts.txt` and `sweep-2.verdicts.txt`: the 256 verdict lines and
the summary of each run, identical (`cmp` exits 0).

## 2. Delta against tools/cts/log/2026-11-t6-sweep-1a22bd38 (both runs)
EOF
    cat > "$d/sweep-1.verdicts.txt" << 'EOF'
# panthor-gltest --sweep, one run per file (README.md): the verdict and summary lines
panthor-gltest: verdict case 0 triangle: PASS
panthor-gltest: verdict case 1 uniform: PASS
panthor-gltest: verdict case 7 ?: NO-CASE
panthor-gltest: verdict case 77 transc: REFUSED compile: online shaderc: arithmetic not lowered yet
panthor-gltest: 256 case(s) run: FAIL 0, NO-CASE 1, PASS 2, REFUSED 1, SKIP 0
EOF
    cp "$d/sweep-1.verdicts.txt" "$d/sweep-2.verdicts.txt"
}

# fresh: a new good synthetic record in $R
fresh() {
    rm -rf "$T/w"; mkdir -p "$T/w"
    R="$T/w/2026-11-25-t6-sweep-abcdef12"
    mkgood "$R"
}

run() { python3 -I "$chk" "$@"; }
show() { printf '%s\n' "$out" | sed 's/^/     /'; }

# expect_good NAME N_OK ARGS...: exit 0, no FAIL line, exactly N_OK ok lines
expect_good() {
    what=$1; nwant=$2; shift 2
    out=$(run "$@"); got=$?
    nok=$(printf '%s\n' "$out" | grep -c '^ok   ')
    if [ "$got" = 0 ] && [ "$nok" = "$nwant" ] && ! printf '%s\n' "$out" | grep -q '^FAIL '; then
        ok "$what"
    else
        bad "$what (exit $got, $nok ok lines)"; show
    fi
}

# expect_only NAME TARGET FAILPAT ARGS...: exit 1, exactly one FAIL line
# (and 5 ok lines of six checks), it names TARGET and matches FAILPAT
expect_only() {
    what=$1; target=$2; pat=$3; shift 3
    out=$(run "$@"); got=$?
    nfail=$(printf '%s\n' "$out" | grep -c '^FAIL ')
    nok=$(printf '%s\n' "$out" | grep -c '^ok   ')
    line=$(printf '%s\n' "$out" | grep '^FAIL ' | head -1)
    if [ "$got" = 1 ] && [ "$nfail" = 1 ] && [ "$nok" = 5 ] \
        && printf '%s' "$line" | grep -qE "FAIL $target:" \
        && printf '%s' "$line" | grep -qE -- "$pat"; then
        ok "$what"
    else
        bad "$what (exit $got, $nfail FAIL, $nok ok)"; show
    fi
}

# expect_rc NAME RC ARGS...: exit code only (argument handling)
expect_rc() {
    what=$1; want=$2; shift 2
    out=$(run "$@" 2>&1); got=$?
    if [ "$got" = "$want" ]; then ok "$what"; else bad "$what (exit $got, want $want)"; show; fi
}

# good records
fresh; expect_good "good record (boot_id unchanged form, 1a22bd38 README style)" 6 "$R"
fresh; sed -i 's/^boot_id unchanged.*/boot_id before (`5e2a981d-…`) and after (`5e2a981d-…`), the same value./' "$R/README.md"
expect_good "good record (boot_id before/after form, equal values)" 6 "$R"

# the real 1a22bd38 record, default --reference: exit 0 with six checks ok
if [ -f "$real_record/README.md" ] && [ -f "$real_record/sweep-1.verdicts.txt" ]; then
    expect_good "real 2026-11-t6-sweep-1a22bd38 record (names 1a22bd38e, its own commit: the default --reference 1a22bd38 rule holds without a flag)" 6 "$real_record"
else
    bad "real record log/2026-11-t6-sweep-1a22bd38 missing from tools/cts"
fi

# verdict file failures: each trips only verdicts-identical / verdicts-summary
fresh; rm "$R/sweep-2.verdicts.txt"
expect_only "sweep-2.verdicts.txt missing" verdicts-identical 'missing: sweep-2' "$R"
fresh; sed -i 's/^panthor-gltest: verdict case 1 uniform: PASS/panthor-gltest: verdict case 1 uniform: FAIL/' "$R/sweep-2.verdicts.txt"
expect_only "the two verdict files differ" verdicts-identical 'first difference at line 3' "$R"
fresh; sed -i '/case(s) run:/d' "$R/sweep-1.verdicts.txt" "$R/sweep-2.verdicts.txt"
expect_only "summary line missing from both files (still identical)" verdicts-summary 'sweep-1.*sweep-2' "$R"

# README field failures: each trips only its own readme check
fresh; sed -i 's/^boot_id unchanged.*/boot_id before (`5e2a981d-…`) and after (`d4c3b2a1-…`), the values differ./' "$R/README.md"
expect_only "boot_id before and after values differ" readme-boot-id 'values differ' "$R"
fresh; sed -i '/^boot_id unchanged/d' "$R/README.md"
expect_only "README names no boot_id" readme-boot-id 'does not name boot_id' "$R"
fresh; sed -i 's#sweep-1a22bd38#the previous sweep record#' "$R/README.md"
expect_only "README does not name 1a22bd38 (default --reference)" readme-reference 'reference commit 1a22bd38' "$R"
expect_good "--reference '' drops the readme-reference check (the same fixture passes)" 5 "$R" --reference ''
fresh; expect_only "--reference 99999999 drives the readme-reference check" readme-reference 'reference commit 99999999' "$R" --reference 99999999
fresh; sed -i 's/`abcdef1234`, nothing/its tip commit, nothing/' "$R/README.md"
expect_only "README names no commit sha besides the reference" readme-commit-sha 'no commit sha other than the reference' "$R"

# redaction failures: each trips only redaction, in README and in a verdict file
fresh; printf 'board 192%s0%s2%s7 reachable now\n' "$D" "$D" "$D" >> "$R/README.md"
expect_only "README holds a dotted quad" redaction ': ipv4' "$R"
out=$(run "$R")
if printf '%s' "$out" | grep -q "192${D}0${D}2${D}7"; then bad "redaction hit text was printed"; else ok "redaction hit text never printed"; fi
fresh; printf 'ethernet 00%s11%s22%s33%s44%s55\n' "$C" "$C" "$C" "$C" "$C" >> "$R/README.md"
expect_only "README holds a MAC address" redaction ': mac' "$R"
fresh; printf 'copied from /ho%sme/x\n' "$M" >> "$R/README.md"
expect_only "README holds a home path" redaction ': home-path' "$R"
fresh; printf 'console on /dev/t%styACM0\n' "$M" >> "$R/README.md"
expect_only "README holds a tty path" redaction ': tty-path' "$R"
fresh; printf 'board 192%s0%s2%s7 reachable now\n' "$D" "$D" "$D" >> "$R/sweep-1.verdicts.txt"
cp "$R/sweep-1.verdicts.txt" "$R/sweep-2.verdicts.txt"
expect_only "verdict files hold a dotted quad (still identical)" redaction 'sweep-1.*ipv4' "$R"

# argument handling
expect_rc "no argument exits 2" 2
expect_rc "missing record directory exits 2" 2 "$T/no-such-record"
fresh; expect_rc "--reference that is not hex exits 2" 2 "$R" --reference zzz

exit $fail
