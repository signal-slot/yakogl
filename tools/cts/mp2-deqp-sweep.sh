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
# mp2-deqp-shards.py into OUT/shards/ (--max-bytes: the image's slot room); (2) for each shard (--first/--last
# pick a range of shard numbers, to resume or split a sweep)
# mp2-deqp-slot.py writes the shard's case list into a copy of <zephyr.bin>,
# OUT/slot-NNNN.bin, and the runner runs it:
# RUNNER run <bin> OUT/run/shard-NNNN shard-NNNN; (3) the shard's log
# OUT/shard-NNNN.log is the runner's own output plus every console.txt it left
# under OUT/run/shard-NNNN; a shard whose log already holds a
# "deqp: <Status> <case>" result line for every case of the shard is skipped (a
# log that stops part way, as a lifetime end leaves one, is run again); (4) mp2-p9-diff.py reads all
# logs, given --cases OUT/cases.txt (only those cases can be missing) and --exclude of the exclude file in use, so the names pending Q11
# are "excluded", not "missing" (its exit 1, new failures or missing cases, is
# this script's); (5) mp2-log-redact-check.sh OUT: a hit (or an unreadable
# input) makes the sweep exit 1, the logs must be redacted before commit.
#
# Env: MP2_SWEEP_RUNNER the runner command (default tools/board-stm32mp2.sh),
# MP2_DEQP_EXCLUDE the exclude file when --exclude is not given (default
# mp2-deqp-gles2-exclude.txt with the default case list; none with --case-list),
# PYTHON (python3), RUN_BOUND (the runner's wait for the image's end; left
# unset it becomes the dEQP lifetime, CONFIG_YAKOGL_STM32MP2_LIFETIME_S of
# boards/stm32mp257f_dk_stm32mp257fxx_a35.conf, plus 100 s, and is printed:
# the runner's own default of 120 s would end a 600 s run as not_back).
# The runner's exit 5 (the board did not come back) and 7 (needs a person)
# and SIGINT/SIGTERM stop the sweep at once: the runner's undo lines are
# printed from the shard log and the sweep says the board needs a person. It
# never retries a shard, resets or disarms the board itself, and runs no diff.
# --dry-run prints every command and runs none: the shard files do not exist
# then, so the per-shard commands are printed once with NNNN for the number.
# Exit: 0 done, 1 the diff or a step failed, 2 usage, 3 stopped on runner exit
# 5 or 7 (the board needs a person), 130/143 stopped by SIGINT/SIGTERM.
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

# ---- RUN_BOUND: the dEQP lifetime + 100 s unless the caller set it ----
if [ -z "${RUN_BOUND:-}" ]; then
    DKCONF=$HERE/../../driver/samples/zephyr/deqp/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf
    LIFE=$(sed -n 's/^CONFIG_YAKOGL_STM32MP2_LIFETIME_S=\([0-9][0-9]*\)[[:space:]]*$/\1/p' "$DKCONF" 2>/dev/null | tail -n 1)
    [ -n "$LIFE" ] || die "RUN_BOUND is unset and CONFIG_YAKOGL_STM32MP2_LIFETIME_S is not in $DKCONF" 2
    RUN_BOUND=$((LIFE + 100))
    export RUN_BOUND
    echo "mp2-deqp-sweep: RUN_BOUND=$RUN_BOUND (lifetime $LIFE s + 100 s; RUN_BOUND unset)"
else
    echo "mp2-deqp-sweep: RUN_BOUND=$RUN_BOUND (from the caller)"
fi

# ---- the case list ----
MUST=$HERE/gles2-mustpass.txt
if [ -z "$LIST" ]; then
    LIST=$HERE/mp2-deqp-gles2.txt
    [ "$EXCL_SET" = 1 ] || EXCL=${MP2_DEQP_EXCLUDE:-$HERE/mp2-deqp-gles2-exclude.txt}
    EXPAND=1
else
    [ "$EXCL_SET" = 1 ] || EXCL=${MP2_DEQP_EXCLUDE:-}
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
# a shard's filter must fit the image's slot: --max-bytes is the slot's room
ROOM=
[ -f "$BIN" ] && ROOM=$("$PY" -I "$HERE/mp2-deqp-slot.py" show "$BIN" 2>/dev/null | sed -n 's/^mp2-deqp-slot: slot at [^,]*, room \([0-9][0-9]*\) bytes.*/\1/p' | head -n 1)
[ -z "$ROOM" ] || SARGS+=(--max-bytes "$ROOM")
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
    echo "$PY -I $HERE/mp2-p9-diff.py ${REPORT:-} --cases $CASES ${EXCL:+--exclude $EXCL }$OUT/shard-*.log"
    echo "bash $HERE/mp2-log-redact-check.sh $OUT   (a hit: exit 1)"
    exit 0
fi

