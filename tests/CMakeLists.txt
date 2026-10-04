# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host tests. These run the GL core on the CPU (YAKOGL_OSAL=linux): nothing is
# rendered, so only API semantics can be checked here -- which is exactly what
# dEQP's negative_api group tests.
#
# negative_api needs the conformance suite's sources to extract the case bodies
# from. Point YAKOGL_CTS_DIR at a VK-GL-CTS checkout; without one the target is
# skipped (with a message) so the build still succeeds.

set(YAKOGL_CTS_DIR "$ENV{HOME}/com/github/KhronosGroup/VK-GL-CTS"
    CACHE PATH "VK-GL-CTS checkout (negative_api extracts its case bodies from it)")
set(_deqp_gles2 "${YAKOGL_CTS_DIR}/modules/gles2/functional")

if(NOT EXISTS "${_deqp_gles2}/es2fNegativeTextureApiTests.cpp")
  message(STATUS "YakoGL tests: no VK-GL-CTS at ${YAKOGL_CTS_DIR}, skipping negative_api")
  return()
endif()

enable_language(CXX)
set(CMAKE_CXX_STANDARD 11)

set(_cases "${CMAKE_CURRENT_BINARY_DIR}/cases.inc")
file(GLOB _deqp_srcs "${_deqp_gles2}/es2fNegative*.cpp")
add_custom_command(
  OUTPUT "${_cases}"
  COMMAND "${CMAKE_COMMAND}" -E env python3
          "${CMAKE_CURRENT_SOURCE_DIR}/negative_api/extract.py" "${_deqp_gles2}" "${_cases}"
  DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/negative_api/extract.py" ${_deqp_srcs}
  COMMENT "Extracting the dEQP-GLES2 negative_api case bodies")

add_executable(negative_api negative_api/main.cpp "${_cases}")
target_include_directories(negative_api PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/negative_api" "${CMAKE_CURRENT_BINARY_DIR}")
target_link_libraries(negative_api PRIVATE GLES2 m)
