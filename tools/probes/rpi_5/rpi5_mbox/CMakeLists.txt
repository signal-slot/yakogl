# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# rpi5_mbox: the Raspberry Pi 5's property mailbox alone, raw status and
# response codes of seven requests (docs/notes/rpi5-display.md, section 3,
# and section 4 Q1). See README.md.
#
#   west build -p always -b rpi_5 -d build/rpi5_mbox yakogl/tools/probes/rpi_5/rpi5_mbox \
#     -- -DYAKOGL_WERROR=ON
cmake_minimum_required(VERSION 3.20.0)

# driver/ is a Zephyr module (the board layer: the watchdog and the mailbox).
get_filename_component(PVR_REPO "${CMAKE_CURRENT_SOURCE_DIR}/../../../../driver" ABSOLUTE)
list(APPEND ZEPHYR_EXTRA_MODULES "${PVR_REPO}")

find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(rpi5_mbox)

# The build prints its own commit first (a result row names its build).
execute_process(COMMAND git -C "${PVR_REPO}" describe --always --dirty
                OUTPUT_VARIABLE RPI5_MBOX_BUILD OUTPUT_STRIP_TRAILING_WHITESPACE
                ERROR_QUIET RESULT_VARIABLE describe_rc)
if(NOT describe_rc EQUAL 0 OR RPI5_MBOX_BUILD STREQUAL "")
  set(RPI5_MBOX_BUILD "unknown")
endif()
target_compile_definitions(app PRIVATE RPI5_MBOX_BUILD="${RPI5_MBOX_BUILD}")
message(STATUS "rpi5_mbox build: ${RPI5_MBOX_BUILD}")

# The board layer's mailbox (rpi4/rpi_mbox.c, CONFIG_YAKOGL_RPI5_MBOX) is the
# third way of every request; the first two are this probe's own copy of its
# transaction (src/main.c), which shows the words rpi_mbox.c keeps to itself.
target_sources(app PRIVATE src/main.c)
target_include_directories(app PRIVATE ${PVR_REPO}/zephyr/lib/board/rpi4)

# Zero warnings in our C files (-DYAKOGL_WERROR=ON, the "zephyr" rows).
if(YAKOGL_WERROR)
  target_compile_options(app PRIVATE -Werror)
endif()
