# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The STM32MP257F-DK's Cortex-A35 board layer core (stm32mp2.c,
# stm32mp2_entry.S; Kconfig here). Added by ../CMakeLists.txt, as the other
# boards are, for every image built for the board.
if(CONFIG_YAKOGL_STM32MP2)
  zephyr_library_named(yakogl_board_stm32mp2)
  zephyr_library_sources(stm32mp2.c stm32mp2_entry.S)
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_STM32MP2_WDOG stm32mp2_wdog.c)
  # The board layer (yakogl_board.h) for gc_fence and gc_clear.
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_BOARD_STM32MP2 board_stm32mp2.c)
  # The GPU powered for the Vivante driver (gc_board_power_on) through the
  # crate's stm32mp2_gpu.h (CONFIG_YAKOGL_STM32MP2_GPU: the crate built by
  # ../../../CMakeLists.txt with the features stm32mp2 and vivante), and
  # the read-back line of its interrupt, GIC SPI 183 (gc_board_irq_connected).
  if(CONFIG_YAKOGL_STM32MP2_GPU AND CONFIG_YAKOGL_DRV_GPU_VIVANTE)
    zephyr_library_sources(gc_power_mp2.c gc_irq_mp2.c)
  endif()
  # The display path powered for its drivers (display_power_mp2.c over the
  # crate's stm32mp2_display.h) and the LTDC driver's board hooks.
  zephyr_library_sources_ifdef(CONFIG_YAKOGL_STM32MP2_DISPLAY display_power_mp2.c)
  zephyr_include_directories(.)
  # stm32mp2_gpu.h: the crate's GPU power C ABI (soc/stm32mp2/gpu_ffi.rs),
  # for the images that link the crate with it (CONFIG_YAKOGL_STM32MP2_GPU:
  # mp2_gpu, gc_fence, gc_clear); stm32mp2_display.h the display path's
  # (soc/stm32mp2/display_ffi.rs, CONFIG_YAKOGL_STM32MP2_DISPLAY).
  zephyr_include_directories(include)
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
