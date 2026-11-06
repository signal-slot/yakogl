# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Run after zephyr.elf links (every image with CONFIG_YAKOGL_STM32MP2). The
# build fails unless the board layer's early code is what the linked image
# runs. Without it the image would start with no MP2:A line and an empty
# hand-over record while looking like it has both (stm32mp2.c, "Order").
#   cmake -DNM=<nm> -DOBJDUMP=<objdump> -DELF=<zephyr.elf> -P check_early_hooks.cmake
execute_process(COMMAND "${NM}" "${ELF}" OUTPUT_VARIABLE syms RESULT_VARIABLE rc)
if(NOT rc EQUAL 0)
  message(FATAL_ERROR "stm32mp2: ${NM} ${ELF} failed (${rc})")
endif()
# Strong ("T"): a weak definition that won would read "W".
foreach(s __wrap___start z_arm64_el1_plat_init k_sys_fatal_error_handler)
  if(NOT syms MATCHES "(^|\n)[0-9a-f]+ T ${s}\n")
    message(FATAL_ERROR "stm32mp2: ${ELF} has no strong ${s}: the early code is not the board layer's")
  endif()
endforeach()
# The image header's first word is "b __start" (header.S); with
# --wrap=__start it must branch to __wrap___start.
string(REGEX MATCH "(^|\n)([0-9a-f]+) [Tt] _image_header\n" m "${syms}")
if(NOT m)
  message(FATAL_ERROR "stm32mp2: ${ELF} has no _image_header (CONFIG_AARCH64_IMAGE_HEADER)")
endif()
math(EXPR hdr_end "0x${CMAKE_MATCH_2} + 4" OUTPUT_FORMAT HEXADECIMAL)
execute_process(COMMAND "${OBJDUMP}" -d --start-address=0x${CMAKE_MATCH_2}
                        --stop-address=${hdr_end} "${ELF}"
                OUTPUT_VARIABLE hdr RESULT_VARIABLE rc)
if(NOT rc EQUAL 0)
  message(FATAL_ERROR "stm32mp2: ${OBJDUMP} ${ELF} failed (${rc})")
endif()
if(NOT hdr MATCHES "\tb\t[0-9a-f]+ <__wrap___start>")
  message(FATAL_ERROR "stm32mp2: the image header does not branch to __wrap___start (--wrap=__start lost):\n${hdr}")
endif()
# Initialised data ("D"), not BSS ("B"): z_prep_c clears BSS after the
# entry code ran.
string(REGEX MATCH "(^|\n)([0-9a-f]+) D mp2_handover\n" m "${syms}")
if(NOT m)
  message(FATAL_ERROR "stm32mp2: mp2_handover is not in .data: the BSS clear would erase the record")
endif()
math(EXPR low "0x${CMAKE_MATCH_2} % 64")
if(NOT low EQUAL 0)
  message(FATAL_ERROR "stm32mp2: mp2_handover at 0x${CMAKE_MATCH_2} is not 64-byte aligned")
endif()
message(STATUS "stm32mp2: the header branches to __wrap___start; hand-over record at 0x${CMAKE_MATCH_2}")
