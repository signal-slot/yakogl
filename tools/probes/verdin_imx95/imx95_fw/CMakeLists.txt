# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx95_fw: a Verdin iMX95 probe (tools/probes/verdin_imx95/README.md; this
# directory's README.md says its question and its lines).
#
#   west build -p always -b verdin_imx95/mimx9596/a55 -d build/imx95_fw yakogl/tools/probes/verdin_imx95/imx95_fw
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(imx95_fw)

target_sources(app PRIVATE src/main.c)
include(../common/vi95_probe.cmake)
# main.c includes "fw/mali_csffw_blob.h" (driver/tools/mkmalifw.sh makes it
# from the board's mali_csffw.bin; gitignored, never committed).
target_include_directories(app PRIVATE "${PVR_REPO}/src")
