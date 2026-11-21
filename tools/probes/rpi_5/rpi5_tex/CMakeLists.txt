# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_tex: one textured quad (T1) and one seamless cube-map sample set (C1)
# on the Raspberry Pi 5's V3D 7.1, read back (roadmap P10, pre-study U8).
# See README.md. rpi5_clear's configuration and board overlay, no copies.
#
#   west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_tex yakogl/tools/probes/rpi_5/rpi5_tex
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(RPI5_CLEAR_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../rpi5_clear" ABSOLUTE)
# rpi5_clear's prj.conf first, then this directory's (empty) fragment; its
# overlay (sram0: the 32 MiB low window the GPU's statics live in)
set(CONF_FILE "${RPI5_CLEAR_DIR}/prj.conf;${CMAKE_CURRENT_SOURCE_DIR}/prj.conf")
set(DTC_OVERLAY_FILE "${RPI5_CLEAR_DIR}/boards/rpi_5.overlay")

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
get_filename_component(YAKOGL_ROOT "${PVR_REPO}/.." ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_tex)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_TEX_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_TEX_BUILD STREQUAL "")
  set(RPI5_TEX_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_TEX_BUILD="${RPI5_TEX_BUILD}")
message(STATUS "rpi5_tex build: ${RPI5_TEX_BUILD}")

# The driver's, unchanged: the V3D sequences (v3d_hw.c, the BCM2712 path),
# the 7.1 clear-then-triangle frame (v3d_frame.c + v3d_frame71_tri.c), the
# 7.1 texture and sampler records (v3d_tex71.c, its own 7.1 unit), and
# Mesa's level layout and texel tiling with the P0/P1 words (v3d_tex.c and
# v3d_tiling_mesa.c, version-neutral, built at the default 4.2 as rpi5_tfu
# builds them). Only the files that need the 7.1 packets (main.c for struct
# v3d_triangle_layout, the frame and the hardware sequences, as
# rpi5_triangle) are built at V3D_CL_VER 71.
set(rpi5_tex_v71
  src/main.c
  ${PVR_REPO}/src/v3d/v3d_hw.c
  ${PVR_REPO}/src/v3d/v3d_frame.c
  ${PVR_REPO}/src/v3d/v3d_frame71_tri.c)
target_sources(app PRIVATE
  ${rpi5_tex_v71}
  ${PVR_REPO}/src/v3d/v3d_tex71.c
  ${PVR_REPO}/src/v3d/v3d_tex.c
  ${PVR_REPO}/src/v3d/v3d_tiling_mesa.c)
set_source_files_properties(${rpi5_tex_v71} PROPERTIES COMPILE_DEFINITIONS V3D_CL_VER=71)
target_include_directories(app PRIVATE
  src
  ${PVR_REPO}/src/v3d
  ${PVR_REPO}/src/v3d/generated
  ${PVR_REPO}/zephyr/lib/board/rpi4
  ${YAKOGL_ROOT}/third_party/mesa-26.2.1/src/broadcom
  ${YAKOGL_ROOT}/third_party/mesa-26.2.1/src
  ${YAKOGL_ROOT}/third_party/mesa-26.2.1/include)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
