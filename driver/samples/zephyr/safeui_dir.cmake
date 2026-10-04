# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Where the SafeUI files the samples share with the safety core are.
#
# Two things in these samples have to be the same on both cores, because an
# agreement that is typed twice is not one:
#
#   include/safeui/frame_ring.h   the frame-ring protocol (route 3)
#   ui/<panel>/safeui-layout.slint  where every element of the cluster goes
#
# The second is why this resolution is not inside the route-3 block: the rich
# UI is laid out by the same generated file as the safety layer the M7 draws
# over it, so every Slint build needs it, ring or no ring.
#
# Both live in this repository, in safeui/ next to this file (with
# ui_layout.h and scripts/gen_layout_slint.py, which generates the layouts).
# -DSAFEUI_DIR=<tree> still names another tree with the same layout, for a
# safety-core repository that carries its own copy. The environment variable
# of that name is no longer read: it used to name the SafeUI checkout that
# supplied the boards, and a stale one must not silently win.

function(safeui_find_dir out_var probe_file what)
  set(dir "${SAFEUI_DIR}")
  if(NOT dir)
    set(dir "${CMAKE_CURRENT_FUNCTION_LIST_DIR}/safeui")
  endif()
  if(NOT EXISTS "${dir}/${probe_file}")
    message(FATAL_ERROR
      "${what} needs the SafeUI layout files, and SAFEUI_DIR='${dir}' has no "
      "${probe_file}. Drop -DSAFEUI_DIR to use the copy in this repository "
      "(driver/samples/zephyr/safeui).")
  endif()
  set(${out_var} "${dir}" PARENT_SCOPE)
endfunction()

# The generated Slint layout for one panel, to put on the Slint include path.
# `panel` is a directory name under ui/ of that tree: 1024x600 for
# the AM62P's DSI panel, 1280x800 for the Verdin iMX8M Plus's.
function(safeui_layout_dir out_layout out_root panel)
  safeui_find_dir(root "ui/${panel}/safeui-layout.slint" "The Slint cluster UI")
  set(${out_layout} "${root}/ui/${panel}" PARENT_SCOPE)
  set(${out_root} "${root}" PARENT_SCOPE)
endfunction()
