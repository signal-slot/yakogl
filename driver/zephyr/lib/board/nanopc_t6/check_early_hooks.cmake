# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Run after zephyr.elf links (lib/board/CMakeLists.txt, every nanopc_t6
# image): the early banner's hooks must be the board glue's, not Zephyr's
# weak defaults, or the image would start with no T6:A line, no HCR_EL2 fix
# and no WDT0 while looking like it has all three (nanopc_t6.c, "Order");
# WDT0 must be armed from z_arm64_el1_plat_init, before every init hook, and
# the code that runs with the MMU off must not call memset/memcpy/memmove.
#   cmake -DNM=<nm> -DOBJDUMP=<objdump> -DELF=<zephyr.elf> -P check_early_hooks.cmake
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
set(vectors "${CMAKE_MATCH_2}")

# disasm(<function> <out var>): the function's instruction lines.
function(disasm fn out)
  execute_process(COMMAND "${OBJDUMP}" -d --no-show-raw-insn "--disassemble=${fn}" "${ELF}"
                  OUTPUT_VARIABLE d RESULT_VARIABLE rc)
  if(NOT rc EQUAL 0)
    message(FATAL_ERROR "nanopc_t6: ${OBJDUMP} --disassemble=${fn} ${ELF} failed (${rc})")
  endif()
  if(NOT d MATCHES "\n[0-9a-f]+ <${fn}>:\n")
    message(FATAL_ERROR "nanopc_t6: ${ELF} has no function ${fn} to disassemble")
  endif()
  string(REGEX MATCHALL " +[0-9a-f]+:\t[^\n]*\n" lines "${d}")
  string(JOIN "" d ${lines})
  set(${out} "${d}" PARENT_SCOPE)
endfunction()

# WDT0 is armed by z_arm64_el1_plat_init, which reset.S calls before
# z_prep_c and so before every init hook of every level; an arming in a
# SYS_INIT hook would run after any other hook of its level and priority that
# the linker put first (with CONFIG_POSIX_API, four at PRE_KERNEL_1 0).
disasm(z_arm64_el1_plat_init d)
if(NOT d MATCHES "\t(bl|b)\t[0-9a-f]+ <yd_t6_early_arm>\n")
  message(FATAL_ERROR "nanopc_t6: ${ELF}: z_arm64_el1_plat_init does not call yd_t6_early_arm: "
    "WDT0 is not armed before the init hooks. It has:\n${d}")
endif()
if(syms MATCHES "(^|\n)[0-9a-f]+ [Rr] __init_t6_wdt_arm\n")
  message(FATAL_ERROR "nanopc_t6: ${ELF} still arms WDT0 in a SYS_INIT hook (__init_t6_wdt_arm)")
endif()

# With the MMU off every access is to Device memory: a memset may use dc zva
# and a memcpy unaligned accesses, and both fault there, before any
# watchdog or with one that then resets a board that printed nothing. Every
# function the MMU-off code reaches by a direct branch is walked; a call
# through a register (blr) cannot be followed and is refused as well.
set(todo z_arm64_el2_plat_prep_c z_arm64_el1_plat_prep_c z_arm64_el1_plat_init yd_t6_el2_trap)
set(seen "")
while(todo)
  list(POP_FRONT todo fn)
  if(fn IN_LIST seen)
    continue()
  endif()
  list(APPEND seen "${fn}")
  if(fn MATCHES "^(memset|memcpy|memmove|bzero|__memset.*|__memcpy.*|__memmove.*)$")
    message(FATAL_ERROR "nanopc_t6: ${ELF}: the MMU-off code reaches ${fn} (walked: ${seen})")
  endif()
  disasm("${fn}" d)
  if(d MATCHES "\tblr\t")
    message(FATAL_ERROR "nanopc_t6: ${ELF}: ${fn}, run with the MMU off, calls through a register:\n${d}")
  endif()
  string(REGEX MATCHALL "\t(bl|b|b\\.[a-z]+|cbn?z|tbn?z)\t[^\n]*<[^>+]+>\n" calls "${d}")
  foreach(c ${calls})
    string(REGEX MATCH "<([^>+]+)>" m "${c}")
    if(NOT CMAKE_MATCH_1 STREQUAL fn)
      list(APPEND todo "${CMAKE_MATCH_1}")
    endif()
  endforeach()
endwhile()
list(LENGTH seen n)
message(STATUS "nanopc_t6: early hooks are the board glue's; EL2 vectors at 0x${vectors}; "
  "WDT0 armed from z_arm64_el1_plat_init; ${n} MMU-off functions call no memset/memcpy")
