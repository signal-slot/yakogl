# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The board layer (include/yakogl_board.h; driver/docs/one-method.md,
# section 2). Added by ../../CMakeLists.txt for every image.

# yakogl_board.h and yakogl_record.h for every image; board_internal.h for
# the board code that is still compiled by samples (rpi4/rpi_wdog.c).
zephyr_include_directories(include common)

# The i.MX8MP board support (watchdog, fatal handler, interrupt guard, SNVS
# record, panel console): the yakogl_gc_board library, kept under its name
# for the applications outside this repository that link it.
add_subdirectory_ifdef(CONFIG_YAKOGL_GC_BOARD imx8mp)

# The AM62P MAIN RTI0 self-arm (am62p/rti_arm.c), for the netboot images.
if(CONFIG_YAKOGL_AM62P_RTI_SELF_ARM)
  zephyr_library_named(yakogl_board_am62p_rti)
  zephyr_library_sources(am62p/rti_arm.c)
endif()

# The NanoPC-T6 glue (nanopc_t6/nanopc_t6.c over the yakogl-drivers crate,
# feature rk3588): every image for the board (nanopc_t6/Kconfig).
if(CONFIG_YAKOGL_NANOPC_T6)
  zephyr_library_named(yakogl_board_nanopc_t6)
  zephyr_library_sources(nanopc_t6/nanopc_t6.c)
  zephyr_include_directories(nanopc_t6)
  add_dependencies(yakogl_board_nanopc_t6 yakogl_drivers_rust)
  # The early banner's reset.S hooks must be ours in the linked image.
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands
    COMMAND ${CMAKE_COMMAND} -DNM=${CMAKE_NM} -DELF=${ZEPHYR_BINARY_DIR}/${KERNEL_ELF_NAME}
            -P ${CMAKE_CURRENT_SOURCE_DIR}/nanopc_t6/check_early_hooks.cmake)
  # tools/check.sh builds t6_hello with -DYAKOGL_WERROR=ON (the "zephyr" rows).
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()
endif()

# The record writer (common/record.c over the Rust rules): for the board
# layer, and for the i.MX8MP's gc_result.c, which is its sink.
if(CONFIG_YAKOGL_BOARD OR CONFIG_GC_COMMON_RESULT_SNVS)
  zephyr_library_named(yakogl_board_record)
  zephyr_library_sources(common/record.c)
  if(TARGET yakogl_drivers_rust)
    add_dependencies(yakogl_board_record yakogl_drivers_rust)
  endif()
endif()

if(CONFIG_YAKOGL_BOARD)
  zephyr_library_named(yakogl_board)
  zephyr_library_sources(common/board.c common/sample_glue.c)

  # The build identity sample_glue.c prints first (sample.cmake does the same
  # for the samples that are not on the board layer yet).
  get_filename_component(_yb_repo "${CMAKE_CURRENT_SOURCE_DIR}/../../../.." ABSOLUTE)
  execute_process(COMMAND git -C "${_yb_repo}" describe --always --dirty
    OUTPUT_VARIABLE _yb_tree OUTPUT_STRIP_TRAILING_WHITESPACE ERROR_QUIET RESULT_VARIABLE _yb_rc)
  if(NOT _yb_rc EQUAL 0 OR _yb_tree STREQUAL "")
    set(_yb_tree "unknown")
  endif()
  set(_yb_table "none")
  if(EXISTS "${_yb_repo}/tests/reference/blobs.rs")
    file(SHA256 "${_yb_repo}/tests/reference/blobs.rs" _yb_sha)
    string(SUBSTRING "${_yb_sha}" 0 8 _yb_table)
  endif()
  zephyr_library_compile_definitions(YAKOGL_SAMPLE_TREE="${_yb_tree}" YAKOGL_SAMPLE_TABLE="${_yb_table}")

  if(CONFIG_YAKOGL_BOARD_AM62P)
    zephyr_library_sources(am62p/board_am62p.c)
    zephyr_library_sources_ifdef(CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP am62p/display_dump.c)
  elseif(CONFIG_YAKOGL_BOARD_IMX8MP)
    zephyr_library_sources(imx8mp/board_imx8mp.c)
  elseif(CONFIG_YAKOGL_BOARD_RPI4)
    zephyr_library_sources(rpi4/board_rpi4.c rpi4/rpi_mbox.c rpi4/fbcon.c rpi4/rpi_wdog.c)
    zephyr_library_include_directories(rpi4 "${_yb_repo}/src/osal")
  endif()
endif()
