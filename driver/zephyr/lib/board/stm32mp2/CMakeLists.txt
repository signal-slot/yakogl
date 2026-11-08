# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The STM32MP257F-DK's Cortex-A35 board layer core (stm32mp2.c,
# stm32mp2_entry.S; Kconfig here). Added by ../CMakeLists.txt, as the other
# boards are, for every image built for the board.
if(CONFIG_YAKOGL_STM32MP2)
  zephyr_library_named(yakogl_board_stm32mp2)
  zephyr_library_sources(stm32mp2.c stm32mp2_entry.S)
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_STM32MP2_WDOG stm32mp2_wdog.c)
  zephyr_include_directories(.)
  # tools/check.sh builds the probes with -DYAKOGL_WERROR=ON.
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()
  # The image header's "b __start" goes to stm32mp2_entry.S's __wrap___start.
  zephyr_link_libraries(-Wl,--wrap=__start)
  # The entry code must be the header's branch target in the linked image,
  # and the record it writes must survive the BSS clear.
  set_property(GLOBAL APPEND PROPERTY extra_post_build_commands
    COMMAND ${CMAKE_COMMAND} -DNM=${CMAKE_NM} -DOBJDUMP=${CMAKE_OBJDUMP}
            -DELF=${ZEPHYR_BINARY_DIR}/${KERNEL_ELF_NAME}
            -P ${CMAKE_CURRENT_SOURCE_DIR}/check_early_hooks.cmake)
endif()
