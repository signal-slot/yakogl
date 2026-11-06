# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_clear: the first clear on the Raspberry Pi 5's V3D 7.1, M1 to M4
# (docs/notes/pre-study-rpi5.md, section 6). See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_clear yakogl/tools/probes/rpi_5/rpi5_clear
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
get_filename_component(YAKOGL_ROOT "${PVR_REPO}/.." ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_clear)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_CLEAR_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_CLEAR_BUILD STREQUAL "")
  set(RPI5_CLEAR_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_CLEAR_BUILD="${RPI5_CLEAR_BUILD}")
message(STATUS "rpi5_clear build: ${RPI5_CLEAR_BUILD}")

# The V3D sequences (v3d_hw.c, the BCM2712 path) and the 7.1 tile size
# (v3d_frame.c's v3d_frame_tiling_gen) are the driver's; the 7.1 packets are
# the probe's own (src/frame71.c). v3d_frame.h brings Mesa's 4.2 packers in:
# the include directories are the ones zephyr/CMakeLists.txt gives gl_test.
target_sources(app PRIVATE
  src/main.c
  src/frame71.c
  ${PVR_REPO}/src/v3d/v3d_hw.c
  ${PVR_REPO}/src/v3d/v3d_frame.c)
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
