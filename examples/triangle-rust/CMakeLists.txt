# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# triangle-rust: examples/triangle with every call (EGL, GL, the board layer)
# made from Rust, for the Raspberry Pi 4, the Verdin AM62P and the Verdin
# iMX8M Plus. The board is chosen by the Zephyr board (-b) only; what differs
# per board is the board layer's, which ../zephyr.cmake applies.
#
# The scene and its check are ../triangle/check's (crate
# yakogl-triangle-check), the one definition both examples use.
cmake_minimum_required(VERSION 3.20.0)
include("${CMAKE_CURRENT_SOURCE_DIR}/../zephyr.cmake")
project(triangle_rust)

# The app library has no C source; Zephyr's empty file keeps it a library
# (as zephyr-lang-rust does).
target_sources(app PRIVATE "${ZEPHYR_BASE}/misc/empty_file.c")

# ---------------------------------------------------------------------------
# The Rust staticlib (src/, the workspace member yakogl-example-triangle-rust),
# built for aarch64-unknown-none as examples/triangle's is
# (yakogl_add_rust_archive).
# ---------------------------------------------------------------------------
include("${YAKOGL_DIR}/capi/yakogl_capi.cmake")
set(triangle_rust_staticlib "${CMAKE_CURRENT_BINARY_DIR}/libtriangle_rust.a")
yakogl_add_rust_archive(
  CRATE yakogl-example-triangle-rust
  MANIFEST "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
  LIB_NAME triangle_rust
  TARGET aarch64-unknown-none
  RUSTUP_TOOLCHAIN "${YAKOGL_RUSTUP_TOOLCHAIN}"
  CARGO_TARGET_DIR "${CMAKE_CURRENT_BINARY_DIR}/cargo-triangle-rust"
  OUTPUT "${triangle_rust_staticlib}"
  TARGET_NAME triangle_rust_build)
add_dependencies(app triangle_rust_build)
# main() is the archive's. The kernel's weak main() (kernel/main_weak.c) is
# in the link before this archive is searched, and the linker never takes an
# archive member only to replace a weak definition, so the link asks for
# triangle_rust_main, which main() calls: that loads the crate's one LTO
# object, and its main() replaces the weak one.
target_link_libraries(app PRIVATE "${triangle_rust_staticlib}")
zephyr_link_libraries(-Wl,--undefined=triangle_rust_main)
