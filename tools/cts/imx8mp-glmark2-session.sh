#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-session.sh <NN|gmNN-label> [--dry-run]
#       [--record-dir DIR] [--ref-dir DIR] [--route ROUTE] [--main REF]
#
# One i.MX8MP (Verdin iMX8M Plus, GC7000UL) glmark2 scene from its frame dump
# to a checked verdict row of docs/notes/imx8mp-glmark2-pictures-by-eye.md.
# The steps run in this order and the first one that fails ends the run with a
# non-zero exit and one line "session: step <name> failed: <why>" on stderr;
# a step that passes prints "session: step <name>: ...".
#
#   resolve       the scene (its number 1-33 or its label, gm24-shadow) through
#                 `imx8mp-glmark2-dumps.sh scenes`.
#   run           unless --record-dir names an existing record of the scene:
#                 `imx8mp-glmark2-dumps.sh run <img-dir> <records-dir> <label>`
#                 (the board; BOARD_UART and the rest are read from the
#                 environment by the dumps script, nothing here takes, prints
#                 or stores them). The record is the directory of the scene
#                 that the run created; an older record of the same scene is
#                 never taken for it. With --dry-run the command is printed
#                 through the dumps script's own --dry-run and nothing runs:
#                 no board, no file.
#   decode        glmark2-fb.py on the record's console-live.txt (written from
#                 console.txt up to a "board: record" line when the record has
#                 none; console.txt is never changed) into <label>.png. Any
#                 damaged or missing row, or no dump at all: exit 3 with
#                 "re-run, never patch"; the PNG of a damaged dump is renamed
#                 <label>.damaged.png and nothing after this step runs.
#                 imx8mp-glmark2-uart-stats.py's line (timing, margin to the
#                 watchdog window) is printed when the tool is there; it is
#                 information, the step does not fail on it.
#   redaction     imx8mp-gles3-redact.py --check on the record: exit 0.
#   record-check  imx8mp-glmark2-record-check.py --scene <label> on the record
#                 (the one-record-per-scene layout): any problem line fails
#                 the step.
#   compare       glmark2-picdiff.py and glmark2-sidebyside.py against
#                 <ref-dir>/<slug>.png (the llvmpipe reference of
#                 glmark2-host-ref.sh) into the record: <label>.picdiff.png
#                 and <label>.sidebyside.png.
#   judge         imx8mp-glmark2-judge.py's proposed row and entry opening
#                 (when the tool is there). The verdict is never decided or
#                 written here: the imx8mp-glmark2-note-fill.py commands are
#                 printed for the person who has looked at the side-by-side
#                 picture, with the commit of meta.txt's commit= line and
#                 --check-main --main <ref>. Route dump, both or
#                 photograph+probes: one command for right, one for wrong;
#                 route photograph: the one for `seen without a value`
#                 (note-fill.py's and imx8mp-glmark2-verdicts.py's rules).
#
# --record-dir   an existing record directory <YYYYMMDD-HHMM>-<label>-<sha8>
#                of this scene: the run step is skipped (also the way to redo
#                the checks of a record, with or without --dry-run).
# --ref-dir      the reference directory; default the newest
#                records/host/*-gmref-* of this checkout.
#
# --route        dump (default), photograph, both or photograph+probes: the
#                route cell of the printed note-fill commands.
# --main         the ref the printed commands check the commit against
#                (--check-main --main REF); default $IMX8MP_MAIN, else
#                origin/main. Also the judge's --main when given.
#
# Environment: IMX8MP_IMG_DIR (the images of `imx8mp-glmark2-dumps.sh build`,
# needed by the run step), IMX8MP_RECORDS (the records directory, default
# records/imx8mp of this checkout), IMX8MP_MAIN (the judge's --main, default
# its origin/main), IMX8MP_SESSION_DUMPS (another dumps script: the self-test's
# stand-in board). Exit: 0 every step passed, 1 a step failed, 2 bad argument,
# unknown scene or a record of another scene, 3 damaged or missing dump (re-run
# the scene). Self-test: tools/cts/imx8mp-glmark2-session-selftest.sh.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
DUMPS=${IMX8MP_SESSION_DUMPS:-$HERE/imx8mp-glmark2-dumps.sh}
NOTE=docs/notes/imx8mp-glmark2-pictures-by-eye.md

usage() {
    echo "usage: tools/cts/imx8mp-glmark2-session.sh <NN|gmNN-label> [--dry-run] [--record-dir DIR] [--ref-dir DIR] [--route ROUTE] [--main REF]" >&2
    exit 2
}
# step_fail NAME WHY [EXIT]
step_fail() {
    echo "session: step $1 failed: $2" >&2
    exit "${3:-1}"
}
# dirs_of DIR NAME: the directories DIR/*-NAME-*, one per line (the
# names start with the date, so the order is by time)
dirs_of() {
    local d
    for d in "$1"/*-"$2"-*/; do
        [ -d "$d" ] && printf '%s\n' "${d%/}"
    done
    return 0
}

