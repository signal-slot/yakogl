#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The self-test of tools/mali/panthor/board-profile.sh (the board profile of
# the Linux route): the NanoPC-T6's defaults are the values the scripts had
# before the profile, the Verdin iMX95 gets its own name, tag and UART speed,
# a value set in the environment wins, and an unknown board or a value that
# is not a plain path is refused. Then run-modes.sh's busy pattern takes a
# render node other than renderD128.
#
#   tools/mali/panthor/board-profile-selftest.sh   prints "board-profile-selftest: N checks passed"
#
# Host only, under a second. Gate: a row of its own (tools/check_selftests.py
# lists it, as no row names it).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
PROFILE=$HERE/board-profile.sh
EXPECT=17
passed=0 failed=0
check() { # <what> <command...>
  local what=$1
  shift
  if "$@" > /dev/null 2>&1; then
    passed=$((passed + 1))
    echo "  ok    $what"
  else
    failed=$((failed + 1))
    echo "  FAIL  $what"
  fi
}
# load <env assignments...>: the profile's variables, one NAME=value a line,
# in a clean environment; the exit status is board_profile_load's
load() {
  env -i PATH="$PATH" "$@" bash -c '. "$1" && board_profile_load && for v in MALI_BOARD MALI_BOARD_NAME MALI_BOARD_TAG MALI_RENDER_NODE MALI_RENDER_NAME MALI_STAGE_ROOT MALI_UART_BAUD MALI_KERNEL_GLOB MALI_OS_CODENAME; do printf "%s=%s\n" "$v" "${!v}"; done' _ "$PROFILE"
}
is() { # <output> <NAME=value>
  grep -qxF -- "$2" <<< "$1"
}

t6=$(load)
check "default board: nanopc-t6" is "$t6" MALI_BOARD=nanopc-t6
check "nanopc-t6: render node /dev/dri/renderD128 (renderD128)" \
  bash -c 'grep -qx MALI_RENDER_NODE=/dev/dri/renderD128 <<< "$1" && grep -qx MALI_RENDER_NAME=renderD128 <<< "$1"' _ "$t6"
check "nanopc-t6: stage root /root/yakogl-mali" is "$t6" MALI_STAGE_ROOT=/root/yakogl-mali
check "nanopc-t6: UART 1500000 baud" is "$t6" MALI_UART_BAUD=1500000
check "nanopc-t6: tag t6, name NanoPC-T6" bash -c 'grep -qx MALI_BOARD_TAG=t6 <<< "$1" && grep -qx MALI_BOARD_NAME=NanoPC-T6 <<< "$1"' _ "$t6"
check "nanopc-t6: the kernel the record is for, 6.18.*rockchip64*" is "$t6" 'MALI_KERNEL_GLOB=6.18.*rockchip64*'
check "nanopc-t6: os codename trixie" is "$t6" MALI_OS_CODENAME=trixie

v=$(load MALI_BOARD=verdin-imx95)
check "verdin-imx95: tag imx95, name Verdin iMX95" \
  bash -c 'grep -qx MALI_BOARD_TAG=imx95 <<< "$1" && grep -qx "MALI_BOARD_NAME=Verdin iMX95" <<< "$1"' _ "$v"
check "verdin-imx95: UART 115200 baud" is "$v" MALI_UART_BAUD=115200
check "verdin-imx95: no kernel record yet (empty glob)" is "$v" MALI_KERNEL_GLOB=
check "verdin-imx95: render node and stage root default as on the T6" \
  bash -c 'grep -qx MALI_RENDER_NODE=/dev/dri/renderD128 <<< "$1" && grep -qx MALI_STAGE_ROOT=/root/yakogl-mali <<< "$1"' _ "$v"

o=$(load MALI_BOARD=verdin-imx95 MALI_RENDER_NODE=/dev/dri/renderD129 MALI_STAGE_ROOT=/home/torizon/yakogl-mali)
check "the environment wins: renderD129, /home/torizon/yakogl-mali" \
  bash -c 'grep -qx MALI_RENDER_NAME=renderD129 <<< "$1" && grep -qx MALI_STAGE_ROOT=/home/torizon/yakogl-mali <<< "$1"' _ "$o"

check "an unknown board is refused" bash -c '! load_out=$(env -i PATH="$PATH" MALI_BOARD=rk3399 bash -c ". \"\$1\" && board_profile_load" _ "$1" 2>&1)' _ "$PROFILE"
check "a render node that is not /dev/dri/renderD<n> is refused" bash -c '! env -i PATH="$PATH" MALI_RENDER_NODE="/dev/dri/card0; reboot" bash -c ". \"\$1\" && board_profile_load" _ "$1"' _ "$PROFILE"
check "a stage root with .. or a space is refused" \
  bash -c '! env -i PATH="$PATH" MALI_STAGE_ROOT=/root/../etc bash -c ". \"\$1\" && board_profile_load" _ "$1" && ! env -i PATH="$PATH" MALI_STAGE_ROOT="/root/a b" bash -c ". \"\$1\" && board_profile_load" _ "$1"' _ "$PROFILE"

# run-modes.sh retries a busy board by the refusal line run-on-board.sh prints;
# the line names the board's render node, renderD128 or another.
pat=$(sed -n "s/.*grep -qE '\\(\\^board_lock: busy|[^']*\\)'.*/\\1/p" "$HERE/run-modes.sh")
check "run-modes.sh's busy pattern is found" test -n "$pat"
check "run-modes.sh's busy pattern takes renderD128 and renderD129" \
  bash -c 'printf "%s\n" "run-on-board.sh: /dev/dri/renderD128 is in use, not starting:" | grep -qE "$1" &&
    printf "%s\n" "run-on-board.sh: /dev/dri/renderD129 is in use, not starting:" | grep -qE "$1"' _ "$pat"

total=$((passed + failed))
if [ "$failed" -ne 0 ] || [ "$total" -ne "$EXPECT" ]; then
  echo "board-profile-selftest: $failed of $total checks failed (the self-test pins $EXPECT checks)" >&2
  exit 1
fi
echo "board-profile-selftest: $passed checks passed"
