#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host test of src/pattern.c: the bars it draws, in every format and with a
# padded stride, are judged by display_check's colorbars.py (PASS, 8 bars in
# its order), and pattern_bar_rgb() equals colorbars.py's BARS table.
#   tools/probes/stm32mp257f_dk/mp2_display/test/pattern_test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
judge=$here/../../display_check/colorbars.py
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cc -std=c11 -Wall -Wextra -Werror -o "$tmp/pt" "$here/pattern_test.c" "$here/../src/pattern.c"
ok=0
failed=0
want="colorbars: PASS: 8 bars white yellow cyan green magenta red blue black"
check() { # name W H STRIDE fmt
  local out
  "$tmp/pt" "$2" "$3" "$4" "$5" >"$tmp/s.ppm" || { echo "FAIL $1: pattern_test"; failed=$((failed + 1)); return; }
  out=$(python3 -I "$judge" "$tmp/s.ppm" || true)
  if [ "$out" = "$want" ]; then echo "ok   $1"; ok=$((ok + 1)); else echo "FAIL $1: $out"; failed=$((failed + 1)); fi
}
check "1280x720 argb8888" 1280 720 5120 argb8888
check "1280x720 argb8888 stride 5184" 1280 720 5184 argb8888
check "1280x720 rgb888" 1280 720 3840 rgb888
check "1280x720 rgb565" 1280 720 2560 rgb565
check "1920x1080 argb8888" 1920 1080 7680 argb8888
check "800x480 rgb565 stride 1664" 800 480 1664 rgb565
# The table equals colorbars.py's BARS.
cat >"$tmp/tab.c" <<'EOC'
#include "pattern.h"
#include <stdio.h>
int main(void)
{
	uint8_t c[3];

	for (unsigned int i = 0; i < 8; i++) {
		pattern_bar_rgb(i, c);
		printf("%u,%u,%u\n", c[0], c[1], c[2]);
	}
	return 0;
}
EOC
cc -std=c11 -Wall -Wextra -Werror -I"$here/../src" -o "$tmp/tab" "$tmp/tab.c" "$here/../src/pattern.c"
"$tmp/tab" >"$tmp/c.txt"
python3 -I - "$judge" >"$tmp/p.txt" <<'EOP'
import importlib.util, sys
s = importlib.util.spec_from_file_location("cb", sys.argv[1])
m = importlib.util.module_from_spec(s)
s.loader.exec_module(m)
for _, c in m.BARS:
    print("%d,%d,%d" % c)
EOP
if cmp -s "$tmp/c.txt" "$tmp/p.txt"; then
  echo "ok   bar table equals colorbars.py BARS"; ok=$((ok + 1))
else
  echo "FAIL bar table"; failed=$((failed + 1))
fi
echo "pattern_test: $ok ok, $failed failed"
[ "$failed" -eq 0 ]
