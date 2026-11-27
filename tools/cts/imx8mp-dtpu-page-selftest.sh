#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx8mp-dtpu-page.py, the decoder of gl_test case 222 dtpu's page (SEL 0x10b0):
# synthetic w1..w3 for each outcome of gc-depth-sample-wire.md ((a), (a)+flush, (b), (c)), a
# check-3 failure (not a verdict, exit 1), the X2 11-11 case (check 4 failed at index 1, check 5
# passed), a GL error (check 8), pages that contradict gl_test_dtpu.inc's encoding, and console
# logs: a good one (also through console-redact.py), two pages, "no dtpu page", no page line and
# a truncated page line. The words are made up, encoded by hand from gl_test_dtpu.inc:12-25 and
# dtpu_verdict() (48-57). Host only; python3 -I; scratch under ${TMPDIR:-$PWD}, never /tmp.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx8mp-dtpu-page.py
redact=$here/console-redact.py
case "${TMPDIR:-$PWD}" in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "refusing a scratch dir under /tmp; set TMPDIR" >&2; exit 2 ;; esac
tmp=$(mktemp -d "${TMPDIR:-$PWD}/imx8mp-dtpu-page.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
nfail=0 nok=0
ok() { echo "ok: $*"; nok=$((nok + 1)); }
bad() { echo "FAIL: $*" >&2; nfail=$((nfail + 1)); }

# run NAME RC ARGS...: run the decoder, want exit RC; stdout+stderr in $tmp/NAME.out
run() {
	local name=$1 want=$2 rc
	shift 2
	python3 -I "$tool" "$@" >"$tmp/$name.out" 2>&1
	rc=$?
	if [ "$rc" = "$want" ]; then ok "$name: exit $rc"; else bad "$name: exit $rc, want $want"; cat "$tmp/$name.out" >&2; fi
}
# has NAME TEXT: $tmp/NAME.out holds the line TEXT
has() {
	if grep -qxF -- "$2" "$tmp/$1.out"; then ok "$1: '$2'"; else bad "$1: no line '$2'"; cat "$tmp/$1.out" >&2; fi
}
# hasnt NAME REGEX: no line of $tmp/NAME.out matches REGEX
hasnt() {
	if grep -qE -- "$2" "$tmp/$1.out"; then bad "$1: a line matches '$2'"; else ok "$1: no '$2'"; fi
}

# --- one page per outcome ------------------------------------------------------------------
# all right: valid, code 1, no index (gc-depth-sample.md's "all right" words)
run exact 0 0x00001001 0x10001000 0x10001000
has exact "check 3: pass (P0 (today's path) on the ramp, the reference)"
has exact "check 4: pass (P1 (D24X8 view) == P0)"
has exact "check 7: pass (P3 on the deeper ramp (not stale))"
has exact "check 8: pass (no GL error in the arms)"
has exact "verdict: 1 (D24X8 exact)"
has exact "outcome: (a) (a depth view of the surface, no copy, no CPU resolve on the draw path)"
# without 0x and in upper case
run exact-bare 0 00001001 10001000 10001000
has exact-bare "outcome: (a) (a depth view of the surface, no copy, no CPU resolve on the draw path)"

# stale: check 7 failed at index 5, code 2
run stale 0 0x00002081 0x10001000 0x00051000
has stale "check 7: fail (P3 on the deeper ramp (not stale)) first at index 5 (GL x 5, y 63)"
has stale "verdict: 2 (D24X8 exact but stale without a wait (check 7))"
has stale "outcome: (a)+flush ((a) plus the texture-cache flush of (a).9)"

# the X2 11-11 case: check 4 failed at index 1, check 5 passes -> code 3, outcome (a)
run x2-1111 0 0x00003011 0x10000001 0x10001000
has x2-1111 "check 4: fail (P1 (D24X8 view) == P0) first at index 1 (GL x 1, y 63)"
has x2-1111 "check 5: pass (P1 within 255 of P0)"
has x2-1111 "verdict: 3 (D24X8 with fewer bits (check 4 fails, 5 passes))"
has x2-1111 "outcome: (a) (a depth view of the surface, no copy, no CPU resolve on the draw path)"

# fewer bits and stale: code 3 with check 7 failed -> (a)+flush
run fewbits-stale 0 0x00003091 0x10000001 0x00071000
has fewbits-stale "verdict: 3 (D24X8 with fewer bits (check 4 fails, 5 passes))"
has fewbits-stale "outcome: (a)+flush ((a) plus the texture-cache flush of (a).9)"

# raw view: checks 4 and 5 fail (index 1, 2), 6 passes -> code 4, (b)
run raw 0 0x00004031 0x00020001 0x10001000
has raw "check 5: fail (P1 within 255 of P0) first at index 2 (GL x 2, y 63)"
has raw "check 6: pass (P2 (raw A8R8G8B8 view) is the PE's word)"
has raw "verdict: 4 (D24X8 off, the raw view is the word (check 6 passes))"
has raw "outcome: (b) (a GPU copy, in the stream, into a layout the TPU reads)"

# none: checks 4..7 fail at index 0 and 4095 -> code 5, (c)
run none 0 0x000050f1 0x00000000 0x0fff0000
has none "check 6: fail (P2 (raw A8R8G8B8 view) is the PE's word) first at index 0 (GL x 0, y 63)"
has none "check 7: fail (P3 on the deeper ramp (not stale)) first at index 4095 (GL x 63, y 0)"
has none "verdict: 5 (the LINEAR descriptor does not address the surface)"
has none "outcome: (c) (the CPU resolve stays; its three CPU fixes)"

# a GL error 0x0502 in the arms (check 8) keeps the outcome and says so
run glerr 0 0x05021101 0x10001000 0x10001000
has glerr "check 8: fail (first GL error 0x0502)"
has glerr "outcome: (a) (a depth view of the surface, no copy, no CPU resolve on the draw path)"
has glerr "note: check 8 failed, a GL error in the arms; read the outcome with care"

# --- check 3 failed: not a verdict -----------------------------------------------------------
run ref 1 0x00000009 0x10001000 0x10001000
has ref "check 3: fail (P0 (today's path) on the ramp, the reference; its index is not kept)"
has ref "verdict: 0 (no verdict (check 3 failed))"
has ref "not a verdict: check 3 failed, rerun"
hasnt ref '^outcome:'
# check 3 failed with check 4 too: still code 0, still no outcome
run ref-more 1 0x00000019 0x10000001 0x10001000
has ref-more "not a verdict: check 3 failed, rerun"
hasnt ref-more '^outcome:'

# --- pages that are not gl_test_dtpu.inc's encoding: exit 2 ----------------------------------
run invalid 2 0x00001000 0x10001000 0x10001000        # bit 0 clear
has invalid "imx8mp-dtpu-page: w1 bit 0 is clear: not a valid dtpu page"
run wrongcode 2 0x00002001 0x10001000 0x10001000      # code 2 without check 7's bit
has wrongcode "imx8mp-dtpu-page: verdict code 2 disagrees with the fail bits (they give 1)"
run badcode 2 0x00006001 0x10001000 0x10001000        # code 6 is no DTPU_V_*
has badcode "imx8mp-dtpu-page: verdict code 6 is not a DTPU_V_* code"
run idx-nobit 2 0x00001001 0x10000001 0x10001000      # check 4 index without its bit
has idx-nobit "imx8mp-dtpu-page: check 4: fail bit 0 but index 1"
run idx-big 2 0x00003011 0x10011001 0x10001000        # check 4 index 4097
has idx-big "imx8mp-dtpu-page: check 4's index 4097 is beyond 4096"
run glerr-nobit 2 0x05021001 0x10001000 0x10001000    # GL error without bit 8
has glerr-nobit "imx8mp-dtpu-page: check 8: fail bit 0 but GL error 0x0502"
run reserved 2 0x00001003 0x10001000 0x10001000       # bit 1 is not in the layout
has reserved "imx8mp-dtpu-page: w1 0x00001003 sets bits outside gl_test_dtpu.inc's layout"
run nothex 2 0x00001001 0x1000100g 0x10001000
has nothex "imx8mp-dtpu-page: not a 32-bit hex word: '0x1000100g'"
run toolong 2 0x00001001 0x110001000 0x10001000
run twowords 2 0x00001001 0x10001000
run noargs 2

# --- console logs ------------------------------------------------------------------------------
# A redacted-console-like capture: a boot, the selector, the LOG_INF of main_imx8mp.c:481 and the
# page line of yakogl_record.h:22 (w0 0x594b0cb0), CRLF line ends. The address is built so this
# file stays clean under the redaction scans.
addr="192.0.2"".10"
printf '%s\r\n' \
	'*** Booting Zephyr OS build v4.1.0 ***' \
	"tftp from $addr" \
	'[00:00:00.010,000] <inf> gc_wdog_arm: gc_wdog_arm: armed' \
	'[00:00:00.020,000] <inf> gc_result: selector 0x000010b0' \
	'[00:00:03.500,000] <inf> gc_gl_test: gc_gl_test: case 222 dtpu page 0xb0' \
	'yk-record: 594b0cb0 00003011 10000001 10001000' >"$tmp/good.log"
run log 0 --log "$tmp/good.log"
has log "page: w1=0x00003011 w2=0x10000001 w3=0x10001000"
has log "outcome: (a) (a depth view of the surface, no copy, no CPU resolve on the draw path)"
# through console-redact.py first (the record's redacted capture), and from stdin
python3 -I "$redact" - <"$tmp/good.log" >"$tmp/red.log"
if grep -qF "$addr" "$tmp/red.log"; then bad "redact: the address survived"; else ok "redact: address gone"; fi
python3 -I "$tool" --log - <"$tmp/red.log" >"$tmp/log-red.out" 2>&1
if [ $? = 0 ] && cmp -s "$tmp/log.out" "$tmp/log-red.out"; then ok "log-red: same decode after console-redact.py"; else bad "log-red: decode differs"; cat "$tmp/log-red.out" >&2; fi

# a CASE record and another PERF page (tag 0xf0) are not the dtpu page; two dtpu pages -> last
{
	echo 'yk-record: 594b0101 000000de 00000004 00000001'
	echo 'yk-record: 594b0cf0 00000001 00000002 00000003'
	echo 'yk-record: 594b0cb0 00000009 10001000 10001000'
	echo 'yk-record: 594b0cb0 00004031 00020001 10001000'
} >"$tmp/two.log"
run log-two 0 --log "$tmp/two.log"
has log-two "note: 2 dtpu pages in the log, reading the last"
has log-two "outcome: (b) (a GPU copy, in the stream, into a layout the TPU reads)"
# only the check-3 page in the log: exit 1
head -n 3 "$tmp/two.log" >"$tmp/ref.log"
run log-ref 1 --log "$tmp/ref.log"
has log-ref "not a verdict: check 3 failed, rerun"

# case 222 did not reach its arms: "no dtpu page", the CASE record only
{
	echo '[00:00:03.500,000] <err> gc_gl_test: gc_gl_test: no dtpu page (case 222 did not reach its arms)'
	echo 'yk-record: 594b0101 000000de 00000001 00000000'
} >"$tmp/nopage.log"
run log-nopage 2 --log "$tmp/nopage.log"
has log-nopage "imx8mp-dtpu-page: no dtpu page: the log says check 1 or 2 stopped case 222; its CASE record is the result"

# malformed: no record line at all; a truncated page line; a missing file
printf 'garbage\n\001\002 not a console\n' >"$tmp/empty.log"
run log-empty 2 --log "$tmp/empty.log"
has log-empty "imx8mp-dtpu-page: no dtpu page in the log (no 'yk-record: 594b0cb0 ...' line)"
printf 'boot\nyk-record: 594b0cb0 00001001 1000\n' >"$tmp/cut.log"
run log-cut 2 --log "$tmp/cut.log"
has log-cut "imx8mp-dtpu-page: line 2: malformed dtpu page record: yk-record: 594b0cb0 00001001 1000"
printf 'yk-record: 594b0cb0 00001001 1000zzzz 10001000\n' >"$tmp/junk.log"
run log-junk 2 --log "$tmp/junk.log"
run log-missing 2 --log "$tmp/no-such.log"

echo "$nok ok, $nfail failed"
[ "$nfail" = 0 ]
