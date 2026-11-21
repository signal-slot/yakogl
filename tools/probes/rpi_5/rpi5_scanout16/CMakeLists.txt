# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_scanout16: the build skeleton of the 16 bpp scanout probe. It only
# dumps HVS and pixel valve registers (src/dump.h). See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_scanout16 yakogl/tools/probes/rpi_5/rpi5_scanout16 \
#     -- -DYAKOGL_WERROR=ON
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

target_sources(app PRIVATE src/main.c)
# rpi_wdog.h lives with the rpi4 board code; the header-only decoders in
# rpi5_scanout (scanout_alive.h, pv_hdmi.h) have no source of their own.
target_include_directories(app PRIVATE
  ${PVR_REPO}/zephyr/lib/board/rpi5_scanout
  ${PVR_REPO}/zephyr/lib/board/rpi4
  src)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
