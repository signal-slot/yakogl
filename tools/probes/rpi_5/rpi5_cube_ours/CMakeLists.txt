# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_cube_ours: one seamless cube-map draw through the driver's V3D 7.1 job
# builder (v3d_job71.c) and a level-1 mip of a cube by the 7.1 TFU, on the
# Raspberry Pi 5 (roadmap P10, pre-study U8). See README.md. rpi5_clear's
# configuration and board overlay, no copies.
#
#   west build -p always -b rpi_5 -d build/rpi5_cube_ours yakogl/tools/probes/rpi_5/rpi5_cube_ours -- -DYAKOGL_WERROR=ON
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
project(rpi5_cube_ours)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_CUBE_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_CUBE_BUILD STREQUAL "")
  set(RPI5_CUBE_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_CUBE_BUILD="${RPI5_CUBE_BUILD}")
message(STATUS "rpi5_cube_ours build: ${RPI5_CUBE_BUILD}")

# The driver's, unchanged: the V3D sequences (v3d_hw.c, the BCM2712 path),
# the 7.1 job builder (v3d_job71.c, with v3d_frame.c for the tiling and the
# viewport), the 7.1 texture and sampler records (v3d_tex71.c, its own 7.1
# unit), the 7.1 TFU fill (v3d_tfu71.c) and Mesa's level layout and texel
# tiling with the P0/P1 words (v3d_tex.c, v3d_tiling_mesa.c: version-neutral,
# built at the default 4.2 as rpi5_tex and rpi5_tfu build them). main.c, the
# hardware sequences and the job builder are built at V3D_CL_VER 71.
set(rpi5_cube_v71
  src/main.c
  ${PVR_REPO}/src/v3d/v3d_hw.c
  ${PVR_REPO}/src/v3d/v3d_frame.c
  ${PVR_REPO}/src/v3d/v3d_job71.c)
target_sources(app PRIVATE
  ${rpi5_cube_v71}
  ${PVR_REPO}/src/v3d/v3d_tex71.c
  ${PVR_REPO}/src/v3d/v3d_tfu71.c
  ${PVR_REPO}/src/v3d/v3d_tex.c
  ${PVR_REPO}/src/v3d/v3d_tiling_mesa.c)
set_source_files_properties(${rpi5_cube_v71} PROPERTIES COMPILE_DEFINITIONS V3D_CL_VER=71)
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
