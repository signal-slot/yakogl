# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# capi/yakogl_capi.cmake - cargo integration shared by ../CMakeLists.txt and
# ../zephyr/CMakeLists.txt.
#
# Cache variables (both builds):
#   YAKOGL_RUST_LINKAGE     staticlib: build libyakogl.a (Rust + C parts) here.
#                           external: build only the C parts; the application
#                           links the `yakogl` crate into its own Rust
#                           staticlib (README.md, "Linking").
#   YAKOGL_CARGO            the cargo executable (default: `cargo` on PATH, as
#                           the driver repository's samples use).
#   YAKOGL_RUSTUP_TOOLCHAIN RUSTUP_TOOLCHAIN for cargo (empty: inherited).
#   YAKOGL_CARGO_BUILD_STD  AUTO|ON|OFF: `-Z build-std=core` (nightly + rust-src)
#                           instead of the target's precompiled core. AUTO uses
#                           it only when the toolchain has no core for the target.
#   YAKOGL_RUST_TARGET      Rust target triple. Empty: the host for a native
#                           build, derived from the C toolchain for a cross build.
#   YAKOGL_RUST_TARGET_CPU  -C target-cpu (empty: the target's default).
#   YAKOGL_MAX_CONTEXTS     GL contexts that can exist at once (default 2).
#   YAKOGL_MAX_SURFACES     EGL surfaces that can exist at once (default 4).
#   YAKOGL_CARGO_PROFILE    release | dev (dev = cargo profile dev-lto: overflow
#                           checks and debug assertions, one LTO object).
#
# Cargo is run through a DEPFILE (cargo's own dep-info), so it runs only when
# a crate source, a manifest or the command line changed, and libyakogl.a is
# re-assembled only when cargo produced a new staticlib or a C object changed.
# Not tracked: RUSTFLAGS / cargo config / toolchain updates - touch Cargo.toml
# (or delete the build's cargo/ directory) after changing those.

include_guard(GLOBAL)

# (Also re-derived inside each function: this file can be included from two
# directories -- yakogl's zephyr/ and driver/zephyr/ -- and the include guard
# leaves the second without this variable.)
set(YAKOGL_CAPI_DIR "${CMAKE_CURRENT_LIST_DIR}")

set(YAKOGL_RUST_LINKAGE "staticlib" CACHE STRING
  "staticlib: build libyakogl.a; external: C parts only, the application links the yakogl crate")
set_property(CACHE YAKOGL_RUST_LINKAGE PROPERTY STRINGS staticlib external)
set(YAKOGL_CARGO "" CACHE FILEPATH "cargo executable (empty: cargo on PATH)")
set(YAKOGL_RUSTUP_TOOLCHAIN "" CACHE STRING "RUSTUP_TOOLCHAIN for cargo (empty: inherited)")
set(YAKOGL_CARGO_BUILD_STD "AUTO" CACHE STRING
  "-Z build-std=core: AUTO (when the toolchain lacks the target's core) | ON | OFF")
set_property(CACHE YAKOGL_CARGO_BUILD_STD PROPERTY STRINGS AUTO ON OFF)
set(YAKOGL_CARGO_PROFILE "release" CACHE STRING "Cargo profile: release|dev")
set_property(CACHE YAKOGL_CARGO_PROFILE PROPERTY STRINGS release dev)
# The context pool's size, GL_MAX_CONTEXTS for the C headers and
# YAKOGL_MAX_CONTEXTS for the crate: one setting, so the two cannot disagree.
# A few MB of static state per context (see src/gl/context.rs).
set(YAKOGL_MAX_CONTEXTS "2" CACHE STRING "GL contexts that can exist at once")
# The EGL surface table. A slot is small (the pixels are the pbuffer's own
# allocation), so this is a table size rather than a memory budget.
set(YAKOGL_MAX_SURFACES "4" CACHE STRING "EGL surfaces that can exist at once")

if(NOT YAKOGL_RUST_LINKAGE MATCHES "^(staticlib|external)$")
  message(FATAL_ERROR "YAKOGL_RUST_LINKAGE must be staticlib|external (got '${YAKOGL_RUST_LINKAGE}')")
endif()
if(NOT YAKOGL_CARGO_BUILD_STD MATCHES "^(AUTO|ON|OFF)$")
  message(FATAL_ERROR "YAKOGL_CARGO_BUILD_STD must be AUTO|ON|OFF (got '${YAKOGL_CARGO_BUILD_STD}')")
endif()
if(NOT YAKOGL_CARGO_PROFILE MATCHES "^(release|dev)$")
  message(FATAL_ERROR "YAKOGL_CARGO_PROFILE must be release|dev (got '${YAKOGL_CARGO_PROFILE}')")
