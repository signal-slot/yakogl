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
  # A GL image (the Mali OSAL in it) embeds the CSF firmware and hands it to
  # the OSAL (nanopc_t6/nanopc_t6_mali.c): driver/src/fw/mali_csffw_blob.h,
  # made by driver/tools/mkmalifw.sh, never committed.
  if(CONFIG_YAKOGL_GPU_MALI)
    get_filename_component(_t6_repo "${CMAKE_CURRENT_SOURCE_DIR}/../../../.." ABSOLUTE)
    zephyr_library_sources(nanopc_t6/nanopc_t6_mali.c)
    zephyr_library_include_directories("${_t6_repo}/src/osal" "${_t6_repo}/driver/src")
  endif()
  # The early banner's reset.S hooks and the WDT0 arming must be ours in
  # the linked image, and its MMU-off code must call no memset/memcpy and
  # branch through no register (nanopc_t6/check_early_hooks.cmake).
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands
    COMMAND ${CMAKE_COMMAND} -DNM=${CMAKE_NM} -DOBJDUMP=${CMAKE_OBJDUMP}
            -DELF=${ZEPHYR_BINARY_DIR}/${KERNEL_ELF_NAME}
            -P ${CMAKE_CURRENT_SOURCE_DIR}/nanopc_t6/check_early_hooks.cmake)
  # The main thread's stack must hold the image's deepest direct call path
  # (common/check_main_stack.py, its own cases first): nothing guards the
  # stack, and below it lie the idle thread's and the interrupt stack.
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands
    COMMAND ${PYTHON_EXECUTABLE}
            ${CMAKE_CURRENT_SOURCE_DIR}/common/check_main_stack.py --selftest
    COMMAND ${PYTHON_EXECUTABLE}
            ${CMAKE_CURRENT_SOURCE_DIR}/common/check_main_stack.py
            ${CMAKE_NM} ${CMAKE_OBJDUMP}
            ${ZEPHYR_BINARY_DIR}/${KERNEL_ELF_NAME})
  # tools/check.sh builds t6_hello with -DYAKOGL_WERROR=ON (the "zephyr" rows).
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()
endif()

# The STM32MP257F-DK's A35 board layer core (stm32mp2/stm32mp2.c): every image
# for the board (stm32mp2/Kconfig).
add_subdirectory_ifdef(CONFIG_YAKOGL_STM32MP2 stm32mp2)

# The Verdin iMX95's A55 glue (verdin_imx95/verdin_imx95.c: the PSCI reset, the
# lifetime timer; verdin_imx95_wdog.c: WDOG3 over ip::wdog32 of the
# yakogl-drivers crate), its board layer (the windows, and the X4 display:
# verdin_imx95_display.c), the raw SCMI channel (verdin_imx95_scmi.c), and in
# a Mali image the OSAL's board half (verdin_imx95_mali.c: the GPU's power-on
# over SCMI and its bring-up, yd_imx95_osal_up, with the CSF firmware
# driver/src/fw/mali_csffw_blob.h embedded, made by driver/tools/mkmalifw.sh,
# never committed): every image for the board (verdin_imx95/Kconfig).
if(CONFIG_YAKOGL_VERDIN_IMX95)
  zephyr_library_named(yakogl_board_verdin_imx95)
  zephyr_library_sources(verdin_imx95/verdin_imx95.c verdin_imx95/verdin_imx95_wdog.c)
  add_dependencies(yakogl_board_verdin_imx95 yakogl_drivers_rust)
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_BOARD_VERDIN_IMX95 verdin_imx95/board_verdin_imx95.c)
  zephyr_include_directories(verdin_imx95)
  if(CONFIG_YAKOGL_GPU_MALI OR CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY OR CONFIG_YAKOGL_VERDIN_IMX95_SCMI)
    # the one raw SCMI channel of the GPU's power-on and the display's (and
    # of the probes, tools/probes/verdin_imx95)
    zephyr_library_sources(verdin_imx95/verdin_imx95_scmi.c)
  endif()
  if(CONFIG_YAKOGL_GPU_MALI)
    get_filename_component(_vi95_repo "${CMAKE_CURRENT_SOURCE_DIR}/../../../.." ABSOLUTE)
    zephyr_library_sources(verdin_imx95/verdin_imx95_mali.c)
    zephyr_library_include_directories("${_vi95_repo}/src/osal" "${_vi95_repo}/driver/src")
  endif()
  if(CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY)
    get_filename_component(_vi95_drv "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
    zephyr_library_sources(verdin_imx95/verdin_imx95_display.c)
    zephyr_library_include_directories("${_vi95_drv}/include")
  endif()
  # tools/check.sh builds the board's images with -DYAKOGL_WERROR=ON.
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()
endif()

# The Raspberry Pi 5's PM watchdog alone (rpi4/rpi_wdog.c, the SoC a parameter).
if(CONFIG_YAKOGL_RPI5_WDOG)
  zephyr_library_named(yakogl_board_rpi5_wdog)
  zephyr_library_sources(rpi4/rpi_wdog.c)
  zephyr_library_include_directories(rpi4)
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()
endif()

# The Raspberry Pi 5's property mailbox (rpi4/rpi_mbox.c, the SoC a parameter):
# the firmware clock 5 and the clock reads, polling, for the probes.
if(CONFIG_YAKOGL_RPI5_MBOX)
  zephyr_library_named(yakogl_board_rpi5_mbox)
  zephyr_library_sources(rpi4/rpi_mbox.c)
  zephyr_library_include_directories(rpi4)
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()
endif()

# The Raspberry Pi 5's screen (rpi5/board_rpi5.c over the Pi 4's rpi4/rpi_mbox.c
# and rpi4/fbcon.c, the SoC a parameter; the decisions from the yakogl-drivers
# crate). YAKOGL_BOARD_RPI5 selects YAKOGL_RPI5_MBOX, whose library carries
# rpi_mbox.c: this one carries the screen and fbcon.
if(CONFIG_YAKOGL_BOARD_RPI5)
  zephyr_library_named(yakogl_board_rpi5)
  zephyr_library_sources(rpi5/board_rpi5.c rpi4/fbcon.c)
  zephyr_library_include_directories(rpi5 rpi4)
  if(TARGET yakogl_drivers_rust)
    add_dependencies(yakogl_board_rpi5 yakogl_drivers_rust)
  endif()
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
