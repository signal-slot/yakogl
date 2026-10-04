#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Differential check of two library builds: build and run both with run.sh
# (every harness program, every OSAL mode, dev and release), then compare
# every log pair and print a summary table. Exits non-zero on any
# difference that allow.txt (plus HH_ALLOW files) does not explain.
#
# usage: compare.sh <out> <libA> <libB> <driver repo> [driver repo for B]
#   <lib> as for run.sh: rust:<dir> | csrc:<dir> | prebuilt:<archive>:<dir>
#   HH_PROFILES   profiles to build and compare (default "dev release")
#   HH_ALLOW      extra allow-list files (space separated)
#   HH_SKIP_RUN=1 only report on existing runs
#   other HH_* variables are passed to run.sh (HH_MODES, HH_SIZE, HH_JOBS, ...)
#
# examples:
#   C at the fork point vs the port:
#     compare.sh out csrc:$W/oldsrc rust:. $W/drvold
#   the port before vs after a change:
#     compare.sh out rust:/path/to/before rust:. $W/drv
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
[ $# -ge 4 ] || { sed -n '3,20p' "$0"; exit 2; }
OUT="$1"; LA="$2"; LB="$3"; DA="$4"; DB="${5:-$4}"
mkdir -p "$OUT"
allow=(--allow "$HERE/allow.txt")
for f in ${HH_ALLOW:-}; do allow+=(--allow "$f"); done
rc=0
report="$OUT/report.txt"
: > "$report"

for p in ${HH_PROFILES:-dev release}; do
  if [ -z "${HH_SKIP_RUN:-}" ]; then
    "$HERE/run.sh" all "$OUT/A.$p" "$LA" "$DA" "$p" || { echo "compare.sh: A ($p) failed to build/run" | tee -a "$report"; exit 3; }
    "$HERE/run.sh" all "$OUT/B.$p" "$LB" "$DB" "$p" || { echo "compare.sh: B ($p) failed to build/run" | tee -a "$report"; exit 3; }
  fi
done

{
  for p in ${HH_PROFILES:-dev release}; do
    echo "################ A vs B, profile $p"
    for s in A B; do echo "$s: $(tr '\n' ' ' < "$OUT/$s.$p/build-info.txt")"; done
    python3 "$HERE/report.py" "${allow[@]}" compare "$OUT/A.$p" "$OUT/B.$p" || rc=1
    echo
  done
  set -- ${HH_PROFILES:-dev release}
  if [ $# -ge 2 ]; then
    for s in A B; do
      echo "################ $s: $1 vs $2 (the same library, overflow checks / optimisation)"
      python3 "$HERE/report.py" "${allow[@]}" compare "$OUT/$s.$1" "$OUT/$s.$2" || echo "(profile differences are reported, not fatal)"
      echo
    done
  fi
  for p in ${HH_PROFILES:-dev release}; do
    for s in A B; do
      echo "################ $s ($p): outcomes"
      python3 "$HERE/report.py" "${allow[@]}" status "$OUT/$s.$p"
      echo
      echo "################ $s ($p): cross-mode checks"
      python3 "$HERE/report.py" "${allow[@]}" modes "$OUT/$s.$p"
      echo
    done
  done
  echo "################ entry-point coverage (B)"
  python3 "$HERE/report.py" coverage "$OUT"/B.*
} 2>&1 | tee -a "$report"
rc=${PIPESTATUS[0]}
# the compare exit codes are recorded in the report; recompute the verdict
if grep -q '^summary: .* [1-9][0-9]* unexpected$' <(awk '/^################ A vs B/{f=1} /^################ [AB]:/{f=0} f' "$report"); then
  echo "compare.sh: UNEXPECTED DIFFERENCES (see $report)"
  exit 1
fi
echo "compare.sh: no unexpected differences between A and B (report: $report)"
exit 0
