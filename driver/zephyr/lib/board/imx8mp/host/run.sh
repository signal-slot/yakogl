#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: build latch_model.c against a board_imx8mp.c at
# CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS 1, 2 and 3 (and the Slint window
# with and without CONFIG_YAKOGL_BOARD_SCANOUT_VIEW) and print the panel
# periods a frame of each application takes, without .wait_vblank in the
# GC7000's OSAL (unwired, main before 2026-10-17) and with it (wired). Host
# only, about 30 s.
#
#   run.sh [board_imx8mp.c] [outdir]
#     board_imx8mp.c  default: the tree's. An older one (git show
#                     <rev>:driver/zephyr/lib/board/imx8mp/board_imx8mp.c)
#                     prints its table without the check.
#     outdir          default: a new directory under ${TMPDIR:-/tmp}
#
# With the tree's board file each row is checked against the expected value
# (docs/notes/imx8mp-frame-floor.md, "Waits per swap"), and the run exits 1
# on a difference: a number is a match within 0.25 periods (the draw is
# 0.3 T, so 1 and 2 periods are far apart); "u" (unpaced) is under 0.8; and
# an in-place draw (a cluster frame submitted into the half on the panel) is
# always a failure.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
top=$(cd "$here/../../../../../.." && pwd)
board=$(realpath "${1:-$here/../board_imx8mp.c}")
out=${2:-$(mktemp -d "${TMPDIR:-/tmp}/latch_model.XXXXXX")}
mkdir -p "$out"
check=0
[ "$board" = "$(realpath "$here/../board_imx8mp.c")" ] && check=1
cc=${CC:-cc}

build() { # build <buffers> <view 0|1>
  local v=()
  [ "$2" = 1 ] && v=(-DCONFIG_YAKOGL_BOARD_SCANOUT_VIEW=1)
  "$cc" -std=gnu11 -O1 -Wall -Wextra -Werror -Wno-unused-parameter -pthread \
    -I"$here/shim" -I"$top/driver/zephyr/lib/board/include" \
    -I"$top/driver/zephyr/lib/board/common" -I"$top/include" \
    -DCONFIG_YAKOGL_BOARD=1 -DCONFIG_YAKOGL_DRV_DISPLAY_IMX8MP=1 -DCONFIG_YAKOGL_DRV_GPU_VIVANTE=1 \
    -DCONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS="$1" -DCONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS=1 "${v[@]}" \
    -o "$out/model-$1-$2" "$board" "$here/latch_model.c"
}

# expected periods/frame: app, buffers, view, unwired, wired
#   panel    Slint at the panel's size (in place)
#   view     Slint 1024x600: view=1 in place (SV), view=0 copied (SO)
#   cluster  two halves in place (CL)
#   glmark2  EGL_BACK_BUFFER, interval 0 (GM)
#   fifo     EGL_BACK_BUFFER, interval 1
expect() {
  cat <<'EOF'
panel   1 0 1 1
panel   2 0 1 1
panel   3 0 1 1
view    1 1 1 1
view    2 1 1 1
view    3 1 1 1
view    1 0 1 1
view    2 0 1 1
view    3 0 u 1
cluster 1 0 1 1
cluster 2 0 1 1
cluster 3 0 1 1
glmark2 1 0 1 1
glmark2 2 0 1 1
glmark2 3 0 u u
fifo    1 0 1 1
fifo    2 0 1 1
fifo    3 0 u 1
EOF
}

match() { # match <got> <want>
  if [ "$2" = u ]; then
    awk -v g="$1" 'BEGIN { exit !(g < 0.8) }'
  else
    awk -v g="$1" -v w="$2" 'BEGIN { d = g - w; if (d < 0) d = -d; exit !(d <= 0.25) }'
  fi
}

for b in 1 2 3; do
  for v in 0 1; do build "$b" "$v"; done
done

fail=0
while read -r app b v want_unwired want_wired; do
  for osal in unwired wired; do
    want=$want_unwired
    [ "$osal" = wired ] && want=$want_wired
    line=$("$out/model-$b-$v" "$app" "$osal" | grep '^app=')
    got=$(sed -n 's/.*periods\/frame=\([0-9.]*\).*/\1/p' <<<"$line")
    if [ "$check" = 0 ]; then
      echo "      $line"
    elif match "$got" "$want" && [[ "$line" == *inplace-draws=0 ]]; then
      echo "ok    $line (expected $want)"
    else
      echo "FAIL  $line (expected $want)"
      fail=1
    fi
  done
done < <(expect)
exit "$fail"
