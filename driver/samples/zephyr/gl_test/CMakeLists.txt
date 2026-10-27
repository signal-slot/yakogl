# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# gl_test: the one GL suite image for the Verdin AM62P (PowerVR), the Verdin
# iMX8M Plus and Armadillo-X2 (Vivante GC7000UL) and the Raspberry Pi 4 (V3D).
# tools/pvrlab/gl_test.c is the on-silicon GL test suite of the Linux harness;
# the same source runs here (printk instead of pr_info). The board is chosen by
# the Zephyr board (-b) only; there is no per-board option here. What differs
# per board is the board's main_*.c (the lines that start the back end and the
# board's way to give a verdict), the board layer (driver/zephyr/lib/board) and
# the board files under boards/.
cmake_minimum_required(VERSION 3.20.0)

# The driver (driver/) and YakoGL (the repository root) are Zephyr modules;
# point ZEPHYR_EXTRA_MODULES at both unless a west manifest lists them.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

# What the two i.MX8M Plus boards share is boards/imx8mp.conf; the Armadillo's
# console (UART2) comes after it, and the direct boot's after that. Zephyr merges EXTRA_CONF_FILE after
# boards/<board>.conf, and after a file named on the command line, so these
# two go first.
if(NOT BOARD AND DEFINED ENV{BOARD})
  set(BOARD "$ENV{BOARD}")
endif()
if(BOARD MATCHES "^(verdin_imx8mp|armadillo_x2)")
  set(_imx8mp_conf "${CMAKE_CURRENT_LIST_DIR}/boards/imx8mp.conf")
  if(BOARD MATCHES "^armadillo_x2")
    list(APPEND _imx8mp_conf "${CMAKE_CURRENT_LIST_DIR}/boards/armadillo_x2.conf")
  endif()
  # The direct boot (armadillo_x2/mimx8ml8/a53/direct) has no display node.
  if(BOARD MATCHES "^armadillo_x2/.*/direct$")
    list(APPEND _imx8mp_conf "${CMAKE_CURRENT_LIST_DIR}/boards/armadillo_x2_direct.conf")
  endif()
  set(EXTRA_CONF_FILE ${_imx8mp_conf} ${EXTRA_CONF_FILE})
endif()

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gl_test)

target_sources(app PRIVATE "${PVR_REPO}/tools/pvrlab/gl_test.c")

# Zero warnings in our C files is the rule: tools/check.sh builds this
# sample with -DYAKOGL_WERROR=ON (the "zephyr" rows). Off by default, so an
# application built with another toolchain is not stopped by a new warning.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
target_include_directories(app PRIVATE
  "${PVR_REPO}/tools/pvrlab"
  "${YAKOGL_DIR}/include"
  "${YAKOGL_DIR}/src"
  "${YAKOGL_DIR}/src/osal"
  "${PVR_REPO}/src")

# GL_TEST_ES100=ON compiles the suite against the GL ES 1.00 shader set
# (tools/pvrlab/gl_test_shaders_es100.h) instead of the default GLSL 450 one:
# the set an ES 2.0 application could have written, and so the only one an
# ES 2.0 compiler can be asked to compile. Off by default, because the default
# set is what the offline table has always been measured with and the two must
# stay comparable on the same board. The Pi 4 always takes it (below).
option(GL_TEST_ES100 "Compile gl_test against the GL ES 1.00 shader set" OFF)
if(GL_TEST_ES100)
  target_compile_definitions(app PRIVATE GL_TEST_ES100=1)
  message(STATUS "gl_test: GL ES 1.00 shader set (GL_TEST_ES100=ON)")
endif()

