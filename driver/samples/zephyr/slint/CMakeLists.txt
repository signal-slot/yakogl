# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# slint: the one Slint sample for the Raspberry Pi 4, the Verdin AM62P and
# the Verdin iMX8M Plus (driver/docs/one-method.md, section 5, step 1). The
# board is chosen by the Zephyr board (-b) only; there is no per-board option
# here. What differs per board is the board layer's
# (driver/zephyr/lib/board, CONFIG_YAKOGL_BOARD) and the board files under
# boards/.
#
# What is shared, by path:
#   - the UI, ui/cluster.slint (moved here from gl_slint in Step 1),
#     with the SafeUI layout for 1024x600 (../safeui_dir.cmake) on every board;
#   - the no_std Slint workspace (SLINT_NOSTD_DIR) and the femtovg/glow forks
#     (NOSTD_PATCH_DIR), recreated from nostd/'s patches (nostd/README.md);
#   - the check, ./check (crate yakogl-slint-check, host-tested by
#     tools/check.sh), and through it driver/rust/src/board/record.rs.
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(slint)

target_sources(app PRIVATE src/main.c)

# ---------------------------------------------------------------------------
# The no_std Slint workspace and the femtovg/glow forks.
# ---------------------------------------------------------------------------
foreach(var SLINT_NOSTD_DIR NOSTD_PATCH_DIR)
  if(NOT ${var} AND DEFINED ENV{${var}})
    set(${var} "$ENV{${var}}")
  endif()
endforeach()
if(NOT SLINT_NOSTD_DIR OR NOT EXISTS "${SLINT_NOSTD_DIR}/internal/renderers/femtovg/Cargo.toml")
  message(FATAL_ERROR
    "slint: SLINT_NOSTD_DIR='${SLINT_NOSTD_DIR}' is not the no_std Slint workspace. "
    "Recreate it as nostd/README.md says and pass -DSLINT_NOSTD_DIR=<path>.")
endif()
if(NOT NOSTD_PATCH_DIR)
  get_filename_component(NOSTD_PATCH_DIR "${SLINT_NOSTD_DIR}/../nostd" ABSOLUTE)
endif()
if(NOT EXISTS "${NOSTD_PATCH_DIR}/femtovg/Cargo.toml" OR NOT EXISTS "${NOSTD_PATCH_DIR}/glow/Cargo.toml")
  message(FATAL_ERROR
    "slint: NOSTD_PATCH_DIR='${NOSTD_PATCH_DIR}' must hold the no_std femtovg/ and "
    "glow/ forks (nostd/README.md). Pass -DNOSTD_PATCH_DIR=<path>.")
endif()

# The AM62P's 1024x600 layout on every board: the window is 1024x600 on all
# three panels, and the reference points are the AM62P's.
include("${CMAKE_CURRENT_SOURCE_DIR}/../safeui_dir.cmake")
safeui_layout_dir(SAFEUI_LAYOUT_DIR SAFEUI_ROOT_DIR 1024x600)

# ---------------------------------------------------------------------------
# The Rust staticlib (rust/): built for aarch64-unknown-none (release) with
# -Z build-std=core,alloc, and linked whole.
# ---------------------------------------------------------------------------
find_program(CARGO cargo REQUIRED)
set(UI_DIR "${CMAKE_CURRENT_SOURCE_DIR}/ui")
set(SLINT_UI_FILE "${UI_DIR}/cluster.slint")
set(CARGO_SRC "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(SLINT_CHECK_DIR "${CMAKE_CURRENT_SOURCE_DIR}/check")
set(cargo_build_dir "${CMAKE_CURRENT_BINARY_DIR}/rust")
set(cargo_target_dir "${cargo_build_dir}/target")
set(RUST_TARGET aarch64-unknown-none)
set(slint_staticlib "${cargo_target_dir}/${RUST_TARGET}/release/libslint_sample.a")
file(MAKE_DIRECTORY "${cargo_build_dir}")
configure_file("${CARGO_SRC}/Cargo.toml.in" "${cargo_build_dir}/Cargo.toml" @ONLY)

file(GLOB_RECURSE slint_rust_srcs CONFIGURE_DEPENDS
     "${CARGO_SRC}/src/*.rs"
     "${CARGO_SRC}/build.rs"
     "${SLINT_CHECK_DIR}/src/*.rs"
     "${PVR_REPO}/rust/src/board/record.rs"
     "${UI_DIR}/*.slint"
     "${SAFEUI_LAYOUT_DIR}/*.slint"
     "${UI_DIR}/assets/*"
     "${SLINT_NOSTD_DIR}/internal/renderers/femtovg/*.rs"
     "${SLINT_NOSTD_DIR}/internal/core/*.rs"
     "${NOSTD_PATCH_DIR}/femtovg/src/*.rs"
     "${NOSTD_PATCH_DIR}/femtovg/src/*.glsl"
     "${NOSTD_PATCH_DIR}/glow/src/*.rs")

add_custom_command(
  OUTPUT "${slint_staticlib}"
  COMMAND ${CMAKE_COMMAND} -E env
          "CARGO_TARGET_DIR=${cargo_target_dir}"
          "SLINT_UI_FILE=${SLINT_UI_FILE}"
          "SAFEUI_LAYOUT_DIR=${SAFEUI_LAYOUT_DIR}"
          ${CARGO} build --release
          -Z build-std=core,alloc
          --target ${RUST_TARGET}
          --manifest-path "${cargo_build_dir}/Cargo.toml"
  DEPENDS ${slint_rust_srcs} "${cargo_build_dir}/Cargo.toml"
  WORKING_DIRECTORY "${cargo_build_dir}"
  COMMENT "cargo build slint_sample (${RUST_TARGET}, release)"
  VERBATIM)
add_custom_target(slint_sample_build DEPENDS "${slint_staticlib}")
add_dependencies(app slint_sample_build)
target_link_libraries(app PRIVATE "${slint_staticlib}")
