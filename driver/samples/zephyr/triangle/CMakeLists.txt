# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# triangle: the one triangle sample for the Raspberry Pi 4, the Verdin AM62P
# and the Verdin iMX8M Plus (driver/docs/one-method.md, section 5, step 3).
# The board is chosen by the Zephyr board (-b) only; there is no per-board
# option here. What differs per board is the board layer's
# (driver/zephyr/lib/board, CONFIG_YAKOGL_BOARD) and the board files under
# boards/.
#
# What is shared, by path: the scene and the check, ./check (crate
# yakogl-triangle-check, host-tested by tools/check.sh), and through it
# tools/shaders/gl_test_es100/passthrough.vert, interp_color.frag and
# driver/rust/src/board/record.rs.
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(triangle)

target_sources(app PRIVATE src/main.c)

# ---------------------------------------------------------------------------
# The Rust staticlib (rust/): built for aarch64-unknown-none (release) with
# -Z build-std=core, as the slint sample's, and linked whole.
# ---------------------------------------------------------------------------
find_program(CARGO cargo REQUIRED)
set(CARGO_SRC "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(TRIANGLE_CHECK_DIR "${CMAKE_CURRENT_SOURCE_DIR}/check")
set(cargo_build_dir "${CMAKE_CURRENT_BINARY_DIR}/rust")
set(cargo_target_dir "${cargo_build_dir}/target")
set(RUST_TARGET aarch64-unknown-none)
set(triangle_staticlib "${cargo_target_dir}/${RUST_TARGET}/release/libtriangle_sample.a")
file(MAKE_DIRECTORY "${cargo_build_dir}")
configure_file("${CARGO_SRC}/Cargo.toml.in" "${cargo_build_dir}/Cargo.toml" @ONLY)

file(GLOB_RECURSE triangle_rust_srcs CONFIGURE_DEPENDS
     "${CARGO_SRC}/src/*.rs"
     "${TRIANGLE_CHECK_DIR}/src/*.rs"
     "${TRIANGLE_CHECK_DIR}/Cargo.toml"
     "${PVR_REPO}/rust/src/board/record.rs"
     "${YAKOGL_DIR}/tools/shaders/gl_test_es100/passthrough.vert"
     "${YAKOGL_DIR}/tools/shaders/gl_test_es100/interp_color.frag")

add_custom_command(
  OUTPUT "${triangle_staticlib}"
  COMMAND ${CMAKE_COMMAND} -E env
          "CARGO_TARGET_DIR=${cargo_target_dir}"
          ${CARGO} build --release
          -Z build-std=core
          --target ${RUST_TARGET}
          --manifest-path "${cargo_build_dir}/Cargo.toml"
  DEPENDS ${triangle_rust_srcs} "${cargo_build_dir}/Cargo.toml"
  WORKING_DIRECTORY "${cargo_build_dir}"
  COMMENT "cargo build triangle_sample (${RUST_TARGET}, release)"
  VERBATIM)
add_custom_target(triangle_sample_build DEPENDS "${triangle_staticlib}")
add_dependencies(app triangle_sample_build)
target_link_libraries(app PRIVATE "${triangle_staticlib}")