DRY=0
RECDIR=
REFDIR=
ROUTE=dump
MAINOPT=
SCENE=
while [ $# -gt 0 ]; do
    case $1 in
    --dry-run) DRY=1 ;;
    --record-dir)
        [ $# -ge 2 ] || usage
        RECDIR=$2
        shift
        ;;
    --ref-dir)
        [ $# -ge 2 ] || usage
        REFDIR=$2
        shift
        ;;
    --route)
        [ $# -ge 2 ] || usage
        ROUTE=$2
        shift
        ;;
    --main)
        [ $# -ge 2 ] || usage
        MAINOPT=$2
        shift
        ;;
    -*) usage ;;
    *)
        [ -z "$SCENE" ] || usage
        SCENE=$1
        ;;
    esac
    shift
done
[ -n "$SCENE" ] || usage
case $ROUTE in
dump | photograph | both | photograph+probes) ;;
*) step_fail resolve "route '$ROUTE' is not one of dump, photograph, both, photograph+probes" 2 ;;
esac

# resolve: "<label> <scene info> -b <glmark2 argument>" per scene
scenes=$(bash "$DUMPS" scenes) || step_fail resolve "imx8mp-glmark2-dumps.sh scenes failed"
line=
if [[ $SCENE =~ ^[0-9]+$ ]]; then
    n=$((10#$SCENE))
    if [ "$n" -ge 1 ]; then line=$(printf '%s\n' "$scenes" | sed -n "${n}p"); fi
else
    line=$(printf '%s\n' "$scenes" | awk -v s="$SCENE" '$1 == s { print; exit }')
fi
[ -n "$line" ] || step_fail resolve "no scene '$SCENE' (imx8mp-glmark2-dumps.sh scenes lists the 33)" 2
LABEL=${line%% *}
rest=${line#* }
INFO=${rest% -b *}
ARG=${rest##* -b }
SCENE_NAME=${ARG%%:*}
[[ $LABEL =~ ^gm([0-9][0-9])- ]] || step_fail resolve "label '$LABEL' is not gmNN-<name>"
NN=${BASH_REMATCH[1]}
# the slug rule of glmark2-host-ref.sh and glmark2-fb.py
SLUG=$(printf '%s' "$INFO" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' | cut -c1-48)
echo "session: step resolve: $LABEL, $INFO (-b $ARG)"

# run
RECORDS=${IMX8MP_RECORDS:-$REPO/records/imx8mp}
runrc=0
if [ -n "$RECDIR" ]; then
    [ -d "$RECDIR" ] || step_fail run "--record-dir is not an existing directory" 2
    RECDIR=$(cd "$RECDIR" && pwd)
    [[ $(basename "$RECDIR") =~ ^[0-9]{8}-[0-9]{4}-"$LABEL"-[0-9a-f]{8}$ ]] ||
        step_fail run "--record-dir is not a record of $LABEL (<YYYYMMDD-HHMM>-$LABEL-<sha8>)" 2
    echo "session: step run: skipped, the record exists ($(basename "$RECDIR"))"
elif [ "$DRY" = 1 ]; then
    echo "session: step run: dry run, nothing runs and no board is touched; the command would be"
    bash "$DUMPS" --dry-run run "${IMX8MP_IMG_DIR:-<img-dir>}" "$RECORDS" "$LABEL" ||
        step_fail run "imx8mp-glmark2-dumps.sh --dry-run run failed"
    echo "session: dry run: decode, redaction, record-check, compare and judge need the record (--record-dir)"
    exit 0
else
    [ -n "${IMX8MP_IMG_DIR:-}" ] ||
        step_fail run "IMX8MP_IMG_DIR is not set (the images of imx8mp-glmark2-dumps.sh build)" 2
    before=$(dirs_of "$RECORDS" "$LABEL")
    bash "$DUMPS" run "$IMX8MP_IMG_DIR" "$RECORDS" "$LABEL" || runrc=$?
    while IFS= read -r d; do
        [ -n "$d" ] || continue
        printf '%s\n' "$before" | grep -Fxq -- "$d" || RECDIR=$d
    done <<< "$(dirs_of "$RECORDS" "$LABEL")"
    [ -n "$RECDIR" ] || step_fail run "the run left no new record of $LABEL (imx8mp-glmark2-dumps.sh run exited $runrc)"
    echo "session: step run: record $(basename "$RECDIR") (imx8mp-glmark2-dumps.sh run exited $runrc)"
fi

# decode
live=$RECDIR/console-live.txt
if [ ! -s "$live" ]; then
    [ -s "$RECDIR/console.txt" ] || step_fail decode "no console.txt in the record"
    awk '/board: record/ { exit } { print }' "$RECDIR/console.txt" > "$live" ||
        step_fail decode "console-live.txt could not be written"
fi
png=$RECDIR/$LABEL.png
decrc=0
decout=$(python3 -I "$HERE/glmark2-fb.py" "$live" "$png" --scene "$SCENE_NAME" 2>&1) || decrc=$?
if [ "$decrc" != 0 ]; then
    printf '%s\n' "$decout" | sed -n '1,60p' >&2
    if [ -e "$png" ]; then mv -f "$png" "$RECDIR/$LABEL.damaged.png"; fi
    if [ "$decrc" = 1 ]; then
        step_fail decode "damaged or missing rows in the dump of $LABEL: re-run, never patch" 3
    fi
    step_fail decode "no dump of $LABEL in the console (glmark2-fb.py exited $decrc): re-run, never patch" 3
fi
printf '%s\n' "$decout"
if printf '%s\n' "$decout" | grep -q "picture value not compared"; then
    if [ -e "$png" ]; then mv -f "$png" "$RECDIR/$LABEL.damaged.png"; fi
    step_fail decode "picture line missing: re-run the scene (the dump of $LABEL has no glmark2-picture line of its own; never patch)" 3
fi
[ "$runrc" = 0 ] || step_fail run "imx8mp-glmark2-dumps.sh run exited $runrc though the dump decodes"
echo "session: step decode: ok ($LABEL.png)"
if [ -f "$HERE/imx8mp-glmark2-uart-stats.py" ]; then
    statsrc=0
    python3 -I "$HERE/imx8mp-glmark2-uart-stats.py" "$RECDIR" || statsrc=$?
    echo "session: uart-stats exited $statsrc (information: 1 is a dump over the watchdog window, 2 a capture without what it reads)"
fi

# redaction
python3 -I "$HERE/imx8mp-gles3-redact.py" --check "$RECDIR" ||
    step_fail redaction "an address, a login or a serial path in the record (the lines above name file and class, never the text)"
echo "session: step redaction: ok"

# record-check
rcrc=0
rcout=$(python3 -I "$HERE/imx8mp-glmark2-record-check.py" --scene "$LABEL" "$RECDIR") || rcrc=$?
[ "$rcrc" -le 1 ] || step_fail record-check "imx8mp-glmark2-record-check.py exited $rcrc"
rcbad=$(printf '%s\n' "$rcout" | awk '/^record-check: / { next } NF { print }')
if [ -n "$rcbad" ]; then
    printf '%s\n' "$rcbad" >&2
    step_fail record-check "the record has the problems above"
fi
echo "session: step record-check: ok (--scene $LABEL)"

# compare
if [ -z "$REFDIR" ]; then
    REFDIR=$(dirs_of "$REPO/records/host" gmref | tail -n 1)
fi
[ -n "$REFDIR" ] || step_fail compare "no reference directory (--ref-dir, or records/host/*-gmref-* of glmark2-host-ref.sh)"
[ -s "$REFDIR/$SLUG.png" ] || step_fail compare "no reference picture $SLUG.png in the reference directory"
python3 -I "$HERE/glmark2-picdiff.py" "$png" "$REFDIR/$SLUG.png" "$RECDIR/$LABEL.picdiff.png" ||
    step_fail compare "glmark2-picdiff.py failed"
python3 -I "$HERE/glmark2-sidebyside.py" "$png" "$REFDIR/$SLUG.png" "$RECDIR/$LABEL.sidebyside.png" \
    --label-board "IMX8MP $LABEL" --label-ref LLVMPIPE > /dev/null ||
    step_fail compare "glmark2-sidebyside.py failed"
echo "session: step compare: ok ($LABEL.picdiff.png, $LABEL.sidebyside.png)"

# judge
rbase=$(basename "$RECDIR")
relrec=records/imx8mp/$rbase/
if [ -f "$HERE/imx8mp-glmark2-judge.py" ]; then
    main=()
    [ -z "${MAINOPT:-${IMX8MP_MAIN:-}}" ] || main=(--main "${MAINOPT:-$IMX8MP_MAIN}")
    python3 -I "$HERE/imx8mp-glmark2-judge.py" "$RECDIR" "$REFDIR" --out "$RECDIR" "${main[@]+"${main[@]}"}" ||
        step_fail judge "imx8mp-glmark2-judge.py failed"
    echo "session: step judge: ok (the row above is a proposal, its verdict is the placeholder)"
else
    echo "session: step judge: no imx8mp-glmark2-judge.py; scene '$INFO', record $relrec, route dump"
fi
commit=$(sed -n 's/^commit=\([0-9a-f]\{40\}\)$/\1/p' "$RECDIR/meta.txt" 2> /dev/null | tail -n 1)
echo "session: look at $LABEL.sidebyside.png in the record; then, by what was seen (not run here), one of:"
mainref=${MAINOPT:-${IMX8MP_MAIN:-origin/main}}
if [ "$ROUTE" = photograph ]; then verdicts=("seen without a value"); else verdicts=(right wrong); fi
for v in "${verdicts[@]}"; do
    q=$v
    [[ $v != *" "* ]] || q="'$v'"
    echo "  python3 tools/cts/imx8mp-glmark2-note-fill.py --note $NOTE --scene $LABEL --commit ${commit:-<the commit the image was built from>} --record $relrec --route $ROUTE --verdict $q --check-main --main $mainref"
done
exit 0
