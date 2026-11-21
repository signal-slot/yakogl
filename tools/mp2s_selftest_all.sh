#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_selftest_all.sh - one entry point for every DK standalone host selftest.
# The selftests are found by glob at run time (a newly merged one needs no
# edit): tools/mp2s_*_selftest.sh, tools/mp2s_*_selftest.py,
# tools/board_stm32mp2_standalone_selftest.sh, tools/fwu_metadata_selftest.py,
# tools/testdata/efi_varfile_selftest.sh, tools/testdata/mp2s-env/test_mp2s_env.py
# (runs standalone) and driver/tools/mp2-standalone/selftest.sh. .sh files run
# with bash, .py files with `python3 -I -B`. Missing ones are skipped
# silently; this runner's own selftest is excluded (no recursion).
#
#   tools/mp2s_selftest_all.sh [--list] [--only REGEX]
#
# MP2S_SELFTEST_TIMEOUT  seconds per selftest (default 300)
# MP2S_SELFTEST_ROOT     tree to search (default: the repository root)
#
# One line per test: "ok NAME (Ns)" or "FAIL NAME (exit N)" followed by the
# last 20 lines of its output. Ends with "mp2s selftests: N ok, M failed";
# exit 0 only when M is 0. A timed out test fails with exit 124.
set -uo pipefail
export LC_ALL=C

root=${MP2S_SELFTEST_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
tmo=${MP2S_SELFTEST_TIMEOUT:-300}
list=0
only=

while [ $# -gt 0 ]; do
    case $1 in
    --list) list=1 ;;
    --only)
        [ $# -ge 2 ] || { echo "mp2s_selftest_all: --only needs a REGEX" >&2; exit 2; }
        only=$2
        shift ;;
    -h|--help) sed -n '2,21p' "$0"; exit 0 ;;
    *) echo "mp2s_selftest_all: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done
case $tmo in
'' | *[!0-9]*) echo "mp2s_selftest_all: MP2S_SELFTEST_TIMEOUT must be seconds" >&2; exit 2 ;;
esac

found=()
for f in "$root"/tools/mp2s_*_selftest.sh \
         "$root"/tools/mp2s_*_selftest.py \
         "$root"/tools/board_stm32mp2_standalone_selftest.sh \
         "$root"/tools/fwu_metadata_selftest.py \
         "$root"/tools/testdata/efi_varfile_selftest.sh \
         "$root"/tools/testdata/mp2s-env/test_mp2s_env.py \
         "$root"/driver/tools/mp2-standalone/selftest.sh; do
    [ -f "$f" ] || continue
    n=${f#"$root"/}
    [ "$n" = tools/mp2s_selftest_all_selftest.sh ] && continue
    if [ -n "$only" ] && ! [[ $n =~ $only ]]; then continue; fi
    found+=("$n")
done
tests=()
if [ ${#found[@]} -gt 0 ]; then
    mapfile -t tests < <(printf '%s\n' "${found[@]}" | sort -u)
fi

if [ "$list" = 1 ]; then
    if [ ${#tests[@]} -gt 0 ]; then printf '%s\n' "${tests[@]}"; fi
    exit 0
fi

out=$(mktemp "${TMPDIR:-/tmp}/mp2s_all.XXXXXX") || exit 2
trap 'rm -f "$out"' EXIT
ok=0
bad=0
for n in "${tests[@]+"${tests[@]}"}"; do
    [ -n "$n" ] || continue
    t0=$SECONDS
    case $n in
    *.py) interp=(python3 -I -B) ;;
    *) interp=(bash) ;;
    esac
    (cd "$root" && timeout -k 5 "$tmo" "${interp[@]}" "$n") >"$out" 2>&1 </dev/null
    rc=$?
    dt=$((SECONDS - t0))
    if [ "$rc" -eq 0 ]; then
        echo "ok $n (${dt}s)"
        ok=$((ok + 1))
    else
        echo "FAIL $n (exit $rc)"
        tail -n 20 "$out" | sed 's/^/    /'
        bad=$((bad + 1))
    fi
done
echo "mp2s selftests: $ok ok, $bad failed"
[ "$bad" -eq 0 ]
