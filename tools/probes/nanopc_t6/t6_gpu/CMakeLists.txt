# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6_gpu: the second half of Z1 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md):
# the GPU's power at hand-over, PD_GPU on, GPU_ID read, soft reset and the
# L2 on, then the reset back into Armbian.
#
#   west build -p always -b nanopc_t6 -d build/t6_gpu yakogl/tools/probes/nanopc_t6/t6_gpu
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(t6_gpu)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE T6_GPU_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR T6_GPU_BUILD STREQUAL "")
  set(T6_GPU_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE T6_GPU_BUILD="${T6_GPU_BUILD}")
message(STATUS "t6_gpu build: ${T6_GPU_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files: tools/check.sh builds this sample with
# -DYAKOGL_WERROR=ON (the "zephyr" rows); off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
