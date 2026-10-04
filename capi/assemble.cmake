# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# capi/assemble.cmake - turn cargo's libyakogl_capi.a into libyakogl.a (or
# another no_std staticlib of this workspace into its archive, e.g.
# driver/rust's libyakogl_drivers.a).
#
#   cmake -DRUST_LIB=<cargo staticlib> -DOUTPUT=<libyakogl.a>
#         -DAR=<ar> -DNM=<nm> -DOBJCOPY=<objcopy>
#         [-DLTO_MEMBER=<crate's [lib] name, default yakogl_capi>]
#         [-DC_OBJECTS=<obj|obj|...>] -P assemble.cmake
#
# LTO_MEMBER names the crate's own LTO object (archive member
# `<LTO_MEMBER>-<hash>...o`); everything else in the archive must be
# compiler_builtins.
#
# Used by all three builds (CMakeLists.txt, baremetal.mk, zephyr/) and by
# driver/zephyr/CMakeLists.txt for the drivers archive. The Rust
# staticlib holds one LTO object (yakogl + core + the panic handler) and the
# members of compiler_builtins. Linked as they are, they would take over
# symbols that belong to the platform or to another Rust staticlib:
#
# - compiler_builtins defines memcpy, memset, memmove, memcmp, bcmp, strlen
#   and ~200 libm functions (sin, pow, ...), weak. An application that links
#   libyakogl.a before its libc would get these instead of the platform's
#   (optimised) ones, for the whole image. They are made LOCAL here: the
#   library then calls the platform's libc/libm like any C library.
# - Rust runtime symbols (`_R...`/`_ZN...` names, `rust_*`) in the LTO object
#   would collide with another Rust staticlib of the same toolchain. The panic
#   handler is already internal after LTO; anything that is not must be
#   LOCAL too. This needs the single LTO object: without LTO (plain `dev`
#   profile) the Rust objects reference each other's mangled symbols, which
#   cannot be hidden, so a non-LTO archive is rejected.
# - Hosted targets: core and compiler_builtins name `rust_eh_personality` in
#   their unwind tables. The reference is renamed to
#   `yakogl_rust_eh_personality`, which capi/eh_personality.c defines (a C
#   forwarder to the GCC C personality), so it cannot collide with the one of
#   a Rust std staticlib linked next to this archive.
# - The C the crate compiles itself (build.rs: for `gpu-v3d` Mesa's
#   qpu_pack.c and qpu_instr.c and our qpu_mesa.c, for PowerVR the csbgen
#   wrappers driver/src/pvr/generated/csbgen_wrap.c; bundled by rustc as the
#   members `<hash>-qpu_pack.o`, `<hash>-csbgen_wrap.o` etc.) is neither Rust
#   runtime nor compiler_builtins, although the cc crate's `<hash>-<source>.o`
#   names look like compiler_builtins' C objects: _defined_globals knows its
#   members by name and leaves them as they are (the LTO object calls into
#   them, so their symbols must stay global).
#
# Symbols with a leading underscore in compiler_builtins (__udivti3, the
# __aarch64 outline atomics, ...) stay global: they are the compiler runtime,
# identical in every copy (weak, hidden), and some are needed across members.
#
# The result is checked: no global Rust or libc/libm definitions left, no
# unrenamed personality reference. The output is written only at the end
# (atomic rename), so a failed run leaves no half-made archive behind.

cmake_minimum_required(VERSION 3.20)

# FATAL_ERROR that first removes the scratch directory.
macro(_fail)
  file(REMOVE_RECURSE "${_work}")
  message(FATAL_ERROR "assemble.cmake: " ${ARGN})
endmacro()

foreach(v RUST_LIB OUTPUT AR NM OBJCOPY)
  if(NOT DEFINED ${v} OR "${${v}}" STREQUAL "")
    message(FATAL_ERROR "assemble.cmake: -D${v}=... is required")
  endif()
endforeach()
if(NOT DEFINED LTO_MEMBER OR "${LTO_MEMBER}" STREQUAL "")
  set(LTO_MEMBER yakogl_capi)
endif()
if(NOT LTO_MEMBER MATCHES "^[A-Za-z0-9_]+$")
  message(FATAL_ERROR "assemble.cmake: LTO_MEMBER '${LTO_MEMBER}' is not a crate name")
endif()

get_filename_component(_out_dir "${OUTPUT}" DIRECTORY)
get_filename_component(_out_name "${OUTPUT}" NAME)
set(_work "${_out_dir}/${_out_name}.assemble")
file(REMOVE_RECURSE "${_work}")
file(MAKE_DIRECTORY "${_work}")
set(_tmp "rust.a")
execute_process(COMMAND "${CMAKE_COMMAND}" -E copy "${RUST_LIB}" "${_work}/${_tmp}"
  COMMAND_ERROR_IS_FATAL ANY)