endif()

# Normalised architecture name of a triple / CMAKE_SYSTEM_PROCESSOR.
function(_yakogl_arch in out)
  string(TOLOWER "${in}" _a)
  string(REGEX REPLACE "-.*" "" _a "${_a}")
  if(_a MATCHES "^(aarch64|arm64)$")
    set(_a aarch64)
  elseif(_a MATCHES "^(x86_64|amd64|x64)$")
    set(_a x86_64)
  elseif(_a MATCHES "^riscv64")
    set(_a riscv64)
  endif()
  set(${out} "${_a}" PARENT_SCOPE)
endfunction()

# yakogl_rust_target(<out>): the Rust triple for this C toolchain, or "" for a
# native build (cargo's host). A cross build never falls back to the host:
# without YAKOGL_RUST_TARGET the triple comes from CMAKE_C_COMPILER_TARGET /
# CMAKE_SYSTEM_PROCESSOR + CMAKE_SYSTEM_NAME, or configuration stops.
function(yakogl_rust_target out)
  if(CMAKE_C_COMPILER_TARGET)
    set(_c "${CMAKE_C_COMPILER_TARGET}")
  else()
    set(_c "${CMAKE_SYSTEM_PROCESSOR}")
  endif()
  _yakogl_arch("${_c}" _carch)

  if(YAKOGL_RUST_TARGET)
    _yakogl_arch("${YAKOGL_RUST_TARGET}" _rarch)
    if(CMAKE_CROSSCOMPILING AND _carch AND NOT _carch STREQUAL _rarch)
      message(FATAL_ERROR
        "YAKOGL_RUST_TARGET=${YAKOGL_RUST_TARGET} does not match the C toolchain "
        "(${_c}): the archive would mix ${_rarch} and ${_carch} objects.")
    endif()
    set(${out} "${YAKOGL_RUST_TARGET}" PARENT_SCOPE)
    return()
  endif()
  if(NOT CMAKE_CROSSCOMPILING)
    set(${out} "" PARENT_SCOPE)
    return()
  endif()

  string(TOLOWER "${_c}" _cl)
  if(CMAKE_SYSTEM_NAME MATCHES "^(Generic|Zephyr)$" OR _cl MATCHES "-none|-elf$")
    set(_os none)
  elseif(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    if(_cl MATCHES "musl")
      set(_os linux-musl)
    else()
      set(_os linux-gnu)
    endif()
  else()
    set(_os "")
  endif()

  set(_t "")
  if(_carch STREQUAL "aarch64" AND _os STREQUAL "none")
    set(_t aarch64-unknown-none)
  elseif(_carch STREQUAL "x86_64" AND _os STREQUAL "none")
    set(_t x86_64-unknown-none)
  elseif(_carch STREQUAL "riscv64" AND _os STREQUAL "none")
    set(_t riscv64gc-unknown-none-elf)
  elseif(_carch MATCHES "^(aarch64|x86_64)$" AND _os MATCHES "^linux")
    set(_t ${_carch}-unknown-${_os})
  elseif(_carch STREQUAL "riscv64" AND _os STREQUAL "linux-gnu")
    set(_t riscv64gc-unknown-linux-gnu)
  endif()
  if(NOT _t)
    message(FATAL_ERROR
      "YakoGL: cannot derive the Rust target for this cross toolchain "
      "(C target '${_c}', system '${CMAKE_SYSTEM_NAME}'). "
      "Set -DYAKOGL_RUST_TARGET=<triple> (64-bit targets only).")
  endif()
  message(STATUS "YakoGL: Rust target ${_t} (from the C toolchain '${_c}', ${CMAKE_SYSTEM_NAME})")
  set(${out} "${_t}" PARENT_SCOPE)
endfunction()

# True for targets with an OS (they need the EH personality shim).
function(yakogl_rust_target_is_hosted triple out)
  if(triple MATCHES "-none")
    set(${out} FALSE PARENT_SCOPE)
  else()
    set(${out} TRUE PARENT_SCOPE)
  endif()
endfunction()

# yakogl_add_capi_archive(OUTPUT <libyakogl.a> OSAL <name> GPU <name>
#                         [SHADERC offline|online]
#                         TARGET_NAME <tgt> [C_OBJECT_LIBRARY <object library>])
# Adds custom target <tgt> that builds OUTPUT: cargo builds yakogl-capi for
# YAKOGL_RUST_TARGET_RESOLVED, capi/assemble.cmake post-processes it and
# appends the objects of C_OBJECT_LIBRARY. The feature mapping is this
# function's; the cargo + assemble steps are yakogl_add_rust_archive().
function(yakogl_add_capi_archive)
  set(YAKOGL_CAPI_DIR "${CMAKE_CURRENT_FUNCTION_LIST_DIR}")
  cmake_parse_arguments(A "NO_SHADER_TABLE" "OUTPUT;OSAL;GPU;SHADERC;TARGET_NAME;C_OBJECT_LIBRARY" "" ${ARGN})
  set(_features "")
  if(A_OSAL)
    list(APPEND _features "osal-${A_OSAL}")
  endif()
  # The GPU's backend: the crate's `gpu-<name>`.
  if(NOT A_GPU MATCHES "^(powervr|vivante|v3d)$")
    message(FATAL_ERROR "yakogl_add_capi_archive: GPU must be powervr|vivante|v3d (got '${A_GPU}')")
  endif()
  list(APPEND _features "gpu-${A_GPU}")
  # `online` is the in-process compiler (no_std). `offline` (the default) is
  # the Mesa reference table behind the offline back end: the crate's
  # `mesa-reference-table`, which a measurement image takes on purpose (the
  # "table image" of the gl_test comparisons) and no other build links. It
  # will be replaced by the table our own compiler precompiles.
  if(A_SHADERC STREQUAL "online")
    list(APPEND _features "shaderc-online")
  else()
    list(APPEND _features "mesa-reference-table")
  endif()
  # An image carrying a compiler carries no table: the online choice already
  # leaves the Mesa reference out, and a board image says so as well (the
  # crate's `no-shader-table`).
  if(A_NO_SHADER_TABLE)
    if(NOT A_SHADERC STREQUAL "online")
      message(FATAL_ERROR
        "NO_SHADER_TABLE needs SHADERC online: nothing else can compile a shader without a table")
    endif()
    list(APPEND _features "no-shader-table")
  endif()
  # A conformance build reports OpenGL ES 2.0 rather than ES 3.0 (a global,
  # like YAKOGL_CARGO_PROFILE: every archive of one build tree agrees).
  if(YAKOGL_REPORT_ES2)
    list(APPEND _features "report-es2")
  endif()
  set(_c_objs "")
  if(A_C_OBJECT_LIBRARY)
    set(_c_objs C_OBJECT_LIBRARY "${A_C_OBJECT_LIBRARY}")
  endif()
  yakogl_add_rust_archive(
    CRATE yakogl-capi
    MANIFEST "${YAKOGL_CAPI_DIR}/Cargo.toml"
    LIB_NAME yakogl_capi
    FEATURES ${_features}
    TARGET "${YAKOGL_RUST_TARGET_RESOLVED}"
    TARGET_CPU "${YAKOGL_RUST_TARGET_CPU}"
    RUSTUP_TOOLCHAIN "${YAKOGL_RUSTUP_TOOLCHAIN}"
    OUTPUT "${A_OUTPUT}"
    TARGET_NAME "${A_TARGET_NAME}"
    ${_c_objs})
endfunction()

# yakogl_add_rust_archive(CRATE <package> MANIFEST <its Cargo.toml>
#                         LIB_NAME <the [lib] name: cargo writes lib<LIB_NAME>.a>
#                         [FEATURES <feature>...] [TARGET <triple>] [TARGET_CPU <cpu>]
#                         [RUSTUP_TOOLCHAIN <toolchain>] [CARGO_TARGET_DIR <dir>]
#                         OUTPUT <archive> TARGET_NAME <tgt>
#                         [C_OBJECT_LIBRARY <object library>])
# One no_std staticlib crate of this workspace as a post-processed archive:
# cargo builds <package> (--no-default-features, the given features, the
# workspace's release or dev-lto profile) for TARGET (empty: the host), and
# capi/assemble.cmake makes its Rust runtime and compiler_builtins' libc/libm
# names local -- which is what lets several such archives (libyakogl.a,
# libyakogl_drivers.a, an application's librustapp.a) sit in one image --
# and appends the objects of C_OBJECT_LIBRARY. Adds custom target <tgt>.
# RUSTUP_TOOLCHAIN empty: inherited. -Z build-std=core follows
# YAKOGL_CARGO_BUILD_STD for this archive's toolchain and target.
function(yakogl_add_rust_archive)
  set(YAKOGL_CAPI_DIR "${CMAKE_CURRENT_FUNCTION_LIST_DIR}")
  cmake_parse_arguments(A "" "CRATE;MANIFEST;LIB_NAME;TARGET;TARGET_CPU;RUSTUP_TOOLCHAIN;CARGO_TARGET_DIR;OUTPUT;TARGET_NAME;C_OBJECT_LIBRARY" "FEATURES" ${ARGN})
  foreach(_req CRATE MANIFEST LIB_NAME OUTPUT TARGET_NAME)
    if(NOT A_${_req})
      message(FATAL_ERROR "yakogl_add_rust_archive: ${_req} is required")
    endif()
  endforeach()
  set(_root "${YAKOGL_CAPI_DIR}/..")
  get_filename_component(_root "${_root}" ABSOLUTE)
  set(_triple "${A_TARGET}")

  if(YAKOGL_CARGO)
    set(_cargo "${YAKOGL_CARGO}")
  else()
    find_program(YAKOGL_CARGO_FOUND cargo)
    if(NOT YAKOGL_CARGO_FOUND)
      message(FATAL_ERROR "YakoGL: cargo not found; install Rust or set -DYAKOGL_CARGO=<path>")
    endif()
    set(_cargo "${YAKOGL_CARGO_FOUND}")
  endif()
  set(_env "YAKOGL_MAX_CONTEXTS=${YAKOGL_MAX_CONTEXTS}" "YAKOGL_MAX_SURFACES=${YAKOGL_MAX_SURFACES}")
  if(A_RUSTUP_TOOLCHAIN)
    list(APPEND _env "RUSTUP_TOOLCHAIN=${A_RUSTUP_TOOLCHAIN}")
  endif()
  # The C a crate compiles itself (yakogl's build.rs: Mesa's QPU packer for
  # gpu-v3d, the PowerVR backend's csbgen packer wrappers for a PowerVR
  # build) is compiled by this build's C compiler: the `cc`
  # crate reads CC_<target>/AR_<target> (CC/AR for a native build), so the
  # objects in the archive come from the same toolchain as the rest of the
  # image, and nothing compiles those files a second time.
  if(_triple)
    string(REPLACE "-" "_" _triple_us "${_triple}")
    list(APPEND _env "CC_${_triple_us}=${CMAKE_C_COMPILER}" "AR_${_triple_us}=${CMAKE_AR}")
  else()
    list(APPEND _env "CC=${CMAKE_C_COMPILER}" "AR=${CMAKE_AR}")
  endif()

  # rustc of the same toolchain (the rustup proxy next to cargo honours
  # RUSTUP_TOOLCHAIN and the override files of the source directory).
  get_filename_component(_cargo_dir "${_cargo}" DIRECTORY)
  if(EXISTS "${_cargo_dir}/rustc")
    set(_rustc "${_cargo_dir}/rustc")
  elseif(EXISTS "${_cargo_dir}/rustc.exe")
    set(_rustc "${_cargo_dir}/rustc.exe")
  else()
    message(FATAL_ERROR "YakoGL: no rustc next to ${_cargo}")
  endif()
  execute_process(COMMAND ${CMAKE_COMMAND} -E env ${_env} "${_rustc}" --print sysroot
    WORKING_DIRECTORY "${_root}" OUTPUT_VARIABLE _sysroot OUTPUT_STRIP_TRAILING_WHITESPACE
    RESULT_VARIABLE _rc)
  execute_process(COMMAND ${CMAKE_COMMAND} -E env ${_env} "${_rustc}" -V
    WORKING_DIRECTORY "${_root}" OUTPUT_VARIABLE _rustc_v OUTPUT_STRIP_TRAILING_WHITESPACE)
  if(NOT _rc EQUAL 0)
    message(FATAL_ERROR "YakoGL: '${_rustc} --print sysroot' failed (RUSTUP_TOOLCHAIN='${A_RUSTUP_TOOLCHAIN}')")
  endif()

  set(_build_std OFF)
  if(_triple)
    file(GLOB _core "${_sysroot}/lib/rustlib/${_triple}/lib/libcore-*.rlib")
    set(_nightly FALSE)
    if(_rustc_v MATCHES "-(nightly|dev)")
      set(_nightly TRUE)
    endif()
    if(YAKOGL_CARGO_BUILD_STD STREQUAL "ON")
      set(_build_std ON)
    elseif(YAKOGL_CARGO_BUILD_STD STREQUAL "AUTO" AND NOT _core)
      if(_nightly AND EXISTS "${_sysroot}/lib/rustlib/src/rust/library/core")
        set(_build_std ON)
      else()
        message(FATAL_ERROR
          "YakoGL: ${_rustc_v} has no precompiled core for ${_triple} (building ${A_CRATE}). Either "
          "`rustup target add ${_triple}` for this toolchain, or use a nightly "
          "toolchain with rust-src (`rustup component add rust-src`) so that "
          "-Z build-std=core is used.")
      endif()
    elseif(YAKOGL_CARGO_BUILD_STD STREQUAL "OFF" AND NOT _core)
      message(FATAL_ERROR "YakoGL: ${_rustc_v} has no precompiled core for ${_triple} and YAKOGL_CARGO_BUILD_STD=OFF")
    endif()
  endif()

  set(_triple_note "${_triple}")
  if(NOT _triple)
    set(_triple_note "host")
  endif()
  # The cargo target directory: <binary dir>/cargo unless the caller names
  # one (two archives built from one CMake directory need two).
  if(A_CARGO_TARGET_DIR)
    set(_target_dir "${A_CARGO_TARGET_DIR}")
  else()
    set(_target_dir "${CMAKE_CURRENT_BINARY_DIR}/cargo")
  endif()
  set(_args build -p "${A_CRATE}" --manifest-path "${_root}/Cargo.toml"
      --target-dir "${_target_dir}" --no-default-features)
  if(A_FEATURES)
    string(REPLACE ";" "," _features "${A_FEATURES}")
    list(APPEND _args --features "${_features}")
  endif()
  if(YAKOGL_CARGO_PROFILE STREQUAL "release")
    list(APPEND _args --release)
    set(_pdir release)
  else()
    list(APPEND _args --profile dev-lto)
    set(_pdir dev-lto)
  endif()
  if(_triple)
    list(APPEND _args --target "${_triple}")
    set(_pdir "${_triple}/${_pdir}")
    set(_rustflags_key "target.${_triple}.rustflags")
  else()
    set(_rustflags_key "build.rustflags")
  endif()
  if(_build_std)
    list(APPEND _args -Z build-std=core)
  endif()
  if(A_TARGET_CPU)
    list(APPEND _args --config "${_rustflags_key}=[\"-Ctarget-cpu=${A_TARGET_CPU}\"]")
  endif()
  set(_rust_lib "${_target_dir}/${_pdir}/lib${A_LIB_NAME}.a")

  add_custom_command(OUTPUT "${_rust_lib}"
    COMMAND ${CMAKE_COMMAND} -E env ${_env} "${_cargo}" ${_args}
    DEPFILE "${_target_dir}/${_pdir}/lib${A_LIB_NAME}.d"
    DEPENDS "${_root}/Cargo.toml" "${A_MANIFEST}"
    WORKING_DIRECTORY "${_root}"
    COMMENT "cargo build ${A_CRATE} (${_rustc_v}; target ${_triple_note})"
    USES_TERMINAL
    VERBATIM)

  foreach(_tool AR NM OBJCOPY)
    if(NOT CMAKE_${_tool})
      message(FATAL_ERROR "YakoGL: CMAKE_${_tool} is not set (needed to assemble ${A_OUTPUT})")
    endif()
  endforeach()
  set(_objs "")
  set(_obj_deps "")
  if(A_C_OBJECT_LIBRARY)
    # '|'-separated: a ';' list would be split into separate arguments.
    set(_objs "$<JOIN:$<TARGET_OBJECTS:${A_C_OBJECT_LIBRARY}>,|>")
    set(_obj_deps "$<TARGET_OBJECTS:${A_C_OBJECT_LIBRARY}>" ${A_C_OBJECT_LIBRARY})
  endif()
  add_custom_command(OUTPUT "${A_OUTPUT}"
    COMMAND ${CMAKE_COMMAND}
            "-DRUST_LIB=${_rust_lib}" "-DOUTPUT=${A_OUTPUT}" "-DLTO_MEMBER=${A_LIB_NAME}"
            "-DAR=${CMAKE_AR}" "-DNM=${CMAKE_NM}" "-DOBJCOPY=${CMAKE_OBJCOPY}"
            "-DC_OBJECTS=${_objs}"
            -P "${YAKOGL_CAPI_DIR}/assemble.cmake"
    DEPENDS "${_rust_lib}" "${YAKOGL_CAPI_DIR}/assemble.cmake" ${_obj_deps}
    VERBATIM
    COMMENT "assemble ${A_OUTPUT}")
  add_custom_target(${A_TARGET_NAME} ALL DEPENDS "${A_OUTPUT}")

  set(_bs "")
  if(_build_std)
    set(_bs ", -Z build-std=core")
  endif()
  message(STATUS "YakoGL: ${A_CRATE}: cargo ${_cargo} (${_rustc_v}), target '${_triple}'${_bs}, profile ${YAKOGL_CARGO_PROFILE}")
endfunction()
