# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# v3d_gl_test: the GL suite (driver/tools/pvrlab/gl_test.c, the same file the
# AM62P and the i.MX8MP run) on the Raspberry Pi 4's V3D, through YakoGL's GL
# core, its in-process compiler and its V3D backend
# (docs/online-shaderc/v3d-backend.md, "M6 step B"). The board support (the
# firmware framebuffer through the mailbox, the HDMI console, the
# bounded-lifetime watchdog) is v3d_clear's, as in v3d_gl_triangle.
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(v3d_gl_test)

execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
  set(V3D_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE V3D_BUILD="${V3D_BUILD}")

set(V3D_BOARD "${CMAKE_CURRENT_SOURCE_DIR}/../../../zephyr/lib/board/rpi4")
target_sources(app PRIVATE
  src/main.c
  "${PVR_REPO}/tools/pvrlab/gl_test.c"
  "${V3D_BOARD}/rpi_mbox.c"
  "${V3D_BOARD}/fbcon.c")
target_include_directories(app PRIVATE
  "${V3D_BOARD}"
  "${PVR_REPO}/tools/pvrlab"
  "${YAKOGL_DIR}/include"
  "${YAKOGL_DIR}/src"
  "${YAKOGL_DIR}/src/osal"
  "${PVR_REPO}/src/v3d"
  "${PVR_REPO}/zephyr/lib/board/common"       # gc_font8x16.h only (fbcon)
  "${YAKOGL_DIR}/tools/v3d")              # tex_linear_case.h (case 109)

# The GL ES 1.00 shader set, always: the in-process compiler is the V3D's only
# shader back end and takes GLSL ES 1.00 (and 3.00); the default GLSL 450 set
# exists for the offline table, which this GPU does not have.
target_compile_definitions(app PRIVATE GL_TEST_ES100=1)

# Zero warnings in our C files is the rule: tools/check.sh builds this
# sample with -DYAKOGL_WERROR=ON (the "zephyr" rows). Off by default, so an
# application built with another toolchain is not stopped by a new warning.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
target_compile_definitions(app PRIVATE GL_TEST_VTL_PASSES=${CONFIG_V3D_GL_TEST_VTL_PASSES})

# The bounded-lifetime watchdog, on by default: every board image carries it.
# OFF is for a QEMU smoke run only (see v3d_gl_triangle's CMakeLists.txt: QEMU
# resets as soon as it is armed).
option(V3D_RPI_WDOG "the PM watchdog that resets the board ~60 s after boot" ON)
if(V3D_RPI_WDOG)
  target_sources(app PRIVATE "${V3D_BOARD}/rpi_wdog.c")
  # the image's whole lifetime, the run and the frozen record together
  # (CONFIG_V3D_GL_TEST_HOLD_S, bounded; see Kconfig)
  target_compile_definitions(app PRIVATE RPI_WDOG_HOLD_S=${CONFIG_V3D_GL_TEST_HOLD_S})
else()
  message(WARNING "v3d_gl_test: built WITHOUT the watchdog (V3D_RPI_WDOG=OFF): QEMU only, never the board")
  target_compile_definitions(app PRIVATE V3D_NO_RPI_WDOG)
endif()
