# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_hello: item H9 of docs/notes/pre-study-rpi5.md. A no-GPU image for the
# Raspberry Pi 5, built and not run: the build line, the board, the console and
# the memory the image was linked into, then a few seconds counted on the
# system timer. tools/check.sh builds it with -DYAKOGL_WERROR=ON.
#
#   west build -p always -b rpi_5 -d build/rpi5_hello yakogl/tools/probes/rpi_5/rpi5_hello
cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_hello)

# The build prints its own commit first (a result row names its build).
get_filename_component(RPI5_HELLO_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../.." ABSOLUTE)
execute_process(COMMAND git -C "${RPI5_HELLO_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_HELLO_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_HELLO_BUILD STREQUAL "")
  set(RPI5_HELLO_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_HELLO_BUILD="${RPI5_HELLO_BUILD}")
message(STATUS "rpi5_hello build: ${RPI5_HELLO_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files: tools/check.sh builds this probe with
# -DYAKOGL_WERROR=ON (the "zephyr" rows); off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
