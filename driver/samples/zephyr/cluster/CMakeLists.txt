# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# cluster: signal-slot/agl-slint-cluster's instrument cluster, the one sample
# for the Raspberry Pi 4, the Verdin AM62P and the Verdin iMX8M Plus
# (driver/docs/one-method.md, section 5, step 5). The board is chosen by the
# Zephyr board (-b) only. What differs per board is the board layer's
# (driver/zephyr/lib/board, CONFIG_YAKOGL_BOARD) and the board files under
# boards/ (the GPU, the display, the memory and the panel size).
#
# What is shared, by path, never copied:
#   - the cluster's UI: the agl-slint-cluster clone's ui/ directory
#     (AGL_SLINT_CLUSTER_DIR, a CMake or environment variable), compiled
#     through ui/zephyr-cluster.slint; the clone is not part of this repository;
#   - the no_std Slint workspace (SLINT_NOSTD_DIR) and the femtovg/glow forks
#     (NOSTD_PATCH_DIR), recreated from ../slint/nostd (README.md there);
#   - Noto Sans Regular, Light and ExtraLight (NOTO_SANS_DIR);
#   - the check, ./check (crate yakogl-cluster-check, host-tested by
#     tools/check.sh), and through it driver/rust/src/board/record.rs.
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(cluster)

target_sources(app PRIVATE src/main.c)

# ---------------------------------------------------------------------------
# The inputs from outside this repository.
# ---------------------------------------------------------------------------
foreach(var AGL_SLINT_CLUSTER_DIR SLINT_NOSTD_DIR NOSTD_PATCH_DIR NOTO_SANS_DIR)
  if(NOT ${var} AND DEFINED ENV{${var}})
    set(${var} "$ENV{${var}}")
  endif()
endforeach()
if(NOT AGL_SLINT_CLUSTER_DIR OR NOT EXISTS "${AGL_SLINT_CLUSTER_DIR}/ui/app-window.slint")
  message(FATAL_ERROR
    "cluster: AGL_SLINT_CLUSTER_DIR='${AGL_SLINT_CLUSTER_DIR}' is not an "
    "agl-slint-cluster clone (no ui/app-window.slint). Clone "
    "https://github.com/signal-slot/agl-slint-cluster and pass -DAGL_SLINT_CLUSTER_DIR=<path>.")
endif()
if(NOT SLINT_NOSTD_DIR OR NOT EXISTS "${SLINT_NOSTD_DIR}/internal/renderers/femtovg/Cargo.toml")
  message(FATAL_ERROR
    "cluster: SLINT_NOSTD_DIR='${SLINT_NOSTD_DIR}' is not the no_std Slint workspace. "
    "Recreate it as ../slint/nostd/README.md says and pass -DSLINT_NOSTD_DIR=<path>.")
endif()
if(NOT NOSTD_PATCH_DIR)
  get_filename_component(NOSTD_PATCH_DIR "${SLINT_NOSTD_DIR}/../nostd" ABSOLUTE)
endif()
if(NOT EXISTS "${NOSTD_PATCH_DIR}/femtovg/Cargo.toml" OR NOT EXISTS "${NOSTD_PATCH_DIR}/glow/Cargo.toml")
  message(FATAL_ERROR
    "cluster: NOSTD_PATCH_DIR='${NOSTD_PATCH_DIR}' must hold the no_std femtovg/ and "
    "glow/ forks (../slint/nostd/README.md). Pass -DNOSTD_PATCH_DIR=<path>.")
endif()
# The fonts are named, not taken from the host's defaults (rust/build.rs).
if(NOT NOTO_SANS_DIR)
  set(NOTO_SANS_DIR /usr/share/fonts/noto)
endif()
foreach(face Regular Light ExtraLight)
  if(NOT EXISTS "${NOTO_SANS_DIR}/NotoSans-${face}.ttf")
    message(FATAL_ERROR
      "cluster: ${NOTO_SANS_DIR}/NotoSans-${face}.ttf is missing. The image embeds "
      "Noto Sans Regular, Light and ExtraLight; pass -DNOTO_SANS_DIR=<dir>.")
  endif()
endforeach()
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/font-sizes.txt" SLINT_FONT_SIZES)
string(STRIP "${SLINT_FONT_SIZES}" SLINT_FONT_SIZES)

