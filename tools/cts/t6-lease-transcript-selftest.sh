#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of t6-lease-transcript.py on synthetic, redacted fixtures.
set -u
here=$(cd "$(dirname "$0")" && pwd)
T="$here/t6-lease-transcript.py"
F="$here/testdata/t6-lease-transcript"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/t6lt.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
run() { python3 -I "$T" "$@" >"$tmp/stdout" 2>"$tmp/stderr"; }
expect_rc() { # name want_rc args...
  local n=$1 want=$2; shift 2
  run "$@"; local rc=$?
  if [ "$rc" = "$want" ]; then ok "$n"; else bad "$n (rc $rc, want $want)"; fi
}
expect_out() { # name expected-string-file
  if printf '%b' "$2" | cmp -s - "$tmp/out"; then ok "$1"; else bad "$1"; diff <(printf '%b' "$2") "$tmp/out"; fi
}

# 1. CRLF, NUL, 0xFF (escaped and raw), cut at the default from-line
expect_rc crlf 0 "$tmp/out" "$F/crlf.txt"
expect_out crlf-content 'Hit any key to stop autoboot:  0 \n=> md.l 0xfd8d814c 1\nfd8d814c: 0000fff9  ....\n=> \n'

# 2. two chunks joined, given in reverse order
expect_rc two-chunks 0 "$tmp/out" "$F/b.json" "$F/a.json"
expect_out two-chunks-content 'Hit any key to stop autoboot:  0 \n=> md.l 0xfd8d814c 1\nfd8d814c: 0000fff9  ....\n=> md.l 0xfd8d810c 1\nfd8d810c: 00000000  ....\n=> boot-done\n=> \n'

# 3. gap and overlap refuse with exit 4
expect_rc gap 4 "$tmp/out" "$F/a.json" "$F/gap.json"
expect_rc overlap 4 "$tmp/out" "$F/a.json" "$F/overlap.json"
expect_rc no-offsets-several 4 "$tmp/out" "$F/a.json" "$F/crlf.txt"

# 4. cut lines
expect_rc to-line 0 "$tmp/out" "$F/a.json" "$F/b.json" --to-line 'md\.l 0xfd8d810c'
expect_out to-line-content 'Hit any key to stop autoboot:  0 \n=> md.l 0xfd8d814c 1\nfd8d814c: 0000fff9  ....\n=> md.l 0xfd8d810c 1\n'
expect_rc from-line 0 "$tmp/out" "$F/a.json" "$F/b.json" --from-line '^=> md\.l 0xfd8d810c' --to-line 'boot-done'
expect_out from-line-content '=> md.l 0xfd8d810c 1\nfd8d810c: 00000000  ....\n=> boot-done\n'
expect_rc from-offset 0 "$tmp/out" "$F/a.json" "$F/b.json" --from-offset 69 --from-line '^fd8d814c'
expect_out from-offset-content 'fd8d814c: 0000fff9  ....\n=> md.l 0xfd8d810c 1\nfd8d810c: 00000000  ....\n=> boot-done\n=> \n'
expect_rc no-from-match 3 "$tmp/out" "$F/a.json" --from-line 'never-there'
expect_rc to-line-missing 3 "$tmp/out" "$F/a.json" --to-line 'never-there'

# 5. usage and unreadable input
expect_rc usage 2 "$tmp/out"
expect_rc bad-option 2 "$tmp/out" "$F/a.json" --nope
expect_rc missing-file 3 "$tmp/out" "$tmp/does-not-exist"

[ "$fail" = 0 ] && echo "t6-lease-transcript-selftest: all passed"
exit "$fail"
