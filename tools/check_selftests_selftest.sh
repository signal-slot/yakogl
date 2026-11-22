#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/check_selftests.py: a stand-in tree with a gate, the
# two self-tests that run others and one self-test per way of being named or
# not, and the list the tool prints for it. No row names this file: the gate
# runs it as one of the self-tests the tool lists.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/check_selftests.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/check_selftests.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok: $1"; }
bad() { echo "FAIL: $1"; fail=1; }

tree="$tmp/tree"
mkdir -p "$tree/tools/cts" "$tree/driver/tools/kexec-zephyr" "$tree/a" "$tree/b" "$tree/third_party/x" || exit 1
cat > "$tree/tools/check.sh" <<'GATE'
path_scopes() {
  case "$1" in
  tools/map_selftest.sh) echo tools ;;
  esac
}
# step "in a comment" bash tools/comment_selftest.sh
step "a row" bash tools/row_selftest.sh
step "a longer name" bash tools/long-short-selftest.sh
step "one of two selftest.sh" a/selftest.sh
step "runs others" tools/cts/cts-selftest.sh
step "runs others" driver/tools/kexec-zephyr/board_lock_selftest.sh
GATE
echo 'bash "$HERE/child-selftest.sh"' > "$tree/tools/cts/cts-selftest.sh"
echo 'sh "$here/lock_child_selftest.sh"' > "$tree/driver/tools/kexec-zephyr/board_lock_selftest.sh"
echo 'bash tools/grandchild_selftest.sh' > "$tree/tools/other_selftest.sh"
for f in tools/map_selftest.sh tools/comment_selftest.sh tools/row_selftest.sh tools/long-short-selftest.sh \
  tools/short-selftest.sh a/selftest.sh b/selftest.sh tools/cts/child-selftest.sh \
  driver/tools/kexec-zephyr/lock_child_selftest.sh tools/grandchild_selftest.sh tools/py_selftest.py \
  third_party/x/vendor_selftest.sh tools/selftest_notes.md; do
  : > "$tree/$f"
done
git -C "$tree" init -q && git -C "$tree" add -A || exit 1
: > "$tree/tools/untracked_selftest.sh"

# Named by a row, by a self-test that runs others, vendored, untracked or not
# a self-test by name: not listed. The rest is.
want="b/selftest.sh
tools/comment_selftest.sh
tools/grandchild_selftest.sh
tools/map_selftest.sh
tools/other_selftest.sh
tools/py_selftest.py
tools/short-selftest.sh"
got=$(cd "$tree" && python3 -I "$tool" 2> "$tmp/err")
rc=$?
if [ "$rc" -eq 0 ] && [ "$got" = "$want" ]; then
  ok "the list of the stand-in tree"
else
  bad "the list of the stand-in tree: rc $rc"
  diff <(echo "$want") <(echo "$got")
  cat "$tmp/err"
fi

if (cd "$tree" && python3 -I "$tool" extra > "$tmp/out" 2> "$tmp/err"); then
  bad "an argument is accepted"
elif grep -q '^usage: ' "$tmp/err" && [ ! -s "$tmp/out" ]; then
  ok "an argument is refused with the usage"
else
  bad "an argument is refused without the usage"
fi

# The path-to-scope map is found by its name: a gate without exactly one is
# refused, never read as if every path in it were run.
printf 'path_scopes() {\n  :\n}\n' >> "$tree/tools/check.sh"
if (cd "$tree" && python3 -I "$tool" > "$tmp/out" 2> "$tmp/err"); then
  bad "a gate with two path_scopes() is accepted"
elif grep -q 'has 2 path_scopes() functions, not 1' "$tmp/err" && [ ! -s "$tmp/out" ]; then
  ok "a gate with two path_scopes() is refused"
else
  bad "a gate with two path_scopes() is refused without saying so"
  cat "$tmp/err"
fi

[ $fail = 0 ] && echo "check_selftests selftest: PASS" || echo "check_selftests selftest: FAIL"
exit $fail
