# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_display_state: the STM32MP257F-DK's display state probe (README.md):
# reads the RCC, RIFSC, LTDC, DSI, GPIO and RISAF registers of the display
# path as ST's chain left them and prints them. It only reads.
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_display_state \
#     yakogl/tools/probes/stm32mp257f_dk/mp2_display_state
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_display_state)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_DISPLAY_STATE_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_DISPLAY_STATE_BUILD STREQUAL "")
  set(MP2_DISPLAY_STATE_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_DISPLAY_STATE_BUILD="${MP2_DISPLAY_STATE_BUILD}")
message(STATUS "mp2_display_state build: ${MP2_DISPLAY_STATE_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
