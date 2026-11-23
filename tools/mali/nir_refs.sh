#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# nir_refs.sh - Mesa 26.2.1's NIR for each committed reference pair, at two
# points: `ssa`, the last print of a stage before its first
# nir_opt_find_array_copies (gl_nir_opts after nir_lower_vars_to_ssa), and
# `select`, the NIR bi_compile_variant_nir prints before selection, per
# variant. Draws each pair on the drm-shim twice, with NIR_DEBUG=print_vs
# and with NIR_DEBUG=print_fs, normalises each dump and writes
# tools/mali/testdata/<name>.nir.sha, the SHA-256 of each normalised text
# (docs/notes/mali-shader-nir.md, 7.2). The rules and the file format are in
# tools/mali/nir_refs.py.
#
#   tools/mali/nir_refs.sh [--build-dir <dir>] [--texts <dir>] [--jobs <n>] [--scratch <dir>] [<name or glob>...]
#   tools/mali/nir_refs.sh normalise < dump.txt
set -euo pipefail
exec python3 -I "$(cd "$(dirname "$0")" && pwd)/nir_refs.py" "$@"
