# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# yakogl/zephyr/linker_snippets.cmake
#
# The #include lines of the generated linker snippets must not depend on the
# build directory's depth.
#
# Zephyr records every linker snippet in
# ${APPLICATION_BINARY_DIR}/zephyr/include/generated/snippets-*.ld as an
# #include path relative to ${ZEPHYR_BASE}/include (file(RELATIVE_PATH) in
# cmake/modules/extensions.cmake:1456) plus a sort-key comment, and the build
# preprocesses the linker script with those relative paths. A relative path
# names the same file only while the build sees one Zephyr spelling; when the
# west workspace is entered through a symbolic link, west list prints the
# Zephyr checkout under its real path while ZEPHYR_BASE stays the linked one,
# the Zephyr checkout re-enters ZEPHYR_MODULES (tools/cts/zephyr-modules.sh
# compares the two spellings), and arch's snippets are recorded a second time
# against the linked ZEPHYR_BASE - as a long ../-chain from the linked include
# directory up to a common ancestor and back down to the real checkout, e.g.
#   /* Sort key: "K" */#include "../../../../../../../../com/github/signal-slot/zephyr-ws-yakogl/zephyr/arch/arm64/core/swi_tables.ld"
# Such a chain lands on a real file only when one of the directories the
# preprocessor searches for a quoted #include (the snippet file's own
# directory, then the build's absolute -I directories) happens to sit at the
# depth the chain was computed for. Passing image builds had that luck -
# this node's deps cache still carries a passing imx8mp build's monster
# line, and its linker.cmd.dep shows it resolved from an absolute include
# directory - and a scratch worktree reached nothing that fit:
#   snippets-rodata.ld: fatal error: .../swi_tables.ld: No such file or directory
# (the am62p and nanopc_t6 image rows of tools/check.sh; runner A scratch
# worktrees 2026-10-18, CI run 2026-10-19).
#
# What this does: at the end of the configure - cmake_language(DEFER CALL ...
# DIRECTORY ${CMAKE_SOURCE_DIR}) runs after the whole project tree, so after
# every zephyr_linker_sources() call (arch records its snippets before the
# module loop, zephyr/CMakeLists.txt:765 and :789) - each snippet #include is
# rewritten to the absolute path of the file it means: the recorded path read
# against ${ZEPHYR_BASE}/include (the base Zephyr computed it from) or, if
# that is not a file, against the snippet file's own directory (the base the
# linker resolves it from). An absolute path resolves from anywhere, so the
# build stops depending on the build directory's depth and on which spelling
# recorded the line. A recorded path that is a file in neither reading means
# the build saw the Zephyr checkout under a spelling ZEPHYR_BASE does not
# name; that line is left as it is (a passing build may resolve it from
# another -I directory) and the configure warns with the line named, so a
# later bare ld error is traceable.
#
# The snippet files are Zephyr's own, regenerated into the build directory at
# every configure; this runs after them, rewrites them in place, and writes
# only when a line changes, so a rerun touches no timestamps. It touches no
# source file, no build setting, and nothing outside the build directory.

