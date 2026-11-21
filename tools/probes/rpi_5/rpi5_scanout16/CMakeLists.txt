# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_scanout16: frame 1 of the Pi 5 display (adopt the firmware's RGB565
# plane, fbcon text and a pattern, read back, long hold, read back). See
# README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_scanout16 yakogl/tools/probes/rpi_5/rpi5_scanout16 \
#     -- -DYAKOGL_WERROR=ON [-DRPI5_SCANOUT16_HOLD_MS=<ms>]
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_scanout16)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_SCANOUT16_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_SCANOUT16_BUILD STREQUAL "")
  set(RPI5_SCANOUT16_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_SCANOUT16_BUILD="${RPI5_SCANOUT16_BUILD}")
message(STATUS "rpi5_scanout16 build: ${RPI5_SCANOUT16_BUILD}")

# The hold (Q2) in ms. The image's end line must come before tools/board.sh's
# pi5 RUN_BOUND (420 s from the reboot, firmware and Linux shutdown included),
# and before the watchdog stops being fed: main.c clamps the hold to
# CONFIG_YAKOGL_RPI5_WDOG_HOLD_S * 1000 - 10000 ms since boot (hold_check.h's
# hold_clamp_end) and says so. 240 s with prj.conf's 280 s hold ends the image
# at about 245 s since boot.
set(RPI5_SCANOUT16_HOLD_MS 240000 CACHE STRING "rpi5_scanout16: hold length in ms")
if(NOT RPI5_SCANOUT16_HOLD_MS MATCHES "^[0-9]+$" OR RPI5_SCANOUT16_HOLD_MS GREATER 360000)
  message(FATAL_ERROR "RPI5_SCANOUT16_HOLD_MS=${RPI5_SCANOUT16_HOLD_MS}: a whole number of ms, "
                      "at most 360000 (board.sh pi5's RUN_BOUND is 420 s from the reboot)")
endif()
target_compile_definitions(app PRIVATE RPI5_SCANOUT16_HOLD_MS=${RPI5_SCANOUT16_HOLD_MS}u)
message(STATUS "rpi5_scanout16 hold: ${RPI5_SCANOUT16_HOLD_MS} ms")

target_sources(app PRIVATE src/main.c)
# rpi_wdog.h lives with the rpi4 board code; the rpi5_scanout headers (adopt.h,
# fb_flush.h, hvs6_dlist.h, scanout_alive.h, ...) have no source of their own;
# plane_match.h (iommu4's pass-through) is rpi5_display's, used as it is.
target_include_directories(app PRIVATE
  ${PVR_REPO}/zephyr/lib/board/rpi5_scanout
  ${PVR_REPO}/zephyr/lib/board/rpi4
  ${CMAKE_CURRENT_SOURCE_DIR}/../../rpi5_display/src
  src)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
