# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# include() after find_package(Zephyr): adds the shared board glue to `app`.
set(YAKOGL_SAMPLE_COMMON "${CMAKE_CURRENT_LIST_DIR}")

# Build identity, captured here and printed by sample_glue.c before the first
# case. A console row from a board run has to say which tree and which shader
# table produced it: the images are built from a worktree with an uncommitted
# blobs.rs (that is the point of the rebuild window), so the commit alone does
# not identify what ran.
get_filename_component(YAKOGL_SAMPLE_REPO "${YAKOGL_SAMPLE_COMMON}/../../../.." ABSOLUTE)
execute_process(
  COMMAND git -C "${YAKOGL_SAMPLE_REPO}" describe --always --dirty
  OUTPUT_VARIABLE YAKOGL_SAMPLE_TREE OUTPUT_STRIP_TRAILING_WHITESPACE
  ERROR_QUIET RESULT_VARIABLE _pvr_git_rc)
if(NOT _pvr_git_rc EQUAL 0 OR YAKOGL_SAMPLE_TREE STREQUAL "")
  set(YAKOGL_SAMPLE_TREE "unknown")
endif()
set(_pvr_blobs "${YAKOGL_SAMPLE_REPO}/tests/reference/blobs.rs")
if(EXISTS "${_pvr_blobs}")
  file(SHA256 "${_pvr_blobs}" _pvr_blobs_sha)
  string(SUBSTRING "${_pvr_blobs_sha}" 0 8 YAKOGL_SAMPLE_TABLE)
else()
  set(YAKOGL_SAMPLE_TABLE "none")
endif()
target_compile_definitions(app PRIVATE
  YAKOGL_SAMPLE_TREE="${YAKOGL_SAMPLE_TREE}" YAKOGL_SAMPLE_TABLE="${YAKOGL_SAMPLE_TABLE}")
message(STATUS "pvr sample build: tree ${YAKOGL_SAMPLE_TREE} table ${YAKOGL_SAMPLE_TABLE}")
# The glue itself moved to the board layer (driver/zephyr/lib/board/common,
# driver/docs/one-method.md); the RTI self-arm is compiled there for every
# image that sets YAKOGL_AM62P_RTI_SELF_ARM. sample_glue.c stays in `app` for
# the samples not yet on the board layer, which still call pvr_sample_*.
set(YAKOGL_SAMPLE_BOARD "${YAKOGL_SAMPLE_COMMON}/../../../zephyr/lib/board/common")
target_include_directories(app PRIVATE "${YAKOGL_SAMPLE_COMMON}" "${YAKOGL_SAMPLE_BOARD}")
if(NOT CONFIG_YAKOGL_BOARD)
  # With the board layer on, its library compiles sample_glue.c.
  target_sources(app PRIVATE "${YAKOGL_SAMPLE_BOARD}/sample_glue.c")
endif()
target_sources_ifdef(CONFIG_YAKOGL_SAMPLE_BOOT_TRACE app PRIVATE "${YAKOGL_SAMPLE_COMMON}/boot_trace.c")
if(CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP AND NOT CONFIG_YAKOGL_BOARD)
  target_sources(app PRIVATE "${YAKOGL_SAMPLE_BOARD}/../am62p/display_dump.c")
endif()
