# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The OS part of an example's build: Zephyr. include() this at the top of an
# example's CMakeLists.txt, right after cmake_minimum_required(); it ends with
# find_package(Zephyr), so the example only adds its sources after it.
#
# An example has no board files of its own. What a board needs (its
# devicetree overlay and its Kconfig fragment: the display path, the GPU
# carve-out, the watchdog) is the board layer's, one pair per board for every
# example, in driver/zephyr/lib/board/<board>/app.overlay and app.conf. The
# board is chosen by west's -b only:
#
#   west build -b verdin_am62p/am62p54/a53 examples/triangle
#   west build -b verdin_imx8mp/mimx8ml8/a53 examples/triangle
#   west build -b rpi_4b examples/triangle

get_filename_component(YAKOGL_EXAMPLES "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
get_filename_component(PVR_REPO "${YAKOGL_EXAMPLES}/../driver" ABSOLUTE)
include("${PVR_REPO}/samples/zephyr/common/yakogl_dir.cmake")

if(NOT BOARD)
  message(FATAL_ERROR "examples: no board (west build -b <board>)")
endif()
# The Zephyr board -> the board layer's directory (driver/zephyr/lib/board/).
string(REGEX REPLACE "/.*" "" _yk_board_name "${BOARD}")
set(_yk_layer_verdin_am62p am62p)
set(_yk_layer_verdin_imx8mp imx8mp)
set(_yk_layer_rpi_4b rpi4)
if(NOT DEFINED _yk_layer_${_yk_board_name})
  message(FATAL_ERROR "examples: ${BOARD} is not a board of the board layer (verdin_am62p, verdin_imx8mp, rpi_4b)")
endif()
set(_yk_board_dir "${PVR_REPO}/zephyr/lib/board/${_yk_layer_${_yk_board_name}}")
if(BOARD MATCHES "/direct$")
  # The direct variant (-b <board>/direct: started at power-on in place of
  # U-Boot proper) has the display path in its own devicetree: no overlay, and
  # its Kconfig fragment is app_direct.conf.
  set(_yk_app_conf "${_yk_board_dir}/app_direct.conf")
  set(_yk_app_overlay "")
else()
  set(_yk_app_conf "${_yk_board_dir}/app.conf")
  set(_yk_app_overlay "${_yk_board_dir}/app.overlay")
endif()
if(NOT EXISTS "${_yk_app_conf}")
  message(FATAL_ERROR "examples: no board files for ${BOARD} (${_yk_app_conf})")
endif()
list(APPEND EXTRA_CONF_FILE "${_yk_app_conf}")
# What one example needs on a board's direct image beyond app_direct.conf
# (the power-on demo's run length, watchdog and window): the board layer's
# <example>_direct.conf, e.g. imx8mp/slint_direct.conf for examples/slint.
get_filename_component(_yk_example "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
if(BOARD MATCHES "/direct$" AND EXISTS "${_yk_board_dir}/${_yk_example}_direct.conf")
  list(APPEND EXTRA_CONF_FILE "${_yk_board_dir}/${_yk_example}_direct.conf")
endif()
if(_yk_app_overlay AND EXISTS "${_yk_app_overlay}")
  list(APPEND EXTRA_DTC_OVERLAY_FILE "${_yk_app_overlay}")
endif()

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
