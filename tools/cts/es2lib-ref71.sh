#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# es2lib-ref71.sh - Mesa's V3D 7.1 output for every expect-pass program of the
# dEQP-GLES2 shader library, as a table of names and counts.
#
# Usage: tools/cts/es2lib-ref71.sh [--cts DIR] [--out FILE] [--jobs N]
#   V3D_SHC  the v3d_shc binary (tools/v3d/README.md, H2: Mesa 26.2.1's build-v3d
#            or drm_shim_ref.sh build); needed. Without it the script exits 3
#            and says `gap: V3D_SHC`.
#   --cts    a tree holding data/gles2/shaders of VK-GL-CTS 93bca01 (or that
#            directory itself). Default: $DEQP_GLES2_TEST_DIR, then
#            $CONNECTED_DEPS_DIR/VK-GL-CTS-gles2-data, then $VK_GL_CTS.
#   --out    where to write (default tools/shaders/deqp_es2_lib_counts_71.tsv).
#   --jobs   parallel v3d_shc runs (default: the CPU count; the table does not
#            depend on it).
#
# The program set is what tools/es100_oracle.py's programs_of() yields for the
# cases with `expect pass` or `build_successful`, version 100 only (its parser
# is imported, not copied). Each program is translated and assembled as
# tools/v3d/es100_ref71.sh does (glsl2vivante's translate_es100_gl,
# glslangValidator -G) and run at `v3d_shc --device 71`. A `both` case gives
# `<case>_vertex` and `<case>_fragment`, one row each (its stage only); a case
# with both stages gives two rows with the same name, vs and fs.
# Columns: case (file.group.case[_vertex|_fragment]), stage, compiled (yes|no),
# Mesa's reason, instructions, threads, uniforms ("-" when not compiled). Rows
# are sorted. The table holds names and counts only, never shader text. Needs
# glslangValidator and python3. A second run writes the same bytes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

if [ -z "${V3D_SHC-}" ] || [ ! -x "${V3D_SHC-}" ]; then
  echo "gap: V3D_SHC (set it to Mesa 26.2.1's v3d_shc, tools/v3d/README.md H2)" >&2
  exit 3
fi
exec python3 -I "$HERE/es2lib-ref71.py" --repo "$REPO" "$@"
