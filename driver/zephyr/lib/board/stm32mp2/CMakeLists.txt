# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The STM32MP257F-DK's Cortex-A35 board layer core (stm32mp2.c,
# stm32mp2_entry.S; Kconfig here).
#
# Not yet added by ../CMakeLists.txt: until the item zephyr-layer-hook adds it
# there, an application adds this directory itself after find_package(Zephyr)
# with YAKOGL_STM32MP2_IN_APP set (tools/probes/stm32mp257f_dk/mp2_hello/
# CMakeLists.txt). A library made then would never be linked: Zephyr takes
# the list of its libraries for the link while find_package(Zephyr) runs
# (zephyr/CMakeLists.txt, "get_property(ZEPHYR_LIBS_PROPERTY ...)"), so in
# that case the sources go into the application's own library, app.
if(CONFIG_YAKOGL_STM32MP2)
  set(_mp2_sources ${CMAKE_CURRENT_SOURCE_DIR}/stm32mp2.c
                   ${CMAKE_CURRENT_SOURCE_DIR}/stm32mp2_entry.S)
  if(YAKOGL_STM32MP2_IN_APP)
    target_sources(app PRIVATE ${_mp2_sources})
    target_include_directories(app PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
    if(YAKOGL_WERROR)
      target_compile_options(app PRIVATE -Werror)
    endif()
  else()
    zephyr_library_named(yakogl_board_stm32mp2)
    zephyr_library_sources(${_mp2_sources})
    zephyr_include_directories(.)
    # tools/check.sh builds the probes with -DYAKOGL_WERROR=ON.
    if(YAKOGL_WERROR)
      zephyr_library_compile_options(-Werror)
    endif()
  endif()
  # The image header's "b __start" goes to stm32mp2_entry.S's __wrap___start.
  zephyr_link_libraries(-Wl,--wrap=__start)
  # The entry code must be the header's branch target in the linked image,
  # and the record it writes must survive the BSS clear.
  set(_mp2_check COMMAND ${CMAKE_COMMAND} -DNM=${CMAKE_NM} -DOBJDUMP=${CMAKE_OBJDUMP}
      -DELF=${ZEPHYR_BINARY_DIR}/${KERNEL_ELF_NAME}
      -P ${CMAKE_CURRENT_SOURCE_DIR}/check_early_hooks.cmake)
  if(YAKOGL_STM32MP2_IN_APP)
    # Zephyr has read extra_post_build_commands too by now: a target of the
    # default build after the ELF's link instead.
    add_custom_target(yakogl_stm32mp2_check_early_hooks ALL ${_mp2_check})
    add_dependencies(yakogl_stm32mp2_check_early_hooks ${logical_target_for_zephyr_elf})
  else()
    set_property(GLOBAL APPEND PROPERTY extra_post_build_commands ${_mp2_check})
  endif()
endif()
