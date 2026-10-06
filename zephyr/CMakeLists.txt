# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# yakogl/zephyr/CMakeLists.txt
#
# Registers yakogl as a Zephyr module that provides YakoGL, gated by
# CONFIG_YAKOGL: the C parts (the Zephyr OSAL and the ABI/FFI layout checks)
# as a Zephyr library, and - with YAKOGL_RUST_LINKAGE=staticlib, the default -
# the Rust archive built by cargo from ../capi (the yakogl crate + its panic
# handler, post-processed by ../capi/assemble.cmake). Point Zephyr at this
# directory as a module (yakogl/zephyr/module.yml, or add yakogl to
# ZEPHYR_EXTRA_MODULES).
#
# The Zephyr OSAL wraps the silicon-proven pvr driver (src/pvr_fw_boot.c,
# src/pvr_render.c) and the PowerVR backend calls the driver's OS-free command
# generators: that driver must be in the same Zephyr build
# (CONFIG_SAFEUI_GPU_PVR) so those symbols resolve; only its headers are
# needed here.
#
# Configuration: CMake cache variables, e.g. `west build ... --
# -DYAKOGL_RUSTUP_TOOLCHAIN=nightly`, or set() before find_package(Zephyr):
#   YAKOGL_RUST_LINKAGE     staticlib (default) | external: the application
#                           links the yakogl crate (features
#                           ["osal-zephyr", "gpu-<name>"]) into its own Rust staticlib,
#                           with its own panic handler; only the C parts are
#                           built here.
#   YAKOGL_CARGO            cargo executable (default: cargo on PATH, like the
#                           driver repository's samples)
#   YAKOGL_RUSTUP_TOOLCHAIN RUSTUP_TOOLCHAIN for cargo (default: inherited)
#   YAKOGL_CARGO_BUILD_STD  AUTO (default: -Z build-std=core when the toolchain
#                           has no precompiled core for the target, e.g. the
#                           nightly the samples use) | ON | OFF
#   YAKOGL_RUST_TARGET      default aarch64-unknown-none on CONFIG_ARM64
#   YAKOGL_RUST_TARGET_CPU  default cortex-a53 on CONFIG_ARM64 (the AM62P
#                           cores); empty for the target's baseline
#   YAKOGL_CARGO_PROFILE    release (default) | dev
#   YAKOGL_PVR_DRIVER_DIR   the GPU driver sources (default: driver/src here,
#                           else ../src for the old submodule layout)

