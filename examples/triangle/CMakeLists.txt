# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# triangle: the triangle example (the GL calls in C, the scene and its check in
# a Rust crate), for the Raspberry Pi 4, the Verdin AM62P and the Verdin
# iMX8M Plus. The board is chosen by the Zephyr board (-b) only; there is no
# per-board option here. What differs per board is the board layer's
# (driver/zephyr/lib/board, CONFIG_YAKOGL_BOARD, and its app.conf and
# app.overlay, which ../zephyr.cmake applies).
#
# What is shared, by path: the scene and the check, ./check (crate
# yakogl-triangle-check, host-tested by tools/check.sh), and through it
# tools/shaders/gl_test_es100/passthrough.vert, interp_color.frag and
# driver/rust/src/board/record.rs.
cmake_minimum_required(VERSION 3.20.0)
include("${CMAKE_CURRENT_SOURCE_DIR}/../zephyr.cmake")
project(triangle)

target_sources(app PRIVATE src/main.c)

# ---------------------------------------------------------------------------
# The Rust staticlib (rust/, the workspace member yakogl-example-triangle),
# built for aarch64-unknown-none as every Rust archive of an image is
# (yakogl_add_rust_archive): the toolchain's precompiled core when it has one,
# else -Z build-std=core on a nightly with rust-src.
# ---------------------------------------------------------------------------
include("${YAKOGL_DIR}/capi/yakogl_capi.cmake")
set(triangle_staticlib "${CMAKE_CURRENT_BINARY_DIR}/libtriangle_sample.a")
yakogl_add_rust_archive(
  CRATE yakogl-example-triangle
  MANIFEST "${CMAKE_CURRENT_SOURCE_DIR}/rust/Cargo.toml"
  LIB_NAME triangle_sample
  TARGET aarch64-unknown-none
  RUSTUP_TOOLCHAIN "${YAKOGL_RUSTUP_TOOLCHAIN}"
  CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-triangle"
  OUTPUT "${triangle_staticlib}"
  TARGET_NAME triangle_sample_build)
add_dependencies(app triangle_sample_build)
target_link_libraries(app PRIVATE "${triangle_staticlib}")
