# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# libyakogl.a for the glmark2 host harness on the Mali-G610 (run.sh, gpu
# `mali`). The top-level CMakeLists.txt takes powervr|vivante|v3d only, so
# this builds the same archive the way it does for those (the yakogl-capi
# crate through capi/yakogl_capi.cmake's yakogl_add_rust_archive, with
# capi/assemble.cmake), with MALI_CONFIG's features of tools/check.sh
# (gpu-mali, shaderc-online, no-shader-table) on the Linux OSAL, and
# report-es2 as the other GPUs' harness builds. The C parts are the
# top-level build's for a GPU without driver C: src/osal/linux.c (which
# answers the PowerVR's contract here; shim.c's GM2_MALI table answers the
# Mali's, README.md), abi_layout_check.c and the EH personality forwarder.
cmake_minimum_required(VERSION 3.20)
project(YakoGLHarnessMali C)
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

get_filename_component(YAKOGL_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../../.." ABSOLUTE)
include("${YAKOGL_DIR}/capi/yakogl_capi.cmake")

add_library(yakogl_c OBJECT
    "${YAKOGL_DIR}/src/osal/linux.c"
    "${YAKOGL_DIR}/src/abi_layout_check.c"
    "${YAKOGL_DIR}/capi/eh_personality.c")
target_include_directories(yakogl_c PRIVATE
    "${YAKOGL_DIR}/include"
    "${YAKOGL_DIR}/src"
    "${YAKOGL_DIR}/src/osal"
    "${YAKOGL_DIR}/src/vivante"
    "${YAKOGL_DIR}/driver/src")
target_compile_definitions(yakogl_c PRIVATE YAKOGL_OSAL_LINUX YAKOGL_GPU_MALI
    "GL_MAX_CONTEXTS=${YAKOGL_MAX_CONTEXTS}")

yakogl_add_rust_archive(
  CRATE yakogl-capi
  MANIFEST "${YAKOGL_DIR}/capi/Cargo.toml"
  LIB_NAME yakogl_capi
  FEATURES osal-linux gpu-mali shaderc-online no-shader-table report-es2
  OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/libyakogl.a"
  TARGET_NAME yakogl_archive
  C_OBJECT_LIBRARY yakogl_c)
