#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-dtpu-page-selftest.sh
#
# Host-only check of tools/cts/imx8mp-dtpu-page.py, the decoder of gl_test
# case 222's dtpu page (docs/notes/gc-depth-sample.md, "Reading the dtpu
# page"). The note's two pages (all right; the 11-11 failure with check 5
# passing), one page per verdict code and outcome, a check-3 failure (no
# verdict), inconsistent words, console captures with and without the page,
# and usage errors. Then the writer: every one of the 64 pass/fail sets of
# checks 3..8, encoded by dtpu_encode() of driver/tools/pvrlab/gl_test_dtpu.inc
# (built with the host's cc, as driver/tools/pvrlab/dtpu_record_test.c
# documents), must decode to the writer's own bits and code. Without a cc
# that builds the .inc the same 64 pages are made from dtpu_encode() restated
# in python, and the skip says so (the .inc's compilability is the nightly
# `cc gl_test.c` row's, gl_test.c includes it).
# Writes only under a mktemp directory of $TMPDIR (or of the current directory
# without one) and removes it.
set -u

HERE=$(dirname "$0")
DEC=$HERE/imx8mp-dtpu-page.py
INC=$HERE/../../driver/tools/pvrlab/gl_test_dtpu.inc
D=$(mktemp -d "${TMPDIR:-.}/dtpu-page.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

# run <name> <expected status> <regex that must match a line>... -- <args>
run() {
  local name=$1 want=$2 rc
  shift 2
  local pats=()
  while [ "$1" != -- ]; do pats+=("$1"); shift; done
  shift
  python3 -I "$DEC" "$@" > "$D/out" 2>&1
  rc=$?
  if [ "$rc" != "$want" ]; then
    bad "$name: exit $rc, want $want"; sed 's/^/        /' "$D/out"; return
  fi
  local p
  for p in "${pats[@]}"; do
    if ! grep -qE -- "$p" "$D/out"; then
      bad "$name: no line /$p/"; sed 's/^/        /' "$D/out"; return
    fi
  done
  ok "$name"
}

run "all right (the note's page)" 0 \
  '^check 3: pass' '^check 4: pass' '^check 7: pass' '^check 8: pass' \
  '^verdict: 1 D24X8 exact$' '^outcome: \(a\)$' -- 0x00001001 0x10001000 0x10001000
run "11-11 rerun: check 4 fails at 1, check 5 passes (code 3)" 0 \
  '^check 4: FAIL .*first at index 1 \(GL x 1, y 63\)' '^check 5: pass' \
  '^verdict: 3 ' '^outcome: \(a\)$' -- 0x00003011 0x10000001 0x10001000
run "code 3 with check 7 failed: (a) plus flush" 0 \
  '^check 7: FAIL .*index 4095 \(GL x 63, y 0\)' '^verdict: 3 ' '^outcome: \(a\) plus flush$' \
  -- 0x00003091 0x10000001 0x0fff1000
run "code 2: stale without a wait" 0 \
  '^check 7: FAIL .*index 64 \(GL x 0, y 62\)' '^verdict: 2 ' '^outcome: \(a\) plus flush$' \
  -- 2081 10001000 00401000
run "code 4: raw view is the word, (b)" 0 \
  '^check 4: FAIL' '^check 5: FAIL .*index 2' '^check 6: pass' '^verdict: 4 ' '^outcome: \(b\)$' \
  -- 0x00004031 0x00020002 0x10001000
run "code 5: nothing addresses the surface, (c)" 0 \
  '^check 6: FAIL .*index 0 ' '^verdict: 5 ' '^outcome: \(c\)$' -- 0x00005071 0x00000000 0x10000000
run "check 8 with code 1 keeps the GL error" 0 \
  '^check 8: FAIL .*first GL error 0x0502' '^verdict: 1 ' '^outcome: \(a\)$' \
  -- 0x05021101 0x10001000 0x10001000
run "check 3 failed: no verdict, rerun" 1 \
  '^check 3: FAIL' '^verdict: 0 ' '^outcome: none: check 3 failed.*rerun' -- 0x00000009 0x10001000 0x10001000
run "page not valid (bit 0 clear)" 1 \
  '^inconsistent: w1 bit 0 clear' '^outcome: none: the page' -- 0x00001000 0x10001000 0x10001000
run "code disagrees with the bits" 1 \
  '^inconsistent: verdict code 1, but the failed checks give 3' -- 0x00001011 0x10000001 0x10001000
run "failed bit without its index" 1 \
  '^inconsistent: check 4 is FAIL but its index is none' -- 0x00003011 0x10001000 0x10001000
run "index without its failed bit" 1 \
  '^inconsistent: check 6 is pass but its index is 5' -- 0x00001001 0x10001000 0x10000005
run "index beyond the target" 1 \
  '^inconsistent: check 5 index 4097 is beyond the 64x64 target' -- 0x00004031 0x10010001 0x10001000
run "GL error without check 8" 1 \
  '^inconsistent: check 8 is pass but the GL error is 0x0500' -- 0x05001001 0x10001000 0x10001000
run "reserved bit set" 1 \
  '^inconsistent: w1 reserved bits set: 0x00000002' -- 0x00001003 0x10001000 0x10001000

# console captures, as board.sh's console.txt ("HH:MM:SS line")
cat > "$D/console.txt" <<'EOF'
10:00:09 *** Booting Zephyr OS build v4.1.0 ***
10:00:09 gc_wdog_arm: armed WCR 0x0000 -> 0x0f84 (was stopped), WMCR 0x0000, flags 0x1: expiry 600000 ms, fed for 590 s
10:00:10 yk-record: 594b0e40 00000000 00000000 00000000
10:00:12 [00:00:02.100,000] <inf> gc_gl_test: case 222 dtpu page 0xb0
10:00:12 yk-record: 594b0cb0 00003011 10000001 10001000
EOF
run "log: the page line of a record directory's console.txt" 0 \
  '^page: w1 0x00003011 w2 0x10000001 w3 0x10001000$' '^verdict: 3 ' -- --log "$D"
run "log: the same from stdin" 0 '^verdict: 3 ' -- --log - < "$D/console.txt"
printf '10:00:20 yk-record: 594b0cb0 00001001 10001000 10001000\n' >> "$D/console.txt"
run "log: two pages, the last is read" 0 '^note: 2 dtpu pages' '^verdict: 1 ' -- --log "$D/console.txt"
cat > "$D/nopage.txt" <<'EOF'
10:00:12 [00:00:02.100,000] <err> gc_gl_test: no dtpu page (case 222 did not reach its arms)
10:00:12 yk-record: 594b0101 00000002 00000002 00000000
EOF
run "log: no dtpu page, the case's own record" 3 "^no dtpu page: the log says 'no dtpu page'" -- --log "$D/nopage.txt"
printf '10:00:12 yk-record: 594b0cb1 00001001 10001000 10001000\n' > "$D/othertag.txt"
run "log: a PERF page of another tag is not the dtpu page" 3 '^no dtpu page: no yk-record line' -- --log "$D/othertag.txt"
run "usage: a word that is not hex" 2 '^error: not a 32-bit hex word' -- 0x1001 0xzz 0x10001000
run "usage: a word wider than 32 bits" 2 '^error: not a 32-bit hex word' -- 0x100000001 0 0
run "usage: two words" 2 '^usage:' -- 0x1001 0x10001000
run "usage: an unreadable log" 2 '^error: ' -- --log "$D/no-such-file"

# the writer: dtpu_encode() over all 64 pass/fail sets of checks 3..8. The
# C encoder of gl_test_dtpu.inc is the board's own; a host cc that cannot
# build it says so once and the same 64 pages come from dtpu_encode()
# restated in python, so the decoder's table is checked either way.
pages_from_cc=0
CC=${CC:-cc}
if command -v "$CC" > /dev/null 2>&1; then
  cat > "$D/enc.c" <<'EOF'
#include <stdio.h>
#include "gl_test_dtpu.inc"
int main(void)
{
	/* each failing check's first index: distinct, inside the target */
	static const uint32_t at[5] = {7u, 1u, 4095u, 64u, 2000u};
	for (unsigned m = 0; m < 64u; m++) {
		uint32_t b[5], w[3];
		for (unsigned k = 0; k < 5u; k++)
			b[k] = (m >> k & 1u) ? at[k] : DTPU_NONE;
		dtpu_encode(b[0], b[1], b[2], b[3], b[4], (m >> 5 & 1u) ? 0x0505u : 0u, w);
		printf("%u %u %08x %08x %08x\n", m, (unsigned)dtpu_verdict(b[0], b[1], b[2], b[3], b[4]), w[0], w[1], w[2]);
	}
	return 0;
}
EOF
  if "$CC" -std=c99 -Wall -Werror -O2 -I"$(dirname "$INC")" "$D/enc.c" -o "$D/enc" > "$D/cc.log" 2>&1 \
     && "$D/enc" > "$D/pages"; then
    pages_from_cc=1
  else
    printf '  skip  writer: %s cannot build gl_test_dtpu.inc (the nightly cc gl_test.c row pins that build); the 64 pages come from the python restatement\n' "$CC"
    sed 's/^/        /' "$D/cc.log"
  fi
else
  printf '  skip  writer: no %s for gl_test_dtpu.inc; the 64 pages come from the python restatement\n' "$CC"
fi
if [ "$pages_from_cc" = 0 ]; then
  # dtpu_verdict() and dtpu_encode() of gl_test_dtpu.inc, restated. The same
  # index table as the C writer above.
  python3 -I > "$D/pages" <<'EOF'
NONE = 4096
AT = [7, 1, 4095, 64, 2000]  # the failing checks' first indices, checks 3..7
for m in range(64):
    b = [AT[k] if m >> k & 1 else NONE for k in range(5)]  # bad3..bad7
    code = 0 if b[0] != NONE else (2 if b[4] != NONE else 1) if b[1] == NONE \
        else 3 if b[2] == NONE else 4 if b[3] == NONE else 5
    bad8 = 0x0505 if m >> 5 & 1 else 0
    bits = 1 | (1 if b[0] != NONE else 0) << 3 | (1 if b[1] != NONE else 0) << 4 \
        | (1 if b[2] != NONE else 0) << 5 | (1 if b[3] != NONE else 0) << 6 \
        | (1 if b[4] != NONE else 0) << 7 | (1 if bad8 else 0) << 8
    w0 = bits | code << 12 | (bad8 & 0xFFFF) << 16
    w1 = (b[1] & 0xFFFF) | (b[2] & 0xFFFF) << 16
    w2 = (b[3] & 0xFFFF) | (b[4] & 0xFFFF) << 16
    print("%d %d %08x %08x %08x" % (m, code, w0, w1, w2))
EOF
fi
if [ ! -s "$D/pages" ]; then
  bad "writer: no pages were made"
else
  n=0 wrong=0
  while read -r m code w1 w2 w3; do
    n=$((n + 1))
    want=0; [ "$code" = 0 ] && want=1
    out=$(python3 -I "$DEC" "$w1" "$w2" "$w3" 2>&1); rc=$?
    exp=""
    for k in 3 4 5 6 7 8; do
      if [ $((m >> (k - 3) & 1)) = 1 ]; then exp="$exp $k:FAIL"; else exp="$exp $k:pass"; fi
    done
    got=$(printf '%s\n' "$out" | sed -n 's/^check \([3-8]\): \(pass\|FAIL\).*/\1:\2/p' | tr '\n' ' ')
    if [ "$rc" != "$want" ] || [ " $got" != "$exp " ] ||
       ! printf '%s\n' "$out" | grep -q "^verdict: $code " ||
       printf '%s\n' "$out" | grep -q '^inconsistent'; then
      wrong=$((wrong + 1))
      [ "$wrong" -le 3 ] && printf '        set %s code %s: exit %s\n%s\n' "$m" "$code" "$rc" "$(printf '%s\n' "$out" | sed 's/^/          /')"
    fi
  done < "$D/pages"
  if [ "$n" = 64 ] && [ "$wrong" = 0 ]; then
    if [ "$pages_from_cc" = 1 ]; then
      ok "writer: the 64 pages of dtpu_encode() decode to their own checks and code"
    else
      ok "writer: the 64 pages of the restated dtpu_encode() decode to their own checks and code"
    fi
  else
    bad "writer: $wrong of $n encoded pages decode wrong (64 expected)"
  fi
fi

echo "imx8mp-dtpu-page-selftest: $pass ok, $fail failed"
[ "$fail" = 0 ]
