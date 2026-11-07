# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_identify: the STM32MP257F-DK's return-trial image (README.md): the
# early banner, the hand-over record, OP-TEE's watchdog armed at boot, then
# PSCI SYSTEM_RESET back into ST's Linux. No GPU access.
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_identify \
#     yakogl/tools/probes/stm32mp257f_dk/mp2_identify
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_identify)

# The stm32mp2 board layer, added by this probe itself as mp2_hello adds it:
# the item zephyr-layer-hook moves this into driver/zephyr/lib/board/
# CMakeLists.txt and removes it here (and the rsource in Kconfig). Added this
# late, its sources go into app (YAKOGL_STM32MP2_IN_APP; the layer's
# CMakeLists.txt).
set(YAKOGL_STM32MP2_IN_APP ON)
add_subdirectory("${PVR_REPO}/zephyr/lib/board/stm32mp2" "${CMAKE_CURRENT_BINARY_DIR}/yakogl_board_stm32mp2")

# The OP-TEE watchdog (stm32mp2_wdog.c), compiled here for now: the layer's
# own CMakeLists.txt takes the source over after zephyr-layer-hook merges
# (the item st-customer-id), and this line goes then.
if(CONFIG_YAKOGL_STM32MP2_WDOG)
  target_sources(app PRIVATE "${PVR_REPO}/zephyr/lib/board/stm32mp2/stm32mp2_wdog.c")
endif()

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_IDENTIFY_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_IDENTIFY_BUILD STREQUAL "")
  set(MP2_IDENTIFY_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_IDENTIFY_BUILD="${MP2_IDENTIFY_BUILD}")
message(STATUS "mp2_identify build: ${MP2_IDENTIFY_BUILD}")

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