if(CONFIG_YAKOGL_GPU_V3D)
  # ---- Raspberry Pi 4, V3D 4.2 ---------------------------------------------
  # Through YakoGL's GL core, its in-process compiler and its V3D backend
  # (docs/online-shaderc/v3d-backend.md, "M6 step B"). The board support (the
  # firmware framebuffer through the mailbox, the HDMI console, the
  # bounded-lifetime watchdog) is the rpi4 board layer's.
  execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                  OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                  RESULT_VARIABLE describe_rc)
  if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
    set(V3D_BUILD "unknown")
  endif()
  target_compile_definitions(app PRIVATE V3D_BUILD="${V3D_BUILD}")

  set(V3D_BOARD "${PVR_REPO}/zephyr/lib/board/rpi4")
  target_sources(app PRIVATE
    src/main_rpi4.c
    "${V3D_BOARD}/rpi_mbox.c"
    "${V3D_BOARD}/fbcon.c")
  target_include_directories(app PRIVATE
    "${V3D_BOARD}"
    "${PVR_REPO}/src/v3d"
    "${PVR_REPO}/zephyr/lib/board/common"       # gc_font8x16.h only (fbcon)
    "${YAKOGL_DIR}/tools/v3d")                  # tex_linear_case.h (case 109)

  # The GL ES 1.00 shader set, always: the in-process compiler is the V3D's only
  # shader back end and takes GLSL ES 1.00 (and 3.00); the default GLSL 450 set
  # exists for the offline table, which this GPU does not have.
  target_compile_definitions(app PRIVATE GL_TEST_ES100=1)

  target_compile_definitions(app PRIVATE GL_TEST_VTL_PASSES=${CONFIG_V3D_GL_TEST_VTL_PASSES})

  # The bounded-lifetime watchdog, on by default: every board image carries it.
  # OFF is for a QEMU smoke run only (QEMU resets as soon as it is armed).
  option(V3D_RPI_WDOG "the PM watchdog that resets the board ~60 s after boot" ON)
  if(V3D_RPI_WDOG)
    target_sources(app PRIVATE "${V3D_BOARD}/rpi_wdog.c")
    # the image's whole lifetime, the run and the frozen record together
    # (CONFIG_V3D_GL_TEST_HOLD_S, bounded; see Kconfig)
    target_compile_definitions(app PRIVATE RPI_WDOG_HOLD_S=${CONFIG_V3D_GL_TEST_HOLD_S})
  else()
    message(WARNING "gl_test: built WITHOUT the watchdog (V3D_RPI_WDOG=OFF): QEMU only, never the board")
    target_compile_definitions(app PRIVATE V3D_NO_RPI_WDOG)
  endif()
elseif(CONFIG_YAKOGL_GPU_VIVANTE)
  # ---- Verdin iMX8M Plus, Armadillo-X2: Vivante GC7000UL -------------------
  # Only the few lines that start the backend know which GPU this is, and they
  # are behind CONFIG_YAKOGL_GPU_VIVANTE. Running the suite here is the point:
  # a triangle proves a pipe, this proves the backend.
  if(NOT CONFIG_YAKOGL_GC_BOARD)
    message(FATAL_ERROR "gl_test: CONFIG_YAKOGL_GC_BOARD is off (boards/imx8mp.conf turns it on)")
  endif()
  target_sources(app PRIVATE src/main_imx8mp.c)
  target_include_directories(app PRIVATE
    "${YAKOGL_DIR}/src/vivante"
    "${PVR_REPO}/src/vivante")

  # The panel console prints these on its first line, so that a photograph of
  # the panel says which sample and which build it is. The stamp comes from
  # CMake rather than __DATE__ so the compiler is never asked for the time of
  # day.
  string(TIMESTAMP GC_BUILD_STAMP "%Y-%m-%dT%H:%M:%SZ" UTC)
  target_compile_definitions(yakogl_gc_board PRIVATE
    GC_SAMPLE_NAME="${PROJECT_NAME}"
    GC_BUILD_STAMP="${GC_BUILD_STAMP}")
else()
  # ---- Verdin AM62P: PowerVR BXS-4-64 --------------------------------------
  target_sources(app PRIVATE src/main_am62p.c)
  target_include_directories(app PRIVATE "${YAKOGL_DIR}/src/pvr")
  include("${CMAKE_CURRENT_SOURCE_DIR}/../common/sample.cmake")
endif()
