#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of pr-regress-compare.py on small synthetic logs.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/pr-regress-compare.py"
t=$(mktemp -d) || exit 2
trap 'rm -rf "$t"' EXIT
fail=0

# mklog FILE CASE2-VERDICT RECOVERIES [omit]
mklog() {
    {
        echo "10:00:00 gl_test: case 1 clear: PASS"
        echo "10:00:01 gl_test: case 2 blend: $2"
        echo "10:00:02 gl_test: case 3 skipme: SKIP (no config)"
        echo "10:01:00 [jellyfish] <default>: FPS: 40 FrameTime: 25.112 ms"
        if [ "${4:-}" != omit ]; then
            echo "10:01:10 [terrain] <default>: FPS: 6 FrameTime: 166.667 ms"
        fi
        echo "10:02:00 glmark2: score 201: 29 scenes ran, 0 failed, 4 unsupported (33 list entries)"
        echo "10:02:01 render freelists: 3 grow requests, 2 served, 1 unserved, 2072 KiB taken on demand; GPU recoveries $3 (HWR counter $3)"
    } > "$1"
}

# check NAME WANT-EXIT WANT-LAST-LINE ARGS...
check() {
    name=$1; want=$2; last=$3; shift 3
    out=$(python3 "$tool" "$@" 2>&1); rc=$?
    got=$(printf '%s\n' "$out" | tail -n 1)
    if [ "$rc" != "$want" ] || [ "$got" != "$last" ]; then
        echo "FAIL $name: exit $rc (want $want), last line '$got' (want '$last')"
        fail=1
    else
        echo "ok   $name"
    fi
}

mklog "$t/a.txt" PASS 0
mklog "$t/b.txt" PASS 0
mklog "$t/c.txt" FAIL 0
mklog "$t/d.txt" PASS 1
mklog "$t/e.txt" PASS 0 omit
mkdir "$t/rec"
cp "$t/a.txt" "$t/rec/console.txt"

check same 0 "pr-regress: SAME" "$t/a.txt" "$t/b.txt"
check record-dir 0 "pr-regress: SAME" "$t/rec" "$t/b.txt"
check differing-case 1 "pr-regress: DIFFERENT (1 cases, 0 resets)" "$t/a.txt" "$t/c.txt"
check new-reset 1 "pr-regress: DIFFERENT (0 cases, 1 resets)" "$t/a.txt" "$t/d.txt"
check missing-scene 1 "pr-regress: DIFFERENT (0 cases, 0 resets)" "$t/a.txt" "$t/e.txt"
check usage 2 "usage: pr-regress-compare.py <run> <run> [<run> ...]" "$t/a.txt"
if ! python3 "$tool" "$t/a.txt" "$t/e.txt" | grep -q 'terrain.*run2=missing'; then
    echo "FAIL missing-scene: scene not reported missing"
    fail=1
fi
exit $fail
