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
# find_package(Zephyr) (samples/zephyr/gl_test/CMakeLists.txt does); these
# are the fallbacks gc_panel_console.c uses otherwise.

# Route 3 from the direct image (board_route3.c; docs/notes/
# imx8mp-direct-boot.md, "Route 3"): the M7 firmware named by
# CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_ELF, stripped to its program headers and
# segments (m7_image.py) and linked into .rodata. The frame-ring protocol is
# the frame-ring header copy in samples/zephyr/cluster.
if(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3)
  set(m7_elf "${CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_ELF}")
  if(m7_elf STREQUAL "" OR NOT EXISTS "${m7_elf}")
    message(FATAL_ERROR "YAKOGL_BOARD_IMX8MP_ROUTE3: the M7 ELF '${m7_elf}' does not exist "
                        "(CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_ELF)")
  endif()
  set(M7_STRIPPED "${CMAKE_CURRENT_BINARY_DIR}/m7_image.elf")
  add_custom_command(
    OUTPUT "${M7_STRIPPED}"
    COMMAND ${PYTHON_EXECUTABLE} "${CMAKE_CURRENT_LIST_DIR}/m7_image.py" "${m7_elf}" "${M7_STRIPPED}"
    DEPENDS "${m7_elf}" "${CMAKE_CURRENT_LIST_DIR}/m7_image.py"
    COMMENT "route 3: stripping the M7 firmware ${m7_elf}"
    VERBATIM)
  add_custom_target(yakogl_m7_image DEPENDS "${M7_STRIPPED}")
  configure_file("${CMAKE_CURRENT_LIST_DIR}/m7_blob.S.in" "${CMAKE_CURRENT_BINARY_DIR}/m7_blob.S" @ONLY)
  set_source_files_properties("${CMAKE_CURRENT_BINARY_DIR}/m7_blob.S" PROPERTIES OBJECT_DEPENDS "${M7_STRIPPED}")
  zephyr_library_sources(board_route3.c "${CMAKE_CURRENT_BINARY_DIR}/m7_blob.S")
  add_dependencies(yakogl_gc_board yakogl_m7_image)
  zephyr_include_directories("${CMAKE_CURRENT_LIST_DIR}/../../../../samples/zephyr/cluster/include")
endif()
