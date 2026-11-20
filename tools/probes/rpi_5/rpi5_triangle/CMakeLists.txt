# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_triangle: the first triangle on the Raspberry Pi 5's V3D 7.1, M5
# (docs/notes/pre-study-rpi5.md, section 6; P6 of section 8), with Mesa's
# shaders. See README.md. rpi5_triangle_ours builds the same sources with our
# compiler's shaders (probe.cmake, RPI5_TRI_SHADERS).
#
#   west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_triangle yakogl/tools/probes/rpi_5/rpi5_triangle
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_triangle)

include(${CMAKE_CURRENT_SOURCE_DIR}/probe.cmake)
