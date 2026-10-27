#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: the glmark2 image's frame-breakdown record on the host
# (docs/notes/imx8mp-frame-floor.md, "The frame-breakdown record"). Compiles
# perf_page_test.c against src/gm_perf_page.h, the header main.cpp packs the
# PERF page with, and checks that known totals give known words. Then it
# decodes the four pages with `yakogl-tools record` and checks each line
# ("experiment 0 page p: v0 .. v5"). Host only, a few seconds once
# yakogl-tools is built. Exit 1 on a difference.
#
#   run.sh [outdir]      outdir: default a new directory under ${TMPDIR:-/tmp}
#
# YAKOGL_TOOLS names a yakogl-tools binary; without it the binary is built
# with cargo from this checkout (as tools/board.sh does).
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
top=$(cd "$here/../../../../.." && pwd)
out=${1:-$(mktemp -d "${TMPDIR:-/tmp}/gm_perf_page.XXXXXX")}
mkdir -p "$out"
cc=${CC:-cc}

"$cc" -std=c11 -O1 -Wall -Wextra -Werror -I"$here/../src" \
  -I"$top/driver/zephyr/lib/board/include" -o "$out/perf_page_test" "$here/perf_page_test.c"
"$out/perf_page_test" | tee "$out/pages.txt"

if [ -z "${YAKOGL_TOOLS:-}" ]; then
  (cd "$top" && cargo build -q -p yakogl-tools)
  td=$(cd "$top" && cargo metadata --no-deps --format-version 1 |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
  YAKOGL_TOOLS=$td/debug/yakogl-tools
fi

# the decoded line of each page perf_page_test.c printed, in its order
want=(
  "experiment 0 page 0: 587 17018 100 200 16000 17018"
  "experiment 0 page 1: 1500 50 3 2000 100 50"
  "experiment 0 page 2: 581 7 3 17018 587 10000"
  "experiment 0 page 3: 13614 10210 2048 1875 800 0"
)
i=0 bad=0
while read -r _ w0 w1 w2 w3; do
  got=$("$YAKOGL_TOOLS" record "$w0" "$w1" "$w2" "$w3")
  echo "$got"
  case $got in
    *"${want[$i]}"*) ;;
    *) echo "FAIL decode of page $i: want '${want[$i]}'"; bad=1 ;;
  esac
  i=$((i + 1))
done < <(grep '^yk-record: ' "$out/pages.txt")
[ "$i" = "${#want[@]}" ] || { echo "FAIL: $i pages decoded, want ${#want[@]}"; bad=1; }
[ "$bad" = 0 ] && echo "gm_perf_page: ok (packed and decoded)"
exit "$bad"
