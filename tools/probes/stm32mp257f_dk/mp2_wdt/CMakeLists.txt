# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_wdt: the opt-in OP-TEE watchdog refresh on the STM32MP257F-DK's A35,
# bounded (README.md): INIT, a PET every few seconds for REFRESH_S, then no
# more, so the IWDG resets the board back into ST's Linux.
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_wdt \
#     yakogl/tools/probes/stm32mp257f_dk/mp2_wdt
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_wdt)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_WDT_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_WDT_BUILD STREQUAL "")
  set(MP2_WDT_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_WDT_BUILD="${MP2_WDT_BUILD}")
message(STATUS "mp2_wdt build: ${MP2_WDT_BUILD}")

target_sources(app PRIVATE src/main.c)

# The refresh is the yakogl-drivers crate's soc::stm32mp2::smc_wdt_refresh
# (src/mp2_wdtr.h): the crate is built with the feature stm32mp2 and linked
# by driver/zephyr/CMakeLists.txt when the board layer's
# CONFIG_YAKOGL_STM32MP2_GPU is on (prj.conf), as for mp2_gpu. Without the
# Vivante driver nothing of the GPU's runs; the archive is only linked.
if(NOT CONFIG_YAKOGL_STM32MP2_GPU)
  message(FATAL_ERROR "mp2_wdt: CONFIG_YAKOGL_STM32MP2_GPU is off (the crate is not linked)")
endif()

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
