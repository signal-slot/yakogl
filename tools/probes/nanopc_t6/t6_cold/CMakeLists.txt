# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6_cold: step D8 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md,
# section 2 route (c)): after a normal kexec, bring the display chain up
# from cold with power-cycle, show a CPU-drawn pattern in Esmart0, hold
# the picture, print the section 5 read-backs, then the reset back into
# Armbian. The chain's stage to stop after is a build option, for a board
# run that bisects, and so is the mode:
#
#   west build -p always -b nanopc_t6 -d build/t6_cold yakogl/tools/probes/nanopc_t6/t6_cold
#   west build -p always -b nanopc_t6 -d build/t6_cold-4 yakogl/tools/probes/nanopc_t6/t6_cold \
#     -- -DT6_COLD_STOP_AFTER=4
#   west build -p always -b nanopc_t6 -d build/t6_cold-1080 yakogl/tools/probes/nanopc_t6/t6_cold \
#     -- -DT6_COLD_MODE=1920x1080
#   west build -p always -b nanopc_t6 -d build/t6_cold-1080-hold90 yakogl/tools/probes/nanopc_t6/t6_cold \
#     -- -DT6_COLD_MODE=1920x1080 -DT6_PICTURE_HOLD_S=90 -DCONFIG_YAKOGL_NANOPC_T6_LIFETIME_S=130
#
# T6_COLD_STOP_AFTER: 0 runs to the end, 1..8 stops after that stage
# (START, POWER, GRF, PHY, DCLK, CONTROLLER, VOP, VIDEO). T6_COLD_FORCE_COLD
# ON also stops a port found running (the chain's force-cold). T6_COLD_MODE:
# 1024x768 (the default) or 1920x1080, both at 60 Hz; the picture is
# 1024x768 in both, at the top left of the larger mode. T6_PICTURE_HOLD_S:
# the picture's hold, 30 s by default (../common/t6_picture.cmake); a longer
# hold needs the board's watchdog lifetime raised with it.
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(t6_cold)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE T6_COLD_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR T6_COLD_BUILD STREQUAL "")
  set(T6_COLD_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE T6_COLD_BUILD="${T6_COLD_BUILD}")
message(STATUS "t6_cold build: ${T6_COLD_BUILD}")

set(T6_COLD_STOP_AFTER 0 CACHE STRING "The chain's stage to stop after: 0 none, 1..8")
option(T6_COLD_FORCE_COLD "Stop a display port found running (the chain's force-cold)" OFF)
set(T6_COLD_MODE 1024x768 CACHE STRING "The mode the chain programs: 1024x768 or 1920x1080 (60 Hz)")
if(NOT T6_COLD_STOP_AFTER MATCHES "^[0-8]$")
  message(FATAL_ERROR "T6_COLD_STOP_AFTER='${T6_COLD_STOP_AFTER}': 0 (run to the end) or a stage, 1..8")
endif()
if(T6_COLD_MODE STREQUAL "1024x768")
  set(_mode_1920x1080 0)
elseif(T6_COLD_MODE STREQUAL "1920x1080")
  set(_mode_1920x1080 1)
else()
  message(FATAL_ERROR "T6_COLD_MODE='${T6_COLD_MODE}': 1024x768 or 1920x1080")
endif()
if(T6_COLD_FORCE_COLD)
  set(_force_cold 1)
else()
  set(_force_cold 0)
endif()
target_compile_definitions(app PRIVATE T6_COLD_STOP_AFTER=${T6_COLD_STOP_AFTER}
                                       T6_COLD_FORCE_COLD=${_force_cold}
                                       T6_COLD_MODE_1920X1080=${_mode_1920x1080})
message(STATUS "t6_cold: stop after ${T6_COLD_STOP_AFTER}, force-cold ${_force_cold}, mode ${T6_COLD_MODE}")

# The picture, the hold (T6_PICTURE_HOLD_S) and the read-backs, shared
# with t6_display.
target_sources(app PRIVATE src/main.c)
include(${CMAKE_CURRENT_SOURCE_DIR}/../common/t6_picture.cmake)

# Zero warnings in our C files: tools/check.sh builds this sample with
# -DYAKOGL_WERROR=ON (the "zephyr" rows); off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
