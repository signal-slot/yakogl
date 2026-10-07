# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# yakogl/CMakeLists.txt
#
# Builds libyakogl.a for host / Linux / generic cross targets: the Rust
# archive (capi/: the yakogl crate + panic handler, post-processed by
# capi/assemble.cmake) plus the C parts that stay C: the selected OSAL
# (src/osal/<platform>.c), the ABI/FFI layout checks, the driver's OS-free
# command generators the PowerVR backend calls through FFI, and on hosted
# targets the EH personality forwarder (capi/eh_personality.c). Apps compile
# against <GLES2/gl2.h> and link target GLES2.
#
# The Zephyr build does NOT use this file - it uses zephyr/ (a Zephyr module).
# The bare-metal build has a plain Makefile (baremetal.mk).
#
# Options (see also capi/yakogl_capi.cmake for the cargo ones):
#   -DYAKOGL_OSAL=zephyr|baremetal|linux  (default: linux)
#   -DYAKOGL_GPU=powervr|vivante|v3d       (default: powervr; v3d: OSAL linux
#                                          only, the host model of the Pi 4)
#   -DYAKOGL_SHADERC=offline|online|precompiled (default: online; precompiled:
#                                          no compiler, needs YAKOGL_PRECOMPILED_SHADERS)
#   -DYAKOGL_PRECOMPILED_SHADERS=ON|OFF    the precompiled tier in front of the
#                                          compiler (default: OFF); its apps:
#   -DYAKOGL_PRECOMPILED_APPS="<app>..."   (default: the whole source list)
#   -DYAKOGL_PVR_DRIVER_DIR=<powervr-bxs-am62p/src>
#   -DYAKOGL_SHARED=ON|OFF                 (also build libEGL.so.1 +
#                                           libGLESv2.so.2; default: OFF)
#   -DYAKOGL_RUST_LINKAGE=staticlib|external
#        staticlib (default): GLES2 = libyakogl.a (Rust + C).
#        external: GLES2 = libyakogl_c.a, the C parts only; the application
#        links the yakogl crate into its own Rust staticlib and links that
#        too (README.md, "Linking").
#   -DYAKOGL_RUST_TARGET=<triple>          (default: host; cross builds derive it
#                                           from the C toolchain)
#   -DYAKOGL_RUST_TARGET_CPU=<cpu>         (default: the target's baseline)
#   -DYAKOGL_CARGO_PROFILE=release|dev     (default: release)
#   -DYAKOGL_CARGO=<cargo> -DYAKOGL_RUSTUP_TOOLCHAIN=<name>
#   -DYAKOGL_CARGO_BUILD_STD=AUTO|ON|OFF
#   -DYAKOGL_EH_PERSONALITY=ON|OFF         (external linkage, hosted targets:
#        libyakogl_c.a defines rust_eh_personality for a no_std Rust
#        staticlib; OFF when that staticlib links std, which has its own)

