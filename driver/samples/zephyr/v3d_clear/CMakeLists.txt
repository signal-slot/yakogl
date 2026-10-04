# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# v3d_clear: V3D 4.2 milestones M1..M4 on the Raspberry Pi 4 (see src/main.c).
# Standalone: it compiles the driver files directly and does not use the
# driver module's Kconfig, whose board support is the i.MX8MP's.
cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(v3d_clear)

get_filename_component(DRIVER "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${DRIVER}" describe --always --dirty
                OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
  set(V3D_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE V3D_BUILD="${V3D_BUILD}")

target_include_directories(app PRIVATE
  "${DRIVER}/src/v3d"
  "${DRIVER}/../third_party/mesa-26.2.1/src/broadcom"   # Mesa's packers (v3d_cl.h)
  "${DRIVER}/../third_party/mesa-26.2.1/src"
  "${DRIVER}/../third_party/mesa-26.2.1/include"
  "${DRIVER}/zephyr/lib/board/rpi4"     # rpi_mbox.h, rpi_wdog.h, fbcon.h
  "${DRIVER}/zephyr/lib/board/common")   # gc_font8x16.h only
target_sources(app PRIVATE
  src/main.c
  "${DRIVER}/zephyr/lib/board/rpi4/rpi_mbox.c"
  "${DRIVER}/zephyr/lib/board/rpi4/rpi_wdog.c"
  "${DRIVER}/zephyr/lib/board/rpi4/fbcon.c"
  "${DRIVER}/src/v3d/v3d_hw.c"
  "${DRIVER}/src/v3d/v3d_frame.c")
