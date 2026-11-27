#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gm-perf-selftest.sh - feeds fixed PERF pages to imx8mp-gm-perf.py and compares the output.
#   bash tools/cts/imx8mp-gm-perf-selftest.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
py=(python3 -I "$here/imx8mp-gm-perf.py")
fail=0

check() { # name, want_rc, want_text, got_rc, got_text
	if [ "$2" != "$4" ] || [ "$3" != "$5" ]; then
		echo "FAIL $1: rc want $2 got $4"; echo "--- want"; echo "$3"; echo "--- got"; echo "$5"; fail=1
	else
		echo "ok   $1"
	fi
}
run_args() { local n=$1 rc=$2 want=$3; shift 3; local got; got=$("${py[@]}" "$@" 2>&1); check "$n" "$rc" "$want" $? "$got"; }
run_stdin() { local n=$1 rc=$2 want=$3 in=$4 got; got=$(printf '%s\n' "$in" | "${py[@]}" 2>&1); check "$n" "$rc" "$want" $? "$got"; }

# normal: 10 frames -> 1000 ms; frame 500 us, gl 300 us, swap 200 us
run_args normal-page0 0 "scene: shadow page 0
  ms a frame: 1000.000
  frames: 10 saturated=no
  frame us: 500 saturated=no
  gl us: 300 saturated=no
  swap us: 200 saturated=no" --scene shadow 594b0c00 01f4000a 00c8012c

# normal page 4: v3 = 15000 KiB
run_args normal-page4 0 "scene: refract page 4
  GPU mipmap chains /100: 0 saturated=no
  CPU mipmap fallbacks /100: 0 saturated=no
  CPU GPU-waits /100: 100 saturated=no
  KiB of CPU passes: 15000 saturated=no" --scene refract 594b0c04 00000000 3a980064

# saturated page 0 v0 is a bound; v2 (gl) saturates too
run_args saturated-page0-v0 0 "scene: terrain page 0
  ms a frame: <= 0.153 (bound: page 0 v0 saturated at 65535)
  frames: >= 65535 saturated=yes
  frame us: 10 saturated=no
  gl us: >= 65535 saturated=yes (65.5 ms or more)
  swap us: 1 saturated=no" --scene terrain 0x594b0c00 000affff 0001ffff

# page 4 without counters
run_args page4-unavailable 0 "scene: ? page 4
  KiB of CPU passes: not available (v0..v3 all 0xffff: no counters)" 594b0c04 ffffffff ffffffff

# page 4 v3 saturated alone
run_args saturated-page4-v3 0 "scene: ? page 4
  GPU mipmap chains /100: 0 saturated=no
  CPU mipmap fallbacks /100: 0 saturated=no
  CPU GPU-waits /100: 0 saturated=no
  KiB of CPU passes: >= 65535 saturated=yes" 594b0c04 00000000 ffff0000

# console lines on stdin, other lines skipped
run_stdin console-lines 0 "scene: shadow page 0
  ms a frame: 500.000
  frames: 20 saturated=no
  frame us: 100 saturated=no
  gl us: 50 saturated=no
  swap us: 25 saturated=no
scene: shadow page 4
  GPU mipmap chains /100: 0 saturated=no
  CPU mipmap fallbacks /100: 0 saturated=no
  CPU GPU-waits /100: 0 saturated=no
  KiB of CPU passes: 7 saturated=no" "boot noise
glmark2-perf: shadow: page 0: 594b0c00 00640014 00190032 00000000
glmark2-perf: shadow: page 4: 594b0c04 00000000 00070000 00000000"

# malformed
run_args bad-short 1 "imx8mp-gm-perf: need w0 w1 w2 [w3] (3 or 4 hex words), got 2" 594b0c00 ffffffff
run_args bad-hex 1 "imx8mp-gm-perf: not a 32-bit hex word: 'zzzz'" 594b0c00 zzzz 0
run_args bad-magic 1 "imx8mp-gm-perf: w0 12345678 is not a PERF record (want 0x594b0c0p)" 12345678 0 0
run_args bad-page 1 "imx8mp-gm-perf: page 5 is not 0..4" 594b0c05 0 0
run_args bad-zero-frames 1 "imx8mp-gm-perf: page 0 v0 = 0 frames: no ms a frame" 594b0c00 0 0
run_stdin bad-console 1 "imx8mp-gm-perf: line 1: malformed glmark2-perf line: 'glmark2-perf: shadow: page 0: 594b0c00 0064'" \
	"glmark2-perf: shadow: page 0: 594b0c00 0064"
run_stdin bad-page-mismatch 1 "imx8mp-gm-perf: scene shadow: console says page 4, w0 says page 0" \
	"glmark2-perf: shadow: page 4: 594b0c00 0001000a 00000000 00000000"
run_stdin bad-empty 1 "imx8mp-gm-perf: no glmark2-perf: line on stdin" "nothing here"

[ $fail = 0 ] && echo "imx8mp-gm-perf-selftest: all ok"
exit $fail
