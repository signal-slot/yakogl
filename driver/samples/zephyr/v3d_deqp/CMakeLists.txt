# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# v3d_deqp: dEQP-GLES2 (or, with -DDEQP_MODULE=gles3, dEQP-GLES3) on the
# Raspberry Pi 4 (V3D 4.2), on Zephyr, through YakoGL's V3D backend
# (docs/online-shaderc/v3d-backend.md, "M8").
#
# The dEQP side is the deqp sample's, used by path and never edited: its
# main loop, test-data archive, QPA-to-console log and stubs. The board side
# is v3d_clear's (mailbox framebuffer, HDMI console, bounded-lifetime
# watchdog). What is new is src/v3d_board.c: the V3D's EGL default display,
# and the result record, one screen the lead photographs.
#
#   -DDEQP_SRC=<CTS copy with cts-yakogl-zephyr.patch applied>
#   -DDEQP_BUILD=<its stage-1 build (static libraries)>
#   -DDEQP_MODULE=gles2|gles3 (default gles2; the stage 1's MODULE)
# tools/cts/cts-build-zephyr.sh makes the first two (BOARD=rpi_4b, MODULE=);
# tools/cts/pi4-deqp-images.sh builds a table of images (DEQP_MODULE=).
cmake_minimum_required(VERSION 3.20.0)
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
include("${CMAKE_CURRENT_SOURCE_DIR}/../common/yakogl_dir.cmake")
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(v3d_deqp)

foreach(v DEQP_SRC DEQP_BUILD)
  if(NOT ${v})
    message(FATAL_ERROR "v3d_deqp: set -D${v}=...")
  endif()
endforeach()

# Which dEQP package: gles2 (default) or gles3, against a stage 1 built with
# the same MODULE (the deqp sample's rule). A gles3 image must report ES 3.0:
# prj.conf says CONFIG_YAKOGL_REPORT_ES2=y for dEQP-GLES2, so a gles3 build
# passes -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n on the command
# line (merged after prj.conf; pi4-deqp-images.sh DEQP_MODULE=gles3 does), and
# a gles3 image whose configuration still says ES 2.0 is refused here, since
# dEQP-GLES3 on an ES 2.0 context is not supported.
if(NOT DEQP_MODULE)
  set(DEQP_MODULE gles2)
endif()
if(NOT DEQP_MODULE MATCHES "^gles[23]$")
  message(FATAL_ERROR "v3d_deqp: DEQP_MODULE must be gles2 or gles3, not ${DEQP_MODULE}")
endif()
if(DEQP_MODULE STREQUAL "gles3" AND (CONFIG_DEQP_REPORT_ES2 OR CONFIG_YAKOGL_REPORT_ES2))
  message(FATAL_ERROR "v3d_deqp: DEQP_MODULE=gles3 needs CONFIG_DEQP_REPORT_ES2=n and CONFIG_YAKOGL_REPORT_ES2=n")
endif()
string(SUBSTRING "${DEQP_MODULE}" 4 1 _deqp_ver)

execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
  set(V3D_BUILD "unknown")
endif()

set(DEQP_SAMPLE "${CMAKE_CURRENT_SOURCE_DIR}/../deqp")
set(V3D_BOARD "${CMAKE_CURRENT_SOURCE_DIR}/../../../zephyr/lib/board/rpi4")

# the test data, embedded (the deqp sample's tool)
set(_data_c "${CMAKE_CURRENT_BINARY_DIR}/deqp_data.c")
add_custom_command(OUTPUT "${_data_c}"
  COMMAND python3 "${DEQP_SAMPLE}/tools/embed_data.py"
          "${DEQP_BUILD}/modules/${DEQP_MODULE}/${DEQP_MODULE}" ${DEQP_MODULE} "${_data_c}"
  DEPENDS "${DEQP_SAMPLE}/tools/embed_data.py"
  COMMENT "embedding the dEQP ${DEQP_MODULE} test data")

target_sources(app PRIVATE
  "${DEQP_SAMPLE}/src/main.cpp"
  "${DEQP_SAMPLE}/src/deqp_archive.cpp"
  "${DEQP_SAMPLE}/src/deqp_log.c"
  "${DEQP_SAMPLE}/src/deqp_stubs.c"
  "${_data_c}"
  # the package registration lives in dEQP's executable, not in a library
  ${DEQP_SRC}/modules/${DEQP_MODULE}/tes${_deqp_ver}TestPackageEntry.cpp
  src/v3d_board.c
  "${V3D_BOARD}/rpi_mbox.c"
  "${V3D_BOARD}/fbcon.c")
