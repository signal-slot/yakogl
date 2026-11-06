# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_hello: the first image on the STM32MP257F-DK's A35 (README.md): the
# early banner, the hand-over record, a few seconds on the system timer,
# then PSCI SYSTEM_RESET back into ST's Linux.
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_hello \
#     yakogl/tools/probes/stm32mp257f_dk/mp2_hello
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_hello)

# The stm32mp2 board layer, added by this probe itself for now: the item
# zephyr-layer-hook moves this into driver/zephyr/lib/board/CMakeLists.txt
# (as nanopc_t6 is added there) and removes it here. Added this late, its
# sources go into app (YAKOGL_STM32MP2_IN_APP; the layer's CMakeLists.txt).
set(YAKOGL_STM32MP2_IN_APP ON)
add_subdirectory("${PVR_REPO}/zephyr/lib/board/stm32mp2" "${CMAKE_CURRENT_BINARY_DIR}/yakogl_board_stm32mp2")

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_HELLO_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_HELLO_BUILD STREQUAL "")
  set(MP2_HELLO_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_HELLO_BUILD="${MP2_HELLO_BUILD}")
message(STATUS "mp2_hello build: ${MP2_HELLO_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
