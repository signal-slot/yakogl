# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# What every Verdin iMX95 probe's CMakeLists.txt shares, included after
# project(): the build's commit as VI95_PROBE_BUILD (a result row names its
# build), the probe's name as VI95_PROBE_NAME, the common record
# (vi95_probe.c: the banner with WDOG3's arming, the raw SCMI helper, the end
# in the PSCI reset), and -Werror under -DYAKOGL_WERROR=ON (tools/check.sh's
# "zephyr verdin_imx95 probe images" row).
get_filename_component(VI95_PROBE_NAME "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE VI95_PROBE_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE _vi95_describe_rc)
if(NOT _vi95_describe_rc EQUAL 0 OR VI95_PROBE_BUILD STREQUAL "")
  set(VI95_PROBE_BUILD "unknown")
endif()
message(STATUS "${VI95_PROBE_NAME} build: ${VI95_PROBE_BUILD}")
target_compile_definitions(app PRIVATE VI95_PROBE_BUILD="${VI95_PROBE_BUILD}"
                                       VI95_PROBE_NAME="${VI95_PROBE_NAME}")
target_sources(app PRIVATE "${CMAKE_CURRENT_LIST_DIR}/vi95_probe.c")
target_include_directories(app PRIVATE "${CMAKE_CURRENT_LIST_DIR}")
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