mkdir -p "$OUT" || die "cannot make $OUT" 2
expand_cases > "$CASES" || die "cannot read the case list" 2
[ -s "$CASES" ] || die "the case list is empty" 2
run "$PY" -I "$HERE/mp2-deqp-shards.py" "$CASES" "${SARGS[@]}" || die "the shard split failed"
# manifest.tsv: header, then index<TAB>cases<TAB>est_ms<TAB>file
mapfile -t IDX < <(awk -F'\t' 'NR > 1 { print $1 }' "$SH/manifest.tsv")
[ ${#IDX[@]} -gt 0 ] || die "no shards in $SH/manifest.tsv"
fails=0 ran=0 skipped=0 lifetimes=0
RPID= SIG= SIGRC= LOG=
undo_lines() {   # what the runner printed about its undo and the stop's cause
    [ -f "$LOG" ] || return 0
    grep -aiE 'undo|NEEDS A PERSON|left behind|disarm|^ +tools/board-stm32mp2' "$LOG" | tail -n 40 | sed 's/^/  runner: /'
}
stop_person() {   # stop_person <why> <exit>: the board needs a person
    echo "mp2-deqp-sweep: STOPPED: $1" >&2
    undo_lines >&2
    echo "mp2-deqp-sweep: the board needs a person: nothing was retried, reset or disarmed by this sweep; read the shard log $LOG and run the undo lines above by hand. No diff was run." >&2
    exit "$2"
}
on_sig() {   # on_sig <INT|TERM> <exit>: pass it to the runner (its own undo runs), then stop
    SIG=$1; SIGRC=$2
    if [ -z "$RPID" ]; then
        echo "mp2-deqp-sweep: SIG$1: stopped, no runner was running" >&2; exit "$2"
    fi
    kill -s "$1" "$RPID" 2>/dev/null
}
trap 'on_sig INT 130' INT
trap 'on_sig TERM 143' TERM
shard_done() {   # shard_done <log> <shard file>: the log has a "deqp: <Status> <case>" line for every case
    [ -f "$1" ] || return 1
    awk 'NR == FNR { if (match($0, /(^|[^A-Za-z])deqp: [A-Za-z]+ [^ \r]+/)) { t = substr($0, RSTART, RLENGTH); n = split(t, f, " "); got[f[n]] = 1 } next }
         $0 != "" { n++; if (!($0 in got)) bad = 1 }
         END { exit (bad || !n) }' "$1" "$2"
}
for i in "${IDX[@]}"; do
    n=$((10#$i))
    [ "$n" -ge "$FIRST" ] || continue
    [ -z "$LAST" ] || [ "$n" -le "$LAST" ] || continue
    nnnn=$(printf '%04d' "$n")
    log=$OUT/shard-$nnnn.log
    if shard_done "$log" "$SH/shard-$nnnn.txt"; then
        echo "mp2-deqp-sweep: shard $nnnn has a result for every case in $log, skipped"
        skipped=$((skipped + 1)); continue
    fi
    bin=$OUT/slot-$nnnn.bin
    if ! run "$PY" -I "$HERE/mp2-deqp-slot.py" write "$BIN" --case-list "$SH/shard-$nnnn.txt" -o "$bin"; then
        echo "mp2-deqp-sweep: shard $nnnn: slot write failed" >&2; fails=$((fails + 1)); continue
    fi
    rd=$OUT/run/shard-$nnnn
    LOG=$log
    printf '%s run %s %s shard-%s\n' "$RUNNER" "$bin" "$rd" "$nnnn"
    # MP2_SWEEP_RUNNER may carry arguments, so it is split on purpose
    # shellcheck disable=SC2086
    # in the background under job control (own process group, default signals)
    # so that a signal reaches this script's trap while the runner runs
    set -m
    $RUNNER run "$bin" "$rd" "shard-$nnnn" > "$log" 2>&1 &
    RPID=$!
    set +m
    wait "$RPID"; rc=$?
    if [ -n "$SIG" ]; then wait "$RPID"; rc=$?; fi
    RPID=
    if [ -d "$rd" ]; then
        find "$rd" -name console.txt -print0 | sort -z | xargs -0 -r cat >> "$log"
    fi
    ran=$((ran + 1))
    if [ -n "$SIG" ]; then
        stop_person "SIG$SIG during shard $nnnn (runner exit $rc)" "$SIGRC"
    fi
    # every code the runner returns (tools/board-stm32mp2.sh, "Exit codes"):
    case $rc in
    0) ;;
    4) echo "mp2-deqp-sweep: shard $nnnn: runner exit 4 (the verdict is not done: failed, no end line or bad record)" >&2; fails=$((fails + 1)) ;;
    8) echo "mp2-deqp-sweep: shard $nnnn: runner exit 8 (verdict=lifetime: the run hit the lifetime; recorded, the diff counts its cases missing)" >&2; lifetimes=$((lifetimes + 1)) ;;
    3) echo "mp2-deqp-sweep: shard $nnnn: runner exit 3 (refused before the board was changed)" >&2; fails=$((fails + 1)) ;;
    6) echo "mp2-deqp-sweep: shard $nnnn: runner exit 6 (the one-shot was still set after the cycle; the runner cleared it)" >&2; fails=$((fails + 1)) ;;
    2) stop_person "shard $nnnn: runner exit 2 (usage: the sweep called the runner wrongly)" 2 ;;
    5) stop_person "shard $nnnn: runner exit 5 (the board did not come back)" 3 ;;
    7) stop_person "shard $nnnn: runner exit 7 (needs a person: stock boot files or the environment)" 3 ;;
    129|130|143) stop_person "shard $nnnn: runner exit $rc (ended by a signal; its state file says what is left)" 3 ;;
    *) stop_person "shard $nnnn: runner exit $rc is not a code the runner documents; stopping rather than going on" 3 ;;
    esac
done
echo "mp2-deqp-sweep: $ran run, $skipped skipped, $fails failed, $lifetimes lifetime"
shopt -s nullglob
LOGS=("$OUT"/shard-*.log)
[ ${#LOGS[@]} -gt 0 ] || die "no shard logs under $OUT"
"$PY" -I "$HERE/mp2-p9-diff.py" ${REPORT:+"$REPORT"} --cases "$CASES" ${EXCL:+--exclude "$EXCL"} "${LOGS[@]}"
drc=$?
bash "$HERE/mp2-log-redact-check.sh" "$OUT"
rrc=$?
if [ "$rrc" -ne 0 ]; then
    echo "mp2-deqp-sweep: the redaction check failed (exit $rrc): the logs under $OUT must be redacted before commit" >&2
    exit 1
fi
[ "$fails" -eq 0 ] || exit 1
exit "$drc"
