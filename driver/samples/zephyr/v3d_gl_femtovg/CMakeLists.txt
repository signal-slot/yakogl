# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# v3d_gl_femtovg: gl_femtovg's FemtoVG frame, drawn on the Raspberry Pi 4's V3D
# through YakoGL (GL core, in-process compiler, V3D backend, the Zephyr OSAL
# src/osal/zephyr_v3d.c), with every pixel of the known scene read back.
#
# What is shared, by path, never copied:
#   - the Rust staticlib (fvg_zephyr): ../gl_femtovg/rust (the Zephyr allocator
#     and panic handler around fvg_demo's fvg_draw_frame);
#   - the no_std femtovg/glow/fvg_demo workspace: FVG_NOSTD_DIR, recreated from
#     ../slint/nostd (README.md there);
#   - femtovg's own shader text (main-vs.glsl, main-fs.glsl) from that
#     workspace, which src/main.c compiles the way femtovg does, to name each
#     program's refusal before the frame (fvg_draw_frame only returns 1);
#   - the board support of v3d_clear (mailbox framebuffer, HDMI console,
#     bounded-lifetime watchdog), as v3d_gl_triangle compiles it.
# docs/online-shaderc/v3d-backend.md, "M7", is the plan this sample measures.
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(v3d_gl_femtovg)

execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
  set(V3D_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE V3D_BUILD="${V3D_BUILD}")

set(V3D_BOARD "${CMAKE_CURRENT_SOURCE_DIR}/../../../zephyr/lib/board/rpi4")
target_include_directories(app PRIVATE
  "${V3D_BOARD}"
  "${YAKOGL_DIR}/src/osal"                # struct osal_zephyr_v3d_init
  "${PVR_REPO}/src/v3d"                   # struct v3d_clock_ops (v3d.h)
  "${PVR_REPO}/zephyr/lib/board/common")      # gc_font8x16.h only
target_sources(app PRIVATE
  src/main.c
  "${V3D_BOARD}/rpi_mbox.c"
  "${V3D_BOARD}/fbcon.c")
# The bounded-lifetime watchdog, on by default: every board image carries it.
# V3D_RPI_WDOG=OFF is for a QEMU smoke run only (see v3d_gl_triangle).
option(V3D_RPI_WDOG "the PM watchdog that resets the board ~60 s after boot" ON)
if(V3D_RPI_WDOG)
  target_sources(app PRIVATE "${V3D_BOARD}/rpi_wdog.c")
else()
  message(WARNING "v3d_gl_femtovg: built WITHOUT the watchdog (V3D_RPI_WDOG=OFF): QEMU only, never the board")
  target_compile_definitions(app PRIVATE V3D_NO_RPI_WDOG)
endif()

# ---------------------------------------------------------------------------
# The no_std FemtoVG workspace.
# ---------------------------------------------------------------------------
if(NOT FVG_NOSTD_DIR AND DEFINED ENV{FVG_NOSTD_DIR})
  set(FVG_NOSTD_DIR "$ENV{FVG_NOSTD_DIR}")
endif()
if(NOT FVG_NOSTD_DIR OR NOT EXISTS "${FVG_NOSTD_DIR}/fvg_demo/Cargo.toml")
  message(FATAL_ERROR
    "v3d_gl_femtovg: FVG_NOSTD_DIR='${FVG_NOSTD_DIR}' is not the no_std FemtoVG "
    "workspace (no fvg_demo/Cargo.toml). Recreate it as "
    "../slint/nostd/README.md says and pass -DFVG_NOSTD_DIR=<path>.")
endif()

# femtovg's 30 programs through GL, from the shader text of the femtovg tree
# the Rust archive builds (src/fvg_programs.h).
include("${CMAKE_CURRENT_SOURCE_DIR}/fvg_programs.cmake")
v3d_fvg_programs("${FVG_NOSTD_DIR}/femtovg")

# ---------------------------------------------------------------------------
# The Rust staticlib: gl_femtovg's crate, built here for aarch64-unknown-none.
# ---------------------------------------------------------------------------
find_program(CARGO cargo REQUIRED)
set(CARGO_SRC "${CMAKE_CURRENT_SOURCE_DIR}/../gl_femtovg/rust")
set(cargo_build_dir "${CMAKE_CURRENT_BINARY_DIR}/rust")
set(cargo_target_dir "${cargo_build_dir}/target")
set(RUST_TARGET aarch64-unknown-none)
set(fvg_staticlib "${cargo_target_dir}/${RUST_TARGET}/release/libfvg_zephyr.a")
file(MAKE_DIRECTORY "${cargo_build_dir}")
configure_file("${CARGO_SRC}/Cargo.toml.in" "${cargo_build_dir}/Cargo.toml" @ONLY)

file(GLOB_RECURSE fvg_rust_srcs CONFIGURE_DEPENDS
     "${CARGO_SRC}/src/*.rs"
     "${FVG_NOSTD_DIR}/fvg_demo/src/*.rs"
     "${FVG_NOSTD_DIR}/femtovg/src/*.rs"
     "${FVG_NOSTD_DIR}/femtovg/src/*.glsl"
     "${FVG_NOSTD_DIR}/glow/src/*.rs")

add_custom_command(
  OUTPUT "${fvg_staticlib}"
  COMMAND ${CMAKE_COMMAND} -E env
          "CARGO_TARGET_DIR=${cargo_target_dir}"
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
target_link_libraries(app PRIVATE "${fvg_staticlib}")
