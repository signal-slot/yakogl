#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/website-check.sh: its README comparison must fail when a
# data.js figure drifts from README.md's glmark2 or dEQP-GLES2 table, and pass
# on the unmodified tree.
#
# Each case runs in a scratch git worktree of HEAD under $TMPDIR (never /tmp),
# so the checker's git-history tests see the real repository. The working
# tree's website/, README.md and tools/website-check.sh are copied in first,
# so uncommitted edits are what is tested. The worktree is removed on exit.
#
# Usage: tools/website_check_selftest.sh     Exit 0 when every case behaves.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRATCH_PARENT="${TMPDIR:-$ROOT/target/tmp}"
case "$SCRATCH_PARENT" in /tmp|/tmp/*|/var/tmp|/var/tmp/*)
    SCRATCH_PARENT="$ROOT/target/tmp" ;;
esac
mkdir -p "$SCRATCH_PARENT"
WT="$(mktemp -d "$SCRATCH_PARENT/website-check-selftest.XXXXXX")"
rmdir "$WT"
cleanup() {
    git -C "$ROOT" worktree remove --force "$WT" >/dev/null 2>&1 || rm -rf "$WT"
    git -C "$ROOT" worktree prune >/dev/null 2>&1 || true
}
trap cleanup EXIT
git -C "$ROOT" worktree add --detach --quiet "$WT" HEAD

reset_tree() {
    rm -rf "$WT/website"
    cp -R "$ROOT/website" "$WT/website"
    cp "$ROOT/README.md" "$WT/README.md"
    cp "$ROOT/tools/website-check.sh" "$WT/tools/website-check.sh"
}

# mutate FIGURE KEY VALUE: set figures[FIGURE].KEY (KEY may be value.pass) in
# the worktree's data.js, keeping the rest of the file byte for byte.
mutate() {
    python3 -I - "$WT/website/data.js" "$1" "$2" "$3" <<'PY'
import json, re, sys
path, fid, key, new = sys.argv[1:]
with open(path, encoding="utf-8") as f:
    text = f.read()
start = text.index(json.dumps(fid) + ": {")
field = key.split(".")[-1]
m = re.compile(r'("' + re.escape(field) + r'": )("[^"]*"|-?\d+)').search(text, start)
if m is None:
    sys.exit(f"selftest: no {key} in {fid}")
value = json.dumps(int(new)) if new.isdigit() else json.dumps(new)
text = text[:m.start(2)] + value + text[m.end(2):]
with open(path, "w", encoding="utf-8") as f:
    f.write(text)
PY
}

fails=0
pass_case() { echo "ok   $1"; }
fail_case() { echo "FAIL $1"; fails=$((fails + 1)); }

# expect_fail NAME FIGURE KEY VALUE MESSAGE
expect_fail() {
    reset_tree
    mutate "$2" "$3" "$4"
    local out rc=0
    out="$(bash "$WT/tools/website-check.sh" 2>&1)" || rc=$?
    if [ "$rc" -ne 0 ] && grep -qF -- "$5" <<<"$out"; then
        pass_case "$1: exit $rc, '$5'"
    else
        fail_case "$1: exit $rc, expected '$5' in:"
        sed 's/^/    /' <<<"$out"
    fi
}

reset_tree
rc=0
out="$(bash "$WT/tools/website-check.sh" 2>&1)" || rc=$?
if [ "$rc" -eq 0 ]; then
    pass_case "unmodified tree: $out"
else
    fail_case "unmodified tree: exit $rc"
    sed 's/^/    /' <<<"$out"
fi

expect_fail "glmark2.pi4 value 194" glmark2.pi4 value 194 \
    "data.js: glmark2.pi4: value is 194, README.md has "
expect_fail "glmark2.am62p commit changed" glmark2.am62p commit deadbeef \
    "data.js: glmark2.am62p: commit is 'deadbeef', README.md has "
expect_fail "deqp2.imx8mp pass changed" deqp2.imx8mp value.pass 17317 \
    "data.js: deqp2.imx8mp: value.pass is 17317, README.md has "

if [ "$fails" -ne 0 ]; then
    echo "website_check_selftest: $fails case(s) failed"
    exit 1
fi
echo "website_check_selftest: ok (4 cases)"
