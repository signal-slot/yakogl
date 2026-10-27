# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# dEQP-GLES2 on Zephyr: the conformance suite as a Zephyr image, EGL/GLES from
# YakoGL, shaders from the in-process compiler. See README.md. One sample for
# every board: the Zephyr board (-b) picks the GPU and its board side, the
# board's files are in boards/. The Raspberry Pi 4 (V3D) adds src/v3d_board.c
# (the V3D's EGL default display and the result record, one screen the lead
# photographs), the mailbox framebuffer, the HDMI console and the
# bounded-lifetime watchdog of the rpi4 board layer.
#   -DDEQP_SRC=<CTS copy with cts-yakogl-zephyr.patch applied>
#   -DDEQP_BUILD=<its stage-1 build (static libraries)>
# tools/cts/cts-build-zephyr.sh makes both.
cmake_minimum_required(VERSION 3.20.0)
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(deqp_zephyr)
set(PVR_SAMPLES "${CMAKE_CURRENT_SOURCE_DIR}/..")

foreach(v DEQP_SRC DEQP_BUILD)
  if(NOT ${v})
    message(FATAL_ERROR "deqp: set -D${v}=...")
  endif()
endforeach()

if(NOT CONFIG_YAKOGL_GPU_V3D)
  include("${PVR_SAMPLES}/common/sample.cmake")
endif()

# Which dEQP package: gles2 (default) or gles3, against a stage 1 built with
# the same MODULE (tools/cts/cts-build-zephyr.sh). A gles3 image must let the
# library report ES 3.0: CONFIG_DEQP_REPORT_ES2=n and CONFIG_YAKOGL_REPORT_ES2=n
# (the latter is default y, so dropping the select alone still reports ES 2.0).
# dEQP-GLES3 on an ES 2.0 context is not supported.
if(NOT DEQP_MODULE)
  set(DEQP_MODULE gles2)
endif()
if(NOT DEQP_MODULE MATCHES "^gles[23]$")
  message(FATAL_ERROR "deqp: DEQP_MODULE must be gles2 or gles3, not ${DEQP_MODULE}")
endif()
if(DEQP_MODULE STREQUAL "gles3" AND CONFIG_DEQP_REPORT_ES2)
  message(FATAL_ERROR "deqp: DEQP_MODULE=gles3 needs CONFIG_DEQP_REPORT_ES2=n")
endif()
if(DEQP_MODULE STREQUAL "gles3" AND CONFIG_YAKOGL_REPORT_ES2)
  # DEQP_REPORT_ES2=n drops the select only; the library's default is y
  message(FATAL_ERROR "deqp: DEQP_MODULE=gles3 needs CONFIG_YAKOGL_REPORT_ES2=n too (the context must report ES 3.0)")
endif()
string(SUBSTRING "${DEQP_MODULE}" 4 1 _deqp_ver)

# the test data, embedded
set(_data_c "${CMAKE_CURRENT_BINARY_DIR}/deqp_data.c")
add_custom_command(OUTPUT "${_data_c}"
  COMMAND python3 "${CMAKE_CURRENT_SOURCE_DIR}/tools/embed_data.py"
          "${DEQP_BUILD}/modules/${DEQP_MODULE}/${DEQP_MODULE}" ${DEQP_MODULE} "${_data_c}"
  DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/tools/embed_data.py"
  COMMENT "embedding the dEQP ${DEQP_MODULE} test data")

target_sources(app PRIVATE src/main.cpp src/deqp_archive.cpp src/deqp_log.c src/deqp_stubs.c "${_data_c}"
  # the package registration lives in the executable's own entry file, not in
  # any library: without it the image has the tests but no package to run
  ${DEQP_SRC}/modules/${DEQP_MODULE}/tes${_deqp_ver}TestPackageEntry.cpp)
# The SNVS tally, on a board with yakogl_gc_board (the Verdin iMX8M Plus).
target_sources_ifdef(CONFIG_GC_COMMON_RESULT_SNVS app PRIVATE src/deqp_snvs.c)
target_include_directories(app PRIVATE src
  ${DEQP_SRC}/framework/common ${DEQP_SRC}/framework/qphelper
  ${DEQP_SRC}/framework/delibs/debase ${DEQP_SRC}/framework/delibs/decpp
  ${DEQP_SRC}/framework/delibs/depool ${DEQP_SRC}/framework/delibs/dethread
  ${DEQP_SRC}/framework/delibs/deutil ${DEQP_SRC}/framework/delibs/destream
  ${DEQP_SRC}/framework/referencerenderer ${DEQP_SRC}/framework/opengl
  ${DEQP_SRC}/framework/opengl/wrapper ${DEQP_SRC}/framework/egl
  ${DEQP_SRC}/framework/egl/wrapper ${DEQP_SRC}/modules/${DEQP_MODULE}
  ${DEQP_SRC}/modules/glshared ${DEQP_SRC}/framework/opengl/simplereference)
target_compile_definitions(app PRIVATE DE_OS=DE_OS_UNIX DE_CPU=DE_CPU_ARM_64
  DE_COMPILER=DE_COMPILER_GCC DE_PTR_SIZE=8)

