# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_triangle_ours: rpi5_triangle (M5) with our 7.1 online compiler's
# pass-through VS + interp_color FS instead of Mesa's
# (tools/v3d/tri-shaders71/ours_passthrough_interp_color.h). No sources of
# its own: rpi5_triangle's sources, overlay and prj.conf, with
# RPI5_TRI_SHADERS=ours. See ../rpi5_triangle/README.md.
#
#   west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_triangle_ours yakogl/tools/probes/rpi_5/rpi5_triangle_ours
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(RPI5_TRI_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../rpi5_triangle" ABSOLUTE)
# rpi5_triangle's configuration first, then this directory's (empty) fragment;
# its board overlay
set(CONF_FILE "${RPI5_TRI_DIR}/prj.conf;${CMAKE_CURRENT_SOURCE_DIR}/prj.conf")
set(DTC_OVERLAY_FILE "${RPI5_TRI_DIR}/boards/rpi_5.overlay")

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_triangle_ours)

set(RPI5_TRI_SHADERS ours)
include(${RPI5_TRI_DIR}/probe.cmake)
