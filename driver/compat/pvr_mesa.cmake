# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The include directories of driver/compat/pvr_mesa.h (Mesa 26.2.1's PowerVR
# PDS generators and csbgen packers, vendored in third_party/mesa-26.2.1, and
# their generated headers in driver/src/pvr/generated), and the one C file that
# compiles the generators, for the CMake builds that compile the PowerVR
# command generators: the host library (CMakeLists.txt) and the Zephyr driver
# (driver/zephyr/CMakeLists.txt). baremetal.mk, tools/hostharness/run.sh,
# tools/check.sh and build.rs list the same directories.
get_filename_component(_pvr_mesa_root "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)
set(_pvr_mesa "${_pvr_mesa_root}/third_party/mesa-26.2.1")
set(PVR_MESA_INCLUDE_DIRS
    "${_pvr_mesa_root}/driver/compat"
    "${_pvr_mesa_root}/driver/src/pvr/generated"
    "${_pvr_mesa}/include"
    "${_pvr_mesa}/src"
    "${_pvr_mesa}/src/imagination"
    "${_pvr_mesa}/src/imagination/include"
    "${_pvr_mesa}/src/imagination/common"
    "${_pvr_mesa}/src/imagination/vulkan"
    "${_pvr_mesa}/src/imagination/vulkan/pds"
    "${_pvr_mesa}/src/imagination/vulkan/pds/pvr_pds_programs")
set(PVR_MESA_SOURCES "${_pvr_mesa_root}/driver/compat/pvr_mesa_pds.c")
