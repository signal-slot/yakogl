#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-g310-cause.py: one synthetic TSV, every cause, the summary
# table, a file argument and the refusals. No board, no network.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/imx95-g310-cause.py
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# row <case> <t6> <imx95>: one TSV line
row() { printf '%s\t%s\t%s\n' "$1" "$2" "${3-}"; }
{
  echo "# comment"
  row a.same Pass Pass
  row a.bothfail Fail NotSupported
  row a.lost Pass Timeout
  row a.multisample.rgba8 Pass Fail
  row a.get_string.renderer Pass Fail
  row a.many_triangles Pass Fail
  row a.thread_sync Pass Fail
  row a.plain Pass QualityWarning
} > "$tmp/in.tsv"
out=$(python3 -I "$tool" --summary < "$tmp/in.tsv")
want() { printf '%s\n' "$out" | grep -qxF -- "$1" || { echo "missing: $1" >&2; exit 1; }; }
want "# comment"
want "$(printf 'a.same\tPass\tPass\tsame')"
want "$(printf 'a.bothfail\tFail\tNotSupported\tboth-fail')"
want "$(printf 'a.lost\tPass\tTimeout\tno-result')"
want "$(printf 'a.multisample.rgba8\tPass\tFail\ttib-budget')"
want "$(printf 'a.get_string.renderer\tPass\tFail\trenderer-name')"
want "$(printf 'a.many_triangles\tPass\tFail\ttiler-levels')"
want "$(printf 'a.thread_sync\tPass\tFail\tsingle-core')"
want "$(printf 'a.plain\tPass\tQualityWarning\tunexplained')"
want "$(printf '# total\t8')"
python3 -I "$tool" "$tmp/in.tsv" > "$tmp/out1.tsv"
grep -qxF "$(printf 'a.same\tPass\tPass\tsame')" "$tmp/out1.tsv"
if printf "x\tPass\n" | python3 -I "$tool" > /dev/null 2>&1; then echo "short line accepted" >&2; exit 1; fi
if row x Pass Bogus | python3 -I "$tool" > /dev/null 2>&1; then echo "bad result accepted" >&2; exit 1; fi
echo "imx95-g310-cause selftest: pass"