# ---------------------------------------------------------------------------
# The Rust staticlib (rust/): the cluster's UI and sim, the check and the
# records, built for aarch64-unknown-none (release) with
# -Z build-std=core,alloc, and linked whole.
# ---------------------------------------------------------------------------
find_program(CARGO cargo REQUIRED)
set(CARGO_SRC "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(CLUSTER_CHECK_DIR "${CMAKE_CURRENT_SOURCE_DIR}/check")
set(cluster_ui_dir "${AGL_SLINT_CLUSTER_DIR}/ui")
set(SLINT_UI_FILE "${CMAKE_CURRENT_SOURCE_DIR}/ui/zephyr-cluster.slint")
set(cargo_build_dir "${CMAKE_CURRENT_BINARY_DIR}/rust")
set(cargo_target_dir "${cargo_build_dir}/target")
set(RUST_TARGET aarch64-unknown-none)
set(slint_staticlib "${cargo_target_dir}/${RUST_TARGET}/release/libslint_cluster.a")
file(MAKE_DIRECTORY "${cargo_build_dir}")
configure_file("${CARGO_SRC}/Cargo.toml.in" "${cargo_build_dir}/Cargo.toml" @ONLY)

file(GLOB_RECURSE slint_rust_srcs CONFIGURE_DEPENDS
     "${CARGO_SRC}/src/*.rs"
     "${CARGO_SRC}/build.rs"
     "${CLUSTER_CHECK_DIR}/src/*.rs"
     "${PVR_REPO}/rust/src/board/record.rs"
     "${CMAKE_CURRENT_SOURCE_DIR}/ui/*.slint"
     "${CMAKE_CURRENT_SOURCE_DIR}/font-sizes.txt"
     "${cluster_ui_dir}/*.slint"
     "${cluster_ui_dir}/assets/*"
     "${SLINT_NOSTD_DIR}/internal/renderers/femtovg/*.rs"
     "${SLINT_NOSTD_DIR}/internal/core/*.rs"
     "${SLINT_NOSTD_DIR}/internal/compiler/passes/embed_glyphs.rs"
     "${NOSTD_PATCH_DIR}/femtovg/src/*.rs"
     "${NOSTD_PATCH_DIR}/femtovg/src/*.glsl"
     "${NOSTD_PATCH_DIR}/glow/src/*.rs")

add_custom_command(
  OUTPUT "${slint_staticlib}"
  COMMAND ${CMAKE_COMMAND} -E env
          "CARGO_TARGET_DIR=${cargo_target_dir}"
          "SLINT_UI_FILE=${SLINT_UI_FILE}"
          "AGL_SLINT_CLUSTER_UI_DIR=${cluster_ui_dir}"
          "SLINT_DEFAULT_FONT=${NOTO_SANS_DIR}/NotoSans-Regular.ttf"
          "SLINT_FONT_PATH=${NOTO_SANS_DIR}/NotoSans-Light.ttf:${NOTO_SANS_DIR}/NotoSans-ExtraLight.ttf"
          "SLINT_FONT_SIZES=${SLINT_FONT_SIZES}"
          ${CARGO} build --release
          -Z build-std=core,alloc
          --target ${RUST_TARGET}
          --manifest-path "${cargo_build_dir}/Cargo.toml"
  # every text size has its own glyph set, and the gradient-stroked Paths are
  # the ones the host reference's blind spot names (host/check_ui.py)
  COMMAND ${PYTHON_EXECUTABLE} "${CMAKE_CURRENT_SOURCE_DIR}/host/check_ui.py"
          "${cluster_ui_dir}" "${cargo_target_dir}"
  DEPENDS ${slint_rust_srcs} "${cargo_build_dir}/Cargo.toml" "${CMAKE_CURRENT_SOURCE_DIR}/host/check_ui.py"
  WORKING_DIRECTORY "${cargo_build_dir}"
  COMMENT "cargo build slint_cluster (${RUST_TARGET}, release)"
  VERBATIM)
add_custom_target(slint_cluster_build DEPENDS "${slint_staticlib}")
add_dependencies(app slint_cluster_build)
target_link_libraries(app PRIVATE "${slint_staticlib}")
