# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
cmake_minimum_required(VERSION 3.20.0)

# The driver (driver/) and YakoGL (the repository root) are Zephyr modules;
# point ZEPHYR_EXTRA_MODULES at both unless a west manifest lists them.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gl_triangle)

target_sources(app PRIVATE src/main.c)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/sample.cmake")
