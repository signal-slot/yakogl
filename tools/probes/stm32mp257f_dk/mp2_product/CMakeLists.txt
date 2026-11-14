# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_product: the STM32MP257F-DK's product image shape for a standalone
# default (README.md): INIT of OP-TEE's watchdog, a PET every 5 s while a
# heartbeat advances, a line a minute, and the product watchdog marker. Its
# own lifetime (CONFIG_MP2_PRODUCT_LIFETIME_S, 120 s) brings a counted run
# back to Linux; 0 is the product's "runs indefinitely".
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_product \
#     yakogl/tools/probes/stm32mp257f_dk/mp2_product
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_product)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_PRODUCT_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_PRODUCT_BUILD STREQUAL "")
  set(MP2_PRODUCT_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_PRODUCT_BUILD="${MP2_PRODUCT_BUILD}")
message(STATUS "mp2_product build: ${MP2_PRODUCT_BUILD}")

target_sources(app PRIVATE src/main.c)
# The refresh's C ABI is mp2_wdt's header (pinned against the Rust side by
# smc_wdt_refresh.rs's host test); one copy, read from there.
target_include_directories(app PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/../mp2_wdt/src)

# The refresh is the yakogl-drivers crate's soc::stm32mp2::smc_wdt_refresh,
# linked by driver/zephyr/CMakeLists.txt when the board layer's
# CONFIG_YAKOGL_STM32MP2_GPU is on (prj.conf), as for mp2_wdt. Without the
# Vivante driver nothing of the GPU's runs; the archive is only linked.
if(NOT CONFIG_YAKOGL_STM32MP2_GPU)
  message(FATAL_ERROR "mp2_product: CONFIG_YAKOGL_STM32MP2_GPU is off (the crate is not linked)")
endif()

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
