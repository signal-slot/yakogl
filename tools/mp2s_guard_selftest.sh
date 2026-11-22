#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_guard_selftest.sh - gate-visible selftest of tools/mp2s_guard.py.
# Runs the guard's built-in --self-test, then black-box cases (exit code and
# the single verdict line, both --for values) over every fixture of
# tools/testdata/mp2s-env/ and over printenv texts built in $TMPDIR.
# No board. Prints "N checks, M failed"; exit 0 only when M is 0.
set -uo pipefail
export LC_ALL=C

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
guard=$root/tools/mp2s_guard.py
fx=$root/tools/testdata/mp2s-env
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-guard-st.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT

n=0
bad=0
fail() { bad=$((bad + 1)); echo "FAIL: $*"; }

# check NAME WANT_RC WANT_STREAM_SUBSTRING GUARD_ARGS...
# rc 0: one "mp2s-guard: allow:" line on stdout, stderr empty.
# rc 3: one "mp2s-guard: REFUSE:" line on stderr, stdout empty.
# rc 2: one "mp2s-guard: error:" line on stderr, stdout empty.
check() {
    local name=$1 want=$2 sub=$3
    shift 3
    n=$((n + 1))
    local out err rc
    python3 -I "$guard" "$@" >"$work/out" 2>"$work/err" </dev/null
    rc=$?
    out=$(cat "$work/out")
    err=$(cat "$work/err")
    if [ "$rc" != "$want" ]; then
        fail "$name: exit $rc, want $want ($out $err)"
        return
    fi
    local line kind other prefix
    case $want in
    0) line=$out; other=$err; prefix='mp2s-guard: allow: ' ;;
    3) line=$err; other=$out; prefix='mp2s-guard: REFUSE: ' ;;
    *) line=$err; other=$out; prefix='mp2s-guard: error: ' ;;
    esac
    if [ -n "$other" ]; then
        fail "$name: unexpected output on the other stream: $other"
    elif [ "$(printf '%s\n' "$line" | wc -l)" != 1 ] || [ -z "$line" ]; then
        fail "$name: not exactly one verdict line: $line"
    elif [ "${line#"$prefix"}" = "$line" ]; then
        fail "$name: line does not start with '$prefix': $line"
    elif [ -n "$sub" ] && [[ $line != *"$sub"* ]]; then
        fail "$name: line lacks '$sub': $line"
    fi
}

# 1. The guard's own built-in self-test.
n=$((n + 1))
if ! python3 -I "$guard" --self-test >"$work/st" 2>&1; then
    fail "built-in --self-test: $(tail -n 5 "$work/st")"
fi

# 2. Every fixture, both --for values. DK layout: two copies at 0 and 0x2000.
lay=(--redundant --size 0x2000)
# fixture: oneshot-verdict standalone-verdict (a = allow, r = refuse).
# mp2s_left is set (counting2, counting1, left0) or a stray mp2s_ver/prefixes
# is present (installed): a one-shot refuses. boot_prefixes carries
# /yakogl/standalone/ in every image but pristine: a standalone refuses.
# one-copy-bad's active copy is counting1's copy 2; both-bad has none.
fixtures=(
    "pristine a a"
    "installed r r"
    "counting2 r r"
    "counting1 r r"
    "left0 r r"
)
for row in "${fixtures[@]}"; do
    read -r f o s <<<"$row"
    img=$fx/$f.bin
    if [ "$o" = a ]; then
        check "$f oneshot" 0 "" --for oneshot --env-image "$img" "${lay[@]}"
    else
        check "$f oneshot" 3 "mp2s_" --for oneshot --env-image "$img" "${lay[@]}"
    fi
    if [ "$s" = a ]; then
        check "$f standalone" 0 "" --for standalone --env-image "$img" "${lay[@]}"
    else
        check "$f standalone" 3 "/yakogl/standalone/" --for standalone --env-image "$img" "${lay[@]}"
    fi
