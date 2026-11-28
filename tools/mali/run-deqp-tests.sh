#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run-deqp-tests.sh - run every tests/mali_deqp_*.rs with the feature set
# named in its header comment (`//! cargo test <flags> --test <name>`).
# Files with the same flags go into one `cargo test --test a --test b ...`.
#
# Usage: tools/mali/run-deqp-tests.sh [--list] [--include-ignored]
#                                     [--only <glob>]...
#   --list              print the planned invocations, run nothing
#   --include-ignored   pass `-- --include-ignored` to the test binaries
#   --only <glob>       only files matching the glob (repeatable; matched
#                       against the path, e.g. 'tests/mali_deqp_t3gap.rs')
# Exit: nonzero if a group fails or a file has no feature-set header.
# See docs/notes/mali-deqp-tests-runner.md.
set -u
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root" || exit 2

list=0
incl=0
only=()
while [ $# -gt 0 ]; do
    case $1 in
    --list) list=1 ;;
    --include-ignored) incl=1 ;;
    --only)
        [ $# -ge 2 ] || { echo "run-deqp-tests: --only needs a glob" >&2; exit 2; }
        only+=("$2"); shift ;;
    -h|--help) sed -n '2,/^set -u/p' "$0" | sed '$d'; exit 0 ;;
    *) echo "run-deqp-tests: unknown option $1" >&2; exit 2 ;;
    esac
    shift
done

files=()
for f in tests/mali_deqp_*.rs; do
    [ -e "$f" ] || continue
    if [ ${#only[@]} -gt 0 ]; then
        match=0
        for g in "${only[@]}"; do
            # shellcheck disable=SC2254
            case $f in $g|${g#./}) match=1 ;; esac
            [ "$(basename "$f")" = "$g" ] && match=1
        done
        [ $match -eq 1 ] || continue
    fi
    files+=("$f")
done
if [ ${#files[@]} -eq 0 ]; then
    echo "run-deqp-tests: no test file matches" >&2
    echo "0 groups, 0 tests files, 1 failed"
    exit 1
fi

bad=0
declare -A group_flags=()
declare -A group_tests=()
order=()
for f in "${files[@]}"; do
    name=$(basename "$f" .rs)
    line=$(grep -m1 -E '^//[!/]? *cargo test ' "$f")
    flags=""
    if [ -n "$line" ]; then
        flags=${line#*cargo test }
        flags=${flags%% --test *}
        flags=$(echo "$flags" | tr -s ' ' | sed 's/^ //; s/ $//')
    fi
    if [ -z "$flags" ] || [ "$flags" = "--test $name" ] || [[ $line != *"--test "* ]]; then
        echo "run-deqp-tests: $f: no feature-set header ('//! cargo test <flags> --test $name')" >&2
        bad=$((bad + 1))
        continue
    fi
    if [ -z "${group_flags[$flags]+x}" ]; then
        group_flags[$flags]=$flags
        order+=("$flags")
    fi
    group_tests[$flags]+=" --test $name"
done

groups=0
nfiles=0
failed=$bad
for flags in "${order[@]}"; do
    tests=${group_tests[$flags]}
    n=$(echo "$tests" | grep -o -- '--test' | wc -l)
    extra=""
    [ $incl -eq 1 ] && extra=" -- --include-ignored"
    cmd="cargo test $flags$tests$extra"
    groups=$((groups + 1))
    nfiles=$((nfiles + n))
    if [ $list -eq 1 ]; then
        echo "$cmd"
        continue
    fi
    echo "== group $groups ($n files): $cmd"
    # shellcheck disable=SC2086
    if [ $incl -eq 1 ]; then
        cargo test $flags $tests -- --include-ignored
    else
        cargo test $flags $tests
    fi || { echo "run-deqp-tests: group $groups FAILED" >&2; failed=$((failed + 1)); }
done

nfiles=$((nfiles + bad))
echo "$groups groups, $nfiles tests files, $failed failed"
[ "$failed" -eq 0 ]
