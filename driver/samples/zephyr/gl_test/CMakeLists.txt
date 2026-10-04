# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
cmake_minimum_required(VERSION 3.20.0)

# The driver (driver/) and YakoGL (the repository root) are Zephyr modules;
# point ZEPHYR_EXTRA_MODULES at both unless a west manifest lists them.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gl_test)

# tools/pvrlab/gl_test.c is the on-silicon GL test suite of the Linux harness;
# the same source runs here (printk instead of pr_info). It reaches into the
# GL core, the backend and the OSAL, hence the private include directories.
target_sources(app PRIVATE src/main.c "${PVR_REPO}/tools/pvrlab/gl_test.c")
target_include_directories(app PRIVATE
  "${PVR_REPO}/tools/pvrlab"
  "${YAKOGL_DIR}/include"
  "${YAKOGL_DIR}/src"
  "${YAKOGL_DIR}/src/pvr"
  "${YAKOGL_DIR}/src/osal"
  "${PVR_REPO}/src")
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/sample.cmake")
