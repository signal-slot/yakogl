# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Run after zephyr.elf links (lib/board/CMakeLists.txt, every nanopc_t6
# image): the early banner's hooks must be the board glue's, not Zephyr's
# weak defaults, or the image would start with no T6:A line, no HCR_EL2 fix
# and no WDT0 while looking like it has all three (nanopc_t6.c, "Order");
# WDT0 must be armed from z_arm64_el1_plat_init, before every init hook, and
# the code that runs with the MMU off must not call memset/memcpy/memmove,
# branch through a register or use a load/store offset that is not a
# multiple of its access size.
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
# function the MMU-off code reaches by a direct branch is walked. A branch
# through a register cannot be followed and is refused: a call or a jump
# (blr, br) and their pointer-authentication forms (blraa, blrab, blraaz,
# blrabz, braa, brab, braaz, brabz). So is a load or store whose immediate
# offset is not a multiple of its access size (a packed field gives one).
# Not checked: whether a base register is aligned; nothing here can know
# what an address register holds.
set(todo z_arm64_el2_plat_prep_c z_arm64_el1_plat_prep_c z_arm64_el1_plat_init yd_t6_el2_trap)
set(seen "")
set(accesses 0)
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
  string(REGEX MATCH "[^\n]*\t(blr|br)(aa|ab|aaz|abz)?\t[^\n]*" ind "${d}")
  if(ind)
    message(FATAL_ERROR "nanopc_t6: ${ELF}: ${fn}, run with the MMU off, branches through a register "
      "(the walk cannot follow it):\n${ind}")
  endif()
  # Loads and stores with an immediate offset inside the brackets (pre-index
  # and plain; a post-index access uses the base alone). The access size is
  # the mnemonic's (b, h, sw) or the first register's (w, x, b, h, s, d, q).
  string(REGEX MATCHALL "\t(ld|st)[a-z0-9]*\t[^\n]*\[[a-z0-9]+, #-?(0x)?[0-9a-f]+\]" mems "${d}")
  foreach(a ${mems})
    string(REGEX MATCH "\t((ld|st)[a-z0-9]*)\t([a-z])[0-9a-z]*,.*, #(-?(0x)?[0-9a-f]+)\]" m "${a}")
    set(op "${CMAKE_MATCH_1}")
    set(reg "${CMAKE_MATCH_3}")
    set(off "${CMAKE_MATCH_4}")
    if(op STREQUAL "")
      continue()
    endif()
    if(op MATCHES "^(ld|st)[a-z]*(sb|b)$")
      set(size 1)
    elseif(op MATCHES "^(ld|st)[a-z]*(sh|h)$")
      set(size 2)
    elseif(op MATCHES "sw$")
      set(size 4)
    elseif(reg STREQUAL "w" OR reg STREQUAL "s")
      set(size 4)
    elseif(reg STREQUAL "x" OR reg STREQUAL "d")
      set(size 8)
    elseif(reg STREQUAL "q")
      set(size 16)
    elseif(reg STREQUAL "h")
      set(size 2)
    elseif(reg STREQUAL "b")
      set(size 1)
    else()
      continue()
    endif()
    math(EXPR accesses "${accesses} + 1")
    math(EXPR r "(${off}) % ${size}")
    if(NOT r EQUAL 0)
      message(FATAL_ERROR "nanopc_t6: ${ELF}: ${fn}, run with the MMU off, has an access whose offset "
        "is not a multiple of its size ${size}:${a}")
    endif()
  endforeach()
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
  "WDT0 armed from z_arm64_el1_plat_init; ${n} MMU-off functions call no memset/memcpy, "
  "branch through no register; ${accesses} offsets are multiples of their access size "
  "(base alignment is not checked)")