if(CONFIG_YAKOGL)
  get_filename_component(YAKOGL_DIR "${CMAKE_CURRENT_SOURCE_DIR}/.." ABSOLUTE)
  set(YAKOGL_SRC "${YAKOGL_DIR}/src")

  # The driver repository lives in this one, under driver/ (it used to be the
  # other way round: yakogl/ a submodule of the driver repository, whose src/
  # was the parent directory's). YAKOGL_PVR_DRIVER_DIR overrides either for
  # other layouts. An application that is not one of the driver's samples
  # (Qt, say) sets none of this, so the in-tree layout has to be the default.
  if(DEFINED YAKOGL_PVR_DRIVER_DIR)
    set(PVR_DRIVER_SRC "${YAKOGL_PVR_DRIVER_DIR}")
  elseif(EXISTS "${YAKOGL_DIR}/driver/src/pvr_render.h")
    set(PVR_DRIVER_SRC "${YAKOGL_DIR}/driver/src")
  else()
    set(PVR_DRIVER_SRC "${YAKOGL_DIR}/../src")
  endif()

  if(CONFIG_ARM64)
    set(_yakogl_default_target aarch64-unknown-none)
    set(_yakogl_default_cpu cortex-a53)
    # aarch64-unknown-none is a hard-float target: the Rust code uses the FP/SIMD
    # registers anywhere, not only in float math.
    if(NOT CONFIG_FPU)
      message(FATAL_ERROR "CONFIG_YAKOGL needs CONFIG_FPU=y (the Rust archive is built for hard float)")
    endif()
    if(NOT CONFIG_FPU_SHARING)
      message(WARNING "CONFIG_YAKOGL without CONFIG_FPU_SHARING: only one thread may use the FPU (GL calls use it)")
    endif()
  else()
    set(_yakogl_default_target "")
    set(_yakogl_default_cpu "")
  endif()
  set(YAKOGL_RUST_TARGET "${_yakogl_default_target}" CACHE STRING "Rust target triple")
  set(YAKOGL_RUST_TARGET_CPU "${_yakogl_default_cpu}" CACHE STRING "Rust -C target-cpu (empty: the target's default)")
  include("${YAKOGL_DIR}/capi/yakogl_capi.cmake")
  if(YAKOGL_RUST_TARGET)
    set(YAKOGL_RUST_TARGET_RESOLVED "${YAKOGL_RUST_TARGET}")
  else()
    message(FATAL_ERROR "YakoGL: set -DYAKOGL_RUST_TARGET=<triple> for this architecture (64-bit only)")
  endif()

  # ---- the GPU this build targets (the Kconfig choice) --------------------------
  # The backend and its offline shader table are in the Rust archive, so what
  # the choice picks here is the cargo feature, the C define, and which C parts
  # come with the backend: the PowerVR one calls the driver's OS-free command
  # generators through FFI and pins their structs, and the GC7000 has neither
  # (it writes its own command stream), but it does bring an OSAL of its own.
  if(CONFIG_YAKOGL_GPU_VIVANTE)
    set(YAKOGL_GPU "vivante")
    set(YAKOGL_GPU_DEF "YAKOGL_GPU_VIVANTE")
    set(_yakogl_osal_c "${YAKOGL_SRC}/osal/zephyr_gc.c")
    set(_yakogl_gpu_c "")
  elseif(CONFIG_YAKOGL_GPU_V3D)
    # The V3D: the job builder the backend calls through FFI (and the frame
    # builder whose tiling it shares), the driver's hardware steps the OSAL
    # calls, and the pins of the FFI's structs against their headers.
    set(YAKOGL_GPU "v3d")
    set(YAKOGL_GPU_DEF "YAKOGL_GPU_V3D")
    set(_yakogl_osal_c "${YAKOGL_SRC}/osal/zephyr_v3d.c")
    set(_yakogl_gpu_c
      "${PVR_DRIVER_SRC}/v3d/v3d_job.c"
      "${PVR_DRIVER_SRC}/v3d/v3d_tex.c"
      "${PVR_DRIVER_SRC}/v3d/v3d_tiling_mesa.c"   # Mesa's v3d_tiling.c, as it is
      "${PVR_DRIVER_SRC}/v3d/v3d_frame.c"
      "${PVR_DRIVER_SRC}/v3d/v3d_hw.c"
      "${YAKOGL_SRC}/v3d/ffi_layout_check.c")
  else()
    set(YAKOGL_GPU "powervr")
    set(YAKOGL_GPU_DEF "YAKOGL_GPU_POWERVR")
    set(_yakogl_osal_c "${YAKOGL_SRC}/osal/zephyr.c")
    set(_yakogl_gpu_c "${YAKOGL_SRC}/pvr/ffi_layout_check.c")
  endif()

  # ---- the shader back end (the Kconfig choice) -----------------------------------
  # The in-process compiler also drops the offline table from the image: the
  # two are separate cargo features because a host build wants both, but an
  # image that compiles its own shaders has no use for the bytes it replaced.
  if(CONFIG_YAKOGL_SHADERC_ONLINE)
    set(YAKOGL_SHADERC "online")
    set(YAKOGL_NO_SHADER_TABLE ON)
  elseif(CONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY)
    # the precompiled shaders alone: no compiler, no Mesa table
    set(YAKOGL_SHADERC "precompiled")
    set(YAKOGL_NO_SHADER_TABLE ON)
  else()
    set(YAKOGL_SHADERC "offline")
    set(YAKOGL_NO_SHADER_TABLE OFF)
  endif()

  # The version this build reports (the crate's `report-es2` feature, which
  # yakogl_add_capi_archive() reads from this variable).
  if(CONFIG_YAKOGL_REPORT_ES2)
    set(YAKOGL_REPORT_ES2 ON)
  else()
    set(YAKOGL_REPORT_ES2 OFF)
  endif()
  # The test knobs (the crate's `test-knobs` feature), which only a test
  # image selects; yakogl_add_capi_archive() reads this variable too.
  if(CONFIG_YAKOGL_TEST_KNOBS)
    set(YAKOGL_TEST_KNOBS ON)
  else()
    set(YAKOGL_TEST_KNOBS OFF)
  endif()

  # ---- the C part: the Zephyr OSAL + the ABI/FFI layout pins ---------------------
  # The in-process compiler is lent its working region by a static this file
  # owns; the offline and the precompiled-only builds have no compiler to
  # lend to. Same on both OSALs,
  # so it lives beside them rather than inside either.
  if(CONFIG_YAKOGL_SHADERC_ONLINE)
    set(_yakogl_shaderc_c "${YAKOGL_SRC}/osal/zephyr_shaderc_scratch.c")
  else()
    set(_yakogl_shaderc_c "")
  endif()

  zephyr_library_named(yakogl)
  zephyr_library_sources(
    "${_yakogl_osal_c}"
    "${YAKOGL_SRC}/osal/zephyr_timing.c"   # the clock and draw stats every Zephyr OSAL answers
    "${YAKOGL_SRC}/abi_layout_check.c"
    ${_yakogl_shaderc_c}
    ${_yakogl_gpu_c})
  zephyr_library_compile_definitions(YAKOGL_OSAL_ZEPHYR ${YAKOGL_GPU_DEF})
  # tools/check.sh builds the samples with -DYAKOGL_WERROR=ON: the OSAL
  # and the layout pins compile without a warning (off by default).
  if(YAKOGL_WERROR)
    zephyr_library_compile_options(-Werror)
  endif()

  # Public Khronos headers for apps; private OSAL/driver includes.
  zephyr_include_directories("${YAKOGL_DIR}/include")
  zephyr_library_include_directories(
    "${YAKOGL_SRC}"
    "${YAKOGL_SRC}/osal"
    "${YAKOGL_SRC}/vivante"   # gc_kick.h: the C view of the Vivante submit descriptor
    "${YAKOGL_SRC}/v3d"       # v3d_kick.h: the C view of the V3D submit descriptor
    "${PVR_DRIVER_SRC}"
    "${PVR_DRIVER_SRC}/v3d")
  if(CONFIG_YAKOGL_GPU_V3D)
    # The V3D's packers and texel tiling are Mesa's (driver/src/v3d/v3d_cl.h,
    # v3d_tiling_mesa.c): headers vendored unmodified in third_party/, and
    # what Mesa's generators make from them in driver/src/v3d/generated/.
    # For every target, not only this library: the samples include the
    # texture headers (v3d_tex.h) too.
    zephyr_include_directories(
      "${PVR_DRIVER_SRC}/v3d/generated"
      "${YAKOGL_DIR}/third_party/mesa-26.2.1/src/broadcom"
      "${YAKOGL_DIR}/third_party/mesa-26.2.1/src"
      "${YAKOGL_DIR}/third_party/mesa-26.2.1/include")
  endif()

  # ---- the Rust archive -----------------------------------------------------------
  if(YAKOGL_NO_SHADER_TABLE)
    set(_yakogl_no_table_arg NO_SHADER_TABLE)
  else()
    set(_yakogl_no_table_arg "")
  endif()
  # The precompiled tier (Kconfig YAKOGL_PRECOMPILED_SHADERS): the table
  # yakogl-precompile writes into this build directory, for these apps.
  set(_yakogl_precompiled_args "")
  if(CONFIG_YAKOGL_PRECOMPILED_SHADERS)
    separate_arguments(_yakogl_apps UNIX_COMMAND "${CONFIG_YAKOGL_PRECOMPILED_APPS}")
    set(_yakogl_precompiled_args PRECOMPILED PRECOMPILED_APPS ${_yakogl_apps})
    if(NOT CONFIG_YAKOGL_PRECOMPILED_LIST STREQUAL "")
      list(APPEND _yakogl_precompiled_args PRECOMPILED_LIST "${CONFIG_YAKOGL_PRECOMPILED_LIST}")
    endif()
  endif()
  if(YAKOGL_RUST_LINKAGE STREQUAL "staticlib")
    set(_yakogl_rust_archive "${CMAKE_CURRENT_BINARY_DIR}/libyakogl_rust.a")
    yakogl_add_capi_archive(
      OUTPUT "${_yakogl_rust_archive}"
      OSAL zephyr
      GPU "${YAKOGL_GPU}"
      SHADERC "${YAKOGL_SHADERC}"
      ${_yakogl_no_table_arg}
      ${_yakogl_precompiled_args}
      TARGET_NAME yakogl_rust)
    add_library(yakogl_rust_lib STATIC IMPORTED GLOBAL)
    set_target_properties(yakogl_rust_lib PROPERTIES IMPORTED_LOCATION "${_yakogl_rust_archive}")
    add_dependencies(yakogl_rust_lib yakogl_rust)
    add_dependencies(yakogl yakogl_rust)
    zephyr_link_libraries(yakogl_rust_lib)
  endif()
  message(STATUS "YakoGL: GPU=${YAKOGL_GPU} shaderc=${YAKOGL_SHADERC} no-table=${YAKOGL_NO_SHADER_TABLE} test-knobs=${YAKOGL_TEST_KNOBS} linkage=${YAKOGL_RUST_LINKAGE} rust-target=${YAKOGL_RUST_TARGET_RESOLVED} cpu='${YAKOGL_RUST_TARGET_CPU}'")
endif()