# Raspberry Pi 4 (V3D 4.2): docs/online-shaderc/v3d-backend.md, "M8".
if(CONFIG_YAKOGL_GPU_V3D)
  execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                  OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                  RESULT_VARIABLE describe_rc)
  if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
    set(V3D_BUILD "unknown")
  endif()
  set(V3D_BOARD "${PVR_REPO}/zephyr/lib/board/rpi4")
  target_sources(app PRIVATE src/v3d_board.c "${V3D_BOARD}/rpi_mbox.c" "${V3D_BOARD}/fbcon.c")
  # The bounded-lifetime watchdog, on by default: every board image carries it.
  # Off only for a QEMU smoke run of the record (QEMU's raspi4b resets as soon
  # as the watchdog is armed).
  option(V3D_RPI_WDOG "the PM watchdog that bounds the image's lifetime" ON)
  if(V3D_RPI_WDOG)
    target_sources(app PRIVATE "${V3D_BOARD}/rpi_wdog.c")
  else()
    message(WARNING "deqp: built WITHOUT the watchdog (V3D_RPI_WDOG=OFF): QEMU only, never the board")
    target_compile_definitions(app PRIVATE V3D_NO_RPI_WDOG)
  endif()
  # The whole QPA to the UART (V3D_DEQP_QPA_UART): --wrap=fdevopen puts a tee
  # in front of the log stream, the one fdevopen call in the image
  # (deqp_log.c's fopen wrapper); Zephyr's own streams are static.
  # --wrap=qpTestLog_createFileLog turns the images and shader sources back on
  # (the command line turns them off).
  if(CONFIG_V3D_DEQP_QPA_UART)
    target_sources(app PRIVATE src/qpa_uart.c)
    zephyr_link_libraries(-Wl,--wrap=fdevopen -Wl,--wrap=qpTestLog_createFileLog)
  endif()
  target_compile_definitions(app PRIVATE
    V3D_BUILD="${V3D_BUILD}"
    # rpi_wdog.c: the image's whole lifetime, run and photo together
    RPI_WDOG_HOLD_S=${CONFIG_V3D_DEQP_LIFETIME_S})
  target_include_directories(app PRIVATE
    "${PVR_REPO}/zephyr/lib/board/common"    # sample_glue.h (the declaration only), gc_font8x16.h
    "${V3D_BOARD}"
    "${YAKOGL_DIR}/src/osal"                 # struct osal_zephyr_v3d_init
    "${PVR_REPO}/src/v3d")                   # struct v3d_clock_ops (v3d.h)
endif()

# PowerVR: the backend's option numbers (pvr/pvr_backend.h) for the A/B switches
if(CONFIG_YAKOGL_GPU_POWERVR)
  target_include_directories(app PRIVATE "${YAKOGL_DIR}/src")
endif()

# The stage-1 archives, in dependency order inside one group.
set(_libs
  modules/${DEQP_MODULE}/libdeqp-${DEQP_MODULE}-package.a
  modules/${DEQP_MODULE}/functional/libdeqp-${DEQP_MODULE}-functional.a
  modules/${DEQP_MODULE}/accuracy/libdeqp-${DEQP_MODULE}-accuracy.a
  modules/${DEQP_MODULE}/performance/libdeqp-${DEQP_MODULE}-performance.a
  modules/${DEQP_MODULE}/stress/libdeqp-${DEQP_MODULE}-stress.a
  modules/glshared/libdeqp-gl-shared.a
  framework/opengl/simplereference/libglutil-sglr.a
  framework/opengl/libglutil.a
  framework/opengl/wrapper/libglwrapper.a
  framework/egl/libeglutil.a
  framework/egl/wrapper/libeglwrapper.a
  framework/platform/libtcutil-platform.a
  framework/common/libtcutil.a
  framework/referencerenderer/libreferencerenderer.a
  framework/randomshaders/librandomshaders.a
  framework/qphelper/libqphelper.a
  framework/xexml/libxexml.a
  framework/delibs/decpp/libdecpp.a
  framework/delibs/deutil/libdeutil.a
  framework/delibs/dethread/libdethread.a
  framework/delibs/depool/libdepool.a
  framework/delibs/debase/libdebase.a
  external/jsoncpp/src/src/lib_json/libjsoncpp.a
  external/libpng/libpng.a
  external/zlib/libz.a)
set(_abs)
foreach(l ${_libs})
  list(APPEND _abs "${DEQP_BUILD}/${l}")
endforeach()
zephyr_link_libraries(-Wl,--start-group ${_abs} -Wl,--end-group -Wl,--wrap=fopen)
if(CONFIG_YAKOGL_GPU_V3D)
  # --wrap=eglGetDisplay and eglGetPlatformDisplay: v3d_board.c gives dEQP's
  # EGL_DEFAULT_DISPLAY the V3D OSAL's init argument (the board's V3D clock
  # switch). --wrap=glGetShaderInfoLog and glGetProgramInfoLog, with
  # glGetShaderiv and glGetProgramiv for their status: v3d_board.c keeps the
  # first line of a failed build's log for the record.
  zephyr_link_libraries(-Wl,--wrap=eglGetDisplay -Wl,--wrap=eglGetPlatformDisplay
    -Wl,--wrap=glGetShaderInfoLog -Wl,--wrap=glGetProgramInfoLog
    -Wl,--wrap=glGetShaderiv -Wl,--wrap=glGetProgramiv)
endif()