# nm -A prints "<archive>:<member>:<value> <type> <name>" (GNU) or with a
# space before the value (llvm-nm). The archive is named relative to _work so
# the path has no colon of its own.
function(_defined_globals archive out_lto out_rt)
  execute_process(COMMAND "${NM}" -A -g --defined-only "${archive}"
    WORKING_DIRECTORY "${_work}"
    OUTPUT_VARIABLE _nm RESULT_VARIABLE _rc ERROR_VARIABLE _err)
  if(NOT _rc EQUAL 0)
    _fail("${NM} failed on ${archive}: ${_err}")
  endif()
  string(REPLACE ";" "\\;" _nm "${_nm}")
  string(REPLACE "\n" ";" _lines "${_nm}")
  set(_lto "")
  set(_rt "")
  set(_lto_members "")
  set(_other_rust "")
  foreach(_l IN LISTS _lines)
    if(NOT _l MATCHES "^[^:]*:([^:]+): *[0-9a-fA-F]* +[A-Za-z] +(.+)$")
      continue()
    endif()
    set(_member "${CMAKE_MATCH_1}")
    set(_sym "${CMAKE_MATCH_2}")
    if(_member MATCHES "^([0-9a-f]+-)?(qpu_pack|qpu_instr|qpu_mesa|csbgen_wrap)\\.o$")
      # build.rs's C (see the header): kept, not localised.
      continue()
    elseif(_member MATCHES "^${LTO_MEMBER}-.*\\.o$")
      list(APPEND _lto_members "${_member}")
      list(APPEND _lto "${_sym}")
    elseif(_member MATCHES "^(compiler_builtins-|[0-9a-f]+-).*\\.o$")
      list(APPEND _rt "${_sym}")
    else()
      list(APPEND _other_rust "${_member}")
    endif()
  endforeach()
  list(REMOVE_DUPLICATES _lto_members)
  list(REMOVE_DUPLICATES _other_rust)
  list(LENGTH _lto_members _n)
  if(NOT _n EQUAL 1 OR _other_rust)
    list(SUBLIST _other_rust 0 3 _other_rust)
    list(JOIN _other_rust " " _other_rust)
    _fail(
      "${RUST_LIB} is not a single-LTO-object archive "
      "(${LTO_MEMBER} objects: ${_n}, other Rust objects: ${_other_rust}). "
      "Build it with a profile that has lto = true and codegen-units = 1 "
      "(release or dev-lto).")
  endif()
  set(${out_lto} "${_lto}" PARENT_SCOPE)
  set(${out_rt} "${_rt}" PARENT_SCOPE)
endfunction()

_defined_globals("${_tmp}" lto_syms rt_syms)

set(localize "")
foreach(s IN LISTS lto_syms)
  if(s MATCHES "^(_R|_ZN|rust_|__rust)")
    list(APPEND localize "${s}")
  endif()
endforeach()
foreach(s IN LISTS rt_syms)
  # C identifiers without a leading underscore: libc / libm names. Dotted
  # names (anon.*, DW.ref.*) are compiler-internal and stay as they are.
  if(s MATCHES "^[A-Za-z][A-Za-z0-9_]*$")
    if(s IN_LIST lto_syms)
      _fail("'${s}' is defined by both yakogl and compiler_builtins")
    endif()
    list(APPEND localize "${s}")
  endif()
endforeach()
list(REMOVE_DUPLICATES localize)
list(JOIN localize "\n" _loc_text)
file(WRITE "${_work}/localize.txt" "${_loc_text}\n")

execute_process(COMMAND "${OBJCOPY}"
    "--localize-symbols=localize.txt"
    "--redefine-sym" "rust_eh_personality=yakogl_rust_eh_personality"
    "${_tmp}"
  WORKING_DIRECTORY "${_work}"
  RESULT_VARIABLE _rc ERROR_VARIABLE _err)
if(NOT _rc EQUAL 0)
  _fail(
    "${OBJCOPY} failed: ${_err}\n"
    "It must handle the Rust target's object format: for cross targets use "
    "the cross binutils' objcopy or llvm-objcopy (CMAKE_OBJCOPY / OBJCOPY=).")
endif()

# Check the result.
_defined_globals("${_tmp}" lto_after rt_after)
set(bad "")
foreach(s IN LISTS lto_after rt_after)
  if(s IN_LIST localize)
    list(APPEND bad "${s}")
  endif()
endforeach()
execute_process(COMMAND "${NM}" -A -u "${_tmp}"
  WORKING_DIRECTORY "${_work}" OUTPUT_VARIABLE _undef)
if(_undef MATCHES "[ \t]rust_eh_personality\n")
  list(APPEND bad "rust_eh_personality (undefined reference)")
endif()
if(bad)
  # (GNU objcopy built without the target's BFD back end copies such
  # objects unchanged and still exits 0.)
  list(LENGTH bad _nbad)
  list(SUBLIST bad 0 8 bad)
  list(JOIN bad " " bad)
  _fail("${OBJCOPY} left ${_nbad} symbols global (${bad} ...).\n"
    "It must handle the Rust target's object format: for cross targets use "
    "the cross binutils' objcopy or llvm-objcopy (CMAKE_OBJCOPY / OBJCOPY=).")
endif()

string(REPLACE "|" ";" C_OBJECTS "${C_OBJECTS}")
if(C_OBJECTS)
  execute_process(COMMAND "${AR}" qs "${_tmp}" ${C_OBJECTS}
    WORKING_DIRECTORY "${_work}" RESULT_VARIABLE _rc ERROR_VARIABLE _err)
  if(NOT _rc EQUAL 0)
    _fail("${AR} failed: ${_err}")
  endif()
endif()

file(RENAME "${_work}/${_tmp}" "${OUTPUT}")
file(REMOVE_RECURSE "${_work}")
list(LENGTH localize _nloc)
message(STATUS "assembled ${OUTPUT} (${_nloc} runtime symbols made local)")
