#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# copy.sh: build copy_model.c against the tree's gc_draw.c and gc_cmd.c and
# run it (copy_model.c's header): gc_draw_copy()'s resolve-engine stream from
# a linear render target to a linear scan-out buffer, executed by a model of
# the resolve engine, and the copies it must refuse. Host only, under a
# second. Exits 1 on a failing case.
#
#   copy.sh [outdir]
#     outdir  default: a new directory under ${TMPDIR:-/tmp}
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
out=${1:-$(mktemp -d "${TMPDIR:-/tmp}/copy_model.XXXXXX")}
mkdir -p "$out"
cc=${CC:-cc}

"$cc" -std=gnu11 -O1 -Wall -Wextra -Werror -Wno-unused-parameter \
  -I"$here/shim" -I"$here/.." \
  -DCONFIG_YAKOGL_DRV_GPU_VIVANTE_LOG_LEVEL=0 \
  -o "$out/copy_model" "$here/../gc_draw.c" "$here/../gc_cmd.c" "$here/copy_model.c"
"$out/copy_model"
