# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
cmake_minimum_required(VERSION 3.20.0)

# The driver (repository root) and YakoGL are Zephyr modules.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)

include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")

# The shared sample shape (bounded watchdog service, PASS/FAIL, PSCI reset).
set(ENV{GC_SAMPLE_COMMON_KCONFIG} "${CMAKE_CURRENT_SOURCE_DIR}/../gc_common/Kconfig")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(gc_gl_triangle)

include("${CMAKE_CURRENT_SOURCE_DIR}/../gc_common/gc_sample.cmake")
target_sources(app PRIVATE src/main.c)
