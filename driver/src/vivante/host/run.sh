#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# run.sh: build wait_model.c against a gc_render.c and print how
# gc_render_wait() ends in each scripted case (wait_model.c's header): the
# fence done before the wait, done during it (woken by the interrupt), the
# wake lost (the 1 ms poll ends it), a fault, a deadline-free wait, and the
# timeout. Host only, under a second; a simulated clock, so no row depends on
# the machine's timing.
#
#   run.sh [gc_render.c] [outdir]
#     gc_render.c  default: the tree's. An older one (git show
#                  <rev>:driver/src/vivante/gc_render.c > old.c) prints its
#                  table without the check: before the interrupt woke the
#                  wait, "woken", "stale", "fault" and "forever" took the
#                  1 ms sleep.
#     outdir       default: a new directory under ${TMPDIR:-/tmp}
#
# With the tree's file each case is checked against its expected return, time,
# blocks and timeouts, and the run exits 1 on a difference.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
src=$(realpath "${1:-$here/../gc_render.c}")
out=${2:-$(mktemp -d "${TMPDIR:-/tmp}/wait_model.XXXXXX")}
mkdir -p "$out"
args=()
[ "$src" = "$(realpath "$here/../gc_render.c")" ] || args=(--table)
cc=${CC:-cc}

"$cc" -std=gnu11 -O1 -Wall -Wextra -Werror -Wno-unused-parameter \
  -I"$here/shim" -I"$here/.." \
  -DCONFIG_YAKOGL_DRV_GPU_VIVANTE_LOG_LEVEL=0 \
  -o "$out/wait_model" "$src" "$here/wait_model.c"
"$out/wait_model" "${args[@]}"
