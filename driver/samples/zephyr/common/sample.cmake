# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# include() after find_package(Zephyr): adds the shared board glue to `app`.
set(PVR_SAMPLE_COMMON "${CMAKE_CURRENT_LIST_DIR}")

# Build identity, captured here and printed by sample_glue.c before the first
# case. A console row from a board run has to say which tree and which shader
# table produced it: the images are built from a worktree with an uncommitted
# blobs.rs (that is the point of the rebuild window), so the commit alone does
# not identify what ran.
get_filename_component(PVR_SAMPLE_REPO "${PVR_SAMPLE_COMMON}/../../../.." ABSOLUTE)
execute_process(
  COMMAND git -C "${PVR_SAMPLE_REPO}" describe --always --dirty
  OUTPUT_VARIABLE PVR_SAMPLE_TREE OUTPUT_STRIP_TRAILING_WHITESPACE
  ERROR_QUIET RESULT_VARIABLE _pvr_git_rc)
if(NOT _pvr_git_rc EQUAL 0 OR PVR_SAMPLE_TREE STREQUAL "")
  set(PVR_SAMPLE_TREE "unknown")
endif()
set(_pvr_blobs "${PVR_SAMPLE_REPO}/tests/reference/blobs.rs")
if(EXISTS "${_pvr_blobs}")
  file(SHA256 "${_pvr_blobs}" _pvr_blobs_sha)
  string(SUBSTRING "${_pvr_blobs_sha}" 0 8 PVR_SAMPLE_TABLE)
else()
  set(PVR_SAMPLE_TABLE "none")
endif()
target_compile_definitions(app PRIVATE
  PVR_SAMPLE_TREE="${PVR_SAMPLE_TREE}" PVR_SAMPLE_TABLE="${PVR_SAMPLE_TABLE}")
message(STATUS "pvr sample build: tree ${PVR_SAMPLE_TREE} table ${PVR_SAMPLE_TABLE}")
# The glue itself moved to the board layer (driver/zephyr/lib/board/common,
# driver/docs/one-method.md); the RTI self-arm is compiled there for every
# image that sets YAKOGL_AM62P_RTI_SELF_ARM. sample_glue.c stays in `app` for
# the samples not yet on the board layer, which still call pvr_sample_*.
set(PVR_SAMPLE_BOARD "${PVR_SAMPLE_COMMON}/../../../zephyr/lib/board/common")
target_include_directories(app PRIVATE "${PVR_SAMPLE_COMMON}" "${PVR_SAMPLE_BOARD}")
if(NOT CONFIG_YAKOGL_BOARD)
  # With the board layer on, its library compiles sample_glue.c.
  target_sources(app PRIVATE "${PVR_SAMPLE_BOARD}/sample_glue.c")
endif()
target_sources_ifdef(CONFIG_PVR_SAMPLE_BOOT_TRACE app PRIVATE "${PVR_SAMPLE_COMMON}/boot_trace.c")
if(CONFIG_PVR_SAMPLE_DISPLAY_DUMP AND NOT CONFIG_YAKOGL_BOARD)
  target_sources(app PRIVATE "${PVR_SAMPLE_BOARD}/../am62p/display_dump.c")
endif()
