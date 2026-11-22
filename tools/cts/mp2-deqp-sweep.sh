#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The STM32MP257F-DK's dEQP-GLES2 sweep (roadmap P9), shard by shard.
#
#   tools/cts/mp2-deqp-sweep.sh [--dry-run] [--case-list FILE] [--exclude FILE]
#       [--first N] [--last N] [--budget-ms MS] [--times TSV] [--report-only]
#       <zephyr.bin> <outdir>
#
# Steps: (1) the case list (default mp2-deqp-gles2.txt minus
# mp2-deqp-gles2-exclude.txt, read the way mp2-deqp-list-check.sh reads them:
# the mustpass cases of gles2-mustpass.txt that a listed group covers and no
# exclusion covers; a --case-list FILE is taken as one case per line as it
# is, minus --exclude if given) is written to OUT/cases.txt and split by
# mp2-deqp-shards.py into OUT/shards/; (2) for each shard (--first/--last
# pick a range of shard numbers, to resume or split a sweep)
# mp2-deqp-slot.py writes the shard's case list into a copy of <zephyr.bin>,
# OUT/slot-NNNN.bin, and the runner runs it:
# RUNNER run <bin> OUT/run/shard-NNNN shard-NNNN; (3) the shard's log
# OUT/shard-NNNN.log is the runner's own output plus every console.txt it left
# under OUT/run/shard-NNNN; a shard whose log already holds a
# "deqp: <Status> <case>" result line is skipped; (4) mp2-p9-diff.py reads all
# logs (its exit 1, new failures or missing cases, is this script's).
#
# Env: MP2_SWEEP_RUNNER the runner command (default tools/board-stm32mp2.sh;
# RUN_BOUND=600 or more is the caller's to set for deqp), PYTHON (python3).
# --dry-run prints every command and runs none: the shard files do not exist
# then, so the per-shard commands are printed once with NNNN for the number.
# Exit: 0 done, 1 the diff or a step failed, 2 usage.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
RUNNER=${MP2_SWEEP_RUNNER:-$HERE/../board-stm32mp2.sh}
PY=${PYTHON:-python3}
DRY=0 FIRST=0 LAST= BUDGET= TIMES= REPORT= LIST= EXCL= EXCL_SET=0
die() { echo "mp2-deqp-sweep: $*" >&2; exit "${2:-1}"; }
usage() { sed -n '5,8p' "$0" >&2; exit 2; }
while [ $# -gt 0 ]; do
    case $1 in
    --dry-run) DRY=1 ;;
    --case-list) [ $# -ge 2 ] || usage; LIST=$2; shift ;;
    --exclude) [ $# -ge 2 ] || usage; EXCL=$2; EXCL_SET=1; shift ;;
    --first) [ $# -ge 2 ] || usage; FIRST=$2; shift ;;
    --last) [ $# -ge 2 ] || usage; LAST=$2; shift ;;
    --budget-ms) [ $# -ge 2 ] || usage; BUDGET=$2; shift ;;
    --times) [ $# -ge 2 ] || usage; TIMES=$2; shift ;;
    --report-only) REPORT=--report-only ;;
    -h|--help) usage ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
    esac
    shift
done
[ $# -eq 2 ] || usage
BIN=$1 OUT=$2
case $FIRST${LAST:-0} in *[!0-9]*) die "--first/--last take numbers" 2 ;; esac
[ "$DRY" = 1 ] || [ -f "$BIN" ] || die "no image $BIN" 2
[ -z "$LIST" ] || [ -f "$LIST" ] || die "no case list $LIST" 2
[ -z "$TIMES" ] || [ -f "$TIMES" ] || die "no time table $TIMES" 2

# ---- the case list ----
MUST=$HERE/gles2-mustpass.txt
if [ -z "$LIST" ]; then
    LIST=$HERE/mp2-deqp-gles2.txt
    [ "$EXCL_SET" = 1 ] || EXCL=$HERE/mp2-deqp-gles2-exclude.txt
    EXPAND=1
else
    EXPAND=0
