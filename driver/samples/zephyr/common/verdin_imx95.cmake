# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# The Verdin iMX95's A55 (verdin_imx95/mimx9596/a55) in a conformance sample
# (gl_test, deqp): include() this before find_package(Zephyr). On any other
# board it does nothing.
#
# The board's files are the board layer's, as every example gets them
# (examples/zephyr.cmake): driver/zephyr/lib/board/verdin_imx95/app.conf (the
# Mali, the carve-out's window memory, the console, the MMU's tables) and
# app.overlay (the X4 display's node). After app.conf comes the sample's own
# verdin_imx95.conf (next to its CMakeLists.txt), which overrides it where a
# conformance image differs: the carve-out, the board layer's lifetime and
# hold, and CONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=n. Then the caller's own
# EXTRA_CONF_FILE (texture-2048.conf, a command line's), so those win.
#
# The display: the overlay is in every image, so the panel is one Kconfig
# switch away, and verdin_imx95.conf turns it off: a conformance image reads
# its pixels back and never brings the panel up unless asked
# (-DCONFIG_YAKOGL_VERDIN_IMX95_DISPLAY=y on the command line, which Zephyr
# merges after every file).
#
# The sample's file is not boards/<board>.conf: Zephyr merges that before
# every EXTRA_CONF_FILE, so app.conf's values would win over it.
if(NOT BOARD AND DEFINED ENV{BOARD})
  set(BOARD "$ENV{BOARD}")
endif()
if(BOARD MATCHES "^verdin_imx95(/|$)")
  if(NOT PVR_REPO)
    message(FATAL_ERROR "verdin_imx95.cmake: PVR_REPO (driver/) is not set")
  endif()
  set(_vi95_layer "${PVR_REPO}/zephyr/lib/board/verdin_imx95")
  set(_vi95_conf "${CMAKE_CURRENT_SOURCE_DIR}/verdin_imx95.conf")
  if(NOT EXISTS "${_vi95_conf}")
    message(FATAL_ERROR "verdin_imx95.cmake: ${_vi95_conf} is missing (this sample has no Verdin iMX95 settings)")
  endif()
  set(EXTRA_CONF_FILE "${_vi95_layer}/app.conf" "${_vi95_conf}" ${EXTRA_CONF_FILE})
  list(APPEND EXTRA_DTC_OVERLAY_FILE "${_vi95_layer}/app.overlay")
endif()
