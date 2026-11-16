# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_display: U3 on the Raspberry Pi 5 (driver/docs/rpi5-display.md): does a
# scanout run when a kernel= image starts, and does the property mailbox
# hand out a framebuffer. See README.md. Not under tools/probes/rpi_5/ until
# plan #321 releases that tree; then it moves there as rpi5_display.
#
#   west build -p always -b rpi_5 -d build/rpi5_display/west \
#     yakogl/tools/probes/rpi5_display -- -DYAKOGL_WERROR=ON
# (-d build/rpi5_display/west: src/plane_match_selftest.sh builds into
#  build/rpi5_display/plane_match/, so the west tree keeps its own directory.)
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_display)

# The build prints its own commit first (a record names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_DISPLAY_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_DISPLAY_BUILD STREQUAL "")
  set(RPI5_DISPLAY_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_DISPLAY_BUILD="${RPI5_DISPLAY_BUILD}")
message(STATUS "rpi5_display build: ${RPI5_DISPLAY_BUILD}")

# The Pi 4's console (fbcon.c, font in board/common) and the Pi 5's screen
# calls (rpi5_screen.c) over the mailbox the board layer owns. The U3 decisions
# are the header-only rpi5_scanout library (scanout_alive.h, hvs6_dlist.h,
# adopt.h, bus_addr.h, pv_hdmi.h, snapshot.h, throttled.h, fw_clk.h, hvs6_fmt.h,
# pattern.h, fb_query.h) plus this probe's own plane_match.h: no .c files of theirs are
# compiled, the same headers the host tests build.
target_sources(app PRIVATE
  src/main.c
  ${PVR_REPO}/zephyr/lib/board/rpi5/rpi5_screen.c
  ${PVR_REPO}/zephyr/lib/board/rpi4/fbcon.c)

# Step 1 reads the DTB the firmware handed over in x0: rpi_5/common/boot keeps
# x0 (boot_x0.S, -Wl,--wrap=__start) and fwfb/ finds the simple-framebuffer
# node with rpi_5/common/fdt's bounds-checked walker. Both are used as they
# are; nothing under tools/probes/rpi_5/ is this probe's.
get_filename_component(RPI5_COMMON "${CMAKE_CURRENT_SOURCE_DIR}/../rpi_5/common" ABSOLUTE)
include(${RPI5_COMMON}/boot/rpi5_boot.cmake)
target_sources(app PRIVATE
  fwfb/fw_fb_dt.c
  ${RPI5_COMMON}/fdt/fdt_walk.c)
target_include_directories(app PRIVATE
  fwfb
  ${RPI5_COMMON}/fdt)

target_include_directories(app PRIVATE
  ${PVR_REPO}/zephyr/lib/board/rpi4
  ${PVR_REPO}/zephyr/lib/board/rpi5
  ${PVR_REPO}/zephyr/lib/board/rpi5_scanout
  ${PVR_REPO}/zephyr/lib/board/common)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
