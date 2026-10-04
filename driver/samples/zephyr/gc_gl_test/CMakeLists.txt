# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
cmake_minimum_required(VERSION 3.20.0)

get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)

include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")
set(ENV{GC_SAMPLE_COMMON_KCONFIG} "${CMAKE_CURRENT_SOURCE_DIR}/../gc_common/Kconfig")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gc_gl_test)

include("${CMAKE_CURRENT_SOURCE_DIR}/../gc_common/gc_sample.cmake")

# The same tools/pvrlab/gl_test.c the AM62P sample compiles. It is a GL-level
# suite and portable; only the few lines that start the backend know which GPU
# this is, and they are behind CONFIG_YAKOGL_GPU_VIVANTE. Running it here is
# the point: a triangle proves a pipe, this proves the backend.
target_sources(app PRIVATE src/main.c "${PVR_REPO}/tools/pvrlab/gl_test.c")
target_include_directories(app PRIVATE
  "${PVR_REPO}/tools/pvrlab"
  "${YAKOGL_DIR}/include"
  "${YAKOGL_DIR}/src"
  "${YAKOGL_DIR}/src/vivante"
  "${YAKOGL_DIR}/src/osal"
  "${PVR_REPO}/src"
  "${PVR_REPO}/src/vivante")

# GL_TEST_ES100=ON compiles the suite against the GL ES 1.00 shader set
# (tools/pvrlab/gl_test_shaders_es100.h) instead of the default GLSL 450 one:
# the set an ES 2.0 application could have written, and so the only one an
# ES 2.0 compiler can be asked to compile. Off by default, because the default
# set is what the offline table has always been measured with and the two must
# stay comparable on the same board.
option(GL_TEST_ES100 "Compile gl_test against the GL ES 1.00 shader set" OFF)
if(GL_TEST_ES100)
  target_compile_definitions(app PRIVATE GL_TEST_ES100=1)
  message(STATUS "gl_test: GL ES 1.00 shader set (GL_TEST_ES100=ON)")
endif()
