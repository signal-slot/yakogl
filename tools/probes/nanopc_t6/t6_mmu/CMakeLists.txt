# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6_mmu: Z2 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md, "Z2"): Z1's
# GPU power-on, then AS0's page tables from the image and a GPU cache flush
# round trip, then the reset back into Armbian.
#
#   west build -p always -b nanopc_t6 -d build/t6_mmu yakogl/tools/probes/nanopc_t6/t6_mmu
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(t6_mmu)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE T6_MMU_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR T6_MMU_BUILD STREQUAL "")
  set(T6_MMU_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE T6_MMU_BUILD="${T6_MMU_BUILD}")
message(STATUS "t6_mmu build: ${T6_MMU_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files: tools/check.sh builds this sample with
# -DYAKOGL_WERROR=ON (the "zephyr" rows); off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
