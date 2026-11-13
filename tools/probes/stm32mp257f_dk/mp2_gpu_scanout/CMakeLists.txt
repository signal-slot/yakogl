# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2_gpu_scanout: the STM32MP257F-DK's GPU-rendered frame on its HDMI
# (README.md). gc_triangle's clear, triangle and three-apex read-back at the
# display mode's size, the resolve engine's copy into a linear XRGB8888
# buffer, and that buffer as the LTDC's layer (LTDC -> DSI -> ADV7535 -> HDMI).
#
#   west build -p always -b stm32mp257f_dk/stm32mp257fxx/a35 -d build/mp2_gpu_scanout \
#     tools/probes/stm32mp257f_dk/mp2_gpu_scanout
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the SoC, the board, the board layer, the GPU
# and display drivers). The overlay boards/stm32mp257f_dk_stm32mp257fxx_a35.overlay
# is picked up by name.
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(mp2_gpu_scanout)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE MP2_GPU_SCANOUT_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR MP2_GPU_SCANOUT_BUILD STREQUAL "")
  set(MP2_GPU_SCANOUT_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE MP2_GPU_SCANOUT_BUILD="${MP2_GPU_SCANOUT_BUILD}")
message(STATUS "mp2_gpu_scanout build: ${MP2_GPU_SCANOUT_BUILD}")

# The GPU driver (src/vivante behind gpu_vivante_gc_rust.c) comes with the
# overlay's &gpu; the display chain with its ltdc, dsi, i2c2 and adv7535.
if(NOT CONFIG_YAKOGL_DRV_GPU_VIVANTE)
  message(FATAL_ERROR "mp2_gpu_scanout: CONFIG_YAKOGL_DRV_GPU_VIVANTE is off (no vivante,gc node)")
endif()
if(NOT CONFIG_YAKOGL_STM32MP2_DISPLAY)
  message(FATAL_ERROR "mp2_gpu_scanout: CONFIG_YAKOGL_STM32MP2_DISPLAY is off (the display path is not powered)")
endif()

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files with -DYAKOGL_WERROR=ON; off by default.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
