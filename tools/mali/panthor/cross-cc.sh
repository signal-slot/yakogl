#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Sourced by run-on-board.sh. panthor_cross_cc <repo root> makes sure an
# aarch64-linux-gnu-gcc is on PATH: one already there wins; otherwise the
# MALI_DEQP_TOOLCHAIN of tools/cts/deps-env.sh (DEPS_ENV overrides the path)
# supplies it. Returns 1 with a message when neither exists.

panthor_cross_cc() {
  local root=$1 deps t lookup_rc=0
  if command -v aarch64-linux-gnu-gcc > /dev/null 2>&1; then
    return 0
  fi
  deps=${DEPS_ENV:-$root/tools/cts/deps-env.sh}
  # deps-env.sh's stderr stays visible.
  t=$("$deps" --present nanopc_t6 | sed -n 's/^MALI_DEQP_TOOLCHAIN=//p') || lookup_rc=$?
  if [ -n "$t" ] && [ -x "$t/bin/aarch64-linux-gnu-gcc" ]; then
    PATH="$t/bin:$PATH"
    export PATH
    return 0
  fi
  {
    echo "panthor_cross_cc: no aarch64-linux-gnu-gcc on PATH and none in MALI_DEQP_TOOLCHAIN"
    echo "  build it: tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN"
    echo "  or install gcc-aarch64-linux-gnu"
    if [ "$lookup_rc" -ne 0 ]; then
      echo "  (the lookup '$deps --present nanopc_t6' itself failed, status $lookup_rc)"
    fi
  } >&2
  return 1
}
