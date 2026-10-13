# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# dEQP-GLES2 on Zephyr: the conformance suite as a Zephyr image, EGL/GLES from
# YakoGL, shaders from the in-process compiler. See README.md.
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

include("${PVR_SAMPLES}/common/sample.cmake")

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