fi
[ -z "$EXCL" ] || [ -f "$EXCL" ] || die "no exclude file $EXCL" 2
XF=${EXCL:-/dev/null}
expand_cases() {   # the case names to stdout
    if [ "$EXPAND" = 1 ]; then
        awk -v FS='\t' -v must="$MUST" -v list="$LIST" -v excl="$XF" '
        function pre(p) { sub(/\*$/, "", p); return p }
        function under(name, g) {
            if (g == name) return 1
            return substr(g, length(g)) == "*" && index(name, pre(g)) == 1
        }
        function covered(name, arr, n,   i) {
            for (i = 1; i <= n; i++) if (under(name, arr[i])) return 1
            return 0
        }
        BEGIN {
            while ((getline l < list) > 0) if (l !~ /^#/ && l != "") { nl++; ls[nl] = l }
            while ((getline l < excl) > 0) if (l !~ /^#/ && l != "") { split(l, f, "\t"); nx++; xs[nx] = f[1] }
            while ((getline l < must) > 0) {
                if (l ~ /^#/ || l == "") continue
                if (covered(l, ls, nl) && !covered(l, xs, nx)) print l
            }
        }'
    else
        awk -v FS='\t' -v excl="$XF" '
        BEGIN { while ((getline l < excl) > 0) if (l !~ /^#/ && l != "") { split(l, f, "\t"); x[f[1]] = 1 } }
        /^#/ || $0 == "" { next }
        !($1 in x) { print $1 }' "$LIST"
    fi
}

run() {   # run <cmd...>: print it, run it unless --dry-run
    printf '%s\n' "$*"
    [ "$DRY" = 1 ] && return 0
    "$@"
}

SH=$OUT/shards
CASES=$OUT/cases.txt
SARGS=(--out "$SH")
[ -z "$BUDGET" ] || SARGS+=(--budget-ms "$BUDGET")
[ -z "$TIMES" ] || SARGS+=(--times "$TIMES")
if [ "$DRY" = 1 ]; then
    n=$(expand_cases | wc -l)
    echo "mp2-deqp-sweep: dry run: $n cases from $LIST (exclude: $XF)"
    echo "mkdir -p $OUT"
    echo "expand $LIST minus $XF > $CASES"
    echo "$PY -I $HERE/mp2-deqp-shards.py $CASES ${SARGS[*]}"
    echo "per shard NNNN (from $FIRST to ${LAST:-the last}), unless $OUT/shard-NNNN.log already holds a 'deqp: <Status> <case>' line:"
    echo "$PY -I $HERE/mp2-deqp-slot.py write $BIN --case-list $SH/shard-NNNN.txt -o $OUT/slot-NNNN.bin"
    echo "$RUNNER run $OUT/slot-NNNN.bin $OUT/run/shard-NNNN shard-NNNN > $OUT/shard-NNNN.log 2>&1"
    echo "cat the console.txt files under $OUT/run/shard-NNNN >> $OUT/shard-NNNN.log"
    echo "$PY -I $HERE/mp2-p9-diff.py ${REPORT:-} $OUT/shard-*.log"
    exit 0
fi

mkdir -p "$OUT" || die "cannot make $OUT" 2
expand_cases > "$CASES" || die "cannot read the case list" 2
[ -s "$CASES" ] || die "the case list is empty" 2
run "$PY" -I "$HERE/mp2-deqp-shards.py" "$CASES" "${SARGS[@]}" || die "the shard split failed"
# manifest.tsv: header, then index<TAB>cases<TAB>est_ms<TAB>file
mapfile -t IDX < <(awk -F'\t' 'NR > 1 { print $1 }' "$SH/manifest.tsv")
[ ${#IDX[@]} -gt 0 ] || die "no shards in $SH/manifest.tsv"
fails=0 ran=0 skipped=0
for i in "${IDX[@]}"; do
    n=$((10#$i))
    [ "$n" -ge "$FIRST" ] || continue
    [ -z "$LAST" ] || [ "$n" -le "$LAST" ] || continue
    nnnn=$(printf '%04d' "$n")
    log=$OUT/shard-$nnnn.log
    if [ -f "$log" ] && grep -aqE '(^|[^A-Za-z])deqp: [A-Za-z]+ [^ ]' "$log"; then
        echo "mp2-deqp-sweep: shard $nnnn has a result in $log, skipped"
        skipped=$((skipped + 1)); continue
    fi
    bin=$OUT/slot-$nnnn.bin
    if ! run "$PY" -I "$HERE/mp2-deqp-slot.py" write "$BIN" --case-list "$SH/shard-$nnnn.txt" -o "$bin"; then
        echo "mp2-deqp-sweep: shard $nnnn: slot write failed" >&2; fails=$((fails + 1)); continue
    fi
    rd=$OUT/run/shard-$nnnn
    printf '%s run %s %s shard-%s\n' "$RUNNER" "$bin" "$rd" "$nnnn"
    # MP2_SWEEP_RUNNER may carry arguments, so it is split on purpose
    # shellcheck disable=SC2086
    $RUNNER run "$bin" "$rd" "shard-$nnnn" > "$log" 2>&1
    rc=$?
    if [ -d "$rd" ]; then
        find "$rd" -name console.txt -print0 | sort -z | xargs -0 -r cat >> "$log"
    fi
    ran=$((ran + 1))
    [ "$rc" -eq 0 ] || { echo "mp2-deqp-sweep: shard $nnnn: runner exit $rc" >&2; fails=$((fails + 1)); }
done
echo "mp2-deqp-sweep: $ran run, $skipped skipped, $fails failed"
shopt -s nullglob
LOGS=("$OUT"/shard-*.log)
[ ${#LOGS[@]} -gt 0 ] || die "no shard logs under $OUT"
"$PY" -I "$HERE/mp2-p9-diff.py" ${REPORT:+"$REPORT"} "${LOGS[@]}"
drc=$?
[ "$fails" -eq 0 ] || exit 1
exit "$drc"
