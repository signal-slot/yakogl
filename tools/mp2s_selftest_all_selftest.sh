#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_selftest_all_selftest.sh - tools/mp2s_selftest_all.sh against a fake
# tree (MP2S_SELFTEST_ROOT) holding one passing, one failing and one hanging
# selftest, plus a passing and a failing .py selftest. Checks the summary, the exit code, the timeout, --list and --only,
# and that an empty match is a pass.
#
#   tools/mp2s_selftest_all_selftest.sh
#
# Prints "mp2s_selftest_all_selftest: N checks, F failed"; exit 0 only when F
# is 0.
set -uo pipefail
export LC_ALL=C

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
run=$here/mp2s_selftest_all.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_all_st.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
checks=0
fails=0
check() { # name, status (0 = pass)
    checks=$((checks + 1))
    if [ "$2" -ne 0 ]; then
        fails=$((fails + 1))
        echo "FAIL: $1"
    fi
}
has() { # pattern, text: status 0 when found
    grep -q -e "$1" <<<"$2"
}
hasnt() {
    ! grep -q -e "$1" <<<"$2"
}

fk=$tmp/fake
mkdir -p "$fk/tools" "$fk/driver/tools/mp2-standalone"
printf '#!/bin/bash\necho fine\nexit 0\n' >"$fk/tools/mp2s_good_selftest.sh"
{
    echo '#!/bin/bash'
    for i in $(seq 1 30); do echo "echo line$i"; done
    echo 'exit 3'
} >"$fk/tools/mp2s_bad_selftest.sh"
printf 'print("pyfine")\n' >"$fk/tools/mp2s_pygood_selftest.py"
printf 'import sys\nprint("pyboom")\nsys.exit(4)\n' >"$fk/tools/mp2s_pybad_selftest.py"
printf '#!/bin/bash\nsleep 60\n' >"$fk/driver/tools/mp2-standalone/selftest.sh"
# must never run: this runner's own selftest
printf '#!/bin/bash\necho RECURSED\nexit 1\n' >"$fk/tools/mp2s_selftest_all_selftest.sh"
export MP2S_SELFTEST_ROOT=$fk

# --list
o=$("$run" --list)
check "list exit" $?
has '^tools/mp2s_good_selftest.sh$' "$o"; check "list good" $?
has '^driver/tools/mp2-standalone/selftest.sh$' "$o"; check "list standalone" $?
hasnt mp2s_selftest_all_selftest "$o"; check "list excludes self" $?
has '^tools/mp2s_pygood_selftest.py$' "$o"; check "list py good" $?
has '^tools/mp2s_pybad_selftest.py$' "$o"; check "list py bad" $?

# full run with a 2 s timeout
export MP2S_SELFTEST_TIMEOUT=2
t0=$SECONDS
o=$("$run" 2>&1)
rc=$?
dt=$((SECONDS - t0))
[ "$rc" -ne 0 ]; check "run fails overall" $?
[ "$dt" -lt 30 ]; check "run bounded by timeout" $?
has '^ok tools/mp2s_good_selftest.sh ([0-9]*s)$' "$o"; check "ok line" $?
has '^FAIL tools/mp2s_bad_selftest.sh (exit 3)$' "$o"; check "bad line" $?
has '^FAIL driver/tools/mp2-standalone/selftest.sh (exit 124)$' "$o"; check "hang line" $?
has '^ok tools/mp2s_pygood_selftest.py ([0-9]*s)$' "$o"; check "py ok line" $?
has '^FAIL tools/mp2s_pybad_selftest.py (exit 4)$' "$o"; check "py bad line" $?
has 'pyboom$' "$o"; check "py bad tail" $?
has 'line30$' "$o"; check "tail has line30" $?
hasnt 'line10$' "$o"; check "tail is 20 lines only" $?
has '^mp2s selftests: 2 ok, 3 failed$' "$o"; check "summary" $?
hasnt RECURSED "$o"; check "no recursion" $?

# --only
o=$("$run" --only 'mp2s_good_' 2>&1)
check "only exit 0" $?
has '^mp2s selftests: 1 ok, 0 failed$' "$o"; check "only summary" $?
o=$("$run" --only pygood 2>&1)
has '^mp2s selftests: 1 ok, 0 failed$' "$o"; check "only py summary" $?
o=$("$run" --list --only 'standalone/' 2>&1)
[ "$o" = driver/tools/mp2-standalone/selftest.sh ]; check "only list" $?
o=$("$run" --only nomatch 2>&1)
check "only none exit 0" $?
has '^mp2s selftests: 0 ok, 0 failed$' "$o"; check "only none summary" $?

# bad usage
"$run" --bogus >/dev/null 2>&1
[ $? -eq 2 ]; check "bad arg exit 2" $?
MP2S_SELFTEST_TIMEOUT=x "$run" >/dev/null 2>&1
[ $? -eq 2 ]; check "bad timeout exit 2" $?

# the real tree lists the standalone selftest and never itself
unset MP2S_SELFTEST_ROOT
o=$("$run" --list)
has '^driver/tools/mp2-standalone/selftest.sh$' "$o"; check "real tree lists mp2-standalone" $?
hasnt mp2s_selftest_all_selftest "$o"; check "real tree excludes self" $?

echo "mp2s_selftest_all_selftest: $checks checks, $fails failed"
[ "$fails" -eq 0 ]
