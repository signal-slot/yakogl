# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The board profile of the Linux route (our user-space library on the board's
# own Linux through the upstream panthor driver), sourced by
# tools/mali/panthor/run-on-board.sh, tools/cts/mali-deqp-run.sh,
# tools/cts/mali-glmark2-run.sh and tools/cts/mali-glmark2-preflight.sh.
#
# MALI_BOARD names the board kind (never its address: BOARD_HOST stays in the
# environment of the lease):
#   nanopc-t6     (the default) NanoPC-T6, RK3588, Mali-G610, Armbian trixie
#   verdin-imx95  Toradex Verdin iMX95, Mali-G310 (docs/notes/imx95-linux-panthor.md)
# It sets the defaults below; a variable already set in the environment wins.
#   MALI_BOARD_NAME   the board's name in records and messages
#   MALI_BOARD_TAG    the board's word in record names (<date>-<tag>-gles2-...)
#   MALI_RENDER_NODE  the panthor render node (default /dev/dri/renderD128 on
#                     both: a KMS-only display driver has no render node, so
#                     panthor's is the first; check with the discovery line of
#                     docs/notes/imx95-linux-panthor.md and set it otherwise)
#   MALI_STAGE_ROOT   the board directory run-on-board.sh and mali-deqp-run.sh
#                     stage under (default /root/yakogl-mali)
#   MALI_UART_BAUD    run-on-board.sh's UART capture speed (RK3588 1500000,
#                     i.MX 95 115200)
#   MALI_KERNEL_GLOB  the uname -r the panthor record is for (a case pattern;
#                     empty: no record yet, the preflight only names it)
#   MALI_OS_CODENAME  the os-release VERSION_CODENAME the builds target
#                     (mali-deqp-build.sh and mali-glmark2-build.sh build for
#                     Debian trixie); the preflight checks it
# board_profile_load returns 1 with a message on stderr for an unknown board
# or a value that is not a plain path (each goes into a remote command line).

board_profile_load() {
  MALI_BOARD=${MALI_BOARD:-nanopc-t6}
  local name tag baud kglob
  case $MALI_BOARD in
    nanopc-t6) name=NanoPC-T6 tag=t6 baud=1500000 kglob='6.18.*rockchip64*' ;;
    verdin-imx95) name='Verdin iMX95' tag=imx95 baud=115200 kglob= ;;
    *)
      echo "board-profile: MALI_BOARD '$MALI_BOARD' is not a known board (nanopc-t6, verdin-imx95)" >&2
      return 1
      ;;
  esac
  MALI_BOARD_NAME=${MALI_BOARD_NAME:-$name}
  MALI_BOARD_TAG=${MALI_BOARD_TAG:-$tag}
  MALI_RENDER_NODE=${MALI_RENDER_NODE:-/dev/dri/renderD128}
  MALI_STAGE_ROOT=${MALI_STAGE_ROOT:-/root/yakogl-mali}
  MALI_UART_BAUD=${MALI_UART_BAUD:-$baud}
  MALI_KERNEL_GLOB=${MALI_KERNEL_GLOB-$kglob}
  MALI_OS_CODENAME=${MALI_OS_CODENAME:-trixie}
  [[ $MALI_RENDER_NODE =~ ^/dev/dri/renderD[0-9]+$ ]] || {
    echo "board-profile: MALI_RENDER_NODE '$MALI_RENDER_NODE' is not /dev/dri/renderD<n>" >&2
    return 1
  }
  [[ $MALI_STAGE_ROOT =~ ^/[A-Za-z0-9._/-]+$ ]] && [[ $MALI_STAGE_ROOT != *..* ]] || {
    echo "board-profile: MALI_STAGE_ROOT '$MALI_STAGE_ROOT' is not a plain absolute path" >&2
    return 1
  }
  [[ $MALI_BOARD_TAG =~ ^[a-z0-9]+$ ]] || {
    echo "board-profile: MALI_BOARD_TAG '$MALI_BOARD_TAG' is not [a-z0-9]+" >&2
    return 1
  }
  [[ $MALI_UART_BAUD =~ ^[0-9]+$ ]] || {
    echo "board-profile: MALI_UART_BAUD '$MALI_UART_BAUD' is not a number" >&2
    return 1
  }
  # The render node's name under /sys/class/drm (renderD128).
  MALI_RENDER_NAME=${MALI_RENDER_NODE##*/}
  export MALI_BOARD MALI_BOARD_NAME MALI_BOARD_TAG MALI_RENDER_NODE MALI_STAGE_ROOT \
    MALI_UART_BAUD MALI_KERNEL_GLOB MALI_OS_CODENAME MALI_RENDER_NAME
}
