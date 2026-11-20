# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_tfu: one V3D 7.1 TFU mip-chain job on the Raspberry Pi 5, read back
# (roadmap P10; does the 7.1.10 TFU run though HUB_IDENT1 bit 17 is clear?).
# See README.md. rpi5_clear's configuration and board overlay, no copies.
#
#   west build -p always -b rpi_5 -DYAKOGL_WERROR=ON -d build/rpi5_tfu yakogl/tools/probes/rpi_5/rpi5_tfu
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
project(rpi5_tfu)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_TFU_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_TFU_BUILD STREQUAL "")
  set(RPI5_TFU_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_TFU_BUILD="${RPI5_TFU_BUILD}")
message(STATUS "rpi5_tfu build: ${RPI5_TFU_BUILD}")

# The driver's: the V3D sequences (v3d_hw.c, the BCM2712 path), Mesa's level
# layout and texel tiling (v3d_tex.c, v3d_tiling_mesa.c: the same on 4.2 and
# 7.1, only the state records differ) and the 7.1 TFU fill (v3d_tfu71.c).
# The probe writes the 7.1 TFU registers itself (src/main.c).
target_sources(app PRIVATE
  src/main.c
  ${PVR_REPO}/src/v3d/v3d_hw.c
  ${PVR_REPO}/src/v3d/v3d_tex.c
  ${PVR_REPO}/src/v3d/v3d_tiling_mesa.c
  ${PVR_REPO}/src/v3d/v3d_tfu71.c)
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
