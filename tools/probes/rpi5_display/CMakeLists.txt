# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_display: U3 of docs/notes/pre-study-rpi5.md (section 8, P11): what the
# Raspberry Pi 5's display path gives a kernel= image (driver/docs/rpi5-display.md).
# See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_display yakogl/tools/probes/rpi5_display
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_display)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_DISPLAY_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_DISPLAY_BUILD STREQUAL "")
  set(RPI5_DISPLAY_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_DISPLAY_BUILD="${RPI5_DISPLAY_BUILD}")
message(STATUS "rpi5_display build: ${RPI5_DISPLAY_BUILD}")

# x0 at entry (boot_x0.S: the DTB the firmware hands over), from the rpi_5
# probes' common files, not copied.
include(${CMAKE_CURRENT_SOURCE_DIR}/../rpi_5/common/boot/rpi5_boot.cmake)
target_sources(app PRIVATE src/main.c ${PVR_REPO}/zephyr/lib/board/rpi4/fbcon.c)
target_include_directories(app PRIVATE src)

# The board library: rpi_mbox.c comes with CONFIG_YAKOGL_RPI5_MBOX (its header
# is in rpi4/); rpi5_scanout/ is where the steps' headers will be.
target_include_directories(app PRIVATE
  ${PVR_REPO}/zephyr/lib/board/rpi4
  ${PVR_REPO}/zephyr/lib/board/rpi5_scanout
  ${PVR_REPO}/zephyr/lib/board/common) # gc_font8x16.h (fbcon)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
