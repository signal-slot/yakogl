# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_gpu: the STM32MP257F-DK's GPU powered from our image (SCMI over
# OP-TEE, the RCC gate and reset), its identity read, HI_INTR_ENBL written
# and read back, the GPU reset to idle; one verdict line, then PSCI
# SYSTEM_RESET back into ST's Linux (README.md).
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_gpu \
#     yakogl/tools/probes/stm32mp257f_dk/mp2_gpu
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_gpu)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_GPU_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_GPU_BUILD STREQUAL "")
  set(MP2_GPU_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_GPU_BUILD="${MP2_GPU_BUILD}")
message(STATUS "mp2_gpu build: ${MP2_GPU_BUILD}")

target_sources(app PRIVATE src/main.c)

# The yakogl-drivers crate with soc::stm32mp2 (gpu_ffi.rs, the C ABI of
# stm32mp2_gpu.h) and gpu::vivante (the identity table it reads through) is
# the board layer's CONFIG_YAKOGL_STM32MP2_GPU (prj.conf): built and linked
# once by driver/zephyr/CMakeLists.txt, the same archive gc_fence and gc_clear
# link on this board.
if(NOT CONFIG_YAKOGL_STM32MP2_GPU)
  message(FATAL_ERROR "mp2_gpu: CONFIG_YAKOGL_STM32MP2_GPU is off")
endif()

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
