# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The rpi5_triangle image's sources, shared by rpi5_triangle and
# rpi5_triangle_ours: included by either CMakeLists.txt after project().
#
# RPI5_TRI_SHADERS selects the three programs (src/shaders.c):
#   mesa  tools/v3d/tri-shaders71/mesa_passthrough_interp_color.h, Mesa's
#         VS/CS/FS out of the board's tools/v3d/pi5/triangle.clif (default)
#   ours  tools/v3d/tri-shaders71/ours_passthrough_interp_color.h, our 7.1
#         online compiler's pass-through VS + interp_color FS
if(NOT DEFINED RPI5_TRI_SHADERS)
  set(RPI5_TRI_SHADERS mesa)
endif()
if(RPI5_TRI_SHADERS STREQUAL "mesa")
  set(rpi5_tri_ours 0)
elseif(RPI5_TRI_SHADERS STREQUAL "ours")
  set(rpi5_tri_ours 1)
else()
  message(FATAL_ERROR "RPI5_TRI_SHADERS is '${RPI5_TRI_SHADERS}': mesa or ours")
endif()

set(rpi5_tri_dir "${CMAKE_CURRENT_LIST_DIR}")
get_filename_component(rpi5_tri_driver "${rpi5_tri_dir}/../../../../driver" ABSOLUTE)
get_filename_component(rpi5_tri_root "${rpi5_tri_driver}/.." ABSOLUTE)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${rpi5_tri_driver}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_TRI_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_TRI_BUILD STREQUAL "")
  set(RPI5_TRI_BUILD "unknown")
endif()
message(STATUS "${PROJECT_NAME} build: ${RPI5_TRI_BUILD}, shaders ${RPI5_TRI_SHADERS}")

# The V3D sequences (v3d_hw.c, the BCM2712 path) and the 7.1 frame
# (v3d_frame.c's clear steps and v3d_frame71_tri.c's triangle) are the
# driver's, the whole image built at V3D_CL_VER 71 so that main.c sees the
# 7.1 struct v3d_triangle_layout. v3d_frame.h brings Mesa's packers in: the
# include directories are rpi5_clear's.
target_compile_definitions(app PRIVATE
  V3D_CL_VER=71
  RPI5_TRI_BUILD="${RPI5_TRI_BUILD}"
  RPI5_TRI_OURS=${rpi5_tri_ours})
target_sources(app PRIVATE
  ${rpi5_tri_dir}/src/main.c
  ${rpi5_tri_dir}/src/shaders.c
  ${rpi5_tri_driver}/src/v3d/v3d_hw.c
  ${rpi5_tri_driver}/src/v3d/v3d_frame.c
  ${rpi5_tri_driver}/src/v3d/v3d_frame71_tri.c)
target_include_directories(app PRIVATE
  ${rpi5_tri_dir}/src
  ${rpi5_tri_root}/tools/v3d/tri-shaders71
  ${rpi5_tri_root}/tools/probes/rpi_5/common/tri_check
  ${rpi5_tri_driver}/src/v3d
  ${rpi5_tri_driver}/src/v3d/generated
  ${rpi5_tri_driver}/zephyr/lib/board/rpi4
  ${rpi5_tri_root}/third_party/mesa-26.2.1/src/broadcom
  ${rpi5_tri_root}/third_party/mesa-26.2.1/src
  ${rpi5_tri_root}/third_party/mesa-26.2.1/include)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
