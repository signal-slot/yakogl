# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6_submit: Z4 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Submit
# (Z4)"): Z3's firmware boot, CSG 0 / CS 0, the tiler heap and one L1 frame,
# with the firmware image embedded from driver/src/fw/mali_csffw_blob.h (made by
# driver/tools/mkmalifw.sh from the board's mali_csffw.bin; gitignored and never
# committed, like pvr_fw_blob.h: the build fails without it).
#
#   west build -p always -b nanopc_t6 -d build/t6_submit yakogl/tools/probes/nanopc_t6/t6_submit
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(t6_submit)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE T6_SUBMIT_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR T6_SUBMIT_BUILD STREQUAL "")
  set(T6_SUBMIT_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE T6_SUBMIT_BUILD="${T6_SUBMIT_BUILD}")
message(STATUS "t6_submit build: ${T6_SUBMIT_BUILD}")

# Z1's record, shared by the probes that run Z1's power-on.
target_sources(app PRIVATE src/main.c ../common/t6_z1.c)
target_include_directories(app PRIVATE ../common)
# main.c includes "fw/mali_csffw_blob.h".
target_include_directories(app PRIVATE "${PVR_REPO}/src")

# Zero warnings in our C files: tools/check.sh builds this sample with
# -DYAKOGL_WERROR=ON (the "zephyr" rows); off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
