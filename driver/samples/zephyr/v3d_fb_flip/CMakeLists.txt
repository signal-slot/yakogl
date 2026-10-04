# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# v3d_fb_flip: does the Raspberry Pi 4 firmware framebuffer pan between two
# halves of a 1920x1440 virtual buffer without tearing, and what does one pan
# cost? No GPU: the display path alone (see src/main.c).
cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(v3d_fb_flip)

get_filename_component(DRIVER "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${DRIVER}" describe --always --dirty
                OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
  set(V3D_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE V3D_BUILD="${V3D_BUILD}")

# The board support is v3d_clear's (mailbox, watchdog); no fbcon: the
# framebuffer is the test pattern, so the console is the UART only.
set(V3D_BOARD "${CMAKE_CURRENT_SOURCE_DIR}/../../../zephyr/lib/board/rpi4")
target_include_directories(app PRIVATE "${V3D_BOARD}")
target_sources(app PRIVATE
  src/main.c
  "${V3D_BOARD}/rpi_mbox.c"
  "${V3D_BOARD}/rpi_wdog.c")
# setup (about 2 s) + four arms of 2.2 s holds and 10 s of pans + a margin
target_compile_definitions(app PRIVATE RPI_WDOG_HOLD_S=70)
