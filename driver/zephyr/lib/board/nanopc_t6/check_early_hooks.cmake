# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Run after zephyr.elf links (lib/board/CMakeLists.txt, every nanopc_t6
# image): the early banner's hooks must be the board glue's, not Zephyr's
# weak defaults, or the image would start with no T6:A line and no HCR_EL2
# fix while looking like it has both (nanopc_t6.c, "Order").
#   cmake -DNM=<nm> -DELF=<zephyr.elf> -P check_early_hooks.cmake
execute_process(COMMAND "${NM}" "${ELF}" OUTPUT_VARIABLE syms RESULT_VARIABLE rc)
if(NOT rc EQUAL 0)
  message(FATAL_ERROR "nanopc_t6: ${NM} ${ELF} failed (${rc})")
endif()
# Strong ("T"): a weak definition that won would read "W".
foreach(s z_arm64_el2_plat_prep_c z_arm64_el1_plat_prep_c z_arm64_el1_plat_init
          yd_t6_early_entry yd_t6_el2_vectors yd_t6_el2_trap)
  if(NOT syms MATCHES "(^|\n)[0-9a-f]+ T ${s}\n")
    message(FATAL_ERROR "nanopc_t6: ${ELF} has no strong ${s}: the early banner hook is not the board glue's")
  endif()
endforeach()
# VBAR_EL2 takes a 2 KiB aligned table.
string(REGEX MATCH "(^|\n)([0-9a-f]+) T yd_t6_el2_vectors\n" m "${syms}")
math(EXPR low "0x${CMAKE_MATCH_2} % 2048")
if(NOT low EQUAL 0)
  message(FATAL_ERROR "nanopc_t6: yd_t6_el2_vectors at 0x${CMAKE_MATCH_2} is not 2 KiB aligned")
endif()
message(STATUS "nanopc_t6: early hooks are the board glue's; EL2 vectors at 0x${CMAKE_MATCH_2}")