function(yakogl_linker_snippets_absolute)
  if(NOT DEFINED ZEPHYR_BASE)
    return()
  endif()
  if(DEFINED APPLICATION_BINARY_DIR)
    set(_yakogl_gen_dir "${APPLICATION_BINARY_DIR}/zephyr/include/generated")
  else()
    set(_yakogl_gen_dir "${CMAKE_BINARY_DIR}/zephyr/include/generated")
  endif()
  file(GLOB _yakogl_snippet_files "${_yakogl_gen_dir}/snippets-*.ld")
  foreach(_yakogl_snippet ${_yakogl_snippet_files})
    file(READ "${_yakogl_snippet}" _yakogl_old)
    file(STRINGS "${_yakogl_snippet}" _yakogl_lines)
    set(_yakogl_new_lines "")
    foreach(_yakogl_line ${_yakogl_lines})
      # Take the line apart at the first #include " and the quote that closes
      # it; anything else on the line is left as it is.
      string(FIND "${_yakogl_line}" "#include \"" _yakogl_open)
      if(_yakogl_open EQUAL -1)
        list(APPEND _yakogl_new_lines "${_yakogl_line}")
        continue()
      endif()
      math(EXPR _yakogl_rel_beg "${_yakogl_open} + 10") # past the opening quote
      string(SUBSTRING "${_yakogl_line}" 0 "${_yakogl_open}" _yakogl_head)
      string(SUBSTRING "${_yakogl_line}" "${_yakogl_rel_beg}" -1 _yakogl_tail)
      string(FIND "${_yakogl_tail}" "\"" _yakogl_close)
      if(_yakogl_close EQUAL -1)
        list(APPEND _yakogl_new_lines "${_yakogl_line}")
        continue()
      endif()
      string(SUBSTRING "${_yakogl_tail}" 0 "${_yakogl_close}" _yakogl_rel)
      math(EXPR _yakogl_suf_beg "${_yakogl_close} + 1")
      string(SUBSTRING "${_yakogl_tail}" "${_yakogl_suf_beg}" -1 _yakogl_suffix)
      if(_yakogl_rel STREQUAL "")
        list(APPEND _yakogl_new_lines "${_yakogl_line}")
        continue()
      endif()
      get_filename_component(_yakogl_abs "${_yakogl_rel}" ABSOLUTE BASE_DIR "${ZEPHYR_BASE}/include")
      if(NOT EXISTS "${_yakogl_abs}")
        get_filename_component(_yakogl_abs "${_yakogl_rel}" ABSOLUTE BASE_DIR "${_yakogl_gen_dir}")
      endif()
      if(NOT EXISTS "${_yakogl_abs}")
        # Left as Zephyr wrote it: the linker may still find the file from
        # another -I directory (a passing build can rely on that). Say which
        # line it is, so a later "No such file" from ld is traceable.
        message(WARNING
          "YakoGL: ${_yakogl_snippet}: the snippet line ${_yakogl_line} "
          "names \"${_yakogl_rel}\", which is no file read from "
          "${ZEPHYR_BASE}/include nor from ${_yakogl_gen_dir}; left as it is. "
          "If the linker cannot find it, ZEPHYR_WS and ZEPHYR_BASE are given "
          "under different spellings (a symbolic link); use real paths "
          "(readlink -f).")
        list(APPEND _yakogl_new_lines "${_yakogl_line}")
        continue()
      endif()
      list(APPEND _yakogl_new_lines "${_yakogl_head}#include \"${_yakogl_abs}\"${_yakogl_suffix}")
    endforeach()
    string(REPLACE ";" "\n" _yakogl_old_cmp "${_yakogl_lines}")
    string(REPLACE ";" "\n" _yakogl_new "${_yakogl_new_lines}")
    string(STRIP "${_yakogl_old}" _yakogl_old_raw)
    # The line model must reproduce the file it read before it writes back:
    # a snippet file whose content the line split cannot name line for line
    # (a ';' inside a line, which CMake reads as its list separator, or
    # shifted blank edges) is left alone rather than rewritten from a
    # reconstruction. Zephyr's zephyr_linker_sources() writes one sort-key
    # and #include line per call: no ';', no blank lines, so real snippet
    # files always take the rewrite path.
    if(_yakogl_old_cmp STREQUAL _yakogl_old_raw AND NOT _yakogl_new STREQUAL _yakogl_old_cmp)
      file(WRITE "${_yakogl_snippet}" "${_yakogl_new}\n")
      message(STATUS "YakoGL: ${_yakogl_snippet}: snippet includes rewritten to absolute paths")
    endif()
  endforeach()
endfunction()

# DIRECTORY ${CMAKE_SOURCE_DIR}: the call is deferred to the end of the
# top-level directory's processing - after every subdirectory, so after all
# zephyr_linker_sources() calls of the app, the modules and the Zephyr tree
# (CMAKE_SOURCE_DIR stays the application directory even though Zephyr's
# kernel.cmake overwrites PROJECT_SOURCE_DIR).
cmake_language(DEFER CALL yakogl_linker_snippets_absolute DIRECTORY ${CMAKE_SOURCE_DIR})
