# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_dtb: the device tree the Raspberry Pi 5 firmware hands to the image in
# x0, and the GPU nodes in it (docs/notes/pre-study-rpi5.md, section 5). See
# README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_dtb yakogl/tools/probes/rpi_5/rpi5_dtb
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer, with the watchdog).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_dtb)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_DTB_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_DTB_BUILD STREQUAL "")
  set(RPI5_DTB_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_DTB_BUILD="${RPI5_DTB_BUILD}")
message(STATUS "rpi5_dtb build: ${RPI5_DTB_BUILD}")

# x0 at entry (boot_x0.S) and the bounds-checked walker.
include(${CMAKE_CURRENT_SOURCE_DIR}/../common/boot/rpi5_boot.cmake)
target_sources(app PRIVATE src/main.c ../common/fdt/fdt_walk.c)
target_include_directories(app PRIVATE ../common/fdt)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
