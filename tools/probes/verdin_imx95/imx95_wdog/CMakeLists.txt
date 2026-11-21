# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx95_wdog: a Verdin iMX95 probe (tools/probes/verdin_imx95/README.md; this
# directory's README.md says its question and its lines).
#
#   west build -p always -b verdin_imx95/mimx9596/a55 -d build/imx95_wdog yakogl/tools/probes/verdin_imx95/imx95_wdog
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board, the board glue, the Rust crate).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(imx95_wdog)

target_sources(app PRIVATE src/main.c)
include(../common/vi95_probe.cmake)