done
# A copy that does not check (one or both): the guard reads strictly and
# exits 2 (a refusal for a caller), in both directions.
check "one-copy-bad oneshot" 2 "bad CRC" --for oneshot --env-image "$fx/one-copy-bad.bin" "${lay[@]}"
check "one-copy-bad standalone" 2 "bad CRC" --for standalone --env-image "$fx/one-copy-bad.bin" "${lay[@]}"
check "both-bad oneshot" 2 "" --for oneshot --env-image "$fx/both-bad.bin" "${lay[@]}"
check "both-bad standalone" 2 "" --for standalone --env-image "$fx/both-bad.bin" "${lay[@]}"
# Wrong layout: size larger than the file.
check "pristine bad layout" 2 "" --for oneshot --env-image "$fx/pristine.bin" --redundant --size 0x8000
# One refusal names the armed variable.
check "counting1 names mp2s_left" 3 "mp2s_left" --for oneshot --env-image "$fx/counting1.bin" "${lay[@]}"

# 3. printenv text files.
pe() { printf '%s\n' "$@" >"$work/$PE.txt"; }
base=('bootcmd=run bootcmd_stm32mp' 'boot_prefixes=/ /boot/')
PE=clean; pe "${base[@]}"
PE=armed; pe 'boot_prefixes=/yakogl/standalone/ / /boot/' mp2s_left=2 mp2s_ver=1 'mp2s_prefixes=/ /boot/'
PE=stray; pe "${base[@]}" mp2s_ver=1
PE=prefix; pe 'boot_prefixes=/yakogl/standalone/ / /boot/'
PE=yako; pe "${base[@]}" yako_boot=1
PE=mp2z; pe "${base[@]}" mp2z_go=1
PE=other; pe "${base[@]}" mp2sx=1 xmp2s_left=1
check "text clean oneshot" 0 "" --for oneshot --printenv "$work/clean.txt"
check "text clean standalone" 0 "" --for standalone --printenv "$work/clean.txt"
check "text armed oneshot" 3 "mp2s_left" --for oneshot --printenv "$work/armed.txt"
check "text armed standalone" 3 "/yakogl/standalone/" --for standalone --printenv "$work/armed.txt"
check "text stray oneshot" 3 "mp2s_ver" --for oneshot --printenv "$work/stray.txt"
check "text stray standalone" 0 "" --for standalone --printenv "$work/stray.txt"
check "text prefix oneshot" 0 "" --for oneshot --printenv "$work/prefix.txt"
check "text prefix standalone" 3 "/yakogl/standalone/" --for standalone --printenv "$work/prefix.txt"
check "text yako oneshot" 0 "" --for oneshot --printenv "$work/yako.txt"
check "text yako standalone" 3 "yako_boot" --for standalone --printenv "$work/yako.txt"
check "text mp2z oneshot" 0 "" --for oneshot --printenv "$work/mp2z.txt"
check "text mp2z standalone" 3 "mp2z_go" --for standalone --printenv "$work/mp2z.txt"
check "text lookalikes oneshot" 0 "" --for oneshot --printenv "$work/other.txt"
check "text lookalikes standalone" 0 "" --for standalone --printenv "$work/other.txt"
# stdin form and error cases.
n=$((n + 1))
if ! python3 -I "$guard" --for oneshot <"$work/armed.txt" >/dev/null 2>"$work/err" \
    && [ "$(grep -c 'REFUSE' "$work/err")" = 1 ]; then :; else fail "stdin armed oneshot did not refuse"; fi
check "missing file" 2 "" --for oneshot --printenv "$work/nonexistent"
n=$((n + 1))
python3 -I "$guard" >/dev/null 2>&1 </dev/null
[ $? = 2 ] || fail "no --for: exit is not 2"

echo "mp2s_guard_selftest: $n checks, $bad failed"
[ "$bad" = 0 ]
