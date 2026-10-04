# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# include() after find_package(Zephyr): adds the shared sample shape to `app`.
set(GC_SAMPLE_COMMON "${CMAKE_CURRENT_LIST_DIR}")
target_include_directories(app PRIVATE "${GC_SAMPLE_COMMON}")

# The board support itself -- gc_sample, the interrupt guard, the panel
# console, the SNVS record -- is the driver module's yakogl_gc_board library
# (driver/zephyr/lib/board/imx8mp), which gc_common/Kconfig turns on for every
# sample. Only the sample-specific pieces stay here.
if(NOT CONFIG_YAKOGL_GC_BOARD)
  message(FATAL_ERROR "gc_sample.cmake: CONFIG_YAKOGL_GC_BOARD is off; gc_common/Kconfig should default it on")
endif()

# The panel console prints these on its first line, so that a photograph of the
# panel says which sample and which build it is. The stamp comes from CMake
# rather than __DATE__ so the compiler is never asked for the time of day.
string(TIMESTAMP GC_BUILD_STAMP "%Y-%m-%dT%H:%M:%SZ" UTC)
target_compile_definitions(yakogl_gc_board PRIVATE
  GC_SAMPLE_NAME="${PROJECT_NAME}"
  GC_BUILD_STAMP="${GC_BUILD_STAMP}")
