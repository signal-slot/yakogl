# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_display: U3 on the Raspberry Pi 5 (driver/docs/rpi5-display.md): does a
# scanout run when a kernel= image starts, and does the property mailbox
# hand out a framebuffer. See README.md. Not under tools/probes/rpi_5/ until
# plan #321 releases that tree; then it moves there as rpi5_display.
#
#   west build -p always -b rpi_5 -d build/rpi5_display yakogl/tools/probes/rpi5_display
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
# calls (rpi5_screen.c) over the mailbox the board layer owns.
target_sources(app PRIVATE
  src/main.c
  ${PVR_REPO}/zephyr/lib/board/rpi5/rpi5_screen.c
  ${PVR_REPO}/zephyr/lib/board/rpi4/fbcon.c)
target_include_directories(app PRIVATE
  ${PVR_REPO}/zephyr/lib/board/rpi4
  ${PVR_REPO}/zephyr/lib/board/rpi5
  ${PVR_REPO}/zephyr/lib/board/common)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
