# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
cmake_minimum_required(VERSION 3.20.0)

# The AM62P second-stage loader (docs/direct-boot.md, "The second stage").
# Built for verdin_am62p/am62p54/a53 with boards/verdin_am62p_am62p54_a53.overlay.
# The board support is in driver/ (a Zephyr module); the loader's logic is the
# Rust crate in loader/ (yakogl-boot-stage2), built as a staticlib through
# capi/yakogl_capi.cmake like the drivers archive.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(boot_stage2)

if(NOT CONFIG_ARM64)
  message(FATAL_ERROR "boot_stage2: an aarch64 loader (verdin_am62p/am62p54/a53)")
endif()

target_sources(app PRIVATE src/main.c)
target_include_directories(app PRIVATE loader/include)

include("${YAKOGL_DIR}/capi/yakogl_capi.cmake")
set(_bs2_archive "${CMAKE_CURRENT_BINARY_DIR}/libboot_stage2.a")
yakogl_add_rust_archive(
  CRATE yakogl-boot-stage2
  MANIFEST "${CMAKE_CURRENT_SOURCE_DIR}/loader/Cargo.toml"
  LIB_NAME boot_stage2
  TARGET aarch64-unknown-none-softfloat
  RUSTUP_TOOLCHAIN "${YAKOGL_RUSTUP_TOOLCHAIN}"
  CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-stage2"
  OUTPUT "${_bs2_archive}"
  TARGET_NAME boot_stage2_rust)
add_library(boot_stage2_rust_lib STATIC IMPORTED GLOBAL)
set_target_properties(boot_stage2_rust_lib PROPERTIES IMPORTED_LOCATION "${_bs2_archive}")
add_dependencies(boot_stage2_rust_lib boot_stage2_rust)
add_dependencies(app boot_stage2_rust)
target_link_libraries(app PRIVATE boot_stage2_rust_lib)
