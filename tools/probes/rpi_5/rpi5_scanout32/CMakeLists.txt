# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_scanout32: stage 2 of docs/notes/rpi5-display.md (our own 32 bpp
# XRGB8888 plane after word 32 of the firmware's display list, swapped in at
# vblank between buffers A and B, then the firmware's list restored
# byte-equal). See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_scanout32 yakogl/tools/probes/rpi_5/rpi5_scanout32 \
#     -- -DYAKOGL_WERROR=ON
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_scanout32)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_SCANOUT32_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_SCANOUT32_BUILD STREQUAL "")
  set(RPI5_SCANOUT32_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_SCANOUT32_BUILD="${RPI5_SCANOUT32_BUILD}")
message(STATUS "rpi5_scanout32 build: ${RPI5_SCANOUT32_BUILD}")

# Hardware-written words of a plane entry (README "Hardware-written words"):
# "<off>:skip|expect" items joined by ',', offsets 0..62 from an entry's
# start, at most 4, each once; "" compares every word. The default is the one
# word the board showed changing (rpi5_scanout16 README "Q2's list word").
#   -DRPI5_SCANOUT32_HW_WORDS="4:skip"
set(RPI5_SCANOUT32_HW_WORDS "4:skip" CACHE STRING
    "hardware-written plane-entry words, e.g. 4:skip or 4:skip,7:expect (empty: none)")
if(NOT RPI5_SCANOUT32_HW_WORDS STREQUAL "")
  if(NOT RPI5_SCANOUT32_HW_WORDS MATCHES "^[0-9]+:(skip|expect)(,[0-9]+:(skip|expect))*$")
    message(FATAL_ERROR "RPI5_SCANOUT32_HW_WORDS '${RPI5_SCANOUT32_HW_WORDS}': "
                        "want <off>:skip|expect[,<off>:skip|expect...]")
  endif()
  string(REPLACE "," ";" hw_items "${RPI5_SCANOUT32_HW_WORDS}")
  list(LENGTH hw_items hw_n)
  if(hw_n GREATER 4)
    message(FATAL_ERROR "RPI5_SCANOUT32_HW_WORDS: ${hw_n} items, at most 4")
  endif()
  set(hw_seen "")
  foreach(it IN LISTS hw_items)
    string(REGEX REPLACE ":.*" "" hw_off "${it}")
    if(hw_off GREATER 62)
      message(FATAL_ERROR "RPI5_SCANOUT32_HW_WORDS: offset ${hw_off} over 62")
    endif()
    math(EXPR hw_off "${hw_off}")
    if(hw_off IN_LIST hw_seen)
      message(FATAL_ERROR "RPI5_SCANOUT32_HW_WORDS: offset ${hw_off} twice")
    endif()
    list(APPEND hw_seen ${hw_off})
  endforeach()
endif()
target_compile_definitions(app PRIVATE RPI5_SCANOUT32_HW_WORDS="${RPI5_SCANOUT32_HW_WORDS}")
message(STATUS "rpi5_scanout32 hardware-written words: '${RPI5_SCANOUT32_HW_WORDS}'")

target_sources(app PRIVATE src/main.c)
# rpi_wdog.h lives with the rpi4 board code; the header-only decoders in
# rpi5_scanout (hvs6_dlist.h, ...) have no source of their own.
target_include_directories(app PRIVATE
  src
  ${PVR_REPO}/zephyr/lib/board/rpi4
  ${PVR_REPO}/zephyr/lib/board/rpi5_scanout)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
