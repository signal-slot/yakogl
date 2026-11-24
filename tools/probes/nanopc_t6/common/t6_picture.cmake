# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The picture both NanoPC-T6 display probes show (t6_picture.c), for
# t6_display and t6_cold. Include from the probe's CMakeLists.txt after
# find_package(Zephyr) and project(...):
#
#   include(${CMAKE_CURRENT_SOURCE_DIR}/../common/t6_picture.cmake)
#
# Adds t6_picture.c to the app and this directory to its include path, and
# takes the hold as a build option:
#
#   west build ... yakogl/tools/probes/nanopc_t6/t6_cold -- \
#     -DT6_PICTURE_HOLD_S=90 -DCONFIG_YAKOGL_NANOPC_T6_LIFETIME_S=130
#
# T6_PICTURE_HOLD_S: the seconds the picture is held, 30 by default. A
# display that boots with the board (powered from its USB ports on the
# U-Boot route) shows a picture only some 40 s after the image switched its
# 5 V on (driver/docs/nanopc-t6-display.md, section 3): such a run is built
# with a longer hold. t6_picture.c holds the rules (a multiple of 10, at
# least 30, and 10 s less than the board's watchdog lifetime,
# CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S, at least); t6-verdict.sh judges a
# record by the hold and the lifetime it prints.
get_filename_component(_t6_picture_dir "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)

set(T6_PICTURE_HOLD_S 30 CACHE STRING "Seconds the picture is held: a multiple of 10, at least 30")
if(NOT T6_PICTURE_HOLD_S MATCHES "^[1-9][0-9]*$")
  message(FATAL_ERROR "T6_PICTURE_HOLD_S='${T6_PICTURE_HOLD_S}': a number of seconds, a multiple of 10, at least 30")
endif()
target_compile_definitions(app PRIVATE T6_PICTURE_HOLD_S=${T6_PICTURE_HOLD_S})
message(STATUS "${PROJECT_NAME}: picture hold ${T6_PICTURE_HOLD_S} s, watchdog lifetime ${CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S} s")

target_sources(app PRIVATE ${_t6_picture_dir}/t6_picture.c)
target_include_directories(app PRIVATE ${_t6_picture_dir})
