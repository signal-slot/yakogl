# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_wdog: the Raspberry Pi 5's first board fact (docs/notes/pre-study-rpi5.md,
# section 6): does the BCM2712 PM watchdog reset the board? The smallest image
# that shows it. See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_wdog yakogl/tools/probes/rpi_5/rpi5_wdog
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer, with the watchdog).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_wdog)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_WDOG_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_WDOG_BUILD STREQUAL "")
  set(RPI5_WDOG_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_WDOG_BUILD="${RPI5_WDOG_BUILD}")
message(STATUS "rpi5_wdog build: ${RPI5_WDOG_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