cmake_minimum_required(VERSION 3.20)
project(YakoGL C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

set(YAKOGL_OSAL "linux" CACHE STRING "OSAL backend: zephyr|baremetal|linux")
set_property(CACHE YAKOGL_OSAL PROPERTY STRINGS zephyr baremetal linux)
set(YAKOGL_GPU "powervr" CACHE STRING "GPU backend: powervr|vivante|v3d (v3d: OSAL linux only)")
set_property(CACHE YAKOGL_GPU PROPERTY STRINGS powervr vivante v3d)
# The shader back end. `online` (the default) compiles src/shaderc/online/
# in: the in-process compiler, which compiles the shaders an application
# writes as it runs. `offline` links the Mesa reference as the table
# (the crate's `mesa-reference-table`): measurement builds only, to be
# replaced by the table our own compiler precompiles.
set(YAKOGL_SHADERC "online" CACHE STRING "Shader back end: online|offline (offline: the Mesa reference, measurement only)")
set_property(CACHE YAKOGL_SHADERC PROPERTY STRINGS offline online precompiled)
if(NOT YAKOGL_SHADERC MATCHES "^(offline|online|precompiled)$")
  message(FATAL_ERROR "YAKOGL_SHADERC must be offline|online|precompiled (got '${YAKOGL_SHADERC}')")
endif()
# The precompiled tier (the crate's `precompiled-shaders`): the source list
# compiled at build time by yakogl-precompile, looked up before compiling.
option(YAKOGL_PRECOMPILED_SHADERS "Precompile the source list in front of the compiler" OFF)
set(YAKOGL_PRECOMPILED_APPS "" CACHE STRING "The source list's apps to precompile (empty: all)")
set(_yakogl_precompiled_args "")
if(YAKOGL_PRECOMPILED_SHADERS)
  separate_arguments(_yakogl_apps UNIX_COMMAND "${YAKOGL_PRECOMPILED_APPS}")
  set(_yakogl_precompiled_args PRECOMPILED PRECOMPILED_APPS ${_yakogl_apps})
elseif(YAKOGL_SHADERC STREQUAL "precompiled")
  message(FATAL_ERROR "YAKOGL_SHADERC=precompiled needs YAKOGL_PRECOMPILED_SHADERS=ON")
endif()
if(NOT YAKOGL_GPU MATCHES "^(powervr|vivante|v3d)$")
  message(FATAL_ERROR "YAKOGL_GPU must be powervr|vivante|v3d (got '${YAKOGL_GPU}')")
endif()
# The Pi 4's V3D outside Zephyr: only the Linux OSAL's host model of it
# (src/osal/linux.c, "the V3D"); the bare-metal OSAL does not drive one.
if(YAKOGL_GPU STREQUAL "v3d" AND NOT YAKOGL_OSAL STREQUAL "linux")
  message(FATAL_ERROR "YAKOGL_GPU=v3d needs YAKOGL_OSAL=linux here (the board build is zephyr/)")
endif()
set(YAKOGL_RUST_TARGET "" CACHE STRING "Rust target triple (empty: host, or derived from the cross C toolchain)")
set(YAKOGL_RUST_TARGET_CPU "" CACHE STRING "Rust -C target-cpu (empty: the target's default)")
# A conformance build reports OpenGL ES 2.0 rather than ES 3.0 (the crate's
# `report-es2` feature; see Cargo.toml for what changes). ON by default: YakoGL
# reports ES 2.0 until the ES 3.0 work has progressed far enough to make 3.0 the
# default. Qt's RHI (Qt Quick 3D), dEQP-GLES3 and anything else that needs an
# ES 3.0 context configures with -DYAKOGL_REPORT_ES2=OFF.
option(YAKOGL_REPORT_ES2 "report OpenGL ES 2.0 (default) rather than ES 3.0 from glGetString/glGetIntegerv" ON)
# The test knobs (the crate's `test-knobs` feature): the C entry points an
# on-silicon suite sets to run an experiment's arms. OFF in a product build.
option(YAKOGL_TEST_KNOBS "compile in the test knobs (gc_debug_set_msaa and the like); never in a product" OFF)
option(YAKOGL_EH_PERSONALITY "external linkage, hosted: define rust_eh_personality in libyakogl_c.a" ON)
include("${CMAKE_CURRENT_SOURCE_DIR}/capi/yakogl_capi.cmake")

set(YAKOGL_DIR "${CMAKE_CURRENT_SOURCE_DIR}")
set(YAKOGL_SRC "${YAKOGL_DIR}/src")

# The shared structs are pinned at their LP64 layout (the crate refuses other
# targets too; this says so before cargo runs).
if(NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
  message(FATAL_ERROR "YakoGL supports 64-bit (LP64) targets only (this C compiler has ${CMAKE_SIZEOF_VOID_P}-byte pointers)")
endif()

# ---- the driver's command generators (C, shared with the driver's own render
# path). The driver lives in driver/ of this repository, so the default is
# driver/src; another driver tree may be named with YAKOGL_PVR_DRIVER_DIR.
set(YAKOGL_PVR_DRIVER_DIR "${YAKOGL_DIR}/driver/src" CACHE PATH "driver/src (the PowerVR driver sources)")
set(PVR_DRIVER_SRC "${YAKOGL_PVR_DRIVER_DIR}")
if(NOT EXISTS "${PVR_DRIVER_SRC}/pvr_render.h")
  message(FATAL_ERROR "YAKOGL_PVR_DRIVER_DIR=${PVR_DRIVER_SRC} has no pvr_render.h: point it at driver/src")
endif()

# ---- OSAL selection ---------------------------------------------------------
if(YAKOGL_OSAL STREQUAL "zephyr")
  set(YAKOGL_OSAL_DEF "YAKOGL_OSAL_ZEPHYR")
elseif(YAKOGL_OSAL STREQUAL "baremetal")
  set(YAKOGL_OSAL_DEF "YAKOGL_OSAL_BAREMETAL")
elseif(YAKOGL_OSAL STREQUAL "linux")
  set(YAKOGL_OSAL_DEF "YAKOGL_OSAL_LINUX")
else()
  message(FATAL_ERROR "YAKOGL_OSAL must be zephyr|baremetal|linux (got '${YAKOGL_OSAL}')")
endif()

# ---- shared libraries: on by default exactly where they are the point ------
if(NOT DEFINED YAKOGL_SHARED)
  set(YAKOGL_SHARED OFF)
endif()
if(YAKOGL_SHARED AND NOT YAKOGL_RUST_LINKAGE STREQUAL "staticlib")
  message(FATAL_ERROR
    "YAKOGL_SHARED needs YAKOGL_RUST_LINKAGE=staticlib: the shared objects are "
    "linked from the assembled libyakogl.a, which the external linkage does not build.")
endif()

yakogl_rust_target(YAKOGL_RUST_TARGET_RESOLVED)
yakogl_rust_target_is_hosted("${YAKOGL_RUST_TARGET_RESOLVED}" _hosted)

# ---- the C parts --------------------------------------------------------------
# The two *_layout_check.c files have no code: they pin the structs the C
# headers share with the crate (abi_layout_check.c) and the driver structs and
# constants src/pvr/ffi.rs mirrors (ffi_layout_check.c), so drift on either
# side fails this build.
set(_c_sources
    "${YAKOGL_SRC}/osal/${YAKOGL_OSAL}.c"
    "${YAKOGL_SRC}/abi_layout_check.c")
# The PowerVR backend calls the driver's OS-free command generators through
# FFI, and pins their structs; the GC7000 has no such driver and writes its
# own command stream, so a vivante build compiles neither. (The OSALs still
# include the driver's render-service headers, so the path is required
# either way.)
if(YAKOGL_GPU STREQUAL "powervr")
  list(APPEND _c_sources
      "${YAKOGL_SRC}/pvr/ffi_layout_check.c"
      "${PVR_DRIVER_SRC}/pvr_vdm_ppp_gen.c"
      "${PVR_DRIVER_SRC}/pvr_pds_render_gen.c"
      "${PVR_DRIVER_SRC}/pvr_pds_pixel_event_gen.c")
  # The generators are Mesa's PDS generators and csbgen packers
  # (third_party/mesa-26.2.1) called by the driver's sequencing.
  include("${YAKOGL_DIR}/driver/compat/pvr_mesa.cmake")
  list(APPEND _c_sources ${PVR_MESA_SOURCES})
  set(YAKOGL_GPU_DEF "YAKOGL_GPU_POWERVR")
elseif(YAKOGL_GPU STREQUAL "v3d")
  # The V3D: the job builder the backend calls through FFI and what it
  # shares (zephyr/CMakeLists.txt lists the same, with the hardware steps
  # v3d_hw.c, which the Linux OSAL does not call), and the FFI's pins.
  list(APPEND _c_sources
      "${PVR_DRIVER_SRC}/v3d/v3d_job.c"
      "${PVR_DRIVER_SRC}/v3d/v3d_tex.c"
      "${PVR_DRIVER_SRC}/v3d/v3d_tiling_mesa.c"
      "${PVR_DRIVER_SRC}/v3d/v3d_frame.c"
      "${YAKOGL_SRC}/v3d/ffi_layout_check.c")
  set(YAKOGL_GPU_DEF "YAKOGL_GPU_V3D")
else()
  set(YAKOGL_GPU_DEF "YAKOGL_GPU_VIVANTE")
endif()
# The C forwarder of rust_eh_personality: a no_std crate's unwind tables name
# it although YakoGL never unwinds, and capi/assemble.cmake renames the Rust
# side's definition to yakogl_rust_eh_personality, which the forwarder defines.
if(_hosted
   AND (YAKOGL_RUST_LINKAGE STREQUAL "staticlib" OR YAKOGL_EH_PERSONALITY))
  list(APPEND _c_sources "${YAKOGL_DIR}/capi/eh_personality.c")
endif()

if(YAKOGL_RUST_LINKAGE STREQUAL "staticlib")
  add_library(yakogl_c OBJECT ${_c_sources})
else()
  # libyakogl_c.a: what a Rust application that links the yakogl crate
  # itself needs from this build.
  add_library(yakogl_c STATIC ${_c_sources})
  if(_hosted)
    set_source_files_properties("${YAKOGL_DIR}/capi/eh_personality.c" PROPERTIES
      COMPILE_DEFINITIONS "YAKOGL_EH_PERSONALITY=rust_eh_personality")
  endif()
endif()
target_include_directories(yakogl_c PRIVATE
    "${YAKOGL_DIR}/include"
    "${YAKOGL_SRC}"
    "${YAKOGL_SRC}/osal"
    "${YAKOGL_SRC}/vivante"   # gc_kick.h: the C view of the Vivante submit descriptor
    "${PVR_DRIVER_SRC}")
if(YAKOGL_GPU STREQUAL "powervr")
  target_include_directories(yakogl_c PRIVATE ${PVR_MESA_INCLUDE_DIRS})
elseif(YAKOGL_GPU STREQUAL "v3d")
  # v3d_kick.h, the job builder's headers, and the Mesa packers they use
  # (vendored unmodified, as zephyr/CMakeLists.txt includes them)
  target_include_directories(yakogl_c PRIVATE
      "${YAKOGL_SRC}/v3d"
      "${PVR_DRIVER_SRC}/v3d"
      "${PVR_DRIVER_SRC}/v3d/generated"
      "${YAKOGL_DIR}/third_party/mesa-26.2.1/src/broadcom"
      "${YAKOGL_DIR}/third_party/mesa-26.2.1/src"
      "${YAKOGL_DIR}/third_party/mesa-26.2.1/include")
endif()
# GL_MAX_CONTEXTS and the crate's YAKOGL_MAX_CONTEXTS are the same setting
# (capi/yakogl_capi.cmake passes it to cargo), so the C headers describe the
# pool the Rust actually has.
target_compile_definitions(yakogl_c PRIVATE ${YAKOGL_OSAL_DEF} ${YAKOGL_GPU_DEF}
    "GL_MAX_CONTEXTS=${YAKOGL_MAX_CONTEXTS}")
# The shared objects are linked from the same objects as the archive, so those
# objects must be position-independent. (The Rust side already is: the Linux
# targets default to relocation-model=pic.)
if(YAKOGL_SHARED)
  set_target_properties(yakogl_c PROPERTIES POSITION_INDEPENDENT_CODE ON)
endif()

# What applications in the same CMake tree link: target GLES2 (as before).
# No libm: neither the Rust archive nor the C parts call it (the archive's
# own math is in the LTO object; compiler_builtins' libm copies are local).
if(YAKOGL_RUST_LINKAGE STREQUAL "staticlib")
  set(YAKOGL_ARCHIVE "${CMAKE_CURRENT_BINARY_DIR}/libyakogl.a")
  yakogl_add_capi_archive(
    OUTPUT "${YAKOGL_ARCHIVE}"
    OSAL "${YAKOGL_OSAL}"
    GPU "${YAKOGL_GPU}"
    SHADERC "${YAKOGL_SHADERC}"
    ${_yakogl_precompiled_args}
    TARGET_NAME yakogl_archive
    C_OBJECT_LIBRARY yakogl_c)
  add_library(GLES2 STATIC IMPORTED GLOBAL)
  set_target_properties(GLES2 PROPERTIES IMPORTED_LOCATION "${YAKOGL_ARCHIVE}")
  target_include_directories(GLES2 INTERFACE "${YAKOGL_DIR}/include")
  add_dependencies(GLES2 yakogl_archive)
else()
  add_library(GLES2 INTERFACE)
  target_include_directories(GLES2 INTERFACE "${YAKOGL_DIR}/include")
  target_link_libraries(GLES2 INTERFACE yakogl_c)
endif()

# ---- libGLESv2.so.2 + libEGL.so.1 -------------------------------------------
#
# An application (and the Khronos conformance suite) links -lEGL -lGLESv2 and
# expects the usual pair of sonames. There is only one copy of the code: the
# GL core, the EGL implementation, the GPU backend and the OSAL share one
# process-wide state, so they must live in ONE object. That object is
# libGLESv2.so.2, which exports both the gl* and the egl* entry points;
# libEGL.so.1 is a forwarder that carries the soname applications look for and
# depends on libGLESv2.so.2 for the symbols.
#
# Both are linked from the assembled libyakogl.a (--whole-archive), so the
# archive and the shared objects can never drift apart. capi/assemble.cmake
# has already made the Rust runtime and compiler_builtins' libc/libm copies
# local, and tools/yakogl.ver keeps everything but the Khronos entry points
# in; what is left to resolve from the platform is libm, hence -lm.
if(YAKOGL_SHARED)
  set(YAKOGL_VERSION_SCRIPT "${YAKOGL_DIR}/tools/yakogl.ver")

  # A shared library needs a translation unit of its own; all the code comes
  # in through --whole-archive below.
  set(YAKOGL_SHARED_TU "${CMAKE_CURRENT_BINARY_DIR}/yakogl_shared_tu.c")
  file(WRITE "${YAKOGL_SHARED_TU}"
"/* SPDX-License-Identifier: Apache-2.0 */\n"
"/* Generated by yakogl/CMakeLists.txt: the anchor translation unit of\n"
" * libGLESv2.so.2. The library's code is linked in from libyakogl.a with\n"
" * --whole-archive; CMake just needs one source file to make a target of. */\n"
"__attribute__((visibility(\"hidden\")))\n"
"const char yakogl_shared_build[] = \"yakogl\";\n")

  add_library(GLESv2_shared SHARED "${YAKOGL_SHARED_TU}")
  set_target_properties(GLESv2_shared PROPERTIES
      OUTPUT_NAME GLESv2 VERSION 2.0.0 SOVERSION 2
      LINK_DEPENDS "${YAKOGL_VERSION_SCRIPT};${YAKOGL_ARCHIVE}")
  add_dependencies(GLESv2_shared yakogl_archive)
  target_include_directories(GLESv2_shared PUBLIC "${YAKOGL_DIR}/include")
  target_link_options(GLESv2_shared PRIVATE
      "-Wl,--whole-archive" "${YAKOGL_ARCHIVE}" "-Wl,--no-whole-archive"
      "-Wl,--version-script=${YAKOGL_VERSION_SCRIPT}")
  target_link_libraries(GLESv2_shared PRIVATE m)

  # The forwarder needs one translation unit (a shared library cannot be built
  # from nothing) and one reference into libGLESv2, so that --as-needed keeps
  # the DT_NEEDED entry that makes dlopen("libEGL.so.1") resolve egl* calls.
  set(YAKOGL_EGL_FORWARD "${CMAKE_CURRENT_BINARY_DIR}/egl_forward.c")
  file(WRITE "${YAKOGL_EGL_FORWARD}"
"/* SPDX-License-Identifier: Apache-2.0 */\n"
"/* Generated by yakogl/CMakeLists.txt. libEGL.so.1 is the soname applications\n"
" * link against; the EGL entry points themselves live in libGLESv2.so.2,\n"
" * because EGL and GL share one process-wide state. The anchor below is a\n"
" * relocation into that library so the dependency survives --as-needed. */\n"
"extern unsigned int eglGetError(void);\n"
"__attribute__((visibility(\"hidden\")))\n"
"unsigned int (*const yakogl_egl_anchor)(void) = eglGetError;\n")

  add_library(EGL_shared SHARED "${YAKOGL_EGL_FORWARD}")
  set_target_properties(EGL_shared PROPERTIES
      OUTPUT_NAME EGL VERSION 1.0.0 SOVERSION 1)
  target_link_options(EGL_shared PRIVATE "-Wl,--no-as-needed")
  target_link_libraries(EGL_shared PRIVATE GLESv2_shared)
endif()

message(STATUS "YakoGL: OSAL=${YAKOGL_OSAL} GPU=${YAKOGL_GPU} shaderc=${YAKOGL_SHADERC} linkage=${YAKOGL_RUST_LINKAGE} shared=${YAKOGL_SHARED} rust-target='${YAKOGL_RUST_TARGET_RESOLVED}' profile=${YAKOGL_CARGO_PROFILE}")

# ---- host tests (tests/): the dEQP-GLES2 negative_api group run against this
# build. Off by default; they need a VK-GL-CTS checkout to extract the case
# bodies from (tests/README.md).
option(YAKOGL_TESTS "build the host tests in tests/" OFF)
if(YAKOGL_TESTS)
  add_subdirectory(tests)
endif()
