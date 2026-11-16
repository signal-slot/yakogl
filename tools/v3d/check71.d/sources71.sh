#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# sources71: tools/v3d/qpu_corpus_71_sources.tsv, the GLSL source and stage of
# every program of qpu_corpus_71.txt, is what tools/v3d/qpu_corpus_71_sources.py
# writes from qpu_counts_71.tsv and tools/shaders: every program of the corpus
# has its three stages, each naming an existing source of its stage, with
# Mesa's count of qpu_counts_71.tsv. A copy with one row changed must be
# refused, so the comparison cannot pass by reading nothing. Host only, no
# Mesa input. Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=sources71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1
gen=tools/v3d/qpu_corpus_71_sources.py
tsv=tools/v3d/qpu_corpus_71_sources.tsv

out=$(python3 -I -B "$gen" --check 2>&1) || {
  printf '%s\n' "$out"
  echo "$name: $tsv is not what $gen writes, 1 failed"
  exit 1
}
# The negative: the last row's count changed by one.
sed '$ s/\t\([0-9]*\)$/\t1\1/' "$tsv" >"$dir/changed.tsv" || exit 1
if cmp -s "$tsv" "$dir/changed.tsv" || python3 -I -B "$gen" --check --out "$dir/changed.tsv" >"$dir/changed.log" 2>&1; then
  echo "$name: a copy with a changed row was not refused, 1 failed"
  exit 1
fi
echo "$name: ${out#qpu_corpus_71_sources: }, a changed copy refused, 0 failed"
