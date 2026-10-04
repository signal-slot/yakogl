# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
cmake_minimum_required(VERSION 3.20.0)

# The driver (driver/) and YakoGL (the repository root) are Zephyr modules;
# point ZEPHYR_EXTRA_MODULES at both unless a west manifest lists them.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}" "${PVR_REPO}/..")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gl_femtovg)

target_sources(app PRIVATE src/main.c)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/sample.cmake")

# ---------------------------------------------------------------------------
# Rust FemtoVG staticlib (fvg_zephyr) - wraps Agent A's no_std fvg_demo crate,
# provides the global allocator + panic handler, re-exports fvg_draw_frame().
# Built for aarch64-unknown-none (release) with -Z build-std=core,alloc; the
# resulting archive is linked into the Zephyr image with the SDK linker.
# ---------------------------------------------------------------------------
find_program(CARGO cargo REQUIRED)

# The no_std FemtoVG workspace (fvg_demo + patched femtovg/glow forks) lives
# outside the repo (not vendored here); point at it with -DFVG_NOSTD_DIR=... or
# $FVG_NOSTD_DIR.
if(NOT FVG_NOSTD_DIR AND DEFINED ENV{FVG_NOSTD_DIR})
  set(FVG_NOSTD_DIR "$ENV{FVG_NOSTD_DIR}")
endif()
if(NOT FVG_NOSTD_DIR)
  message(FATAL_ERROR
    "gl_femtovg: FVG_NOSTD_DIR is not set. Point it at the no_std FemtoVG "
    "workspace (fvg_demo + patched femtovg/glow): "
    "-DFVG_NOSTD_DIR=<path> or export FVG_NOSTD_DIR=<path>.")
endif()
if(NOT EXISTS "${FVG_NOSTD_DIR}/fvg_demo/Cargo.toml")
  message(FATAL_ERROR
    "gl_femtovg: FVG_NOSTD_DIR='${FVG_NOSTD_DIR}' has no fvg_demo/Cargo.toml. "
    "Set -DFVG_NOSTD_DIR=<path to the no_std FemtoVG workspace>.")
endif()

set(CARGO_SRC "${CMAKE_CURRENT_SOURCE_DIR}/rust")
set(cargo_build_dir "${CMAKE_CURRENT_BINARY_DIR}/rust")
set(cargo_target_dir "${cargo_build_dir}/target")
set(RUST_TARGET aarch64-unknown-none)
set(fvg_staticlib "${cargo_target_dir}/${RUST_TARGET}/release/libfvg_zephyr.a")
file(MAKE_DIRECTORY "${cargo_build_dir}")

# Generate the manifest with the (absolute) source and dependency paths baked in.
configure_file("${CARGO_SRC}/Cargo.toml.in" "${cargo_build_dir}/Cargo.toml" @ONLY)

# The wrapper's own sources AND the no_std crates it pulls in (fvg_demo, and the
# patched femtovg/glow under FVG_NOSTD_DIR): editing any of them must re-run cargo.
file(GLOB_RECURSE fvg_rust_srcs CONFIGURE_DEPENDS
     "${CARGO_SRC}/src/*.rs"
     "${FVG_NOSTD_DIR}/fvg_demo/src/*.rs")

add_custom_command(
  OUTPUT "${fvg_staticlib}"
  COMMAND ${CMAKE_COMMAND} -E env
          "CARGO_TARGET_DIR=${cargo_target_dir}"
          "CARGO_NET_OFFLINE=false"
          ${CARGO} build --release
          -Z build-std=core,alloc
          --target ${RUST_TARGET}
          --manifest-path "${cargo_build_dir}/Cargo.toml"
  DEPENDS ${fvg_rust_srcs} "${cargo_build_dir}/Cargo.toml"
  WORKING_DIRECTORY "${cargo_build_dir}"
  COMMENT "cargo build fvg_zephyr (${RUST_TARGET}, release)"
  VERBATIM)

add_custom_target(fvg_zephyr_build DEPENDS "${fvg_staticlib}")
add_dependencies(app fvg_zephyr_build)

# Link the whole archive: fvg_draw_frame is referenced by main.c, which pulls in
# fvg_demo/femtovg/glow and the allocator/panic objects transitively.
target_link_libraries(app PRIVATE "${fvg_staticlib}")
