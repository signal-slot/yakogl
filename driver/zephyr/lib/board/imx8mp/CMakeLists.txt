# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# yakogl_gc_board -- see Kconfig. Added by ../CMakeLists.txt when
# CONFIG_YAKOGL_GC_BOARD is set; everything here is plain C for the A53.

zephyr_library_named(yakogl_gc_board)
zephyr_include_directories(. ../common) # ../common: gc_font8x16.h
zephyr_library_sources(gc_sample.c)
zephyr_library_sources_ifdef(CONFIG_GC_BOARD_GIC_EOI_RESET gc_board_gic.c)
zephyr_library_sources_ifdef(CONFIG_GC_COMMON_IRQ_GUARD gc_irq_guard.c)
zephyr_library_sources_ifdef(CONFIG_GC_COMMON_PANEL_CONSOLE gc_panel_console.c)
zephyr_library_sources_ifdef(CONFIG_GC_COMMON_RESULT_SNVS gc_result.c)
zephyr_library_sources_ifdef(CONFIG_GC_BOARD_WDOG_SELF_ARM gc_wdog_arm.c)

# The panel console prints the application's name and build time on its first
# line, so that a photograph of the panel says which image it is. The
# application sets GC_SAMPLE_NAME / GC_BUILD_STAMP on this library after
# find_package(Zephyr) (samples/zephyr/gc_common/gc_sample.cmake does); these
# are the fallbacks gc_panel_console.c uses otherwise.
