# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6_display: step D2 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md,
# section 2 route (a)): after a crash kexec, adopt the display Linux left
# running, show a CPU-drawn pattern in Esmart0, hold the picture, print
# the section 5 read-backs, then the reset back into Armbian.
#
#   west build -p always -b nanopc_t6 -d build/t6_display yakogl/tools/probes/nanopc_t6/t6_display
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(t6_display)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE T6_DISPLAY_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR T6_DISPLAY_BUILD STREQUAL "")
  set(T6_DISPLAY_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE T6_DISPLAY_BUILD="${T6_DISPLAY_BUILD}")
message(STATUS "t6_display build: ${T6_DISPLAY_BUILD}")

# The picture, the hold and the read-backs, shared with t6_cold.
target_sources(app PRIVATE src/main.c ../common/t6_picture.c)
target_include_directories(app PRIVATE ../common)

# Zero warnings in our C files: tools/check.sh builds this sample with
# -DYAKOGL_WERROR=ON (the "zephyr" rows); off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
