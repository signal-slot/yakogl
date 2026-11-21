# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_dispregs: a read-only look at the firmware's display state on the
# Pi 5: pixel valves 0 and 1, HVS channels 0 and 1, the display clocks and
# the display count through the mailbox, and (only when the HSM clock runs)
# the HDMI hotplug and HD_VID_CTL words (docs/notes/rpi5-display.md section 4,
# Q3, Q5 and Q6). See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_dispregs yakogl/tools/probes/rpi_5/rpi5_dispregs \
#     -- -DYAKOGL_WERROR=ON
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_dispregs)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_DISPREGS_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_DISPREGS_BUILD STREQUAL "")
  set(RPI5_DISPREGS_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_DISPREGS_BUILD="${RPI5_DISPREGS_BUILD}")
message(STATUS "rpi5_dispregs build: ${RPI5_DISPREGS_BUILD}")

target_sources(app PRIVATE src/main.c)
# rpi_wdog.h lives with the rpi4 board code; the rpi5_scanout headers
# (pv_hdmi.h, pv_timing.h, scanout_alive.h, hvs6_chan.h, fw_clk.h) have no
# source of their own.
target_include_directories(app PRIVATE
  ${PVR_REPO}/zephyr/lib/board/rpi5_scanout
  ${PVR_REPO}/zephyr/lib/board/rpi4)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
