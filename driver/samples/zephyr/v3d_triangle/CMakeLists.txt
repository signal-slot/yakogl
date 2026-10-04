# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# v3d_triangle: V3D 4.2 milestone M5 on the Raspberry Pi 4 (see src/main.c).
# Standalone like v3d_clear, whose mailbox and console files it reuses.
cmake_minimum_required(VERSION 3.20.0)

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(v3d_triangle)

get_filename_component(DRIVER "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
get_filename_component(REPO "${DRIVER}/.." ABSOLUTE)

execute_process(COMMAND git -C "${DRIVER}" describe --always --dirty
                OUTPUT_VARIABLE V3D_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR V3D_BUILD STREQUAL "")
  set(V3D_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE V3D_BUILD="${V3D_BUILD}")

# The program: tools/shaders/passthrough.vert + interp_color.frag, compiled by
# tools/v3d/v3d_shc into shaders/passthrough_interp_color.json:
#   glslangValidator -G passthrough.vert -o vs.spv
#   glslangValidator -G interp_color.frag -o fs.spv
#   v3d_shc vs.spv fs.spv > passthrough_interp_color.json
# The header is generated from that JSON at build time. Every prog_data field
# the frame builder reads must be in the JSON; the generator refuses an absent
# one rather than defaulting it, and takes no --assume here.
# Which compiler's code the image carries: the Mesa-compiled JSON above by
# default, or YakoGL's own (src/shaderc/online/v3d), written by
# `V3D_WRITE_OURS_JSON=1 cargo test ... the_triangle_samples_json` into
# shaders/ours_passthrough_interp_color.json:
#   cmake ... -DTRI_JSON=<sample>/shaders/ours_passthrough_interp_color.json
# The image prints the JSON's gpu.compiler and sha256 before anything else.
# The uniform variant (-DTRI_UNIFORMS=ON): tools/shaders/es100/utri.vert +
# utri.frag, whose program reads uniform storage (QUNIFORM_UNIFORM); main.c
# fills it and expects the uniforms' effect. Its JSONs: shaders/utri.json
# (Mesa, the default; made as tools/v3d/glsl2v3d.py makes SPIR-V, then
# v3d_shc vs.spv fs.spv --vpm-size 65536) and shaders/ours_utri.json
# (V3D_WRITE_OURS_JSON=1 cargo test ... samples_json). The image refuses to
# build when the JSON and the option disagree about uniform storage.
option(TRI_UNIFORMS "Build the uniform variant (es100/utri)" OFF)
# The if variant (-DTRI_IF=ON): tools/shaders/es100/tri_if.vert + tri_if.frag,
# an FS if whose threshold crosses the triangle. Its JSONs: shaders/tri_if.json
# (Mesa, the default, made as utri.json is) and shaders/ours_tri_if.json.
option(TRI_IF "Build the if variant (es100/tri_if)" OFF)
# The int variant (-DTRI_INT=ON): tools/shaders/es100/tri_int.vert +
# tri_int.frag, the int and bool operations in a picture. Its JSONs:
# shaders/tri_int.json (Mesa, the default, made as utri.json is) and
# shaders/ours_tri_int.json.
option(TRI_INT "Build the int variant (es100/tri_int)" OFF)
# The discard variant (-DTRI_DISCARD=ON): tools/shaders/es100/tri_discard.*,
# a nested-condition discard whose discarded part keeps the clear colour.
# Its JSONs: shaders/tri_discard.json (Mesa, the default) and
# shaders/ours_tri_discard.json.
option(TRI_DISCARD "Build the discard variant (es100/tri_discard)" OFF)
# The gl_FragCoord variant (-DTRI_FRAGCOORD=ON): tools/shaders/es100/
# tri_fragcoord.*, red = window x / 1280 and green = window y / 512, so the
# window flip (the FB_WPOS_Y_SCALE / _OFFSET words the frame writes from
# f.wpos) is visible. Our compiler's JSON only (shaders/ours_tri_fragcoord.json):
# Mesa's program takes the flip from a gallium state uniform this frame
# builder does not fill.
option(TRI_FRAGCOORD "Build the gl_FragCoord variant (es100/tri_fragcoord)" OFF)
set(TRI_N_VARIANTS 0)
foreach(v TRI_IF TRI_UNIFORMS TRI_INT TRI_DISCARD TRI_FRAGCOORD)
  if(${v})
    math(EXPR TRI_N_VARIANTS "${TRI_N_VARIANTS} + 1")
  endif()
endforeach()
if(TRI_N_VARIANTS GREATER 1)
  message(FATAL_ERROR "TRI_IF, TRI_UNIFORMS, TRI_INT, TRI_DISCARD and TRI_FRAGCOORD are separate variants")
endif()
if(TRI_FRAGCOORD)
  set(TRI_JSON_DEFAULT "${CMAKE_CURRENT_SOURCE_DIR}/shaders/ours_tri_fragcoord.json")
  target_compile_definitions(app PRIVATE TRI_FRAGCOORD=1)
elseif(TRI_DISCARD)
  set(TRI_JSON_DEFAULT "${CMAKE_CURRENT_SOURCE_DIR}/shaders/tri_discard.json")
  target_compile_definitions(app PRIVATE TRI_DISCARD=1)
elseif(TRI_INT)
  set(TRI_JSON_DEFAULT "${CMAKE_CURRENT_SOURCE_DIR}/shaders/tri_int.json")
  target_compile_definitions(app PRIVATE TRI_INT=1)
elseif(TRI_IF)
  set(TRI_JSON_DEFAULT "${CMAKE_CURRENT_SOURCE_DIR}/shaders/tri_if.json")
  target_compile_definitions(app PRIVATE TRI_IF=1)
elseif(TRI_UNIFORMS)
  set(TRI_JSON_DEFAULT "${CMAKE_CURRENT_SOURCE_DIR}/shaders/utri.json")
  target_compile_definitions(app PRIVATE TRI_UNIFORMS=1)
else()
  set(TRI_JSON_DEFAULT "${CMAKE_CURRENT_SOURCE_DIR}/shaders/passthrough_interp_color.json")
endif()
set(TRI_JSON "${TRI_JSON_DEFAULT}"
    CACHE FILEPATH "The v3d_shc-schema JSON whose QPU code the image embeds")
if(NOT EXISTS "${TRI_JSON}")
  message(FATAL_ERROR "TRI_JSON ${TRI_JSON} does not exist")
endif()
message(STATUS "v3d_triangle: shaders from ${TRI_JSON}, uniform variant ${TRI_UNIFORMS}, if variant ${TRI_IF}, int variant ${TRI_INT}, discard variant ${TRI_DISCARD}, gl_FragCoord variant ${TRI_FRAGCOORD}")
set(TRI_GEN "${CMAKE_CURRENT_BINARY_DIR}/gen")
if(DEFINED Python3_EXECUTABLE)
  set(TRI_PY "${Python3_EXECUTABLE}")
else()
  set(TRI_PY "${PYTHON_EXECUTABLE}")
endif()
add_custom_command(
  OUTPUT "${TRI_GEN}/tri_prog.h"
  COMMAND ${CMAKE_COMMAND} -E make_directory "${TRI_GEN}"
  COMMAND "${TRI_PY}" "${REPO}/tools/v3d/v3d_prog_header.py" "${TRI_JSON}"
          "${TRI_GEN}/tri_prog.h" --name tri
  DEPENDS "${TRI_JSON}" "${REPO}/tools/v3d/v3d_prog_header.py"
  COMMENT "v3d_prog_header: ${TRI_JSON}")
add_custom_target(tri_prog_h DEPENDS "${TRI_GEN}/tri_prog.h")
add_dependencies(app tri_prog_h)

target_include_directories(app PRIVATE
  "${TRI_GEN}"
  "${CMAKE_CURRENT_SOURCE_DIR}/../../../zephyr/lib/board/rpi4"
  "${DRIVER}/src/v3d"
  "${DRIVER}/../third_party/mesa-26.2.1/src/broadcom"   # Mesa's packers (v3d_cl.h)
  "${DRIVER}/../third_party/mesa-26.2.1/src"
  "${DRIVER}/../third_party/mesa-26.2.1/include"
  "${DRIVER}/zephyr/lib/board/common")   # gc_font8x16.h only
target_sources(app PRIVATE
  src/main.c
  ../../../zephyr/lib/board/rpi4/rpi_mbox.c
  ../../../zephyr/lib/board/rpi4/rpi_wdog.c
  ../../../zephyr/lib/board/rpi4/fbcon.c
  "${DRIVER}/src/v3d/v3d_hw.c"
  "${DRIVER}/src/v3d/v3d_frame.c")
