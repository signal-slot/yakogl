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
# stm32mp2_gpu.h) and gpu::vivante (the identity table it reads through):
# features stm32mp2 and vivante. No driver of this image builds the crate
# (driver/zephyr/CMakeLists.txt picks its features from the drivers), so
# the probe builds it here, as driver/zephyr/CMakeLists.txt does.
if(TARGET yakogl_drivers_rust)
  message(FATAL_ERROR "mp2_gpu: this image already builds libyakogl_drivers.a "
    "(driver/zephyr/CMakeLists.txt) without the stm32mp2 feature")
endif()
include("${PVR_REPO}/../capi/yakogl_capi.cmake")
set(_mp2_archive "${CMAKE_CURRENT_BINARY_DIR}/libyakogl_drivers.a")
yakogl_add_rust_archive(
  CRATE yakogl-drivers
  MANIFEST "${PVR_REPO}/rust/Cargo.toml"
  LIB_NAME yakogl_drivers
  FEATURES stm32mp2 vivante
  TARGET aarch64-unknown-none-softfloat
  RUSTUP_TOOLCHAIN "${YAKOGL_RUSTUP_TOOLCHAIN}"
  CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-drivers"
  OUTPUT "${_mp2_archive}"
  TARGET_NAME mp2_gpu_drivers_rust)
add_library(mp2_gpu_drivers_lib STATIC IMPORTED GLOBAL)
set_target_properties(mp2_gpu_drivers_lib PROPERTIES IMPORTED_LOCATION "${_mp2_archive}")
add_dependencies(mp2_gpu_drivers_lib mp2_gpu_drivers_rust)
target_link_libraries(app PRIVATE mp2_gpu_drivers_lib)

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
