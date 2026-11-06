# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Include from a probe's CMakeLists.txt after find_package(Zephyr) and
# project(...):
#
#   include(${CMAKE_CURRENT_SOURCE_DIR}/../common/boot/rpi5_boot.cmake)
#
# Adds boot_x0.S to the app, the header directory to its include path and
# -Wl,--wrap=__start to the link (README.md).
get_filename_component(_rpi5_boot_dir "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)
target_sources(app PRIVATE ${_rpi5_boot_dir}/boot_x0.S)
target_include_directories(app PRIVATE ${_rpi5_boot_dir})
zephyr_link_libraries(-Wl,--wrap=__start)
