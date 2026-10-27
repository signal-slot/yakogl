# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Locate the YakoGL tree and register it, with the driver, as a Zephyr module.
#
# include() this BEFORE find_package(Zephyr), from a sample that has already
# set PVR_REPO to this repository's root.
#
# YakoGL is the repository root: this driver tree is its driver/ directory.
# Another YakoGL tree may be named instead:
#
#   west build ... -- -DYAKOGL_DIR=/path/to/yakogl
#
# or through an environment variable of the same name.
if(NOT YAKOGL_DIR)
  if(DEFINED ENV{YAKOGL_DIR})
    set(YAKOGL_DIR "$ENV{YAKOGL_DIR}")
  else()
    set(YAKOGL_DIR "${PVR_REPO}/..")
  endif()
endif()
get_filename_component(YAKOGL_DIR "${YAKOGL_DIR}" ABSOLUTE)
if(NOT EXISTS "${YAKOGL_DIR}/zephyr/module.yml")
  message(FATAL_ERROR
    "YAKOGL_DIR=${YAKOGL_DIR} is not a YakoGL checkout (no zephyr/module.yml). "
    "Pass -DYAKOGL_DIR=<path to the yakogl repository root>.")
endif()

# YakoGL's module build looks for the driver's headers in driver/src under its
# own root. Say where they actually are, so YAKOGL_DIR may point anywhere.
set(YAKOGL_PVR_DRIVER_DIR "${PVR_REPO}/src")

list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}" "${YAKOGL_DIR}")
