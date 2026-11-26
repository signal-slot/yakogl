#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-finish.sh - the end-of-campaign chain of docs/es3/board-recipe-imx8mp.md step 14 in one script.
#
#   tools/cts/imx8mp-gles3-finish.sh [--partial] [--apply] [--commit SHA8] [--date YYYY-MM-DD] OUTDIR
#
# Runs over tools/cts/log/*-imx8mp-gles3-<SHA8>-rows* (default SHA8 9bc82915, date 2026-11-14):
#   1 check     merge.py --check            -> check.txt (stdout), check-warnings.txt (stderr WARNING lines);
#                                              stops on a non-zero exit unless --partial (entries without a record allowed)
#   2 notpass   merge.py --notpass          -> notpass.tsv (the SPDX header of docs/es3/imx8mp-gles3-notpass.tsv first)
#   3 filter    grep -v ':NoResult '        -> notpass-run.tsv
#   4 compare   compare.py --tsv --causes --table, board = notpass-run.tsv, host = the vivante rows of
#               docs/es3/host-gc-gles3-5454be1b/host-gc-gles3-notpass.tsv -> compare.tsv (with the header comments of the
#               committed docs/es3/imx8mp-gles3-compare-9bc82915.tsv)
#   5 issues    issues.py --commit SHA8     -> issue-drafts.md
#   6 report    report.py --date DATE       -> report.md (stderr -> report.err); its exit code is shown, and is the
#                                              script's exit code unless --partial
# One summary line per step on stdout. --apply also copies notpass.tsv and compare.tsv over the two committed docs
# files; without it no committed file changes. Needs python3, awk, grep.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
SHA=9bc82915 DATE=2026-11-14 APPLY=0 PARTIAL=
OUT=
while [ $# -gt 0 ]; do
  case $1 in
    --apply) APPLY=1 ;;
    --partial) PARTIAL=--partial ;;
    --commit) SHA=${2:?--commit needs a value}; shift ;;
    --date) DATE=${2:?--date needs a value}; shift ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) OUT=$1 ;;
  esac
  shift
done
[ -n "$OUT" ] || { echo "usage: $0 [--partial] [--apply] [--commit SHA8] [--date D] OUTDIR" >&2; exit 2; }
mkdir -p "$OUT" || exit 2
OUT=$(cd "$OUT" && pwd)
cd "$ROOT" || exit 2
C=tools/cts
NP=docs/es3/imx8mp-gles3-notpass.tsv
CMP=docs/es3/imx8mp-gles3-compare-9bc82915.tsv
HOST=docs/es3/host-gc-gles3-5454be1b/host-gc-gles3-notpass.tsv
D=$(ls -d $C/log/*-imx8mp-gles3-"$SHA"-rows* 2>/dev/null)
[ -n "$D" ] || { echo "no record directories $C/log/*-imx8mp-gles3-$SHA-rows*" >&2; exit 2; }
py() { python3 -I "$@"; }

# 1 check
py $C/imx8mp-gles3-merge.py --check $PARTIAL $D >"$OUT/check.txt" 2>"$OUT/check.err"; rc=$?
grep WARNING "$OUT/check.err" >"$OUT/check-warnings.txt"
grep -v WARNING "$OUT/check.err" >&2
echo "1 check:   rc=$rc, $(tail -n 1 "$OUT/check.txt"), $(wc -l <"$OUT/check-warnings.txt" | tr -d ' ') WARNING line(s) -> check-warnings.txt"
[ $rc -eq 0 ] || { echo "stop: merge.py --check failed (see $OUT/check.txt)" >&2; exit 1; }

# 2 notpass
{ head -n 2 $NP; py $C/imx8mp-gles3-merge.py --notpass $D 2>/dev/null; } >"$OUT/notpass.tsv" || exit 1
echo "2 notpass: $(grep -vc '^#' "$OUT/notpass.tsv") rows -> notpass.tsv"

# 3 filter
grep -v ':NoResult ' "$OUT/notpass.tsv" >"$OUT/notpass-run.tsv"
echo "3 filter:  $(grep -vc '^#' "$OUT/notpass-run.tsv") rows without NoResult -> notpass-run.tsv"

# 4 compare (vivante host rows only: the host file mixes GPUs)
awk -F'\t' '/^#/ || $1=="gpu" || $1=="vivante"' $HOST >"$OUT/host-vivante.tsv"
{ awk '/^#/{print; next} {exit}' $CMP
  py $C/imx8mp-gles3-compare.py --tsv --causes $C/imx8mp-gles3-causes.tsv --table $C/imx8mp-gles3-tree.txt \
    "$OUT/notpass-run.tsv" "$OUT/host-vivante.tsv"; } >"$OUT/compare.tsv" || { echo "stop: compare.py failed" >&2; exit 1; }
echo "4 compare: $(grep -vc '^#' "$OUT/compare.tsv") rows (host vivante $(grep -c '^vivante' "$OUT/host-vivante.tsv")) -> compare.tsv"

# 5 issues
py $C/imx8mp-gles3-issues.py --commit "$SHA" "$OUT/compare.tsv" >"$OUT/issue-drafts.md" || { echo "stop: issues.py failed" >&2; exit 1; }
echo "5 issues:  $(grep -c '^## ' "$OUT/issue-drafts.md") draft(s) -> issue-drafts.md"

# 6 report
py $C/imx8mp-gles3-report.py --date "$DATE" $D >"$OUT/report.md" 2>"$OUT/report.err"; rrc=$?
echo "6 report:  rc=$rrc, $(wc -l <"$OUT/report.md" | tr -d ' ') lines -> report.md"

if [ $APPLY -eq 1 ]; then
  cp "$OUT/notpass.tsv" $NP && cp "$OUT/compare.tsv" $CMP || exit 1
  echo "apply:     wrote $NP and $CMP"
fi
[ -n "$PARTIAL" ] && exit 0
exit $rrc
