# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_ident: D2 part 1 (docs/notes/pre-study-rpi5.md, section 6): the V3D 7.1
# hub and core identity and MMU_DEBUG_INFO read from Zephyr with no power or
# clock setup. See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_ident yakogl/tools/probes/rpi_5/rpi5_ident
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer, with the watchdog).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_ident)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_IDENT_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_IDENT_BUILD STREQUAL "")
  set(RPI5_IDENT_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_IDENT_BUILD="${RPI5_IDENT_BUILD}")
message(STATUS "rpi5_ident build: ${RPI5_IDENT_BUILD}")

# Only the register map (v3d_regs.h: the BCM2712 bases and the offsets); no
# driver code runs.
target_sources(app PRIVATE src/main.c)
target_include_directories(app PRIVATE ${PVR_REPO}/src/v3d)

# The fallback build (-DEXTRA_CONF_FILE=fallback.conf, CONFIG_YAKOGL_RPI5_MBOX):
# the driver's own BCM2712 power-on (v3d_hw.c) and the board layer's mailbox.
if(CONFIG_YAKOGL_RPI5_MBOX)
  get_filename_component(YAKOGL_ROOT "${PVR_REPO}/.." ABSOLUTE)
  target_sources(app PRIVATE ${PVR_REPO}/src/v3d/v3d_hw.c)
  target_include_directories(app PRIVATE
    ${PVR_REPO}/src/v3d/generated
    ${PVR_REPO}/zephyr/lib/board/rpi4
    ${YAKOGL_ROOT}/third_party/mesa-26.2.1/src/broadcom
    ${YAKOGL_ROOT}/third_party/mesa-26.2.1/src
    ${YAKOGL_ROOT}/third_party/mesa-26.2.1/include)
endif()

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
