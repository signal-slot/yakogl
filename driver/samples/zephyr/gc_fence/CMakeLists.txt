# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# gc_fence: one command buffer whose fence retires through the interrupt, on
# any board of the board layer with a Vivante GPU (README.md). The board is
# chosen by west's -b only: what a board needs (its GPU node, carve-out,
# watchdog, record sink) is the board layer's app.conf and app.overlay, which
# examples/zephyr.cmake applies.
cmake_minimum_required(VERSION 3.20.0)
include("${CMAKE_CURRENT_SOURCE_DIR}/../../../../examples/zephyr.cmake")
project(gc_fence)

if(NOT CONFIG_YAKOGL_DRV_GPU_VIVANTE)
  message(FATAL_ERROR "gc_fence: ${BOARD} has no Vivante GPU (CONFIG_YAKOGL_DRV_GPU_VIVANTE is off)")
endif()

target_sources(app PRIVATE src/main.c)

# Zero warnings in our C files: the gate builds samples with -DYAKOGL_WERROR=ON.
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