# The bounded-lifetime watchdog, on by default: every board image carries it.
# Off only for a QEMU smoke run of the record (QEMU's raspi4b resets as soon
# as the watchdog is armed; see v3d_gl_triangle).
option(V3D_RPI_WDOG "the PM watchdog that bounds the image's lifetime" ON)
if(V3D_RPI_WDOG)
  target_sources(app PRIVATE "${V3D_BOARD}/rpi_wdog.c")
else()
  message(WARNING "v3d_deqp: built WITHOUT the watchdog (V3D_RPI_WDOG=OFF): QEMU only, never the board")
  target_compile_definitions(app PRIVATE V3D_NO_RPI_WDOG)
endif()
# The whole QPA to the UART (V3D_DEQP_QPA_UART): --wrap=fdevopen puts a tee
# in front of the deqp sample's log stream, the one fdevopen call in the
# image (deqp_log.c's fopen wrapper); Zephyr's own streams are static.
# --wrap=qpTestLog_createFileLog turns the images and shader sources back on
# (the deqp sample's command line turns them off).
if(CONFIG_V3D_DEQP_QPA_UART)
  target_sources(app PRIVATE src/qpa_uart.c)
  zephyr_link_libraries(-Wl,--wrap=fdevopen -Wl,--wrap=qpTestLog_createFileLog)
endif()
target_compile_definitions(app PRIVATE
  V3D_BUILD="${V3D_BUILD}"
  # rpi_wdog.c: the image's whole lifetime, run and photo together
  RPI_WDOG_HOLD_S=${CONFIG_V3D_DEQP_LIFETIME_S}
  DE_OS=DE_OS_UNIX DE_CPU=DE_CPU_ARM_64 DE_COMPILER=DE_COMPILER_GCC DE_PTR_SIZE=8)
target_include_directories(app PRIVATE
  "${DEQP_SAMPLE}/src"
  "${CMAKE_CURRENT_SOURCE_DIR}/../../../zephyr/lib/board/common"  # sample_glue.h (the declaration only)
  "${V3D_BOARD}"
  "${YAKOGL_DIR}/src/osal"                 # struct osal_zephyr_v3d_init
  "${PVR_REPO}/src/v3d"                    # struct v3d_clock_ops (v3d.h)
  "${PVR_REPO}/zephyr/lib/board/common"        # gc_font8x16.h only
  ${DEQP_SRC}/framework/common ${DEQP_SRC}/framework/qphelper
  ${DEQP_SRC}/framework/delibs/debase ${DEQP_SRC}/framework/delibs/decpp
  ${DEQP_SRC}/framework/delibs/depool ${DEQP_SRC}/framework/delibs/dethread
  ${DEQP_SRC}/framework/delibs/deutil ${DEQP_SRC}/framework/delibs/destream
  ${DEQP_SRC}/framework/referencerenderer ${DEQP_SRC}/framework/opengl
  ${DEQP_SRC}/framework/opengl/wrapper ${DEQP_SRC}/framework/egl
  ${DEQP_SRC}/framework/egl/wrapper ${DEQP_SRC}/modules/${DEQP_MODULE}
  ${DEQP_SRC}/modules/glshared ${DEQP_SRC}/framework/opengl/simplereference)

# The stage-1 archives, in dependency order inside one group (the deqp
# sample's list).
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
# --wrap=fopen: deqp_log.c takes the QPA log. --wrap=eglGetDisplay and
# eglGetPlatformDisplay: v3d_board.c gives dEQP's EGL_DEFAULT_DISPLAY the V3D
# OSAL's init argument (the board's V3D clock switch). --wrap=glGetShaderInfoLog
# and glGetProgramInfoLog, with glGetShaderiv and glGetProgramiv for their
# status: v3d_board.c keeps the first line of a failed build's log for the
# record.
zephyr_link_libraries(-Wl,--start-group ${_abs} -Wl,--end-group -Wl,--wrap=fopen
  -Wl,--wrap=eglGetDisplay -Wl,--wrap=eglGetPlatformDisplay
  -Wl,--wrap=glGetShaderInfoLog -Wl,--wrap=glGetProgramInfoLog
  -Wl,--wrap=glGetShaderiv -Wl,--wrap=glGetProgramiv)
