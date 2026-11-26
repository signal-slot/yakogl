#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Sourced by run-on-board.sh (and its selftest): the aarch64 C compiler of the
# board build. build.rs cross-compiles gl_test with the linker named by
# CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER (aarch64-linux-gnu-gcc, looked
# up on PATH), and panics when it is missing.
#
#   panthor_cross_cc <repo root>   0: aarch64-linux-gnu-gcc is on PATH, or
#                                     MALI_DEQP_TOOLCHAIN's bin/ was put first
#                                     on PATH; 1: neither (message on stderr)
#
# MALI_DEQP_TOOLCHAIN (tools/cts/deps.toml) comes from
# `tools/cts/deps-env.sh --present nanopc_t6`; DEPS_ENV overrides that script
# (the selftest's fake layout). A compiler already on PATH wins.
panthor_cross_cc() {
  local root=$1 deps t
  command -v aarch64-linux-gnu-gcc > /dev/null 2>&1 && return 0
  deps=${DEPS_ENV:-$root/tools/cts/deps-env.sh}
  t=$("$deps" --present nanopc_t6 2> /dev/null | sed -n 's/^MALI_DEQP_TOOLCHAIN=//p') || t=
  if [ -n "$t" ] && [ -x "$t/bin/aarch64-linux-gnu-gcc" ]; then
    PATH=$t/bin:$PATH
    export PATH
    return 0
  fi
  echo "run-on-board.sh: no aarch64-linux-gnu-gcc on PATH and no MALI_DEQP_TOOLCHAIN: tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN (or install gcc-aarch64-linux-gnu)" >&2
  return 1
}
