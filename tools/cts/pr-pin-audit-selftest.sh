#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-pin-audit-selftest.sh
#
# Host-only checks of tools/cts/pr-pin-audit.py on the fixture trees in
# tools/cts/testdata/pr-pin-audit/: `pinned` (every PR constant has a PIN:
# exit 0) and `unpinned` (PM_THING, used in an `if pr` block, has none: exit 1).
# UNRELATED is named in a comment and in a test module only, so it must not be
# listed.
set -u

HERE=$(dirname "$0")
FIX=$HERE/testdata/pr-pin-audit
TOOL=$HERE/pr-pin-audit.py
fail=0

check() { # name root want-exit pattern...
    local name=$1 root=$2 want=$3 out rc
    shift 3
    out=$(python3 -I "$TOOL" --root "$FIX/$root" 2>&1)
    rc=$?
    if [ "$rc" != "$want" ]; then
        echo "FAIL $name: exit $rc, want $want"
        fail=1
        return
    fi
    for pat in "$@"; do
        if ! printf "%s\n" "$out" | grep -q -- "$pat"; then
            echo "FAIL $name: missing '$pat'"
            fail=1
            return
        fi
    done
    if printf "%s\n" "$out" | grep -q "^[a-zA-Z]* UNRELATED"; then
        echo "FAIL $name: UNRELATED listed"
        fail=1
        return
    fi
    echo "ok   $name"
}

check pinned pinned 0 \
    "^pinned CCB_CMD_TYPE_3D_PR " "^pinned RTA_CTL_FLAGS " \
    "^pinned PM_THING driver/rust/src/gpu/pvr/render.rs:6 " "3 constants, 3 pinned, 0 unpinned"
check unpinned unpinned 1 \
    "^UNPINNED PM_THING driver/rust/src/gpu/pvr/render.rs:6 " "^pinned RTA_CTL_FLAGS " \
    "3 constants, 2 pinned, 1 unpinned"

exit $fail
