#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# deqp-repro-ref71.sh - Mesa's V3D 7.1 output for the dEQP-GLES2 shaders kept
# in the tree as Rust string constants, as a table of names and counts.
#
# Usage: V3D_SHC=<v3d_shc> tools/cts/deqp-repro-ref71.sh [--out FILE]
#   V3D_SHC  the v3d_shc binary (tools/v3d/README.md, H2: Mesa 26.2.1's build-v3d
#            or drm_shim_ref.sh build); needed. Without it the script exits 3
#            and says `gap: V3D_SHC`.
#   --out    where to write (default tools/shaders/deqp_repro_counts_71.tsv).
#
# The program set is every `const NAME: &str = ...;` of tests/deqp_repro_shaders.rs
# and tests/mali_deqp_*.rs whose text has a `void main` (a plain or raw string
# literal; the mali_deqp tests read their pairs from tools/mali/testdata and
# embed none at the time of writing, so only the repro file gives rows). A
# `<X>_VS` and `<X>_FS` pair is one program (two rows); a lone constant is a
# program of its own stage and gets a stub partner made as
# tools/v3d/es100_ref71.sh does (the stub's numbers are not recorded). The
# sources are extracted to a scratch directory under $TMPDIR, translated and
# assembled as tools/v3d/glsl2v3d.py does (glsl2vivante's translate_es100_gl,
# glslangValidator -G) and run at `v3d_shc --device 71` (rev 7, compat_rev 0).
#
# Columns: case (<file>.<constant name lowercased, without _VS/_FS>), stage
# (vs|fs), compiled (yes|no), Mesa's reason ("ok" when compiled, else the
# failing step and its message), instructions, threads, uniform count ("-" when
# not compiled). Rows are sorted by case and stage. The table holds names and
# counts only, never shader text. Needs glslangValidator and python3. A second
# run writes the same bytes.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

if [ -z "${V3D_SHC-}" ] || [ ! -x "${V3D_SHC-}" ]; then
  echo "gap: V3D_SHC (set it to Mesa 26.2.1's v3d_shc, tools/v3d/README.md H2)" >&2
  exit 3
fi
exec python3 -I "$HERE/deqp-repro-ref71.py" --repo "$REPO" "$@"
